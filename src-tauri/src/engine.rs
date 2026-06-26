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

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use socket2::{Domain, Protocol, Socket, Type};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
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
    /// 调度权重（默认 100；越大分到越多连接）
    #[serde(default)]
    pub weight: Option<u32>,
    /// 单卡下行限速（MB/s，0/缺省=不限速）
    #[serde(default)]
    pub limit_mbps: Option<f64>,
}

/// 前端下发的分流规则：pattern 为域名（支持子域、可带 :port），action 为
/// "direct"(直连) / "aggregate"(走聚合，默认) / "nic:<ifindex>"(钉死到指定网卡)。
#[derive(Debug, Clone, Deserialize)]
pub struct RouteRuleDef {
    pub pattern: String,
    pub action: String,
}

/// 单张网卡的运行时状态
pub struct NicRuntime {
    pub name: String,
    pub ip: Ipv4Addr,
    pub if_index: u32,
    pub active: AtomicI64,
    /// 最近一秒下行速率（MB/s × 100），供按速度加权调度使用
    pub speed: AtomicU64,
    /// 是否在线（掉线守护：失联时置 false 并移出调度轮换）
    pub alive: AtomicBool,
    /// 用户设定的调度权重（默认 100）
    pub weight: u32,
    /// 单卡下行限速器（None=不限速，回退全局限速）
    pub limiter: Option<Arc<RateLimiter>>,
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
    fn label_en(&self) -> &'static str {
        match self {
            Strategy::RoundRobin => "Round-Robin",
            Strategy::LeastConn => "Least-Connections",
            Strategy::WeightedSpeed => "Weighted-Speed",
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

/// 网卡上下线告警（掉线守护推送给前端）
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct NicAlert {
    name: String,
    alive: bool,
}

/// 全局下行令牌桶限速器：限制所有连接合计的下载速率（字节/秒）。
/// 仅在用户设置了限速（>0）时启用，cap=0 时引擎走零开销的直通中继。
struct RateLimiter {
    rate: f64,            // 每秒补充的令牌（字节）
    capacity: f64,        // 桶容量（允许 1 秒突发）
    tokens: Mutex<f64>,
    last: Mutex<std::time::Instant>,
}

impl RateLimiter {
    fn new(bytes_per_sec: u64) -> Self {
        let r = bytes_per_sec as f64;
        Self {
            rate: r,
            capacity: r,
            tokens: Mutex::new(r),
            last: Mutex::new(std::time::Instant::now()),
        }
    }

    /// 获取 want 字节的下载额度；不足时返回需等待的时长，调用方 sleep 后重试。
    fn try_take(&self, want: f64) -> Result<(), std::time::Duration> {
        let mut tokens = match self.tokens.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        {
            let mut last = match self.last.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            let now = std::time::Instant::now();
            let elapsed = now.duration_since(*last).as_secs_f64();
            *last = now;
            *tokens = (*tokens + elapsed * self.rate).min(self.capacity);
        }
        if *tokens >= want {
            *tokens -= want;
            Ok(())
        } else {
            let deficit = want - *tokens;
            *tokens = 0.0;
            Err(std::time::Duration::from_secs_f64((deficit / self.rate).min(1.0)))
        }
    }

    async fn acquire(&self, want: usize) {
        let mut remaining = want as f64;
        while remaining > 0.0 {
            let chunk = remaining.min(self.capacity.max(1.0));
            loop {
                match self.try_take(chunk) {
                    Ok(()) => break,
                    Err(d) => tokio::time::sleep(d).await,
                }
            }
            remaining -= chunk;
        }
    }
}

/// 中继客户端与上游之间的双向流量。limiter 存在时对下行（上游→客户端）限速。
///
/// 采用显式双向中继：两个方向各用独立大缓冲，任一方向读到 EOF 后，
/// 对对端写半执行 flush + shutdown，正确传递半关闭（half-close）语义。
/// 这对“上传方向”至关重要：客户端上传完毕（EOF）后必须向上游发送 FIN，
/// 否则服务器会一直挂起等待请求体，导致 Speedtest 等上传测速“socket error”。
async fn relay(client: &mut TcpStream, upstream: &mut TcpStream, limiter: Option<Arc<RateLimiter>>) {
    let (mut cr, mut cw) = client.split();
    let (mut ur, mut uw) = upstream.split();

    // 下行：上游 -> 客户端（limiter 存在时限速）
    let down = async {
        let mut buf = vec![0u8; 65536];
        loop {
            let n = match ur.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            if let Some(lim) = &limiter {
                lim.acquire(n).await;
            }
            if cw.write_all(&buf[..n]).await.is_err() {
                break;
            }
        }
        // 上游已无更多数据：刷新并向客户端发送 FIN（半关闭）
        let _ = cw.flush().await;
        let _ = cw.shutdown().await;
    };

    // 上行：客户端 -> 上游（不限速，上传方向）
    let up = async {
        let mut buf = vec![0u8; 65536];
        loop {
            let n = match cr.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            if uw.write_all(&buf[..n]).await.is_err() {
                break;
            }
        }
        // 客户端上传完毕：刷新并向上游发送 FIN，避免服务器无限等待请求体
        let _ = uw.flush().await;
        let _ = uw.shutdown().await;
    };

    tokio::join!(down, up);
}

/// 活跃连接信息（实时连接列表用）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnInfo {
    pub target: String,
    pub nic: String,
    pub proto: &'static str,
}

/// 代理引擎核心，含调度器与网卡集合
pub struct Engine {
    nics: Vec<Arc<NicRuntime>>,
    rr: AtomicUsize,
    strategy: Strategy,
    /// 平滑加权轮询的动态权重累加器（仅 WeightedSpeed 使用）
    wrr: Mutex<Vec<i64>>,
    /// 活跃连接表（id -> 信息），供实时连接列表展示
    conns: Arc<Mutex<HashMap<u64, ConnInfo>>>,
    conn_id: AtomicU64,
    app: AppHandle,
    /// 日志语言：true=中文，false=英文（跟随前端界面语言）
    zh: bool,
    /// 全局下行限速器（None=不限速）
    limiter: Option<Arc<RateLimiter>>,
    /// 直连白名单（小写域名，命中则走默认网关直连、不参与分流）
    bypass: Vec<String>,
    /// 域名→指定网卡规则：(小写 pattern, 目标 if_index)，命中则钉死到该网卡
    rules_nic: Vec<(String, u32)>,
    /// DNS 解析缓存（host -> (真实IP, 解析时刻)）：经物理网卡解析后缓存，绕过 fake-ip
    dns_cache: Mutex<HashMap<String, (Ipv4Addr, std::time::Instant)>>,
}

impl Engine {
    /// 判断目标主机是否命中直连白名单（精确或子域匹配）
    fn is_bypass(&self, host: &str) -> bool {
        if self.bypass.is_empty() {
            return false;
        }
        let h = host.to_lowercase();
        self.bypass
            .iter()
            .any(|b| pattern_match(b, &h, 0))
    }

