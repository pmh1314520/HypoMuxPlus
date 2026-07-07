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
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use parking_lot::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_util::sync::CancellationToken;

use ipstack::{IpStack, IpStackConfig, IpStackStream};

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

    /// 判断是否为 fake-ip（198.18.0.0/15）。保留供未来按段快速判定使用。
    #[allow(dead_code)]
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

/// 解析 DNS 查询，取出事务 ID、首个问题域名与是否为 A 记录查询。
fn parse_dns_question(buf: &[u8]) -> Option<(u16, String, bool)> {
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
    Some((id, domain, qtype == 1))
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

/// 处理一条被截获的 UDP 流：仅接管 53 端口 DNS，返回 fake-ip；其余 UDP（含 QUIC）暂丢弃。
async fn handle_udp<S>(mut udp: S, dst: SocketAddr, fake: Arc<FakeDns>)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let port = match dst {
        SocketAddr::V4(v4) => v4.port(),
        SocketAddr::V6(_) => return,
    };
    if port != 53 {
        // 非 DNS 的 UDP（如 QUIC/HTTP3）暂不支持，直接结束——应用会自动回落到 TCP
        return;
    }
    let mut buf = [0u8; 1500];
    let n = match tokio::time::timeout(std::time::Duration::from_secs(3), udp.read(&mut buf)).await {
        Ok(Ok(n)) if n > 0 => n,
        _ => return,
    };
    let answer = match parse_dns_question(&buf[..n]) {
        Some((_, domain, true)) if !domain.is_empty() => Some(fake.allocate(&domain)),
        _ => None, // 非 A 查询：回空答案，促使客户端走 IPv4
    };
    if let Some(resp) = build_dns_response(&buf[..n], answer) {
        let _ = udp.write_all(&resp).await;
    }
}

/// ipstack 接受循环：把每条流派发给对应处理器，直到取消。
async fn run_stack<D>(device: D, socks_port: u16, fake: Arc<FakeDns>, cancel: CancellationToken)
where
    D: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let mut cfg = IpStackConfig::default();
    let _ = cfg.mtu(MTU);
    let mut stack = IpStack::new(cfg, device);
    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            res = stack.accept() => {
                let stream = match res {
                    Ok(s) => s,
                    Err(_) => break,
                };
                // 在匹配具体变体前取出原始目标地址（IpStackStream 提供 peer_addr）
                let dst = stream.peer_addr();
                match stream {
                    IpStackStream::Tcp(tcp) => {
                        let f = fake.clone();
                        tokio::spawn(handle_tcp(tcp, dst, socks_port, f));
                    }
                    IpStackStream::Udp(udp) => {
                        let f = fake.clone();
                        tokio::spawn(handle_udp(udp, dst, f));
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
            run_stack(device, socks_port, fake, c).await;
        });
    }

    Ok(TunHandle { cancel })
}
