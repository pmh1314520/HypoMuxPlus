//! 全局接管（TUN）模式
//!
//! 通过 WireGuard 的 wintun 虚拟网卡在三层截获**全部**系统流量，用 `ipstack`
//! 用户态 TCP/IP 栈把每条连接还原成 TCP/UDP 流，再转交给本地 SOCKS5 引擎
//! （`engine.rs`），从而复用现有的多网卡逐卡绑定与聚合调度——用户无需再逐个
//! 程序手动配置代理即可享受多网卡加速。
//!
//! 架构：
//! ```text
//!   全系统流量 → wintun(默认路由) → ipstack 用户态栈 → 每条流
//!                                       ├─ TCP  → 本地 SOCKS5(127.0.0.1) → 多网卡出网
//!                                       └─ UDP:53 → fake-ip DNS 应答（本模块自答）
//! ```
//!
//! DNS 采用 fake-ip：对 A 查询返回 198.18.0.0/15 段内的占位 IP，并记录
//! `fakeip → 域名` 映射；当应用随后 TCP 连接该 fakeip 时，本模块用域名（SOCKS5
//! 域名地址类型）交给引擎，由引擎经物理网卡做真实 DNS 解析后逐卡直连，避免多网卡
//! 塌缩为单一上游。
//!
//! 依赖：运行需管理员权限（创建虚拟网卡 / 改路由），且 `wintun.dll`（x64）须位于
//! 程序目录。仅 Windows。

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use socket2::{Domain, Protocol, Socket, Type};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};
use tokio_util::sync::CancellationToken;

use ipstack::{IpStack, IpStackConfig, IpStackStream};

use crate::engine::{Engine, Family, NicRuntime};

#[cfg(windows)]
use std::os::windows::io::AsRawSocket;

/// IPv4 下强制指定物理网卡出口（与 `engine.rs` 同值，UDP 出口绑定复用）。
const IP_UNICAST_IF: i32 = 31;
const IPPROTO_IP: i32 = 0;
/// IPv6 下强制指定物理网卡出口（数值同为 31，但 level 不同）。
const IPV6_UNICAST_IF: i32 = 31;
const IPPROTO_IPV6: i32 = 41;

/// UDP 中继会话空闲回收阈值：超过该时长无数据往返即释放（Req 3.3）。
const UDP_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
/// 空闲会话回收后台任务的巡检间隔。
const UDP_REAP_INTERVAL: Duration = Duration::from_secs(5);
/// 单个 UDP 数据报中继缓冲上限（QUIC/HTTP3 数据报通常 < 1500，留足余量）。
const UDP_BUF: usize = 65_535;

/// 虚拟网卡友好名（用于 netsh / route 命令定位接口）
const TUN_NAME: &str = "HypoMuxPlus";
/// 虚拟网卡自身地址（同时作为路由下一跳）
const TUN_IP: Ipv4Addr = Ipv4Addr::new(198, 18, 0, 1);
/// 掩码 255.254.0.0 = /15，覆盖 198.18.0.0 ~ 198.19.255.255
const TUN_NETMASK: Ipv4Addr = Ipv4Addr::new(255, 254, 0, 0);
/// fake-ip DNS 服务地址（劫持此地址的 53 端口查询）
const FAKE_DNS_IP: Ipv4Addr = Ipv4Addr::new(198, 18, 0, 2);
/// 链路 MTU
const MTU: u16 = 1500;
/// 固定 GUID，保证适配器身份稳定（避免每次新建残留旧适配器）
const TUN_GUID: u128 = 0x4859_504F_4D55_5850_4C55_5300_0000_0001;
/// 隐藏子进程控制台窗口
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// fake-ip 分配器：域名 ↔ 占位 IP 双向映射。
struct FakeDns {
    inner: Mutex<FakeDnsInner>,
}

struct FakeDnsInner {
    to_domain: HashMap<Ipv4Addr, String>,
    to_ip: HashMap<String, Ipv4Addr>,
    /// 下一个可分配的主机序号（相对 198.18.0.0 的偏移，从 0x10 起，跳过 .1 网关与 .2 DNS）
    next: u32,
}

impl FakeDns {
    fn new() -> Self {
        FakeDns {
            inner: Mutex::new(FakeDnsInner {
                to_domain: HashMap::new(),
                to_ip: HashMap::new(),
                next: 0x10,
            }),
        }
    }

    /// 判断是否为 fake-ip（198.18.0.0/15）。
    fn is_fake(ip: Ipv4Addr) -> bool {
        let o = ip.octets();
        o[0] == 198 && (o[1] == 18 || o[1] == 19)
    }

    /// 为域名分配（或复用）一个 fake-ip。
    fn allocate(&self, domain: &str) -> Ipv4Addr {
        let mut g = self.inner.lock();
        if let Some(ip) = g.to_ip.get(domain) {
            return *ip;
        }
        // /15 内可用主机数约 13.1 万；回绕复用最早的槽位以防耗尽
        let span: u32 = 0x0002_0000; // 2^17
        if g.next >= span - 2 {
            g.next = 0x10;
        }
        let offset = g.next;
        g.next += 1;
        let base: u32 = u32::from(TUN_IP) & 0xFFFE_0000; // 198.18.0.0
        let ip = Ipv4Addr::from(base + offset);
        g.to_domain.insert(ip, domain.to_string());
        g.to_ip.insert(domain.to_string(), ip);
        ip
    }