    /// 按域名→网卡规则选出指定网卡（命中且该网卡在线时），否则回退到策略调度。
    fn pick_nic(&self, host: &str, port: u16) -> Arc<NicRuntime> {
        if !self.rules_nic.is_empty() {
            let h = host.to_lowercase();
            for (pat, ifindex) in &self.rules_nic {
                if pattern_match(pat, &h, port) {
                    if let Some(n) = self
                        .nics
                        .iter()
                        .find(|n| n.if_index == *ifindex && n.alive.load(Ordering::Relaxed))
                    {
                        return n.clone();
                    }
                }
            }
        }
        self.next_nic()
    }
}

/// 规则匹配：pattern 可为 "域名" 或 "域名:port"。域名支持精确 / 子域 / `*` 通配。
fn pattern_match(pattern: &str, host: &str, port: u16) -> bool {
    let (pat_host, pat_port) = match pattern.rsplit_once(':') {
        Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) && !p.is_empty() => {
            (h, p.parse::<u16>().ok())
        }
        _ => (pattern, None),
    };
    if let Some(pp) = pat_port {
        if port != 0 && pp != port {
            return false;
        }
    }
    let pat_host = pat_host.trim_start_matches("*.").trim();
    if pat_host == "*" || pat_host.is_empty() {
        return pat_port.is_some();
    }
    host == pat_host || host.ends_with(&format!(".{pat_host}"))
}

impl Engine {
    /// 经所选物理网卡解析目标域名：DoH(443) → UDP 直发(53) → 系统解析，并缓存 60s。
    /// 关键：绕过 Clash/Mihomo 的 fake-ip DNS 劫持，确保每条连接拿到真实公网 IP 后
    /// 物理绑定到各自网卡直连出网，避免多网卡塌缩为单一上游。
    async fn resolve_host(&self, nic: &NicRuntime, host: &str, port: u16) -> Option<SocketAddrV4> {
        if let Ok(ip) = host.parse::<Ipv4Addr>() {
            return Some(SocketAddrV4::new(ip, port));
        }
        if let Ok(cache) = self.dns_cache.lock() {
            if let Some((ip, t)) = cache.get(host) {
                if t.elapsed() < std::time::Duration::from_secs(60) {
                    return Some(SocketAddrV4::new(*ip, port));
                }
            }
        }
        let ip = match resolve_via_doh(nic, host).await {
            Some(ip) => ip,
            None => match resolve_via_nic(nic, host).await {
                Some(ip) => ip,
                None => match resolve_ipv4(host, port).await {
                    Ok(v4) => *v4.ip(),
                    Err(_) => return None,
                },
            },
        };
        if let Ok(mut cache) = self.dns_cache.lock() {
            cache.insert(host.to_string(), (ip, std::time::Instant::now()));
        }
        Some(SocketAddrV4::new(ip, port))
    }

    fn next_nic(&self) -> Arc<NicRuntime> {
        // 仅在存活网卡间调度；若全部掉线则回退到全部，保证仍有出口可用
        let alive: Vec<usize> = (0..self.nics.len())
            .filter(|&i| self.nics[i].alive.load(Ordering::Relaxed))
            .collect();
        let pool: Vec<usize> = if alive.is_empty() {
            (0..self.nics.len()).collect()
        } else {
            alive
        };

        match self.strategy {
            Strategy::RoundRobin => {
                // 加权平滑轮询：默认权重相等时等价于经典轮询；权重不同则按比例倾斜
                self.swrr_pick(&pool, |n| n.weight as i64)
            }
            Strategy::LeastConn => {
                // 最少连接：按 活跃连接数 / 权重 归一，权重大的可承载更多连接
                let mut best = pool[0];
                let mut best_v = f64::MAX;
                for &i in &pool {
                    let w = self.nics[i].weight.max(1) as f64;
                    let v = self.nics[i].active.load(Ordering::Relaxed) as f64 / w;
                    if v < best_v {
                        best_v = v;
                        best = i;
                    }
                }
                self.nics[best].clone()
            }
            Strategy::WeightedSpeed => {
                // 平滑加权轮询：eff = (实时速度 + 基值) × 用户权重，速度快 + 权重高者多分流量
                self.swrr_pick(&pool, |n| {
                    (n.speed.load(Ordering::Relaxed) as i64 + 100) * n.weight.max(1) as i64 / 100
                })
            }
        }
    }

