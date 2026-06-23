//! 多网卡分流代理引擎（HypoMuxPlus 核心）
//!
//! 在 tokio 异步运行时上同时提供 SOCKS5 与 HTTP/HTTPS 本地转发服务，对每条
//! 出站连接执行 Round-Robin 网卡轮询，并通过 `IP_UNICAST_IF` 接口索引强绑定
//! 把流量物理钉死在指定网卡上，实现多网卡带宽叠加。
//!
//! 【神圣地基】出站 socket 的双保险绑定：
//!   1) 先 `setsockopt(IPPROTO_IP, IP_UNICAST_IF, htonl(if_index))` 锁死出口网卡
//!   2) 再 `bind(local_ip, 0)` 固定源地址（失败可降级忽略）
//! 以及前置异步 DNS 解析，逻辑一字不差地继承自原 Python 项目的验证成果，
//! 根治同网段双网卡的 WinError 10049 错网卡问题。

use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::atomic::{AtomicI64, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use socket2::{Domain, Protocol, Socket, Type};
use tauri::{AppHandle, Emitter};
use tokio::io::{copy_bidirectional, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

#[cfg(windows)]
use std::os::windows::io::AsRawSocket;

/// IPv4 下强制指定物理网卡出口，绕过 Windows 默认路由判定
const IP_UNICAST_IF: i32 = 31;
const IPPROTO_IP: i32 = 0;
const WSAEWOULDBLOCK: i32 = 10035;
const MAX_HEADER_BYTES: usize = 64 * 1024;

/// 前端勾选并下发的网卡（index 为 scan 阶段拿到的权威 IfIndex）
#[derive(Debug, Clone, Deserialize)]
pub struct SelectedNic {
    pub index: u32,
    pub name: String,
    pub ip: String,
}

/// 单张网卡的运行时状态
pub struct NicRuntime {
    pub name: String,
    pub ip: Ipv4Addr,
    pub if_index: u32,
    pub active: AtomicI64,
    /// 最近一秒下行速率（MB/s × 100），供按速度加权调度使用
    pub speed: AtomicU64,
}

/// 连接调度策略
#[derive(Clone, Copy, PartialEq)]
pub enum Strategy {
    /// 经典轮询（与原版一致）
    RoundRobin,
    /// 最少连接优先（自动均衡负载）
    LeastConn,
    /// 按实时下行速度加权（快的网卡多分流量）
    WeightedSpeed,
}

impl Strategy {
    fn parse(s: &str) -> Strategy {
        match s {
            "least" => Strategy::LeastConn,
            "weighted" => Strategy::WeightedSpeed,
            _ => Strategy::RoundRobin,
        }
    }
    fn label(&self) -> &'static str {
        match self {
            Strategy::RoundRobin => "Round-Robin 轮询",
            Strategy::LeastConn => "最少连接优先",
            Strategy::WeightedSpeed => "按速度加权",
        }
    }
}

/// 遥测载荷（emit 给前端）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct NicTelemetry {
    index: u32,
    name: String,
    down_mbps: f64,
    up_mbps: f64,
    connections: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TotalTelemetry {
    down_mbps: f64,
    up_mbps: f64,
    connections: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TelemetryPayload {
    per_nic: Vec<NicTelemetry>,
    total: TotalTelemetry,
}

/// 代理引擎核心，含调度器与网卡集合
pub struct Engine {
    nics: Vec<Arc<NicRuntime>>,
    rr: AtomicUsize,
    strategy: Strategy,
    /// 平滑加权轮询的动态权重累加器（仅 WeightedSpeed 使用）
    wrr: Mutex<Vec<i64>>,
    app: AppHandle,
}

impl Engine {
    fn next_nic(&self) -> Arc<NicRuntime> {
        match self.strategy {
            Strategy::RoundRobin => {
                let i = self.rr.fetch_add(1, Ordering::Relaxed) % self.nics.len();
                self.nics[i].clone()
            }
            Strategy::LeastConn => {
                let mut best = 0usize;
                let mut best_v = i64::MAX;
                for (i, n) in self.nics.iter().enumerate() {
                    let v = n.active.load(Ordering::Relaxed);
                    if v < best_v {
                        best_v = v;
                        best = i;
                    }
                }
                self.nics[best].clone()
            }
            Strategy::WeightedSpeed => {
                // 有效权重 = 实时速度(MB/s×100) + 基准 100，保证慢/空闲网卡也有最低份额
                let eff: Vec<i64> = self
                    .nics
                    .iter()
                    .map(|n| n.speed.load(Ordering::Relaxed) as i64 + 100)
                    .collect();
                let total: i64 = eff.iter().sum();
                // 平滑加权轮询（nginx SWRR 算法），分配均匀且按权重倾斜
                let mut cur = self.wrr.lock().unwrap();
                let mut best = 0usize;
                let mut best_v = i64::MIN;
                for i in 0..eff.len() {
                    cur[i] += eff[i];
                    if cur[i] > best_v {
                        best_v = cur[i];
                        best = i;
                    }
                }
                cur[best] -= total;
                self.nics[best].clone()
            }
        }
    }

    fn log(&self, msg: impl Into<String>) {
        let _ = self.app.emit("hmx-log", msg.into());
    }
}

/// 运行句柄：停止时取消所有任务并强制断开在途连接
pub struct EngineHandle {
    cancel: CancellationToken,
}

impl EngineHandle {
    pub fn stop(&self) {
        self.cancel.cancel();
    }
}

/// 启动引擎：绑定 SOCKS5 与 HTTP 监听端口，spawn 调度与遥测任务。
/// 监听绑定成功后才返回，便于上层据此接管系统代理。
pub async fn start(
    app: AppHandle,
    selected: Vec<SelectedNic>,
    socks_port: u16,
    http_port: u16,
    strategy: String,
) -> Result<EngineHandle, String> {
    if selected.is_empty() {
        return Err("至少需要选择一张网卡".into());
    }
    let strategy = Strategy::parse(&strategy);

    let mut nics: Vec<Arc<NicRuntime>> = Vec::with_capacity(selected.len());
    for s in &selected {
        let ip: Ipv4Addr = s
            .ip
            .parse()
            .map_err(|_| format!("网卡 {} 的 IPv4 地址非法: {}", s.name, s.ip))?;
        nics.push(Arc::new(NicRuntime {
            name: s.name.clone(),
            ip,
            if_index: s.index,
            active: AtomicI64::new(0),
            speed: AtomicU64::new(0),
        }));
    }

    let socks_listener = TcpListener::bind(("127.0.0.1", socks_port))
        .await
        .map_err(|e| format!("无法监听 SOCKS5 端口 127.0.0.1:{socks_port} -- {e}"))?;
    let http_listener = TcpListener::bind(("127.0.0.1", http_port))
        .await
        .map_err(|e| format!("无法监听 HTTP 端口 127.0.0.1:{http_port} -- {e}"))?;

    let cancel = CancellationToken::new();
    let engine = Arc::new(Engine {
        nics: nics.clone(),
        rr: AtomicUsize::new(0),
        strategy,
        wrr: Mutex::new(vec![0i64; nics.len()]),
        app: app.clone(),
    });

    let nic_names: Vec<&str> = nics.iter().map(|n| n.name.as_str()).collect();
    engine.log(format!(
        "[HypoMux] SOCKS5+HTTP 分流引擎已启动 | SOCKS 127.0.0.1:{socks_port} | HTTP 127.0.0.1:{http_port} | 调度策略: {} | 参与分流网卡: {}",
        strategy.label(),
        nic_names.join(", ")
    ));

    // SOCKS5 接受循环
    {
        let eng = engine.clone();
        let c = cancel.clone();
        tauri::async_runtime::spawn(async move {
            accept_loop(socks_listener, eng, c, Protocol_::Socks).await;
        });
    }
    // HTTP 接受循环
    {
        let eng = engine.clone();
        let c = cancel.clone();
        tauri::async_runtime::spawn(async move {
            accept_loop(http_listener, eng, c, Protocol_::Http).await;
        });
    }
    // 遥测循环
    {
        let app2 = app.clone();
        let c = cancel.clone();
        tauri::async_runtime::spawn(async move {
            telemetry_loop(app2, nics, c).await;
        });
    }

    Ok(EngineHandle { cancel })
}

/// 单张网卡的连通性 / 延迟探测结果
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LatencyResult {
    pub index: u32,
    pub name: String,
    /// 连接 RTT（毫秒），-1 表示探测失败
    pub latency_ms: i64,
    pub ok: bool,
}

/// 逐张网卡探测出口连通性与延迟：经各网卡 TCP 连接公共节点测 RTT。
pub async fn test_latency(selected: Vec<SelectedNic>) -> Vec<LatencyResult> {
    // 国内外均可达的稳定节点（AliDNS:443），仅测 TCP 握手 RTT，不传输数据
    let target = SocketAddrV4::new(Ipv4Addr::new(223, 5, 5, 5), 443);
    let mut out = Vec::with_capacity(selected.len());
    for s in selected {
        let ip: Ipv4Addr = match s.ip.parse() {
            Ok(v) => v,
            Err(_) => {
                out.push(LatencyResult { index: s.index, name: s.name, latency_ms: -1, ok: false });
                continue;
            }
        };
        let nic = NicRuntime {
            name: s.name.clone(),
            ip,
            if_index: s.index,
            active: AtomicI64::new(0),
            speed: AtomicU64::new(0),
        };
        let start = std::time::Instant::now();
        let res =
            tokio::time::timeout(std::time::Duration::from_secs(2), connect_via_nic(&nic, target)).await;
        match res {
            Ok(Ok(_stream)) => out.push(LatencyResult {
                index: s.index,
                name: s.name,
                latency_ms: start.elapsed().as_millis() as i64,
                ok: true,
            }),
            _ => out.push(LatencyResult { index: s.index, name: s.name, latency_ms: -1, ok: false }),
        }
    }
    out
}

#[derive(Clone, Copy)]
enum Protocol_ {
    Socks,
    Http,
}

async fn accept_loop(
    listener: TcpListener,
    engine: Arc<Engine>,
    cancel: CancellationToken,
    proto: Protocol_,
) {
    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            res = listener.accept() => {
                match res {
                    Ok((stream, _)) => {
                        let _ = stream.set_nodelay(true);
                        let eng = engine.clone();
                        let c = cancel.clone();
                        tauri::async_runtime::spawn(async move {
                            tokio::select! {
                                _ = c.cancelled() => {}
                                _ = async {
                                    let r = match proto {
                                        Protocol_::Socks => handle_socks(eng.clone(), stream).await,
                                        Protocol_::Http => handle_http(eng.clone(), stream).await,
                                    };
                                    if let Err(e) = r {
                                        // 仅记录非常规错误，常见的连接重置忽略
                                        let s = e.to_string();
                                        if !s.is_empty() {
                                            eng.log(format!("[连接异常] {s}"));
                                        }
                                    }
                                } => {}
                            }
                        });
                    }
                    Err(_) => break,
                }
            }
        }
    }
}