    /// 反查 fake-ip 对应域名。
    fn lookup(&self, ip: Ipv4Addr) -> Option<String> {
        self.inner.lock().to_domain.get(&ip).cloned()
    }
}

/// UDP 中继会话键：`(客户端源端点, 真实目标端点)`，唯一标识一条 UDP 会话。
pub(crate) type UdpKey = (SocketAddr, SocketAddr);

/// 一条 UDP 中继会话：持有经所选网卡 Egress_Binding 的上游 socket、
/// 最近活跃时间、所用网卡名与用于空闲回收的取消令牌。
struct UdpSession {
    /// 经所选网卡绑定的上游 UDP socket。
    #[allow(dead_code)]
    upstream: Arc<UdpSocket>,
    /// 最近一次上下行数据往返的时刻，用于空闲超时判定。
    last_active: Instant,
    /// 该会话所用出口网卡名（用于日志/出口追踪）。
    #[allow(dead_code)]
    nic_name: String,
    /// 取消令牌：空闲回收时触发，令该会话的中继任务结束并释放 socket。
    cancel: CancellationToken,
}

/// UDP 会话表：`UdpKey → UdpSession` 的并发映射。
struct UdpSessionTable {
    inner: Mutex<HashMap<UdpKey, UdpSession>>,
}

impl UdpSessionTable {
    /// 创建空会话表。
    fn new() -> Self {
        UdpSessionTable {
            inner: Mutex::new(HashMap::new()),
        }
    }

    /// 登记一条会话；若同 key 已存在旧会话，先取消旧会话再替换（复用键位）。
    fn insert(&self, key: UdpKey, session: UdpSession) {
        let mut g = self.inner.lock();
        if let Some(old) = g.insert(key, session) {
            old.cancel.cancel();
        }
    }

    /// 刷新某会话的最近活跃时刻（每次上下行数据往返调用）。
    fn touch(&self, key: &UdpKey, now: Instant) {
        if let Some(s) = self.inner.lock().get_mut(key) {
            s.last_active = now;
        }
    }

    /// 移除并返回某会话（中继任务自然结束时清理映射）。
    fn remove(&self, key: &UdpKey) -> Option<UdpSession> {
        self.inner.lock().remove(key)
    }

    /// 快照当前所有会话的 `(key, last_active)`，供纯函数 `expired_udp_keys` 计算超时集合。
    fn snapshot(&self) -> Vec<(UdpKey, Instant)> {
        self.inner
            .lock()
            .iter()
            .map(|(k, s)| (*k, s.last_active))
            .collect()
    }

    /// 回收所有空闲超过 `idle` 的会话：取消其中继任务并移出映射（drop 上游 socket）。
    fn reap(&self, now: Instant, idle: Duration) {
        let expired = expired_udp_keys(&self.snapshot(), now, idle);
        if expired.is_empty() {
            return;
        }
        let mut g = self.inner.lock();
        for key in expired {
            if let Some(s) = g.remove(&key) {
                s.cancel.cancel();
            }
        }
    }
}

/// 空闲会话筛选（纯函数）：给定各会话的 `(key, last_active)` 快照、当前时刻 `now`
/// 与空闲阈值 `idle`，返回所有满足 `now - last_active > idle`（严格大于）的 key。
///
/// 使用饱和时间差以避免 `last_active` 处于未来（`now < last_active`）时 panic：
/// 此情形下时间差为 0，绝不会超过任何非负 `idle`，因此判为未超时。
pub(crate) fn expired_udp_keys(
    entries: &[(UdpKey, Instant)],
    now: Instant,
    idle: Duration,
) -> Vec<UdpKey> {
    entries
        .iter()
        .filter(|(_, last_active)| now.saturating_duration_since(*last_active) > idle)
        .map(|(key, _)| *key)
        .collect()
}

/// 解析 DNS 查询，取出事务 ID、首个问题域名与查询类型（qtype，A=1、AAAA=28）。
fn parse_dns_question(buf: &[u8]) -> Option<(u16, String, u16)> {
    if buf.len() < 12 {
        return None;
    }
    let id = u16::from_be_bytes([buf[0], buf[1]]);
    let qd = u16::from_be_bytes([buf[4], buf[5]]);
    if qd == 0 {
        return None;
    }
    let mut pos = 12usize;
    let mut labels: Vec<String> = Vec::new();
    loop {
        let len = *buf.get(pos)? as usize;
        if len == 0 {
            pos += 1;
            break;
        }
        if len & 0xC0 == 0xC0 {
            // 查询段一般不含压缩指针，遇到则视为异常
            return None;
        }
        pos += 1;
        let end = pos + len;
        let label = std::str::from_utf8(buf.get(pos..end)?).ok()?;
        labels.push(label.to_string());
        pos = end;
    }
    let qtype = u16::from_be_bytes([*buf.get(pos)?, *buf.get(pos + 1)?]);
    let domain = labels.join(".");
    Some((id, domain, qtype))
}