    /// 平滑加权轮询（nginx SWRR）：按 eff 权重在 pool 内选出本次网卡。
    fn swrr_pick(&self, pool: &[usize], eff_of: impl Fn(&NicRuntime) -> i64) -> Arc<NicRuntime> {
        let mut cur = match self.wrr.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        let total: i64 = pool.iter().map(|&i| eff_of(&self.nics[i]).max(1)).sum();
        let mut best = pool[0];
        let mut best_v = i64::MIN;
        for &i in pool {
            cur[i] += eff_of(&self.nics[i]).max(1);
            if cur[i] > best_v {
                best_v = cur[i];
                best = i;
            }
        }
        cur[best] -= total;
        self.nics[best].clone()
    }

    fn log(&self, msg: impl Into<String>) {
        let _ = self.app.emit("hmx-log", msg.into());
    }

    fn register_conn(&self, target: String, nic: String, proto: &'static str) -> ConnTableGuard {
        let id = self.conn_id.fetch_add(1, Ordering::Relaxed);
        let info = ConnInfo { target, nic, proto };
        if let Ok(mut map) = self.conns.lock() {
            map.insert(id, info.clone());
        }
        ConnTableGuard {
            conns: self.conns.clone(),
            id,
            app: self.app.clone(),
            info,
        }
    }
}