/// 在途连接计数 RAII 守卫：drop 时自动减一
struct ConnGuard(Arc<NicRuntime>);
impl Drop for ConnGuard {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::Relaxed);
    }
}

/// 异步解析域名为首个 IPv4 地址
async fn resolve_ipv4(host: &str, port: u16) -> std::io::Result<SocketAddrV4> {
    let addrs = tokio::net::lookup_host((host, port)).await?;
    for a in addrs {
        if let SocketAddr::V4(v4) = a {
            return Ok(v4);
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AddrNotAvailable,
        "无可用 IPv4 地址",
    ))
}

/// 【神圣地基】创建出站 socket：先 IP_UNICAST_IF 锁死网卡，再 bind 源地址，
/// 最后异步连接目标。根治同网段 WinError 10049。
async fn connect_via_nic(nic: &NicRuntime, dst: SocketAddrV4) -> std::io::Result<TcpStream> {
    let socket = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))?;

    // 1) 接口索引强绑定（必须在 bind/connect 之前）。IPv4 下索引为网络字节序。
    #[cfg(windows)]
    {
        let raw = socket.as_raw_socket() as windows_sys::Win32::Networking::WinSock::SOCKET;
        let value: u32 = nic.if_index.to_be();
        let rc = unsafe {
            windows_sys::Win32::Networking::WinSock::setsockopt(
                raw,
                IPPROTO_IP,
                IP_UNICAST_IF,
                &value as *const u32 as *const u8,
                std::mem::size_of::<u32>() as i32,
            )
        };
        if rc != 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("IP_UNICAST_IF 绑定失败 (IfIndex={})", nic.if_index),
            ));
        }
    }

    // 2) bind 本地出口 IP（仅固定源地址，失败可降级忽略）
    let bind_addr: socket2::SockAddr = SocketAddr::new(IpAddr::V4(nic.ip), 0).into();
    let _ = socket.bind(&bind_addr);

    // 3) 非阻塞连接，交给 tokio 等待可写
    socket.set_nonblocking(true)?;
    let target: socket2::SockAddr = SocketAddr::V4(dst).into();
    match socket.connect(&target) {
        Ok(_) => {}
        Err(e) if e.raw_os_error() == Some(WSAEWOULDBLOCK) => {}
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
        Err(e) => return Err(e),
    }

    let std_stream: std::net::TcpStream = socket.into();
    let stream = TcpStream::from_std(std_stream)?;
    stream.writable().await?;
    if let Some(err) = stream.take_error()? {
        return Err(err);
    }
    let _ = stream.set_nodelay(true);
    Ok(stream)
}