/// 依据原始查询构造 DNS 应答：`is_a && Some(ip)` 时回一条 A 记录，否则回空答案。
fn build_dns_response(query: &[u8], answer: Option<Ipv4Addr>) -> Option<Vec<u8>> {
    if query.len() < 12 {
        return None;
    }
    // 问题段结束位置
    let mut pos = 12usize;
    loop {
        let len = *query.get(pos)? as usize;
        if len == 0 {
            pos += 1;
            break;
        }
        if len & 0xC0 == 0xC0 {
            return None;
        }
        pos += 1 + len;
    }
    pos += 4; // QTYPE + QCLASS
    if pos > query.len() {
        return None;
    }

    let mut resp = Vec::with_capacity(pos + 16);
    resp.extend_from_slice(&query[0..2]); // 事务 ID
    // 标志：QR=1 响应, RD 沿用请求, RA=1 支持递归
    let rd = query[2] & 0x01;
    resp.push(0x80 | rd);
    resp.push(0x80);
    resp.extend_from_slice(&query[4..6]); // QDCOUNT 沿用
    let ancount: u16 = if answer.is_some() { 1 } else { 0 };
    resp.extend_from_slice(&ancount.to_be_bytes()); // ANCOUNT
    resp.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // NSCOUNT + ARCOUNT
    resp.extend_from_slice(&query[12..pos]); // 原样拷贝问题段

    if let Some(ip) = answer {
        resp.extend_from_slice(&[0xC0, 0x0C]); // 指向问题段域名的压缩指针
        resp.extend_from_slice(&[0x00, 0x01]); // TYPE A
        resp.extend_from_slice(&[0x00, 0x01]); // CLASS IN
        resp.extend_from_slice(&[0x00, 0x00, 0x00, 0x3C]); // TTL 60s
        resp.extend_from_slice(&[0x00, 0x04]); // RDLENGTH
        resp.extend_from_slice(&ip.octets());
    }
    Some(resp)
}

/// SOCKS5 目标地址：真实 IP 或域名（fake-ip 反查后走域名，交由引擎逐卡解析）。
enum Target {
    Ip(Ipv4Addr),
    Domain(String),
}

/// 与本地 SOCKS5 引擎建立一条到 target 的隧道连接（完成 CONNECT 握手后返回）。
async fn socks_connect(port: u16, target: &Target, dport: u16) -> std::io::Result<TcpStream> {
    let mut s = TcpStream::connect(("127.0.0.1", port)).await?;
    let _ = s.set_nodelay(true);

    // 握手：无认证
    s.write_all(&[0x05, 0x01, 0x00]).await?;
    let mut sel = [0u8; 2];
    s.read_exact(&mut sel).await?;
    if sel[0] != 0x05 || sel[1] != 0x00 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "SOCKS5 握手失败",
        ));
    }

    // CONNECT 请求
    let mut req: Vec<u8> = vec![0x05, 0x01, 0x00];
    match target {
        Target::Ip(ip) => {
            req.push(0x01);
            req.extend_from_slice(&ip.octets());
        }
        Target::Domain(d) => {
            let b = d.as_bytes();
            if b.is_empty() || b.len() > 255 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "域名长度非法",
                ));
            }
            req.push(0x03);
            req.push(b.len() as u8);
            req.extend_from_slice(b);
        }
    }
    req.extend_from_slice(&dport.to_be_bytes());
    s.write_all(&req).await?;

    // 应答：VER REP RSV ATYP + BND.ADDR + BND.PORT
    let mut head = [0u8; 4];
    s.read_exact(&mut head).await?;
    if head[1] != 0x00 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            format!("SOCKS5 CONNECT 被拒绝 (REP={})", head[1]),
        ));
    }
    let addr_len = match head[3] {
        0x01 => 4,
        0x04 => 16,
        0x03 => {
            let mut l = [0u8; 1];
            s.read_exact(&mut l).await?;
            l[0] as usize
        }
        _ => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "未知的 SOCKS5 地址类型",
            ))
        }
    };
    let mut waste = vec![0u8; addr_len + 2]; // 绑定地址 + 端口，丢弃
    s.read_exact(&mut waste).await?;
    Ok(s)
}