/// 活跃连接表 RAII 守卫：drop 时移除该连接记录，并推送一条"已结束连接"用于历史留存
struct ConnTableGuard {
    conns: Arc<Mutex<HashMap<u64, ConnInfo>>>,
    id: u64,
    app: AppHandle,
    info: ConnInfo,
}
impl Drop for ConnTableGuard {
    fn drop(&mut self) {
        if let Ok(mut map) = self.conns.lock() {
            map.remove(&self.id);
        }
        let _ = self.app.emit("hmx-conn-closed", self.info.clone());
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
    lang: String,
    down_limit_mbps: f64,
    bypass: Vec<String>,
    rules: Vec<RouteRuleDef>,
) -> Result<EngineHandle, String> {
    if selected.is_empty() {
        return Err("至少需要选择一张网卡".into());
    }
    let strategy = Strategy::parse(&strategy);
    let zh = lang != "en";
    // 下行限速：MB/s 转字节/秒；<=0 表示不限速
    let limiter = if down_limit_mbps > 0.0 {
        Some(Arc::new(RateLimiter::new(
            (down_limit_mbps * 1024.0 * 1024.0) as u64,
        )))
    } else {
        None
    };
    // 规整白名单：去空白、转小写、去空项
    let mut bypass: Vec<String> = bypass
        .into_iter()
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();

    // 解析分流规则：direct → 并入直连白名单；nic:<ifindex> → 域名钉死到指定网卡；aggregate → 默认
    let mut rules_nic: Vec<(String, u32)> = Vec::new();
    for r in &rules {
        let pat = r.pattern.trim().to_lowercase();
        if pat.is_empty() {
            continue;
        }
        let act = r.action.trim().to_lowercase();
        if act == "direct" {
            bypass.push(pat);
        } else if let Some(idx) = act.strip_prefix("nic:") {
            if let Ok(ifindex) = idx.trim().parse::<u32>() {
                rules_nic.push((pat, ifindex));
            }
        }
        // "aggregate" 即默认行为，无需处理
    }

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
            alive: AtomicBool::new(true),
            weight: s.weight.unwrap_or(100).clamp(1, 10_000),
            limiter: match s.limit_mbps {
                Some(m) if m > 0.0 => Some(Arc::new(RateLimiter::new((m * 1024.0 * 1024.0) as u64))),
                _ => None,
            },
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
        conns: Arc::new(Mutex::new(HashMap::new())),
        conn_id: AtomicU64::new(0),
        app: app.clone(),
        zh,
        limiter,
        bypass,
        rules_nic,
        dns_cache: Mutex::new(HashMap::new()),
    });

    let nic_names: Vec<&str> = nics.iter().map(|n| n.name.as_str()).collect();
    engine.log(if zh {
        format!(
            "[HypoMux] SOCKS5+HTTP 分流引擎已启动 | SOCKS 127.0.0.1:{socks_port} | HTTP 127.0.0.1:{http_port} | 调度策略: {} | 参与分流网卡: {}",
            strategy.label(),
            nic_names.join(", ")
        )
    } else {
        format!(
            "[HypoMux] SOCKS5+HTTP splitting engine started | SOCKS 127.0.0.1:{socks_port} | HTTP 127.0.0.1:{http_port} | strategy: {} | adapters: {}",
            strategy.label_en(),
            nic_names.join(", ")
        )
    });

    // 网卡出网自检：逐张测试能否经该网卡独立连通公网，结果写入调度日志，
    // 便于排查"多网卡只跑一张卡"——若某卡显示"失败"，说明它无法独立出网，分流到它的流量会失败。
    {
        let nics2 = nics.clone();
        let app2 = app.clone();
        let zh2 = zh;
        tauri::async_runtime::spawn(async move {
            let target = SocketAddrV4::new(Ipv4Addr::new(223, 5, 5, 5), 443);
            for n in &nics2 {
                let ok = matches!(
                    tokio::time::timeout(std::time::Duration::from_secs(4), connect_via_nic(n, target)).await,
                    Ok(Ok(_))
                );
                let _ = app2.emit(
                    "hmx-log",
                    if zh2 {
                        if ok {
                            format!("[网卡自检] {} 独立出网正常，可参与分流", n.name)
                        } else {
                            format!("[网卡自检] {} 无法独立连到公网！分流到它的流量会失败——请检查该网卡是否真的有独立的上网出口", n.name)
                        }
                    } else if ok {
                        format!("[NIC self-test] {} egress OK", n.name)
                    } else {
                        format!("[NIC self-test] {} CANNOT reach the internet independently — traffic routed to it will fail", n.name)
                    },
                );
            }
        });
    }

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
        let conns = engine.conns.clone();
        tauri::async_runtime::spawn(async move {
            telemetry_loop(app2, nics, conns, c, zh).await;
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
            alive: AtomicBool::new(true),
            weight: 100,
            limiter: None,
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

/// 单张网卡测速结果（MB/s）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeedResult {
    pub index: u32,
    pub name: String,
    pub mbps: f64,
    pub ok: bool,
}

/// 测速候选节点：(host, path, port)。port=443 走 HTTPS，port=80 走明文 HTTP。
/// 多目标 + 明文兜底：当网关对外 HTTPS 做拦截/重置或 TLS 中间人时，HTTP(80) 仍可能跑通。
struct BenchTarget {
    host: &'static str,
    path: &'static str,
    port: u16,
}
const BENCH_TARGETS: &[BenchTarget] = &[
    // 国内公共镜像优先：阿里云 / 清华 / 腾讯，国内极快、HTTP/HTTPS 均可达。
    // Contents-amd64.gz 是 Ubuntu 镜像 dists 下恒久存在的大文件（约 40MB），适合吞吐采样。
    BenchTarget { host: "mirrors.aliyun.com", path: "/ubuntu/dists/jammy/Contents-amd64.gz", port: 80 },
    BenchTarget { host: "mirrors.aliyun.com", path: "/ubuntu/dists/jammy/Contents-amd64.gz", port: 443 },
    BenchTarget { host: "mirrors.tuna.tsinghua.edu.cn", path: "/ubuntu/dists/jammy/Contents-amd64.gz", port: 443 },
    BenchTarget { host: "mirrors.cloud.tencent.com", path: "/ubuntu/dists/jammy/Contents-amd64.gz", port: 80 },
    BenchTarget { host: "mirrors.ustc.edu.cn", path: "/ubuntu/dists/jammy/Contents-amd64.gz", port: 443 },
    // 教育网 / 国际兜底
    BenchTarget { host: "test.ustc.edu.cn", path: "/backend/garbage.php?ckSize=512", port: 443 },
    BenchTarget { host: "speed.cloudflare.com", path: "/__down?bytes=104857600", port: 443 },
    BenchTarget { host: "speedtest.tele2.net", path: "/100MB.zip", port: 80 },
];
const BENCH_PARALLEL: usize = 6;

/// DoH 解析器：(字面 IP, SNI/Host)。走 443 的 HTTPS，绕过 TUN 的 53 端口 DNS 劫持。
const DOH_RESOLVERS: &[(&str, &str)] = &[
    ("1.1.1.1", "cloudflare-dns.com"),
    ("223.5.5.5", "dns.alidns.com"),
    ("1.0.0.1", "cloudflare-dns.com"),
];

fn tls_connector() -> tokio_rustls::TlsConnector {
    use std::sync::OnceLock;
    use tokio_rustls::rustls::{ClientConfig, RootCertStore};
    static CFG: OnceLock<Arc<ClientConfig>> = OnceLock::new();
    let cfg = CFG
        .get_or_init(|| {
            let mut roots = RootCertStore::empty();
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            Arc::new(ClientConfig::builder().with_root_certificates(roots).with_no_client_auth())
        })
        .clone();
    tokio_rustls::TlsConnector::from(cfg)
}

/// 逐张网卡跑分：经各网卡多条并发连接从测速节点下载，测真实聚合吞吐（MB/s）。
/// 所有网卡**并发**测试，既加速诊断，也支撑控制台"一键聚合测速"的同时跑分。
pub async fn speed_test(app: AppHandle, selected: Vec<SelectedNic>, duration_secs: u64) -> Vec<SpeedResult> {
    let dur = std::time::Duration::from_secs(duration_secs.clamp(2, 15));
    let mut handles = Vec::with_capacity(selected.len());
    for s in selected {
        let app2 = app.clone();
        handles.push(tauri::async_runtime::spawn(async move {
            let r = match bench_one(&app2, &s, dur).await {
                Some(mbps) => SpeedResult { index: s.index, name: s.name.clone(), mbps, ok: true },
                None => SpeedResult { index: s.index, name: s.name.clone(), mbps: 0.0, ok: false },
            };
            let _ = app2.emit("hmx-speedtest", r.clone());
            r
        }));
    }
    let mut out = Vec::with_capacity(handles.len());
    for h in handles {
        if let Ok(r) = h.await {
            out.push(r);
        }
    }
    out
}

/// 收集候选 IP：优先系统解析（非 fake-ip 即用），失败再 DoH(443) → UDP(53)。
async fn resolve_candidates(nic: &NicRuntime, host: &str) -> Vec<Ipv4Addr> {
    if let Ok(ip) = host.parse::<Ipv4Addr>() {
        return vec![ip];
    }
    let mut ips: Vec<Ipv4Addr> = Vec::new();
    if let Ok(Ok(d)) = tokio::time::timeout(std::time::Duration::from_secs(3), resolve_ipv4(host, 443)).await {
        let ip = *d.ip();
        let o = ip.octets();
        let fake = o[0] == 198 && (o[1] == 18 || o[1] == 19);
        if !fake {
            ips.push(ip);
        }
    }
    if ips.is_empty() {
        if let Some(ip) = resolve_via_doh(nic, host).await {
            ips.push(ip);
        }
    }
    if ips.is_empty() {
        if let Some(ip) = resolve_via_nic(nic, host).await {
            ips.push(ip);
        }
    }
    ips
}

async fn bench_one(app: &AppHandle, s: &SelectedNic, dur: std::time::Duration) -> Option<f64> {
    let ip: Ipv4Addr = s.ip.parse().ok()?;
    let nic = Arc::new(NicRuntime {
        name: s.name.clone(),
        ip,
        if_index: s.index,
        active: AtomicI64::new(0),
        speed: AtomicU64::new(0),
        alive: AtomicBool::new(true),
        weight: 100,
        limiter: None,
    });
    let log = |m: String| {
        let _ = app.emit("hmx-log", m);
    };

    // 逐个候选节点探测：先解析候选 IP，再连通 + (TLS) + HTTP 200/206 校验
    let mut chosen: Option<(SocketAddrV4, &'static str, &'static str, bool)> = None;
    'outer: for t in BENCH_TARGETS {
        let tls = t.port == 443;
        let ips = resolve_candidates(&nic, t.host).await;
        if ips.is_empty() {
            log(format!("[测速] [{}] 无法解析 {}，跳过", nic.name, t.host));
            continue;
        }
        for ip in ips {
            let dst = SocketAddrV4::new(ip, t.port);
            let (ok, info) = probe_target(&nic, dst, t.host, t.path, tls).await;
            if ok {
                log(format!("[测速] [{}] 选用节点 {}:{} ({}) [{}]", nic.name, t.host, t.port, ip, info));
                chosen = Some((dst, t.host, t.path, tls));
                break 'outer;
            }
            log(format!("[测速] [{}] 节点 {}:{} 不可用: {}", nic.name, t.host, t.port, info));
        }
    }
    let (dst, host, path, tls) = match chosen {
        Some(c) => c,
        None => {
            log(format!("[测速] [{}] 所有测速节点均不可达（可能被网关阻断）", nic.name));
            return None;
        }
    };

    let total = Arc::new(AtomicU64::new(0));
    let mut handles = Vec::with_capacity(BENCH_PARALLEL);
    for _ in 0..BENCH_PARALLEL {
        let nic = nic.clone();
        let total = total.clone();
        handles.push(tauri::async_runtime::spawn(async move {
            let _ = bench_conn(&nic, dst, host, path, tls, dur, &total).await;
        }));
    }
    for h in handles {
        let _ = tokio::time::timeout(dur + std::time::Duration::from_secs(8), h).await;
    }

    // 吞吐 = 总下载字节 / 测速窗口时长
    let bytes = total.load(Ordering::Relaxed);
    let secs = dur.as_secs_f64().max(0.001);
    if bytes == 0 {
        log(format!("[测速] [{}] 已连通但未下行任何数据", nic.name));
        return None;
    }
    Some(bytes as f64 / 1024.0 / 1024.0 / secs)
}