// ============================== SOCKS5 ==============================

async fn handle_socks(engine: Arc<Engine>, mut client: TcpStream) -> std::io::Result<()> {
    // 1) 握手：版本 + 方法列表
    let mut head = [0u8; 2];
    client.read_exact(&mut head).await?;
    if head[0] != 0x05 {
        return Ok(());
    }
    let nmethods = head[1] as usize;
    let mut methods = vec![0u8; nmethods];
    client.read_exact(&mut methods).await?;
    client.write_all(&[0x05, 0x00]).await?; // 无需认证

    // 2) 请求：VER CMD RSV ATYP
    let mut req = [0u8; 4];
    client.read_exact(&mut req).await?;
    if req[1] != 0x01 {
        // 仅支持 CONNECT
        client
            .write_all(&[0x05, 0x07, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
            .await?;
        return Ok(());
    }

    let atyp = req[3];
    let mut domain: Option<String> = None;
    let mut literal_ip: Option<Ipv4Addr> = None;
    match atyp {
        0x01 => {
            let mut a = [0u8; 4];
            client.read_exact(&mut a).await?;
            literal_ip = Some(Ipv4Addr::new(a[0], a[1], a[2], a[3]));
        }
        0x03 => {
            let mut len = [0u8; 1];
            client.read_exact(&mut len).await?;
            let mut buf = vec![0u8; len[0] as usize];
            client.read_exact(&mut buf).await?;
            domain = Some(String::from_utf8_lossy(&buf).to_string());
        }
        _ => {
            // IPv6 等暂不支持
            client
                .write_all(&[0x05, 0x08, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                .await?;
            return Ok(());
        }
    }
    let mut port_buf = [0u8; 2];
    client.read_exact(&mut port_buf).await?;
    let port = u16::from_be_bytes(port_buf);

    // 前置异步 DNS 解析
    let dst = if let Some(ip) = literal_ip {
        SocketAddrV4::new(ip, port)
    } else {
        let host = domain.clone().unwrap_or_default();
        match resolve_ipv4(&host, port).await {
            Ok(v4) => v4,
            Err(e) => {
                engine.log(format!("[DNS失败] 无法解析域名 {host}: {e}"));
                client
                    .write_all(&[0x05, 0x04, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                    .await?;
                return Ok(());
            }
        }
    };

    let target_display = domain.unwrap_or_else(|| dst.ip().to_string());

    // 调度 + 物理绑定连接
    let nic = engine.next_nic();
    nic.active.fetch_add(1, Ordering::Relaxed);
    let _guard = ConnGuard(nic.clone());
    engine.log(format!(
        "[调度分配] 新连接 -> [{}] | 目标: {}:{}",
        nic.name, target_display, port
    ));

    let mut upstream = match connect_via_nic(&nic, dst).await {
        Ok(s) => s,
        Err(e) => {
            engine.log(format!(
                "[连通失败] 网卡: {} 无法连接目标 {}:{}: {}",
                nic.name, target_display, port, e
            ));
            client
                .write_all(&[0x05, 0x05, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                .await?;
            return Ok(());
        }
    };

    // 连接成功，回应客户端
    client
        .write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
        .await?;

    let _ = copy_bidirectional(&mut client, &mut upstream).await;
    Ok(())
}

// ============================== HTTP / HTTPS ==============================

async fn handle_http(engine: Arc<Engine>, mut client: TcpStream) -> std::io::Result<()> {
    // 逐字节读取直到 \r\n\r\n（请求头），避免吞掉请求体
    let mut header = Vec::with_capacity(1024);
    let mut byte = [0u8; 1];
    loop {
        client.read_exact(&mut byte).await?;
        header.push(byte[0]);
        if header.len() >= 4 && &header[header.len() - 4..] == b"\r\n\r\n" {
            break;
        }
        if header.len() > MAX_HEADER_BYTES {
            let _ = client
                .write_all(b"HTTP/1.1 431 Request Header Fields Too Large\r\nConnection: close\r\n\r\n")
                .await;
            return Ok(());
        }
    }

    let header_text = String::from_utf8_lossy(&header).to_string();
    let mut lines = header_text.split("\r\n");
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.splitn(3, ' ');
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("").to_string();
    let version = parts.next().unwrap_or("HTTP/1.1").to_string();

    if method.is_empty() || target.is_empty() {
        let _ = client
            .write_all(b"HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\n")
            .await;
        return Ok(());
    }

    let header_lines: Vec<&str> = header_text.split("\r\n").collect();
    let is_connect = method.eq_ignore_ascii_case("CONNECT");

    let (dst_host, dst_port, outbound_header): (String, u16, Option<Vec<u8>>) = if is_connect {
        let (h, p) = split_host_port(&target, 443);
        (h, p, None)
    } else if let Some((scheme, rest)) = target.split_once("://") {
        // 绝对形式 URL：http://host:port/path
        let (authority, path) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, "/"),
        };
        let default_port = if scheme.eq_ignore_ascii_case("https") { 443 } else { 80 };
        let (h, p) = split_host_port(authority, default_port);
        let origin = build_origin_header(&method, path, &version, &header_lines);
        (h, p, Some(origin))
    } else {
        // 退化：从 Host 头取目标
        let host_header = find_header(&header_lines, "host");
        if host_header.is_empty() {
            let _ = client
                .write_all(b"HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\n")
                .await;
            return Ok(());
        }
        let (h, p) = split_host_port(&host_header, 80);
        (h, p, Some(header.clone()))
    };

    if dst_host.is_empty() || dst_port == 0 {
        let _ = client
            .write_all(b"HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\n")
            .await;
        return Ok(());
    }

    // DNS 解析
    let dst = match resolve_ipv4(&dst_host, dst_port).await {
        Ok(v4) => v4,
        Err(e) => {
            engine.log(format!("[HTTP DNS失败] {dst_host}:{dst_port} -- {e}"));
            let _ = client
                .write_all(b"HTTP/1.1 502 Bad Gateway\r\nConnection: close\r\n\r\n")
                .await;
            return Ok(());
        }
    };

    // 调度 + 物理绑定连接
    let nic = engine.next_nic();
    nic.active.fetch_add(1, Ordering::Relaxed);
    let _guard = ConnGuard(nic.clone());
    engine.log(format!(
        "[HTTP 调度分配] 新连接 -> [{}] | 目标: {}({}):{}",
        nic.name, dst_host, dst.ip(), dst_port
    ));

    let mut upstream = match connect_via_nic(&nic, dst).await {
        Ok(s) => s,
        Err(e) => {
            engine.log(format!("[HTTP 连通失败] {dst_host}:{dst_port} -- {e}"));
            let _ = client
                .write_all(b"HTTP/1.1 502 Bad Gateway\r\nConnection: close\r\n\r\n")
                .await;
            return Ok(());
        }
    };

    if is_connect {
        client
            .write_all(b"HTTP/1.1 200 Connection Established\r\nProxy-Agent: HypoMuxPlus\r\n\r\n")
            .await?;
    } else if let Some(hdr) = outbound_header {
        upstream.write_all(&hdr).await?;
    }

    let _ = copy_bidirectional(&mut client, &mut upstream).await;
    Ok(())
}

fn find_header(lines: &[&str], name: &str) -> String {
    let prefix = format!("{}:", name.to_lowercase());
    for line in lines.iter().skip(1) {
        if line.to_lowercase().starts_with(&prefix) {
            return line.splitn(2, ':').nth(1).unwrap_or("").trim().to_string();
        }
    }
    String::new()
}

fn build_origin_header(method: &str, path: &str, version: &str, lines: &[&str]) -> Vec<u8> {
    let hop = ["proxy-connection", "proxy-authorization"];
    let mut out = String::new();
    out.push_str(&format!("{method} {path} {version}\r\n"));
    for line in lines.iter().skip(1) {
        if line.is_empty() {
            continue;
        }
        let name = line.splitn(2, ':').next().unwrap_or("").trim().to_lowercase();
        if hop.contains(&name.as_str()) {
            continue;
        }
        out.push_str(line);
        out.push_str("\r\n");
    }
    out.push_str("\r\n");
    out.into_bytes()
}

fn split_host_port(value: &str, default_port: u16) -> (String, u16) {
    let host = value.trim();
    if host.is_empty() || host.starts_with('[') {
        return (String::new(), 0);
    }
    if let Some((h, p)) = host.rsplit_once(':') {
        // 避免把 IPv6 误判；这里只处理 host:port 形式
        if let Ok(port) = p.parse::<u16>() {
            return (h.trim().to_string(), port);
        }
        return (String::new(), 0);
    }
    (host.to_string(), default_port)
}

// ============================== 遥测 ==============================

async fn telemetry_loop(app: AppHandle, nics: Vec<Arc<NicRuntime>>, cancel: CancellationToken) {
    let mut last: Vec<(u64, u64)> = nics
        .iter()
        .map(|n| crate::telemetry::read_octets(n.if_index))
        .collect();

    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {}
        }

        let mut per_nic = Vec::with_capacity(nics.len());
        let mut total_down = 0.0;
        let mut total_up = 0.0;
        let mut total_conn = 0i64;

        for (i, nic) in nics.iter().enumerate() {
            let (recv, sent) = crate::telemetry::read_octets(nic.if_index);
            let (lr, ls) = last[i];
            let down = recv.saturating_sub(lr) as f64 / 1024.0 / 1024.0;
            let up = sent.saturating_sub(ls) as f64 / 1024.0 / 1024.0;
            last[i] = (recv, sent);

            let conn = nic.active.load(Ordering::Relaxed).max(0);
            let down = (down * 100.0).round() / 100.0;
            let up = (up * 100.0).round() / 100.0;

            // 写入实时速度，供按速度加权调度（WeightedSpeed）参考
            nic.speed.store((down * 100.0) as u64, Ordering::Relaxed);

            total_down += down;
            total_up += up;
            total_conn += conn;

            per_nic.push(NicTelemetry {
                index: nic.if_index,
                name: nic.name.clone(),
                down_mbps: down,
                up_mbps: up,
                connections: conn,
            });
        }

        let payload = TelemetryPayload {
            per_nic,
            total: TotalTelemetry {
                down_mbps: (total_down * 100.0).round() / 100.0,
                up_mbps: (total_up * 100.0).round() / 100.0,
                connections: total_conn,
            },
        };
        let _ = app.emit("hmx-telemetry", payload);
    }
}