/// 经指定网卡创建一个出站 UDP socket 并绑定其源地址（Egress_Binding）。
///
/// 与 `engine::connect_via_nic` 的 socket 装配一致，但用于 `Type::DGRAM`：
/// - IPv4：`Domain::IPV4` + `setsockopt(IPPROTO_IP, IP_UNICAST_IF, htonl(if_index))` + `bind(nic.ipv4, 0)`
/// - IPv6：`Domain::IPV6` + `setsockopt(IPPROTO_IPV6, IPV6_UNICAST_IF, htonl(if_index))` + `bind(nic.ipv6, 0)`
///
/// 接口索引以网络字节序（`to_be()`）传入。IPv6 分支要求该网卡具备可用的 IPv6 源地址，
/// 否则返回 `AddrNotAvailable`，由上层记录日志并结束该流。
async fn udp_socket_via_nic(nic: &NicRuntime, family: Family) -> std::io::Result<UdpSocket> {
    let (domain, if_level, if_optname, bind_ip) = match family {
        Family::V4 => (Domain::IPV4, IPPROTO_IP, IP_UNICAST_IF, IpAddr::V4(nic.ipv4)),
        Family::V6 => {
            let v6 = nic.ipv6.ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::AddrNotAvailable,
                    format!("网卡 {} 无可用 IPv6 出口源地址", nic.name),
                )
            })?;
            (Domain::IPV6, IPPROTO_IPV6, IPV6_UNICAST_IF, IpAddr::V6(v6))
        }
    };

    let socket = Socket::new(domain, Type::DGRAM, Some(Protocol::UDP))?;

    // 1) 接口索引强绑定（以网络字节序传入），锁死出口网卡。
    #[cfg(windows)]
    {
        let raw = socket.as_raw_socket() as windows_sys::Win32::Networking::WinSock::SOCKET;
        let value: u32 = nic.if_index.to_be();
        let rc = unsafe {
            windows_sys::Win32::Networking::WinSock::setsockopt(
                raw,
                if_level,
                if_optname,
                &value as *const u32 as *const u8,
                std::mem::size_of::<u32>() as i32,
            )
        };
        if rc != 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("UDP UNICAST_IF 绑定失败 (IfIndex={})", nic.if_index),
            ));
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (if_level, if_optname);
    }

    // 2) bind 本地出口源地址（固定源地址；失败可降级忽略，与 TCP 路径一致）。
    let bind_addr: socket2::SockAddr = SocketAddr::new(bind_ip, 0).into();
    let _ = socket.bind(&bind_addr);

    // 3) 转为非阻塞的 tokio UdpSocket。
    socket.set_nonblocking(true)?;
    let std_sock: std::net::UdpSocket = socket.into();
    UdpSocket::from_std(std_sock)
}

/// 为一条非 53 端口的 UDP 流确定真实目标并建立经所选网卡绑定的上游 UDP socket。
///
/// 返回 `(上游 socket, 真实目标端点, 出口网卡名)`：
/// - 有 `engine`：复用 `engine.pick_nic` 选出口网卡；目标为 fake-ip 时经 `FakeDns::lookup`
///   反查域名并用所选网卡 `resolve_host_dual` 解析真实地址，再按 IP 版本偏好择族绑定出口。
/// - 无 `engine`（如服务模式）：无法反查 fake-ip 域名，命中 fake-ip 时返回 `None`；
///   真实（非 fake）目标则用默认路由的 UDP socket 直接中继。
async fn establish_udp_upstream(
    dst: SocketAddr,
    fake: &Arc<FakeDns>,
    engine: &Option<Arc<Engine>>,
) -> Option<(UdpSocket, SocketAddr, String)> {
    // fake-ip 仅存在于 IPv4 段（198.18.0.0/15）。
    let fake_domain = match dst {
        SocketAddr::V4(v4) if FakeDns::is_fake(*v4.ip()) => fake.lookup(*v4.ip()),
        _ => None,
    };
    let port = dst.port();

    match engine {
        Some(eng) => {
            // 1) 确定用于网卡选择/解析的 host 与真实目标地址
            let (host_for_pick, resolved) = match &fake_domain {
                Some(domain) => {
                    let nic = eng.pick_nic(domain, port, None);
                    let addrs = eng.resolve_host_dual(&nic, domain, port).await;
                    (domain.clone(), Some((nic, addrs)))
                }
                None => {
                    // 真实目标：以目标 IP 字面量做网卡选择，直接绑定该族
                    (dst.ip().to_string(), None)
                }
            };

            let (nic, family, real_dst) = match resolved {
                Some((nic, addrs)) => {
                    // 依据 IP 版本偏好在解析出的候选地址上择族
                    let families =
                        crate::engine::pick_family(eng.ip_pref(), addrs.v4.is_some(), addrs.v6.is_some());
                    let fam = match families.first() {
                        Some(f) => *f,
                        None => {
                            eprintln!(
                                "[TUN/UDP] 域名 {host_for_pick} 无可用地址，放弃该 UDP 流"
                            );
                            return None;
                        }
                    };
                    let real = match fam {
                        Family::V4 => SocketAddr::V4(SocketAddrV4::new(addrs.v4?, port)),
                        Family::V6 => SocketAddr::V6(SocketAddrV6::new(addrs.v6?, port, 0, 0)),
                    };
                    (nic, fam, real)
                }
                None => {
                    let nic = eng.pick_nic(&host_for_pick, port, None);
                    let fam = match dst {
                        SocketAddr::V4(_) => Family::V4,
                        SocketAddr::V6(_) => Family::V6,
                    };
                    (nic, fam, dst)
                }
            };

            let nic_name = nic.name.clone();
            let sock = match udp_socket_via_nic(&nic, family).await {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("[TUN/UDP] 经网卡 {nic_name} 创建上游 UDP socket 失败: {e}");
                    return None;
                }
            };
            if let Err(e) = sock.connect(real_dst).await {
                eprintln!("[TUN/UDP] 连接上游 {real_dst}（网卡 {nic_name}）失败: {e}");
                return None;
            }
            Some((sock, real_dst, nic_name))
        }
        None => {
            // 无引擎：无法反查 fake-ip 域名，直接放弃该流（应用会回落到 TCP）。
            if fake_domain.is_some() {
                eprintln!("[TUN/UDP] 无引擎上下文，无法反查 fake-ip 域名，放弃该 UDP 流");
                return None;
            }
            let bind_any: SocketAddr = match dst {
                SocketAddr::V4(_) => SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)),
                SocketAddr::V6(_) => {
                    SocketAddr::V6(SocketAddrV6::new(std::net::Ipv6Addr::UNSPECIFIED, 0, 0, 0))
                }
            };
            let sock = match UdpSocket::bind(bind_any).await {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("[TUN/UDP] 创建默认路由 UDP socket 失败: {e}");
                    return None;
                }
            };
            if let Err(e) = sock.connect(dst).await {
                eprintln!("[TUN/UDP] 连接上游 {dst} 失败: {e}");
                return None;
            }
            Some((sock, dst, "default".to_string()))
        }
    }
}