/// 通用：在已建立的流上发 GET 并读首包，返回 HTTP 首行（状态行）。
async fn http_probe_line<S>(mut s: S, host: &str, path: &str) -> Option<String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let req = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: HypoMuxPlus\r\nAccept: */*\r\nConnection: close\r\n\r\n"
    );
    if s.write_all(req.as_bytes()).await.is_err() {
        return None;
    }
    let mut buf = [0u8; 4096];
    match s.read(&mut buf).await {
        Ok(n) if n > 0 => {
            let head = String::from_utf8_lossy(&buf[..n]);
            Some(head.lines().next().unwrap_or("").trim().to_string())
        }
        _ => None,
    }
}

/// 通用：在已建立的流上发 GET 并持续读取累加字节，直到时间窗结束。
async fn http_pump_stream<S>(mut s: S, host: &str, path: &str, dur: std::time::Duration, total: &AtomicU64) -> Option<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let req = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: HypoMuxPlus\r\nAccept: */*\r\nConnection: close\r\n\r\n"
    );
    s.write_all(req.as_bytes()).await.ok()?;
    let start = std::time::Instant::now();
    let mut buf = vec![0u8; 65536];
    loop {
        let elapsed = start.elapsed();
        if elapsed >= dur {
            break;
        }
        match tokio::time::timeout(dur - elapsed, s.read(&mut buf)).await {
            Ok(Ok(0)) => break,
            Ok(Ok(n)) => {
                total.fetch_add(n as u64, Ordering::Relaxed);
            }
            _ => break,
        }
    }
    Some(())
}

/// 经网卡探测候选节点：连通 +（按需 TLS）+ 取 HTTP 状态行。
/// 返回 (是否 200/206 可用, 诊断信息：状态行或失败原因)。
async fn probe_target(nic: &NicRuntime, dst: SocketAddrV4, host: &str, path: &str, tls: bool) -> (bool, String) {
    let fut = async {
        let tcp = match connect_via_nic(nic, dst).await {
            Ok(t) => t,
            Err(e) => return format!("连接失败: {e}"),
        };
        let line = if tls {
            let connector = tls_connector();
            let server_name = match tokio_rustls::rustls::pki_types::ServerName::try_from(host.to_string()) {
                Ok(n) => n,
                Err(_) => return "无效 SNI".to_string(),
            };
            match connector.connect(server_name, tcp).await {
                Ok(stream) => http_probe_line(stream, host, path).await,
                Err(e) => return format!("TLS 失败: {e}"),
            }
        } else {
            http_probe_line(tcp, host, path).await
        };
        line.unwrap_or_else(|| "无响应".to_string())
    };
    match tokio::time::timeout(std::time::Duration::from_secs(6), fut).await {
        Ok(info) => {
            let ok = info.contains(" 200") || info.contains(" 206");
            (ok, info)
        }
        Err(_) => (false, "探测超时".to_string()),
    }
}

/// 单条下载连接，绑定到指定网卡，持续读取并累加字节数（HTTPS 或明文 HTTP）。
async fn bench_conn(
    nic: &NicRuntime,
    dst: SocketAddrV4,
    host: &str,
    path: &str,
    tls: bool,
    dur: std::time::Duration,
    total: &AtomicU64,
) -> Option<()> {
    let tcp = tokio::time::timeout(std::time::Duration::from_secs(6), connect_via_nic(nic, dst))
        .await
        .ok()?
        .ok()?;
    if tls {
        let connector = tls_connector();
        let server_name = tokio_rustls::rustls::pki_types::ServerName::try_from(host.to_string()).ok()?;
        let stream = tokio::time::timeout(std::time::Duration::from_secs(6), connector.connect(server_name, tcp))
            .await
            .ok()?
            .ok()?;
        http_pump_stream(stream, host, path, dur, total).await
    } else {
        http_pump_stream(tcp, host, path, dur, total).await
    }
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
                                            let pfx = if eng.zh { "[连接异常] " } else { "[Connection error] " };
                                            eng.log(format!("{pfx}{s}"));
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

/// 构造一个 DNS A 记录查询报文。
fn build_dns_query(host: &str) -> Vec<u8> {
    let mut q = Vec::with_capacity(host.len() + 18);
    q.extend_from_slice(&[0x12, 0x34]); // ID
    q.extend_from_slice(&[0x01, 0x00]); // 标志：递归查询
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
    q.extend_from_slice(&[0x00, 0x01]); // QTYPE = A
    q.extend_from_slice(&[0x00, 0x01]); // QCLASS = IN
    q
}

/// 跳过 DNS 报文中的域名字段（处理压缩指针）。
fn dns_skip_name(buf: &[u8], mut pos: usize) -> Option<usize> {
    loop {
        let len = *buf.get(pos)?;
        if len & 0xC0 == 0xC0 {
            return Some(pos + 2);
        }
        if len == 0 {
            return Some(pos + 1);
        }
        pos += 1 + len as usize;
    }
}

/// 从 DNS 响应中解析首个 A 记录。
fn parse_dns_a(buf: &[u8]) -> Option<Ipv4Addr> {
    if buf.len() < 12 {
        return None;
    }
    let qd = u16::from_be_bytes([buf[4], buf[5]]) as usize;
    let an = u16::from_be_bytes([buf[6], buf[7]]) as usize;
    let mut pos = 12;
    for _ in 0..qd {
        pos = dns_skip_name(buf, pos)?;
        pos += 4;
    }
    for _ in 0..an {
        pos = dns_skip_name(buf, pos)?;
        if pos + 10 > buf.len() {
            return None;
        }
        let rtype = u16::from_be_bytes([buf[pos], buf[pos + 1]]);
        let rdlen = u16::from_be_bytes([buf[pos + 8], buf[pos + 9]]) as usize;
        pos += 10;
        if pos + rdlen > buf.len() {
            return None;
        }
        if rtype == 1 && rdlen == 4 {
            return Some(Ipv4Addr::new(buf[pos], buf[pos + 1], buf[pos + 2], buf[pos + 3]));
        }
        pos += rdlen;
    }
    None
}

/// 经指定网卡向真实公共 DNS（223.5.5.5）直接发起 UDP 查询解析域名。
/// 用 IP_UNICAST_IF 把查询钉死在物理网卡上，绕过本地 DNS 劫持 / 代理 fake-ip，
/// 拿到真实公网 IP，避免测速误连到不可路由的假地址而超时。
async fn resolve_via_nic(nic: &NicRuntime, host: &str) -> Option<Ipv4Addr> {
    if let Ok(ip) = host.parse::<Ipv4Addr>() {
        return Some(ip);
    }
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP)).ok()?;
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
            return None;
        }
    }
    let bind_addr: socket2::SockAddr = SocketAddr::new(IpAddr::V4(nic.ip), 0).into();
    socket.bind(&bind_addr).ok()?;
    socket.set_nonblocking(true).ok()?;
    let std_udp: std::net::UdpSocket = socket.into();
    let udp = tokio::net::UdpSocket::from_std(std_udp).ok()?;
    let query = build_dns_query(host);
    udp.send_to(&query, "223.5.5.5:53").await.ok()?;
    let mut buf = [0u8; 512];
    let n = tokio::time::timeout(std::time::Duration::from_secs(3), udp.recv(&mut buf))
        .await
        .ok()?
        .ok()?;
    parse_dns_a(&buf[..n])
}

/// 经指定网卡用 DoH（DNS over HTTPS，443 端口）解析域名。
/// 关键：走 HTTPS 到字面解析器 IP，TUN/fake-ip 只劫持 53 端口 DNS，无法干预此查询，
/// 因此能在 Clash/Mihomo TUN 模式下拿到真实公网 IP，根治"吞吐全超时"。
async fn resolve_via_doh(nic: &NicRuntime, host: &str) -> Option<Ipv4Addr> {
    if let Ok(ip) = host.parse::<Ipv4Addr>() {
        return Some(ip);
    }
    let query = build_dns_query(host);
    for (rip, rhost) in DOH_RESOLVERS {
        let ip: Ipv4Addr = match rip.parse() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let dst = SocketAddrV4::new(ip, 443);
        let fut = async {
            let tcp = connect_via_nic(nic, dst).await.ok()?;
            let connector = tls_connector();
            let server_name = tokio_rustls::rustls::pki_types::ServerName::try_from(rhost.to_string()).ok()?;
            let mut tls = connector.connect(server_name, tcp).await.ok()?;
            let head = format!(
                "POST /dns-query HTTP/1.1\r\nHost: {rhost}\r\nAccept: application/dns-message\r\nContent-Type: application/dns-message\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                query.len()
            );
            tls.write_all(head.as_bytes()).await.ok()?;
            tls.write_all(&query).await.ok()?;
            // 读取完整响应（DoH POST 响应体通常带 Content-Length，非 chunked）
            let mut resp: Vec<u8> = Vec::with_capacity(1024);
            let mut buf = [0u8; 2048];
            loop {
                let n = tls.read(&mut buf).await.ok()?;
                if n == 0 {
                    break;
                }
                resp.extend_from_slice(&buf[..n]);
                if resp.len() > 16384 {
                    break;
                }
            }
            let sep = resp.windows(4).position(|w| w == b"\r\n\r\n")?;
            let head_str = String::from_utf8_lossy(&resp[..sep]);
            let first = head_str.lines().next().unwrap_or("");
            if !first.contains(" 200") {
                return None;
            }
            let mut body = &resp[sep + 4..];
            // 兼容分块传输：去掉首个 chunk-size 行
            if head_str.to_ascii_lowercase().contains("transfer-encoding: chunked") {
                if let Some(p) = body.windows(2).position(|w| w == b"\r\n") {
                    body = &body[p + 2..];
                }
            }
            parse_dns_a(body)
        };
        if let Ok(Some(ip)) = tokio::time::timeout(std::time::Duration::from_secs(5), fut).await {
            return Some(ip);
        }
    }
    None
}