/// 处理一条被截获的 TCP 流：还原目标（fake-ip → 域名）后经本地 SOCKS 转发。
/// 泛型化以避免依赖 ipstack 内部具体类型；`dst` 为原始目标地址（取自 IpStackStream）。
async fn handle_tcp<S>(mut tcp: S, dst: SocketAddr, socks_port: u16, fake: Arc<FakeDns>)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let v4 = match dst {
        SocketAddr::V4(v4) => v4,
        SocketAddr::V6(_) => return, // 引擎为 IPv4 分流，暂不处理 IPv6
    };
    let port = v4.port();
    let target = match fake.lookup(*v4.ip()) {
        Some(domain) => Target::Domain(domain),
        None => Target::Ip(*v4.ip()),
    };
    if let Ok(mut up) = socks_connect(socks_port, &target, port).await {
        let _ = tokio::io::copy_bidirectional(&mut tcp, &mut up).await;
        let _ = up.shutdown().await;
        let _ = tcp.shutdown().await;
    }
}

/// 处理一条被截获的 UDP 流：
/// - 53 端口：接管 DNS，返回 fake-ip（A 记录），行为与既有完全一致（Req 3.5）。
/// - 非 53 端口：建立/复用经所选网卡 Egress_Binding 的 UDP 中继，双向搬运数据报（Req 3.1/3.2/3.4）。
///
/// `client_src` 为客户端在虚拟网卡上的源端点，`(client_src, real_dst)` 构成会话键，
/// 供空闲回收与键位复用。`engine` 为 `None` 时（如服务模式）退化为仅中继真实目标。
async fn handle_udp<S>(
    mut udp: S,
    dst: SocketAddr,
    client_src: SocketAddr,
    fake: Arc<FakeDns>,
    engine: Option<Arc<Engine>>,
    sessions: Arc<UdpSessionTable>,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let port = dst.port();

    // ---- 53 端口 DNS：保持既有 fake-ip 应答行为不变 ----
    if port == 53 {
        // 既有 DNS 接管仅处理 IPv4（fake-ip 段），IPv6 目标的 53 端口直接结束。
        if matches!(dst, SocketAddr::V6(_)) {
            return;
        }
        let mut buf = [0u8; 1500];
        let n =
            match tokio::time::timeout(std::time::Duration::from_secs(3), udp.read(&mut buf)).await {
                Ok(Ok(n)) if n > 0 => n,
                _ => return,
            };
        // DNS 查询类型常量：A=1（IPv4）、AAAA=28（IPv6）
        const QTYPE_A: u16 = 1;
        const QTYPE_AAAA: u16 = 28;
        let answer = match parse_dns_question(&buf[..n]) {
            // A 记录：分配 fake-ip 并回一条 A 记录（行为与既有完全一致）
            Some((_, domain, QTYPE_A)) if !domain.is_empty() => Some(fake.allocate(&domain)),
            // AAAA 记录：按策略回空答案，促使客户端回落到 A 记录 / IPv4（build_dns_response 仅写 A 记录）
            Some((_, _, QTYPE_AAAA)) => None,
            // 其余查询类型：同样回空答案
            _ => None,
        };
        if let Some(resp) = build_dns_response(&buf[..n], answer) {
            let _ = udp.write_all(&resp).await;
        }
        return;
    }

    // ---- 非 53 端口：UDP / QUIC 中继（不再丢弃） ----
    let (upstream, real_dst, nic_name) =
        match establish_udp_upstream(dst, &fake, &engine).await {
            Some(v) => v,
            None => return, // 上游建立失败：已记录日志并结束该流，不影响其他会话
        };

    let upstream = Arc::new(upstream);
    let key: UdpKey = (client_src, real_dst);
    let cancel = CancellationToken::new();
    sessions.insert(
        key,
        UdpSession {
            upstream: upstream.clone(),
            last_active: Instant::now(),
            nic_name,
            cancel: cancel.clone(),
        },
    );

    relay_udp(udp, upstream, key, &sessions, cancel).await;

    // 中继结束（任一方向关闭 / 取消 / 出错）：清理会话映射并释放 socket。
    sessions.remove(&key);
}