/// 直连：不绑定物理网卡，交由系统默认网关连接（用于白名单命中的目标）。
async fn connect_direct(dst: SocketAddrV4) -> std::io::Result<TcpStream> {
    let stream = TcpStream::connect(SocketAddr::V4(dst)).await?;
    let _ = stream.set_nodelay(true);
    Ok(stream)
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

    // 目标显示名：域名优先，否则用字面 IP
    let target_display = domain
        .clone()
        .unwrap_or_else(|| literal_ip.map(|i| i.to_string()).unwrap_or_default());

    // 白名单命中：走默认网关直连，不参与多网卡分流（用系统解析即可）
    if engine.is_bypass(&target_display) {
        let dst = if let Some(ip) = literal_ip {
            SocketAddrV4::new(ip, port)
        } else {
            match resolve_ipv4(&target_display, port).await {
                Ok(v4) => v4,
                Err(e) => {
                    engine.log(if engine.zh {
                        format!("[DNS失败] 无法解析域名 {target_display}: {e}")
                    } else {
                        format!("[DNS failed] cannot resolve {target_display}: {e}")
                    });
                    client.write_all(&[0x05, 0x04, 0x00, 0x01, 0, 0, 0, 0, 0, 0]).await?;
                    return Ok(());
                }
            }
        };
        let _ctg = engine.register_conn(format!("{target_display}:{port}"), "Direct".to_string(), "SOCKS");
        engine.log(if engine.zh {
            format!("[直连] 白名单命中 -> 默认网关 | 目标: {target_display}:{port}")
        } else {
            format!("[Direct] bypass match -> default gateway | target: {target_display}:{port}")
        });
        match connect_direct(dst).await {
            Ok(mut upstream) => {
                client
                    .write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                    .await?;
                relay(&mut client, &mut upstream, engine.limiter.clone()).await;
            }
            Err(_) => {
                client
                    .write_all(&[0x05, 0x05, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                    .await?;
            }
        }
        return Ok(());
    }

    // 调度 + 物理绑定连接（先查域名→网卡规则，否则按策略调度）
    let nic = engine.pick_nic(&target_display, port);
    nic.active.fetch_add(1, Ordering::Relaxed);
    let _guard = ConnGuard(nic.clone());

    // 经所选物理网卡解析（绕过 fake-ip/TUN 劫持），失败回退系统解析
    let dst = if let Some(ip) = literal_ip {
        SocketAddrV4::new(ip, port)
    } else {
        match engine.resolve_host(&nic, &target_display, port).await {
            Some(v4) => v4,
            None => {
                engine.log(if engine.zh {
                    format!("[DNS失败] 无法解析域名 {target_display}")
                } else {
                    format!("[DNS failed] cannot resolve {target_display}")
                });
                client.write_all(&[0x05, 0x04, 0x00, 0x01, 0, 0, 0, 0, 0, 0]).await?;
                return Ok(());
            }
        }
    };

    engine.log(if engine.zh {
        format!("[调度分配] 新连接 -> [{}] | 目标: {}:{}", nic.name, target_display, port)
    } else {
        format!("[Dispatch] new connection -> [{}] | target: {}:{}", nic.name, target_display, port)
    });
    let _ctg = engine.register_conn(format!("{target_display}:{port}"), nic.name.clone(), "SOCKS");

    let mut upstream = match connect_via_nic(&nic, dst).await {
        Ok(s) => s,
        Err(e) => {
            engine.log(if engine.zh {
                format!("[连通失败] 网卡: {} 无法连接目标 {}:{}: {}", nic.name, target_display, port, e)
            } else {
                format!("[Connect failed] adapter {} cannot reach {}:{}: {}", nic.name, target_display, port, e)
            });
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

    relay(&mut client, &mut upstream, nic.limiter.clone().or_else(|| engine.limiter.clone())).await;
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

    // 白名单命中：走默认网关直连，不参与多网卡分流（系统解析即可）
    if engine.is_bypass(&dst_host) {
        let dst = match resolve_ipv4(&dst_host, dst_port).await {
            Ok(v4) => v4,
            Err(e) => {
                engine.log(if engine.zh {
                    format!("[HTTP DNS失败] {dst_host}:{dst_port} -- {e}")
                } else {
                    format!("[HTTP DNS failed] {dst_host}:{dst_port} -- {e}")
                });
                let _ = client
                    .write_all(b"HTTP/1.1 502 Bad Gateway\r\nConnection: close\r\n\r\n")
                    .await;
                return Ok(());
            }
        };
        let _ctg = engine.register_conn(format!("{dst_host}:{dst_port}"), "Direct".to_string(), "HTTP");
        engine.log(if engine.zh {
            format!("[HTTP 直连] 白名单命中 -> 默认网关 | 目标: {dst_host}:{dst_port}")
        } else {
            format!("[HTTP Direct] bypass match -> default gateway | target: {dst_host}:{dst_port}")
        });
        match connect_direct(dst).await {
            Ok(mut upstream) => {
                if is_connect {
                    client
                        .write_all(b"HTTP/1.1 200 Connection Established\r\nProxy-Agent: HypoMuxPlus\r\n\r\n")
                        .await?;
                } else if let Some(hdr) = outbound_header {
                    upstream.write_all(&hdr).await?;
                }
                relay(&mut client, &mut upstream, engine.limiter.clone()).await;
            }
            Err(_) => {
                let _ = client
                    .write_all(b"HTTP/1.1 502 Bad Gateway\r\nConnection: close\r\n\r\n")
                    .await;
            }
        }
        return Ok(());
    }

    // 调度 + 物理绑定连接（先查域名→网卡规则，否则按策略调度）
    let nic = engine.pick_nic(&dst_host, dst_port);
    nic.active.fetch_add(1, Ordering::Relaxed);
    let _guard = ConnGuard(nic.clone());

    // 经所选物理网卡解析（绕过 fake-ip/TUN 劫持），失败回退系统解析
    let dst = match engine.resolve_host(&nic, &dst_host, dst_port).await {
        Some(v4) => v4,
        None => {
            engine.log(if engine.zh {
                format!("[HTTP DNS失败] {dst_host}:{dst_port}")
            } else {
                format!("[HTTP DNS failed] {dst_host}:{dst_port}")
            });
            let _ = client
                .write_all(b"HTTP/1.1 502 Bad Gateway\r\nConnection: close\r\n\r\n")
                .await;
            return Ok(());
        }
    };

    engine.log(if engine.zh {
        format!("[HTTP 调度分配] 新连接 -> [{}] | 目标: {}({}):{}", nic.name, dst_host, dst.ip(), dst_port)
    } else {
        format!("[HTTP Dispatch] new connection -> [{}] | target: {}({}):{}", nic.name, dst_host, dst.ip(), dst_port)
    });
    let _ctg = engine.register_conn(format!("{dst_host}:{dst_port}"), nic.name.clone(), "HTTP");

    let mut upstream = match connect_via_nic(&nic, dst).await {
        Ok(s) => s,
        Err(e) => {
            engine.log(if engine.zh {
                format!("[HTTP 连通失败] {dst_host}:{dst_port} -- {e}")
            } else {
                format!("[HTTP Connect failed] {dst_host}:{dst_port} -- {e}")
            });
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

    relay(&mut client, &mut upstream, nic.limiter.clone().or_else(|| engine.limiter.clone())).await;
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

async fn telemetry_loop(
    app: AppHandle,
    nics: Vec<Arc<NicRuntime>>,
    conns: Arc<Mutex<HashMap<u64, ConnInfo>>>,
    cancel: CancellationToken,
    zh: bool,
) {
    let mut last: Vec<(u64, u64)> = nics
        .iter()
        .map(|n| crate::telemetry::read_octets(n.if_index))
        .collect();

    let mut tick: u32 = 0;

    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {}
        }

        tick = tick.wrapping_add(1);

        // 掉线守护：每 3 秒巡检一次参与分流的网卡是否仍在线（IfIndex + 绑定 IP 仍存在）
        if tick % 3 == 0 {
            if let Ok(current) = crate::netadapter::scan_adapters() {
                for nic in &nics {
                    let ip_str = nic.ip.to_string();
                    let present = current
                        .iter()
                        .any(|a| a.index == nic.if_index && a.ipv4 == ip_str);
                    let was = nic.alive.swap(present, Ordering::Relaxed);
                    if was && !present {
                        let _ = app.emit(
                            "hmx-nic-alert",
                            NicAlert { name: nic.name.clone(), alive: false },
                        );
                        let _ = app.emit(
                            "hmx-log",
                            if zh {
                                format!("[网卡掉线] {} 已失去连接，自动移出分流轮换", nic.name)
                            } else {
                                format!("[NIC down] {} lost connectivity, removed from rotation", nic.name)
                            },
                        );
                    } else if !was && present {
                        let _ = app.emit(
                            "hmx-nic-alert",
                            NicAlert { name: nic.name.clone(), alive: true },
                        );
                        let _ = app.emit(
                            "hmx-log",
                            if zh {
                                format!("[网卡恢复] {} 已恢复连接，重新纳入分流", nic.name)
                            } else {
                                format!("[NIC up] {} recovered, back in rotation", nic.name)
                            },
                        );
                    }
                }
            }
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

        // 托盘悬停提示：加速期间实时展示聚合下行速率
        if let Some(tray) = app.tray_by_id("main") {
            let tip = format!(
                "HypoMuxPlus · ↓ {:.1} MB/s · {} 连接",
                (total_down * 10.0).round() / 10.0,
                total_conn
            );
            let _ = tray.set_tooltip(Some(tip.as_str()));
        }

        // 实时连接列表快照（最多 80 条，避免高并发刷爆 webview）
        let snapshot: Vec<ConnInfo> = match conns.lock() {
            Ok(map) => map.values().take(80).cloned().collect(),
            Err(_) => Vec::new(),
        };
        let _ = app.emit("hmx-connections", snapshot);
    }

    // 停止后还原默认托盘提示
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_tooltip(Some("HypoMuxPlus · 多网卡带宽聚合工具"));
    }
}