/// 在客户端 UDP 流（虚拟网卡侧）与上游经网卡绑定的 UDP socket 之间双向搬运数据报。
///
/// 上行（客户端→上游）与下行（上游→客户端）并发进行，任一方向出错或读到 EOF、
/// 或收到空闲回收的取消信号，即结束整条会话。每次数据往返刷新会话 `last_active`。
async fn relay_udp<S>(
    udp: S,
    upstream: Arc<UdpSocket>,
    key: UdpKey,
    sessions: &Arc<UdpSessionTable>,
    cancel: CancellationToken,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (mut client_r, mut client_w) = tokio::io::split(udp);

    // 上行：客户端 → 上游
    let up = upstream.clone();
    let sess_up = sessions.clone();
    let up_task = async move {
        let mut buf = vec![0u8; UDP_BUF];
        loop {
            let n = match client_r.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            if up.send(&buf[..n]).await.is_err() {
                break;
            }
            sess_up.touch(&key, Instant::now());
        }
    };

    // 下行：上游 → 客户端
    let down = upstream.clone();
    let sess_down = sessions.clone();
    let down_task = async move {
        let mut buf = vec![0u8; UDP_BUF];
        loop {
            let n = match down.recv(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            if client_w.write_all(&buf[..n]).await.is_err() {
                break;
            }
            sess_down.touch(&key, Instant::now());
        }
    };

    tokio::select! {
        _ = cancel.cancelled() => {}
        _ = up_task => {}
        _ = down_task => {}
    }
}

/// ipstack 接受循环：把每条流派发给对应处理器，直到取消。
///
/// `engine` 为可选的分流引擎上下文：`Some` 时 UDP 中继复用其网卡选择与双栈解析（在同进程
/// 直连模式下可用）；`None` 时（如服务模式无同进程引擎）UDP 中继退化为仅转发真实目标。
async fn run_stack<D>(
    device: D,
    socks_port: u16,
    fake: Arc<FakeDns>,
    engine: Option<Arc<Engine>>,
    cancel: CancellationToken,
) where
    D: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let mut cfg = IpStackConfig::default();
    let _ = cfg.mtu(MTU);
    let mut stack = IpStack::new(cfg, device);

    // UDP 中继会话表 + 后台空闲回收任务（每 UDP_REAP_INTERVAL 巡检，回收 idle>UDP_IDLE_TIMEOUT 的会话）
    let sessions = Arc::new(UdpSessionTable::new());
    {
        let sessions_reap = sessions.clone();
        let c = cancel.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(UDP_REAP_INTERVAL);
            loop {
                tokio::select! {
                    _ = c.cancelled() => break,
                    _ = tick.tick() => {
                        sessions_reap.reap(Instant::now(), UDP_IDLE_TIMEOUT);
                    }
                }
            }
        });
    }

    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            res = stack.accept() => {
                let stream = match res {
                    Ok(s) => s,
                    Err(_) => break,
                };
                // 在匹配具体变体前取出原始目标与客户端源端点（IpStackStream 提供 peer_addr/local_addr）
                let dst = stream.peer_addr();
                let src = stream.local_addr();
                match stream {
                    IpStackStream::Tcp(tcp) => {
                        let f = fake.clone();
                        tokio::spawn(handle_tcp(tcp, dst, socks_port, f));
                    }
                    IpStackStream::Udp(udp) => {
                        let f = fake.clone();
                        let eng = engine.clone();
                        let s = sessions.clone();
                        tokio::spawn(handle_udp(udp, dst, src, f, eng, s));
                    }
                    // ICMP / 未知网络层：丢弃（ping 等本模块不代理）
                    _ => {}
                }
            }
        }
    }
}

/// 执行一条命令（隐藏窗口），返回是否成功。
fn run_cmd(program: &str, args: &[&str]) -> bool {
    use std::os::windows::process::CommandExt;
    std::process::Command::new(program)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// 接管路由与 DNS：把默认路由（拆成两条 /1）指向 TUN，并将 DNS 指向 fake-ip 服务。
fn setup_routes() -> Result<(), String> {
    let gw = TUN_IP.to_string();
    // 0.0.0.0/1 + 128.0.0.0/1 覆盖默认路由（保留系统原默认路由不被删除），下一跳为 TUN 自身
    let ok1 = run_cmd("route", &["ADD", "0.0.0.0", "MASK", "128.0.0.0", &gw, "METRIC", "1"]);
    let ok2 = run_cmd("route", &["ADD", "128.0.0.0", "MASK", "128.0.0.0", &gw, "METRIC", "1"]);
    if !ok1 || !ok2 {
        return Err("写入默认路由失败（需管理员权限）".into());
    }
    // 将 TUN 接口的 DNS 指向 fake-ip 服务，并压低接口跃点使其优先
    let dns = FAKE_DNS_IP.to_string();
    run_cmd(
        "netsh",
        &[
            "interface", "ipv4", "set", "dnsservers",
            &format!("name={TUN_NAME}"), "static", &dns, "primary",
        ],
    );
    run_cmd(
        "netsh",
        &["interface", "ipv4", "set", "interface", &format!("name={TUN_NAME}"), "metric=1"],
    );
    Ok(())
}

/// 撤销路由接管（DNS 随适配器移除自动失效）。
fn teardown_routes() {
    let gw = TUN_IP.to_string();
    run_cmd("route", &["DELETE", "0.0.0.0", "MASK", "128.0.0.0", &gw]);
    run_cmd("route", &["DELETE", "128.0.0.0", "MASK", "128.0.0.0", &gw]);
}

/// 启动时清理可能残留的 TUN 接管路由（上次崩溃遗留）。适配器已随进程退出移除，
/// 这里仅补删路由，幂等且无副作用。
pub fn cleanup_residual_routes() {
    teardown_routes();
}

/// TUN 运行句柄：停止时取消接受循环并撤销路由。
pub struct TunHandle {
    cancel: CancellationToken,
}

impl TunHandle {
    pub fn stop(&self) {
        self.cancel.cancel();
        teardown_routes();
    }
}

/// 启动 TUN 全局接管：创建 wintun 适配器 → 接管路由/DNS → 运行用户态栈。
/// `socks_port` 为已在运行的本地 SOCKS5 引擎端口。
/// 本函数不依赖 Tauri，可在 GUI 进程（管理员直连模式）或服务进程（服务模式）中调用。
pub async fn start(socks_port: u16) -> Result<TunHandle, String> {
    start_inner(socks_port, None).await
}

/// 启动 TUN 全局接管并注入同进程引擎（进程内直连/管理员模式）。
///
/// 与 `start` 行为一致，但把 `Some(engine)` 下沉至用户态栈：UDP/QUIC 中继据此
/// 复用引擎的网卡选择（`pick_nic`）、fake-ip 反查与双栈解析（`resolve_host_dual`），
/// 实现逐卡 UDP 出口绑定（Req 3.1/3.2/3.4），而非丢弃 fake-ip UDP 流。
pub async fn start_with_engine(
    socks_port: u16,
    engine: std::sync::Arc<crate::engine::Engine>,
) -> Result<TunHandle, String> {
    start_inner(socks_port, Some(engine)).await
}

/// TUN 启动内部实现：`engine` 为 `Some` 时把引擎上下文下沉给 `run_stack`，
/// 供 UDP 中继逐卡出口 + fake-ip 反查；为 `None` 时退化为仅中继真实目标。
async fn start_inner(
    socks_port: u16,
    engine: Option<Arc<Engine>>,
) -> Result<TunHandle, String> {
    // 1) 创建 wintun 虚拟网卡并配置地址
    let mut config = tun::Configuration::default();
    config
        .tun_name(TUN_NAME)
        .address(TUN_IP)
        .netmask(TUN_NETMASK)
        .mtu(MTU)
        .up();
    config.platform_config(|p| {
        p.device_guid(TUN_GUID);
    });
    let device = tun::create_as_async(&config).map_err(|e| {
        format!("创建 TUN 虚拟网卡失败（需管理员/服务权限，且 wintun.dll 须位于程序目录）: {e}")
    })?;

    // 2) 适配器注册需要片刻，稍等后再接管路由/DNS
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    if let Err(e) = setup_routes() {
        // 路由接管失败：设备将随 device 释放而移除，避免半接管导致断网
        drop(device);
        teardown_routes();
        return Err(e);
    }

    let cancel = CancellationToken::new();
    let fake = Arc::new(FakeDns::new());

    {
        let c = cancel.clone();
        tokio::spawn(async move {
            // engine 为 Some（进程内直连模式）时，UDP 中继复用引擎做 fake-ip 反查 +
            // 逐卡出口绑定；为 None（服务模式无同进程引擎）时退化为仅中继真实目标。
            run_stack(device, socks_port, fake, engine, c).await;
        });
    }

    Ok(TunHandle { cancel })
}

#[cfg(test)]
mod tests {
    use super::{build_dns_response, expired_udp_keys, parse_dns_question, FakeDns, UdpKey};
    use proptest::prelude::*;
    use std::collections::HashSet;
    use std::net::{Ipv4Addr, SocketAddr};
    use std::time::{Duration, Instant};

    /// 测试辅助：构造一个标准 DNS 查询报文（header + 单一问题段）。
    /// header: id, flags(0x0100 递归), QDCOUNT=1, 其余计数为 0；
    /// 问题段为 len-前缀标签 + 0x00 + qtype + qclass(IN)。
    fn build_query(id: u16, host: &str, qtype: u16) -> Vec<u8> {
        let mut q = Vec::with_capacity(host.len() + 18);
        q.extend_from_slice(&id.to_be_bytes());
        q.extend_from_slice(&[0x01, 0x00]); // 递归查询标志
        q.extend_from_slice(&[0x00, 0x01]); // QDCOUNT = 1
        q.extend_from_slice(&[0x00, 0x00, 0x00, 0x00, 0x00, 0x00]); // AN/NS/AR = 0
        for label in host.split('.') {
            if label.is_empty() {
                continue;
            }
            q.push(label.len() as u8);
            q.extend_from_slice(label.as_bytes());
        }
        q.push(0); // 名称结束
        q.extend_from_slice(&qtype.to_be_bytes()); // QTYPE
        q.extend_from_slice(&[0x00, 0x01]); // QCLASS = IN
        q
    }

    proptest! {
        // Feature: network-capability-expansion, Property 6
        // Fake-IP 分配 round-trip 与幂等：
        //   - 域名 allocate 得到的 fake-ip 经 lookup 反查应还原为同一域名（round-trip）；
        //   - 对同一域名多次 allocate 返回同一 fake-ip（幂等）；
        //   - 分配出的地址落在 fake-ip 段（is_fake 为真）。
        // Validates: Requirements 3.2, 6.5
        #![proptest_config(ProptestConfig { cases: 100, ..ProptestConfig::default() })]

        #[test]
        fn prop_fake_ip_roundtrip_idempotent(
            domains in proptest::collection::vec(
                proptest::string::string_regex("[a-z]{1,8}(\\.[a-z]{2,4}){0,2}").unwrap(),
                1..20,
            )
        ) {
            let fake = FakeDns::new();
            for domain in &domains {
                // round-trip：allocate 得到的 fake-ip 反查应还原同一域名
                let ip1 = fake.allocate(domain);
                let recovered = fake.lookup(ip1);
                prop_assert_eq!(recovered.as_deref(), Some(domain.as_str()));
                // 分配的地址必须落在 fake-ip 段（198.18.0.0/15）
                prop_assert!(FakeDns::is_fake(ip1));
                // 幂等：同一域名再次 allocate 返回同一 fake-ip
                let ip2 = fake.allocate(domain);
                prop_assert_eq!(ip1, ip2);
            }
        }
    }

    proptest! {
        // Feature: network-capability-expansion, Property 7
        // UDP 会话空闲回收（expired_udp_keys）：给定各会话 (key, last_active)、当前时刻 now
        // 与空闲阈值 idle，结果必须恰好包含所有满足 `now - last_active > idle`（严格大于）
        // 的 key，且不含任何未超时者。
        // 生成器把序号并入端口，保证每个 UdpKey 唯一，便于集合比较。
        // Validates: Requirements 3.3
        #![proptest_config(ProptestConfig { cases: 100, ..ProptestConfig::default() })]

        #[test]
        fn prop_expired_udp_keys_selects_exactly_overdue(
            ages_ms in proptest::collection::vec(0u64..20_000, 0..40),
            idle_ms in 0u64..20_000,
        ) {
            // 以一个远离程序启动点的基准时刻，确保 now - age 不下溢。
            let now = Instant::now() + Duration::from_secs(3600);
            let idle = Duration::from_millis(idle_ms);

            // 构造唯一 key：序号写入源/目的端口；last_active = now - age。
            let entries: Vec<(UdpKey, Instant)> = ages_ms
                .iter()
                .enumerate()
                .map(|(i, &age)| {
                    let src = SocketAddr::from((Ipv4Addr::new(10, 0, 0, 1), i as u16));
                    let dst = SocketAddr::from((Ipv4Addr::new(93, 184, 216, 34), 40000 + i as u16));
                    let last_active = now
                        .checked_sub(Duration::from_millis(age))
                        .unwrap_or(now);
                    ((src, dst), last_active)
                })
                .collect();

            let got: HashSet<UdpKey> = expired_udp_keys(&entries, now, idle).into_iter().collect();

            // 期望集合：age（= now - last_active）严格大于 idle 的项。
            let expected: HashSet<UdpKey> = entries
                .iter()
                .filter(|(_, la)| now.saturating_duration_since(*la) > idle)
                .map(|(k, _)| *k)
                .collect();

            prop_assert_eq!(&got, &expected);

            // 反向确认：结果中不含任何未超时的 key。
            for (k, la) in &entries {
                let overdue = now.saturating_duration_since(*la) > idle;
                prop_assert_eq!(got.contains(k), overdue);
            }
        }
    }

    proptest! {
        // Feature: network-capability-expansion, Property 8
        // DNS 问题/应答 round-trip：
        //   - 构造的 A 查询经 parse_dns_question 应还原同一事务 ID、域名与 qtype；
        //   - build_dns_response 回填答案后再解析，事务 ID 与问题段域名保持一致。
        // Validates: Requirements 3.5, 6.5, 6.6
        #![proptest_config(ProptestConfig { cases: 100, ..ProptestConfig::default() })]

        #[test]
        fn prop_dns_question_response_roundtrip(
            id in any::<u16>(),
            labels in proptest::collection::vec("[a-z][a-z0-9]{0,7}", 1..=4),
        ) {
            let host = labels.join(".");

            // 1) A 查询问题段 round-trip：id / 域名 / qtype 均还原。
            let query = build_query(id, &host, 1);
            let parsed = parse_dns_question(&query);
            prop_assert_eq!(parsed, Some((id, host.clone(), 1u16)));

            // 2) 应答回填后仍能还原同一 id 与问题段域名（应答回显问题段）。
            let resp = build_dns_response(&query, Some(Ipv4Addr::new(93, 184, 216, 34)))
                .expect("build_dns_response 应对合法查询返回 Some");
            let (rid, rhost, _rqtype) =
                parse_dns_question(&resp).expect("应答问题段应可解析");
            prop_assert_eq!(rid, id);
            prop_assert_eq!(rhost, host);
        }
    }
}
