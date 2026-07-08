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
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
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
/// IPv6 下强制指定物理网卡出口（数值与 IPv4 的 IP_UNICAST_IF 同为 31，但 level 不同）
const IPV6_UNICAST_IF: i32 = 31;
const IPPROTO_IPV6: i32 = 41;
const WSAEWOULDBLOCK: i32 = 10035;
const MAX_HEADER_BYTES: usize = 64 * 1024;
/// 握手/请求头读取超时：客户端连上却迟迟不发协议数据时，回收该僵死连接，
/// 避免任务与套接字无限泄漏。仅覆盖握手阶段，不影响后续长时间下载中继。
const HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
/// Happy-Eyeballs 双栈拨号中每个地址族的连接超时：首选族在该时长内未建成且存在
/// 备选族时，回退尝试另一地址族。仅作用于域名目标的双栈拨号，不影响字面 IPv4 路径。
const DIAL_FAMILY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// 前端勾选并下发的网卡（index 为 scan 阶段拿到的权威 IfIndex）
#[derive(Debug, Clone, Deserialize)]
pub struct SelectedNic {
    pub index: u32,
    pub name: String,
    pub ip: String,
    /// 网卡 IPv6 源地址（可选；缺省/空表示该网卡无可用 IPv6 出口）
    #[serde(default)]
    pub ipv6: Option<String>,
    /// 调度权重（默认 100；越大分到越多连接）
    #[serde(default)]
    pub weight: Option<u32>,
    /// 单卡下行限速（MB/s，0/缺省=不限速）
    #[serde(default)]
    pub limit_mbps: Option<f64>,
}

/// 规则类型缺省值：旧配置不含 `kind` 字段时按域名规则解析（向后兼容、零迁移）。
fn default_kind() -> String {
    "domain".to_string()
}

/// 前端下发的分流规则：pattern 为域名（支持子域、可带 :port），action 为
/// "direct"(直连) / "aggregate"(走聚合，默认) / "nic:<ifindex>"(钉死到指定网卡)。
///
/// `kind` 区分规则类型：`"domain"`（默认，匹配目标域名/端口）或 `"process"`
/// （匹配发起连接的可执行文件名，如 `steam.exe`）。缺省 `kind` 视为 `"domain"`，
/// 旧配置零迁移。
#[derive(Debug, Clone, Deserialize)]
pub struct RouteRuleDef {
    pub pattern: String,
    pub action: String,
    #[serde(default = "default_kind")]
    pub kind: String,
}

/// 单张网卡的运行时状态
pub struct NicRuntime {
    pub name: String,
    /// 出口 IPv4 源地址（内部字段，非序列化）
    pub ipv4: Ipv4Addr,
    /// 出口 IPv6 源地址（None 表示该网卡无可用 IPv6 出口）
    pub ipv6: Option<Ipv6Addr>,
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
pub(crate) struct RateLimiter {
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
    /// 连接唯一自增 ID（＝发生顺序），供前端稳定排序与 key，避免列表每秒抖动
    pub id: u64,
    pub target: String,
    pub nic: String,
    pub proto: &'static str,
}

/// 代理引擎核心，含调度器与网卡集合
pub struct Engine {
    nics: Vec<Arc<NicRuntime>>,
    strategy: Strategy,
    /// 平滑加权轮询的动态权重累加器（仅 WeightedSpeed 使用）
    wrr: Mutex<Vec<i64>>,
    /// 活跃连接表（id -> 信息），供实时连接列表展示
    conns: Arc<Mutex<HashMap<u64, ConnInfo>>>,
    conn_id: AtomicU64,
    app: AppHandle,
    /// 日志语言：true=中文，false=英文（跟随前端界面语言）
    zh: bool,
    /// IP 版本偏好：{"auto","v4first","v6first","v4only"}，供双栈拨号的地址族决策使用。
    /// 默认 "auto"；后续任务（2.5）会从前端设置经 start() 参数透传覆盖此默认值。
    ip_version: String,
    /// 是否启用 SOCKS5 UDP ASSOCIATE（默认 false）。未启用时 `CMD=0x03` 走既有非 CONNECT
    /// 拒绝分支（REP=0x07）。由 `handle_socks` 的 `CMD=0x03` 处理逻辑消费。
    udp_associate: bool,
    /// 全局下行限速器（None=不限速）
    limiter: Option<Arc<RateLimiter>>,
    /// 直连白名单（小写域名，命中则走默认网关直连、不参与分流）
    bypass: Vec<String>,
    /// 域名→指定网卡规则：(小写 pattern, 目标 if_index)，命中则钉死到该网卡
    rules_nic: Vec<(String, u32)>,
    /// 进程规则：(小写可执行文件名, 动作)，命中则按动作选择出口。
    /// 优先级高于域名规则（Req 5.3）。
    rules_proc: Vec<(String, RuleAction)>,
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

    /// IP 版本偏好（"auto"/"v4first"/"v6first"/"v4only"）。
    /// `pub(crate)`：供 TUN 模式 UDP 中继在双栈候选地址上复用 `pick_family` 决策。
    pub(crate) fn ip_pref(&self) -> &str {
        &self.ip_version
    }

    /// 选出本次连接的出口网卡：规则决策（进程优先于域名，见 `decide_rule_action`）优先，
    /// 未命中任何规则或规则指向的网卡不在线 / 非 `Nic` 动作时回退调度策略（`next_nic`）。
    ///
    /// 关键不变量：当 `proc_name=None` 时，规则决策退化为「仅域名规则」，与既有行为一致
    /// （Req 5.6）；命中进程规则时以进程规则动作为准（优先级高于域名，Req 5.3）。
    ///
    /// `pub(crate)`：供 TUN 模式（`tunmode.rs`）的 UDP 中继复用同一套网卡选择逻辑
    /// （按网卡规则 / 调度策略），确保 UDP 与 TCP 出口选择一致（Req 3.4）。
    pub(crate) fn pick_nic(&self, host: &str, port: u16, proc_name: Option<&str>) -> Arc<NicRuntime> {
        if let Some(RuleAction::Nic(ifindex)) =
            decide_rule_action(&self.rules_proc, &self.rules_nic, proc_name, host, port)
        {
            if let Some(n) = self
                .nics
                .iter()
                .find(|n| n.if_index == ifindex && n.alive.load(Ordering::Relaxed))
            {
                return n.clone();
            }
        }
        // 无规则命中 / 规则为直连或聚合 / 指定网卡不在线：回退调度策略
        self.next_nic()
    }
}

/// 规则决策（纯函数）：按「进程规则优先于域名规则」给出出口动作，不含在线性 / 调度判断。
///
/// - `proc_name=Some` 且命中进程规则 => 该进程规则动作（`Nic`/`Aggregate`/`Direct`），
///   无论是否同时命中域名规则（进程规则优先，Req 5.3）。
/// - 否则按域名 `rules_nic` 首个匹配 => `Nic(ifindex)`。
/// - 均未命中 => `None`（上层回退调度策略）。
///
/// `proc_name=None` 时结果仅由域名规则决定，与「无进程规则」路径一致（Req 5.6）。
pub(crate) fn decide_rule_action(
    rules_proc: &[(String, RuleAction)],
    rules_nic: &[(String, u32)],
    proc_name: Option<&str>,
    host: &str,
    port: u16,
) -> Option<RuleAction> {
    if let Some(name) = proc_name {
        if let Some(action) = match_proc_rule(rules_proc, name) {
            return Some(action);
        }
    }
    let h = host.to_lowercase();
    for (pat, ifindex) in rules_nic {
        if pattern_match(pat, &h, port) {
            return Some(RuleAction::Nic(*ifindex));
        }
    }
    None
}

/// 平滑加权轮询选择（纯函数，nginx SWRR）：`pool` 为候选 NIC 下标，`eff[k]` 为 `pool[k]`
/// 的有效权重，`cur` 为按 NIC 下标索引的动态累加状态（跨调用持久）。返回本次选中的 NIC 下标。
///
/// 每次将各候选的有效权重累加进 `cur`，选出 `cur` 最大者，再从其中减去总权重，
/// 使长期选择比例趋近有效权重比例。
pub(crate) fn swrr_pick_index(pool: &[usize], eff: &[i64], cur: &mut [i64]) -> usize {
    let total: i64 = eff.iter().map(|&w| w.max(1)).sum();
    let mut best = pool[0];
    let mut best_v = i64::MIN;
    for (k, &i) in pool.iter().enumerate() {
        cur[i] += eff[k].max(1);
        if cur[i] > best_v {
            best_v = cur[i];
            best = i;
        }
    }
    cur[best] -= total;
    best
}

/// 最少连接选择（纯函数）：在并行数组 `active`（活跃连接数）与 `weights`（权重）上，
/// 选出 `活跃 / 权重` 最小者的位置（0..len）。权重按 `max(1)` 归一避免除零。
pub(crate) fn least_conn_pick_pos(active: &[i64], weights: &[u32]) -> usize {
    let mut best = 0usize;
    let mut best_v = f64::MAX;
    for i in 0..active.len() {
        let w = weights[i].max(1) as f64;
        let v = active[i] as f64 / w;
        if v < best_v {
            best_v = v;
            best = i;
        }
    }
    best
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

/// 分流规则动作：直连 / 走聚合 / 钉死到指定网卡（IfIndex）。
/// 供进程规则与域名规则复用（解析 / 回写双向）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuleAction {
    Direct,
    Aggregate,
    Nic(u32),
}

/// 解析规则动作字符串（纯函数）：`"direct"` / `"aggregate"` / `"nic:<ifindex>"`。
/// 与 `engine::start` 既有动作解析语义一致（trim + 小写；`nic:<n>` 需 `n` 可解析为 u32）。
/// 非法或未知形式返回 `None`。
#[allow(dead_code)]
pub(crate) fn parse_rule_action(s: &str) -> Option<RuleAction> {
    let act = s.trim().to_lowercase();
    match act.as_str() {
        "direct" => Some(RuleAction::Direct),
        "aggregate" => Some(RuleAction::Aggregate),
        _ => {
            let idx = act.strip_prefix("nic:")?;
            idx.trim().parse::<u32>().ok().map(RuleAction::Nic)
        }
    }
}

/// 将规则动作回写为字符串（纯函数）：与 `parse_rule_action` 构成 round-trip。
/// `Direct` => `"direct"`，`Aggregate` => `"aggregate"`，`Nic(n)` => `"nic:<n>"`。
#[allow(dead_code)]
pub(crate) fn rule_action_to_string(a: RuleAction) -> String {
    match a {
        RuleAction::Direct => "direct".to_string(),
        RuleAction::Aggregate => "aggregate".to_string(),
        RuleAction::Nic(n) => format!("nic:{n}"),
    }
}

/// 进程规则匹配（纯函数）：大小写不敏感精确匹配进程可执行文件名。
///
/// `rules` 中的名称在解析阶段已转为小写；此处将 `proc_name` 亦转小写后逐项精确比较，
/// 返回首个命中规则的动作；无命中返回 `None`。
pub(crate) fn match_proc_rule(rules: &[(String, RuleAction)], proc_name: &str) -> Option<RuleAction> {
    let name = proc_name.to_lowercase();
    rules
        .iter()
        .find(|(exe, _)| *exe == name)
        .map(|(_, action)| *action)
}

/// 编码 SOCKS5 `ATYP=0x04`（IPv6）地址段（纯函数）：`ATYP(1)=0x04 + ADDR(16) + PORT(2, 大端)`。
/// 与 `handle_socks` 中 IPv6 目标（ATYP=0x04）的字节布局一致，抽出以便属性测试其解析 round-trip。
#[allow(dead_code)]
pub(crate) fn build_socks5_v6_addr(addr: Ipv6Addr, port: u16) -> Vec<u8> {
    let mut out = Vec::with_capacity(19);
    out.push(0x04);
    out.extend_from_slice(&addr.octets());
    out.extend_from_slice(&port.to_be_bytes());
    out
}

/// 解析 SOCKS5 `ATYP=0x04`（IPv6）地址段（纯函数），与 [`build_socks5_v6_addr`] 互逆：
/// 校验首字节为 `0x04` 且长度足够，返回 `(Ipv6Addr, port)`；否则 `None`。
#[allow(dead_code)]
pub(crate) fn parse_socks5_v6_addr(buf: &[u8]) -> Option<(Ipv6Addr, u16)> {
    if buf.len() < 19 || buf[0] != 0x04 {
        return None;
    }
    let mut o = [0u8; 16];
    o.copy_from_slice(&buf[1..17]);
    let port = u16::from_be_bytes([buf[17], buf[18]]);
    Some((Ipv6Addr::from(o), port))
}

/// 双栈地址族。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Family {
    V4,
    V6,
}

/// 双栈地址族决策纯函数：依据 IP 版本偏好与目标可用地址族，返回按优先级排列的可尝试族列表。
///
/// - `pref ∈ {"auto","v4first","v6first","v4only"}`（`auto` 等价 `v6first`，未知值同 `auto`）。
/// - `v4only`：绝不返回 V6，仅在具备 IPv4 时返回 `[V4]`，否则 `[]`。
/// - 仅单族可用时只返回该族（`v4only` 仍排除 V6）。
/// - 双栈同在时返回两族，首位由 `pref` 决定并包含备选族用于回退。
/// - 无任何可用地址族时返回 `[]`。
pub(crate) fn pick_family(pref: &str, has_v4: bool, has_v6: bool) -> Vec<Family> {
    // v4only：永不包含 V6
    if pref == "v4only" {
        return if has_v4 { vec![Family::V4] } else { vec![] };
    }
    match (has_v4, has_v6) {
        (false, false) => vec![],
        (true, false) => vec![Family::V4],
        (false, true) => vec![Family::V6],
        (true, true) => match pref {
            "v4first" => vec![Family::V4, Family::V6],
            // "v6first" 与 "auto"（及未知值）均以 V6 为首
            _ => vec![Family::V6, Family::V4],
        },
    }
}

/// 双栈域名解析结果：v4/v6 任一族解析失败仅将对应字段留空，不影响另一族。
pub(crate) struct ResolvedAddrs {
    pub v4: Option<Ipv4Addr>,
    pub v6: Option<Ipv6Addr>,
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

    /// 经所选物理网卡对目标进行双栈解析：复用既有 A 记录路径得 IPv4，
    /// 并新增平行 AAAA 路径得 IPv6。任一族失败仅将对应字段留空。
    /// 目标为字面 IPv4/IPv6 时直接填入对应字段，不发起解析。
    ///
    /// `pub(crate)`：供 TUN 模式 UDP 中继在反查 fake-ip 域名后，经所选网卡解析真实
    /// 目标地址（Req 3.2）。
    pub(crate) async fn resolve_host_dual(&self, nic: &NicRuntime, host: &str, port: u16) -> ResolvedAddrs {
        // 字面 IPv6：直接作为 v6 结果
        if let Ok(ip) = host.parse::<Ipv6Addr>() {
            return ResolvedAddrs {
                v4: None,
                v6: Some(ip),
            };
        }
        // 字面 IPv4：直接作为 v4 结果
        if let Ok(ip) = host.parse::<Ipv4Addr>() {
            return ResolvedAddrs {
                v4: Some(ip),
                v6: None,
            };
        }
        // 域名：复用既有 A 记录解析（含缓存/DoH/UDP/系统回退），并追加 AAAA 平行路径
        let v4 = self.resolve_host(nic, host, port).await.map(|a| *a.ip());
        let v6 = resolve_aaaa_via_nic(nic, host).await;
        ResolvedAddrs { v4, v6 }
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
                let active: Vec<i64> =
                    pool.iter().map(|&i| self.nics[i].active.load(Ordering::Relaxed)).collect();
                let weights: Vec<u32> = pool.iter().map(|&i| self.nics[i].weight).collect();
                let pos = least_conn_pick_pos(&active, &weights);
                self.nics[pool[pos]].clone()
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
        let eff: Vec<i64> = pool.iter().map(|&i| eff_of(&self.nics[i])).collect();
        let mut cur = match self.wrr.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        let idx = swrr_pick_index(pool, &eff, &mut cur);
        self.nics[idx].clone()
    }

    fn log(&self, msg: impl Into<String>) {
        // 统一日志入口：既有 emit("hmx-log") 行为不变，附加写入本地滚动日志文件
        crate::hmx_log(&self.app, crate::logger::LogLevel::Info, &msg.into());
    }

    fn register_conn(&self, target: String, nic: String, proto: &'static str) -> ConnTableGuard {
        let id = self.conn_id.fetch_add(1, Ordering::Relaxed);
        let info = ConnInfo { id, target, nic, proto };
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
    /// 同进程内的引擎实例：供 TUN 直连模式复用其网卡选择与双栈解析（Req 3.1/3.2/3.4）。
    engine: Arc<Engine>,
}

impl EngineHandle {
    pub fn stop(&self) {
        self.cancel.cancel();
    }

    /// 返回同进程引擎实例的克隆句柄。
    /// 供进程内直连 TUN 模式把 UDP/QUIC 中继下沉到本引擎（逐卡出口 + fake-ip 反查）。
    pub fn engine(&self) -> std::sync::Arc<Engine> {
        self.engine.clone()
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
    ip_version: String,
    udp_associate: bool,
) -> Result<EngineHandle, String> {
    let zh = lang != "en";
    if selected.is_empty() {
        return Err(if zh {
            "至少需要选择一张网卡".into()
        } else {
            "At least one network adapter must be selected".to_string()
        });
    }
    let strategy = Strategy::parse(&strategy);
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

    // 解析分流规则，按 kind 分派：
    // - domain（默认）：direct → 并入直连白名单；nic:<ifindex> → 域名钉死到指定网卡；aggregate → 默认。
    // - process：pattern 为可执行文件名（小写），动作经 parse_rule_action 解析后入 rules_proc（优先级高于域名）。
    let mut rules_nic: Vec<(String, u32)> = Vec::new();
    let mut rules_proc: Vec<(String, RuleAction)> = Vec::new();
    for r in &rules {
        let pat = r.pattern.trim().to_lowercase();
        if pat.is_empty() {
            continue;
        }
        if r.kind.trim().eq_ignore_ascii_case("process") {
            // 进程规则：可执行文件名（小写）+ 动作
            if let Some(action) = parse_rule_action(&r.action) {
                rules_proc.push((pat, action));
            }
            continue;
        }
        // 域名规则（默认，行为不变）
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
        let ip: Ipv4Addr = s.ip.parse().map_err(|_| {
            if zh {
                format!("网卡 {} 的 IPv4 地址非法: {}", s.name, s.ip)
            } else {
                format!("Adapter {} has an invalid IPv4 address: {}", s.name, s.ip)
            }
        })?;
        let ipv6 = s
            .ipv6
            .as_ref()
            .and_then(|v| v.trim().parse::<Ipv6Addr>().ok());
        nics.push(Arc::new(NicRuntime {
            name: s.name.clone(),
            ipv4: ip,
            ipv6,
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

    // 说明：上层已在启动前做过端口预检，但预检与实际 bind 之间存在极短的 TOCTOU
    // 竞争窗口（端口可能被其它进程抢占）。这里的 bind 失败即最终裁决，给出本地化的
    // 明确提示，便于用户改端口或排查占用进程。
    let socks_listener = TcpListener::bind(("127.0.0.1", socks_port)).await.map_err(|e| {
        if zh {
            format!("无法监听 SOCKS5 端口 127.0.0.1:{socks_port}（可能已被占用，请在设置里更换端口）-- {e}")
        } else {
            format!("Cannot listen on SOCKS5 port 127.0.0.1:{socks_port} (possibly in use, change it in Settings) -- {e}")
        }
    })?;
    let http_listener = TcpListener::bind(("127.0.0.1", http_port)).await.map_err(|e| {
        if zh {
            format!("无法监听 HTTP 端口 127.0.0.1:{http_port}（可能已被占用，请在设置里更换端口）-- {e}")
        } else {
            format!("Cannot listen on HTTP port 127.0.0.1:{http_port} (possibly in use, change it in Settings) -- {e}")
        }
    })?;

    let cancel = CancellationToken::new();
    let engine = Arc::new(Engine {
        nics: nics.clone(),
        strategy,
        wrr: Mutex::new(vec![0i64; nics.len()]),
        conns: Arc::new(Mutex::new(HashMap::new())),
        conn_id: AtomicU64::new(0),
        app: app.clone(),
        zh,
        // IP 版本偏好来自前端设置（auto/v4first/v6first/v4only），经 start_boost 透传；
        // 非法值回退为 "auto"（等价 v6first）。
        ip_version: match ip_version.trim() {
            "v4first" | "v6first" | "v4only" | "auto" => ip_version.trim().to_string(),
            _ => "auto".to_string(),
        },
        udp_associate,
        limiter,
        bypass,
        rules_nic,
        rules_proc,
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
                    tokio::time::timeout(std::time::Duration::from_secs(4), connect_via_nic(n, SocketAddr::V4(target))).await,
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

    Ok(EngineHandle { cancel, engine })
}

/// 单张网卡的连通性 / 延迟探测结果（多采样统计）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LatencyResult {
    pub index: u32,
    pub name: String,
    /// 代表性 RTT（毫秒）：成功时=平均值（avg），全失败为 -1（兼容既有前端字段语义）
    pub latency_ms: i64,
    pub ok: bool,
    /// 最小 RTT（毫秒），全失败为 -1
    pub min_ms: i64,
    /// 平均 RTT（毫秒），全失败为 -1
    pub avg_ms: i64,
    /// 抖动（成功 RTT 样本标准差，毫秒），全失败为 -1（不可用）
    pub jitter_ms: i64,
    /// 丢包率：失败次数 / 总次数，∈ [0,1]
    pub loss_pct: f64,
}

/// 多采样延迟统计（纯函数结果）
pub(crate) struct LatencyStats {
    pub min: i64,
    pub avg: i64,
    pub jitter: i64,
    pub loss_pct: f64,
}

/// 从 RTT 样本序列（`Some(ms)`=成功、`None`=失败）计算 min/avg/jitter/loss（纯函数）。
///
/// 语义（对应设计 Property 22）：
/// - 存在成功样本时：`min = 成功样本最小值`、`avg = 成功样本均值（四舍五入）`，`min <= avg`；
///   `jitter = 成功样本标准差（四舍五入，>=0）`，成功样本数 <2 或全相等时 `jitter = 0`。
/// - `loss_pct = 失败数 / 总数 ∈ [0,1]`。
/// - 全部失败（或无样本）时：`loss_pct = 1.0`、`jitter = -1`（不可用）、`min = avg = -1`。
pub(crate) fn compute_latency_stats(samples: &[Option<u64>]) -> LatencyStats {
    let total = samples.len();
    let ok: Vec<u64> = samples.iter().filter_map(|s| *s).collect();
    if total == 0 || ok.is_empty() {
        return LatencyStats { min: -1, avg: -1, jitter: -1, loss_pct: 1.0 };
    }
    let failed = total - ok.len();
    let loss_pct = failed as f64 / total as f64;
    let min = *ok.iter().min().unwrap();
    let sum: u64 = ok.iter().sum();
    let mean = sum as f64 / ok.len() as f64;
    let avg = mean.round() as i64;
    // 抖动：成功样本标准差（总体标准差）；样本 <2 时为 0
    let jitter = if ok.len() < 2 {
        0
    } else {
        let var = ok.iter().map(|&x| {
            let d = x as f64 - mean;
            d * d
        }).sum::<f64>() / ok.len() as f64;
        var.sqrt().round() as i64
    };
    LatencyStats { min: min as i64, avg, jitter, loss_pct }
}

/// 每张网卡的延迟采样次数（多采样以计算抖动与丢包率）
const LATENCY_SAMPLES: usize = 10;

/// 逐张网卡探测出口连通性与延迟：经各网卡多次 TCP 握手采样，统计 min/avg/jitter/loss。
pub async fn test_latency(selected: Vec<SelectedNic>) -> Vec<LatencyResult> {
    // 国内外均可达的稳定节点（AliDNS:443），仅测 TCP 握手 RTT，不传输数据
    let target = SocketAddrV4::new(Ipv4Addr::new(223, 5, 5, 5), 443);
    let mut out = Vec::with_capacity(selected.len());
    for s in selected {
        let ip: Ipv4Addr = match s.ip.parse() {
            Ok(v) => v,
            Err(_) => {
                out.push(LatencyResult {
                    index: s.index,
                    name: s.name,
                    latency_ms: -1,
                    ok: false,
                    min_ms: -1,
                    avg_ms: -1,
                    jitter_ms: -1,
                    loss_pct: 1.0,
                });
                continue;
            }
        };
        let ipv6 = s
            .ipv6
            .as_ref()
            .and_then(|v| v.trim().parse::<Ipv6Addr>().ok());
        let nic = NicRuntime {
            name: s.name.clone(),
            ipv4: ip,
            ipv6,
            if_index: s.index,
            active: AtomicI64::new(0),
            speed: AtomicU64::new(0),
            alive: AtomicBool::new(true),
            weight: 100,
            limiter: None,
        };
        // 多次采样：Some(ms)=握手成功耗时，None=超时/失败
        let mut samples: Vec<Option<u64>> = Vec::with_capacity(LATENCY_SAMPLES);
        for _ in 0..LATENCY_SAMPLES {
            let start = std::time::Instant::now();
            let res = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                connect_via_nic(&nic, SocketAddr::V4(target)),
            )
            .await;
            match res {
                Ok(Ok(_stream)) => samples.push(Some(start.elapsed().as_millis() as u64)),
                _ => samples.push(None),
            }
        }
        let stats = compute_latency_stats(&samples);
        let ok = stats.loss_pct < 1.0;
        out.push(LatencyResult {
            index: s.index,
            name: s.name,
            latency_ms: stats.avg, // 成功时=avg，全失败为 -1（兼容既有字段）
            ok,
            min_ms: stats.min,
            avg_ms: stats.avg,
            jitter_ms: stats.jitter,
            loss_pct: stats.loss_pct,
        });
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

/// 全局测速互斥标志：防止诊断页与"一键聚合测速"并发触发跑分，
/// 多任务同时争抢带宽会让各自测得的吞吐互相压制、结果严重失真。
static BENCH_RUNNING: AtomicBool = AtomicBool::new(false);

/// 测速运行守卫：drop 时自动释放标志，确保任何退出路径都能复位。
struct BenchGuard;
impl Drop for BenchGuard {
    fn drop(&mut self) {
        BENCH_RUNNING.store(false, Ordering::SeqCst);
    }
}

/// 逐张网卡跑分：经各网卡多条并发连接从测速节点下载，测真实聚合吞吐（MB/s）。
/// 所有网卡**并发**测试，既加速诊断，也支撑控制台"一键聚合测速"的同时跑分。
pub async fn speed_test(app: AppHandle, selected: Vec<SelectedNic>, duration_secs: u64) -> Vec<SpeedResult> {
    // 后端重入保护：已有测速在跑时直接忽略本次请求。
    // 返回值不被前端直接消费（结果经 hmx-speedtest 事件推送），返回空安全无副作用。
    if BENCH_RUNNING.swap(true, Ordering::SeqCst) {
        let _ = app.emit("hmx-log", "[测速] 已有测速任务进行中，已忽略本次并发请求 / benchmark already running, request ignored");
        return Vec::new();
    }
    let _bench_guard = BenchGuard;
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
    let ipv6 = s
        .ipv6
        .as_ref()
        .and_then(|v| v.trim().parse::<Ipv6Addr>().ok());
    let nic = Arc::new(NicRuntime {
        name: s.name.clone(),
        ipv4: ip,
        ipv6,
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
        let tcp = match connect_via_nic(nic, SocketAddr::V4(dst)).await {
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
    let tcp = tokio::time::timeout(std::time::Duration::from_secs(6), connect_via_nic(nic, SocketAddr::V4(dst)))
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

/// 构造一个指定记录类型的 DNS 查询报文（A=1, AAAA=28）。
/// 与 `build_dns_query` 平行，仅 QTYPE 字段按 `qtype` 写入；`build_dns_query`
/// 的既有行为保持不变，此处为 IPv6/双栈解析新增平行路径。
fn build_dns_query_type(host: &str, qtype: u16) -> Vec<u8> {
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
    q.extend_from_slice(&qtype.to_be_bytes()); // QTYPE（A=1 / AAAA=28）
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

/// 从 DNS 响应中解析首个 AAAA 记录（与 `parse_dns_a` 平行，匹配 rtype==28 && rdlen==16）。
fn parse_dns_aaaa(buf: &[u8]) -> Option<Ipv6Addr> {
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
        if rtype == 28 && rdlen == 16 {
            let mut octets = [0u8; 16];
            octets.copy_from_slice(&buf[pos..pos + 16]);
            return Some(Ipv6Addr::from(octets));
        }
        pos += rdlen;
    }
    None
}

/// SOCKS5 UDP 请求头中的目标地址（RFC 1928 §7）。
///
/// 覆盖三种地址类型并可无损 round-trip：IPv4（ATYP=0x01）、域名（ATYP=0x03）、
/// IPv6（ATYP=0x04）。派生 `PartialEq/Eq/Debug` 以便单元/属性测试比对。
/// 供后续 UDP ASSOCIATE（`CMD=0x03`）转发路径消费。
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SocksUdpTarget {
    V4(SocketAddrV4),
    V6(SocketAddrV6),
    Domain(String, u16),
}

/// 解析 SOCKS5 UDP 请求头：`RSV(2) FRAG(1) ATYP(1) DST.ADDR DST.PORT DATA`。
///
/// 成功时返回 `(目标地址, 载荷偏移)`，其中载荷偏移为 `DATA` 段在 `buf` 中的起始
/// 下标（即请求头长度）。RSV 按 RFC 1928 要求为 `0x0000`（否则视为畸形拒绝）；
/// FRAG 字段被跳过（本实现不支持分片，故不纳入目标）。任何长度不足或畸形输入
/// 返回 `None`。DST.PORT 为大端序。
#[allow(dead_code)]
pub(crate) fn parse_socks_udp_header(buf: &[u8]) -> Option<(SocksUdpTarget, usize)> {
    // 至少需要 RSV(2) + FRAG(1) + ATYP(1)
    if buf.len() < 4 {
        return None;
    }
    // RSV 必须为 0x0000（RFC 1928 §7）
    if buf[0] != 0x00 || buf[1] != 0x00 {
        return None;
    }
    // buf[2] = FRAG：不支持分片，解析时跳过其值，不纳入目标表示
    let atyp = buf[3];
    let mut pos = 4usize;
    let target = match atyp {
        0x01 => {
            // IPv4：4 字节地址 + 2 字节端口
            if buf.len() < pos + 4 + 2 {
                return None;
            }
            let ip = Ipv4Addr::new(buf[pos], buf[pos + 1], buf[pos + 2], buf[pos + 3]);
            pos += 4;
            let port = u16::from_be_bytes([buf[pos], buf[pos + 1]]);
            pos += 2;
            SocksUdpTarget::V4(SocketAddrV4::new(ip, port))
        }
        0x04 => {
            // IPv6：16 字节地址 + 2 字节端口
            if buf.len() < pos + 16 + 2 {
                return None;
            }
            let mut octets = [0u8; 16];
            octets.copy_from_slice(&buf[pos..pos + 16]);
            pos += 16;
            let port = u16::from_be_bytes([buf[pos], buf[pos + 1]]);
            pos += 2;
            SocksUdpTarget::V6(SocketAddrV6::new(Ipv6Addr::from(octets), port, 0, 0))
        }
        0x03 => {
            // 域名：1 字节长度 + N 字节域名 + 2 字节端口
            let len = *buf.get(pos)? as usize;
            pos += 1;
            if buf.len() < pos + len + 2 {
                return None;
            }
            let host = std::str::from_utf8(&buf[pos..pos + len]).ok()?.to_string();
            pos += len;
            let port = u16::from_be_bytes([buf[pos], buf[pos + 1]]);
            pos += 2;
            SocksUdpTarget::Domain(host, port)
        }
        _ => return None,
    };
    Some((target, pos))
}

/// 封装 SOCKS5 UDP 请求头（不含 DATA）：`RSV(2)=0x0000 FRAG(1)=0x00 ATYP ADDR PORT`。
///
/// 与 [`parse_socks_udp_header`] 互逆：`parse(build(t) ++ payload)` 应还原等价目标，
/// 且返回的载荷偏移等于本函数输出的字节长度。DST.PORT 以大端序写入。
#[allow(dead_code)]
pub(crate) fn build_socks_udp_header(target: &SocksUdpTarget) -> Vec<u8> {
    let mut out = Vec::with_capacity(22);
    out.extend_from_slice(&[0x00, 0x00]); // RSV
    out.push(0x00); // FRAG
    match target {
        SocksUdpTarget::V4(addr) => {
            out.push(0x01);
            out.extend_from_slice(&addr.ip().octets());
            out.extend_from_slice(&addr.port().to_be_bytes());
        }
        SocksUdpTarget::V6(addr) => {
            out.push(0x04);
            out.extend_from_slice(&addr.ip().octets());
            out.extend_from_slice(&addr.port().to_be_bytes());
        }
        SocksUdpTarget::Domain(host, port) => {
            out.push(0x03);
            out.push(host.len() as u8);
            out.extend_from_slice(host.as_bytes());
            out.extend_from_slice(&port.to_be_bytes());
        }
    }
    out
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
    let bind_addr: socket2::SockAddr = SocketAddr::new(IpAddr::V4(nic.ipv4), 0).into();
    socket.bind(&bind_addr).ok()?;
    socket.set_nonblocking(true).ok()?;
    let std_udp: std::net::UdpSocket = socket.into();
    let udp = tokio::net::UdpSocket::from_std(std_udp).ok()?;
    let query = build_dns_query(host);
    let server: SocketAddr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(223, 5, 5, 5), 53));
    udp.send_to(&query, server).await.ok()?;
    // 循环接收直到拿到「来源正确 + 事务 ID 匹配」的响应或超时，
    // 丢弃伪造/延迟/串扰的 UDP 包（查询 ID 见 build_dns_query 首两字节 0x1234）。
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let mut buf = [0u8; 512];
    loop {
        let remaining = deadline.checked_duration_since(std::time::Instant::now())?;
        let (n, from) = tokio::time::timeout(remaining, udp.recv_from(&mut buf))
            .await
            .ok()?
            .ok()?;
        // 来源必须是我们发出查询的 DNS 服务器
        if from != server {
            continue;
        }
        // 事务 ID 必须与查询一致（build_dns_query 使用 0x1234）
        if n < 2 || buf[0] != 0x12 || buf[1] != 0x34 {
            continue;
        }
        if let Some(ip) = parse_dns_a(&buf[..n]) {
            return Some(ip);
        }
        // ID 与来源都对但无 A 记录：视为无结果，停止等待
        return None;
    }
}

/// 经指定网卡向真实公共 DNS（223.5.5.5）直接发起 AAAA UDP 查询解析域名的 IPv6 地址。
/// 与 `resolve_via_nic` 平行：查询走 IPv4 UDP（DNS 服务器为 IPv4 字面地址），
/// 用 IP_UNICAST_IF 钉死物理网卡出口，仅记录类型改为 AAAA(28) 并以 `parse_dns_aaaa` 解析。
async fn resolve_aaaa_via_nic(nic: &NicRuntime, host: &str) -> Option<Ipv6Addr> {
    if let Ok(ip) = host.parse::<Ipv6Addr>() {
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
    let bind_addr: socket2::SockAddr = SocketAddr::new(IpAddr::V4(nic.ipv4), 0).into();
    socket.bind(&bind_addr).ok()?;
    socket.set_nonblocking(true).ok()?;
    let std_udp: std::net::UdpSocket = socket.into();
    let udp = tokio::net::UdpSocket::from_std(std_udp).ok()?;
    let query = build_dns_query_type(host, 28);
    let server: SocketAddr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(223, 5, 5, 5), 53));
    udp.send_to(&query, server).await.ok()?;
    // 循环接收直到拿到「来源正确 + 事务 ID 匹配」的响应或超时，
    // 丢弃伪造/延迟/串扰的 UDP 包（查询 ID 见 build_dns_query_type 首两字节 0x1234）。
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let mut buf = [0u8; 512];
    loop {
        let remaining = deadline.checked_duration_since(std::time::Instant::now())?;
        let (n, from) = tokio::time::timeout(remaining, udp.recv_from(&mut buf))
            .await
            .ok()?
            .ok()?;
        // 来源必须是我们发出查询的 DNS 服务器
        if from != server {
            continue;
        }
        // 事务 ID 必须与查询一致（build_dns_query_type 使用 0x1234）
        if n < 2 || buf[0] != 0x12 || buf[1] != 0x34 {
            continue;
        }
        if let Some(ip) = parse_dns_aaaa(&buf[..n]) {
            return Some(ip);
        }
        // ID 与来源都对但无 AAAA 记录：视为无结果，停止等待
        return None;
    }
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
            let tcp = connect_via_nic(nic, SocketAddr::V4(dst)).await.ok()?;
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
///
/// 按目标地址族分派：IPv4 走既有 `Domain::IPV4` + `IP_UNICAST_IF`（level=IPPROTO_IP）+
/// bind 该网卡 IPv4 源地址；IPv6 走 `Domain::IPV6` + `IPV6_UNICAST_IF`
/// （level=IPPROTO_IPV6）+ bind 该网卡 IPv6 源地址。两族接口索引均以网络字节序传入。
async fn connect_via_nic(nic: &NicRuntime, dst: SocketAddr) -> std::io::Result<TcpStream> {
    // 按目标地址族确定 socket 域、UNICAST_IF 的 level 与本地绑定源地址。
    let (domain, if_level, bind_ip) = match dst {
        SocketAddr::V4(_) => (Domain::IPV4, IPPROTO_IP, IpAddr::V4(nic.ipv4)),
        SocketAddr::V6(_) => {
            let v6 = nic.ipv6.ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::AddrNotAvailable,
                    format!("网卡 {} 无可用 IPv6 出口源地址", nic.name),
                )
            })?;
            (Domain::IPV6, IPPROTO_IPV6, IpAddr::V6(v6))
        }
    };
    let if_optname = match dst {
        SocketAddr::V4(_) => IP_UNICAST_IF,
        SocketAddr::V6(_) => IPV6_UNICAST_IF,
    };

    let socket = Socket::new(domain, Type::STREAM, Some(Protocol::TCP))?;

    // 1) 接口索引强绑定（必须在 bind/connect 之前）。接口索引以网络字节序传入。
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
                format!("UNICAST_IF 绑定失败 (IfIndex={})", nic.if_index),
            ));
        }
    }

    // 2) bind 本地出口 IP（仅固定源地址，失败可降级忽略）
    let bind_addr: socket2::SockAddr = SocketAddr::new(bind_ip, 0).into();
    let _ = socket.bind(&bind_addr);

    // 3) 非阻塞连接，交给 tokio 等待可写
    socket.set_nonblocking(true)?;
    let target: socket2::SockAddr = dst.into();
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

/// 创建经指定网卡出口绑定的出站 UDP socket（Egress_Binding）：
/// IPv4 走 `IP_UNICAST_IF`（level=IPPROTO_IP）+ bind 网卡 IPv4 源地址；
/// IPv6 走 `IPV6_UNICAST_IF`（level=IPPROTO_IPV6）+ bind 网卡 IPv6 源地址。
/// 用于 SOCKS5 UDP ASSOCIATE 的上游转发（Req 4.2）。
async fn udp_bind_via_nic(nic: &NicRuntime, family: Family) -> std::io::Result<tokio::net::UdpSocket> {
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
    let bind_addr: socket2::SockAddr = SocketAddr::new(bind_ip, 0).into();
    socket.bind(&bind_addr)?;
    socket.set_nonblocking(true)?;
    let std_udp: std::net::UdpSocket = socket.into();
    tokio::net::UdpSocket::from_std(std_udp)
}

/// SOCKS5 UDP ASSOCIATE（CMD=0x03）：在 `127.0.0.1` 分配 UDP 中继端口并应答 BND，
/// 随后按客户端数据报中的 SOCKS5 UDP 请求头目标，经所选网卡出口转发数据报，并把
/// 上游响应封回请求头后回送客户端。TCP 控制连接关闭即拆除该关联（RFC 1928）。
///
/// 网卡选择复用 `pick_nic`；域名目标经 `resolve_host_dual` + `pick_family` 解析真实地址。
/// 所有 socket 失败均记录日志后跳过该数据报 / 会话，绝不 panic。
async fn udp_associate(engine: Arc<Engine>, mut client: TcpStream) -> std::io::Result<()> {
    // 1) 分配中继 UDP 端口（仅监听本机回环）
    let relay = match tokio::net::UdpSocket::bind(("127.0.0.1", 0u16)).await {
        Ok(s) => Arc::new(s),
        Err(e) => {
            let _ = client
                .write_all(&[0x05, 0x01, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                .await;
            return Err(e);
        }
    };
    let port = relay.local_addr()?.port();

    // 2) 应答：VER REP=0x00 RSV ATYP=0x01 BND.ADDR=127.0.0.1 BND.PORT
    client
        .write_all(&[0x05, 0x00, 0x00, 0x01, 127, 0, 0, 1, (port >> 8) as u8, (port & 0xff) as u8])
        .await?;

    engine.log(if engine.zh {
        format!("[UDP关联] 已在 127.0.0.1:{port} 分配 UDP 中继端口")
    } else {
        format!("[UDP associate] relay bound at 127.0.0.1:{port}")
    });

    let cancel = CancellationToken::new();

    // 控制连接守卫：读到 EOF（客户端关闭）即取消整个关联
    let ctrl_cancel = cancel.clone();
    let ctrl = async move {
        let mut buf = [0u8; 256];
        loop {
            match client.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {} // 控制连接一般无数据，忽略
            }
        }
        ctrl_cancel.cancel();
    };

    // UDP 中继泵：上行（客户端->真实目标）+ 为每个上游 spawn 下行转发
    let relay_recv = relay.clone();
    let eng = engine.clone();
    let pump_cancel = cancel.clone();
    let pump = async move {
        let client_addr: Arc<Mutex<Option<SocketAddr>>> = Arc::new(Mutex::new(None));
        let mut upstreams: HashMap<SocketAddr, Arc<tokio::net::UdpSocket>> = HashMap::new();
        let mut buf = vec![0u8; 65536];
        loop {
            let (n, src) = tokio::select! {
                _ = pump_cancel.cancelled() => break,
                r = relay_recv.recv_from(&mut buf) => match r {
                    Ok(v) => v,
                    Err(_) => break,
                },
            };
            if let Ok(mut g) = client_addr.lock() {
                *g = Some(src);
            }
            let (target, off) = match parse_socks_udp_header(&buf[..n]) {
                Some(v) => v,
                None => continue,
            };
            let (host, port) = match &target {
                SocksUdpTarget::V4(a) => (a.ip().to_string(), a.port()),
                SocksUdpTarget::V6(a) => (a.ip().to_string(), a.port()),
                SocksUdpTarget::Domain(h, p) => (h.clone(), *p),
            };
            let nic = eng.pick_nic(&host, port, None);
            // 解析真实目标 + 选择地址族
            let (real_dst, family) = match &target {
                SocksUdpTarget::V4(a) => (SocketAddr::V4(*a), Family::V4),
                SocksUdpTarget::V6(a) => (SocketAddr::V6(*a), Family::V6),
                SocksUdpTarget::Domain(h, p) => {
                    let addrs = eng.resolve_host_dual(&nic, h, *p).await;
                    let fams = pick_family(eng.ip_pref(), addrs.v4.is_some(), addrs.v6.is_some());
                    match fams.first() {
                        Some(Family::V4) => match addrs.v4 {
                            Some(ip) => (SocketAddr::V4(SocketAddrV4::new(ip, *p)), Family::V4),
                            None => continue,
                        },
                        Some(Family::V6) => match addrs.v6 {
                            Some(ip) => (SocketAddr::V6(SocketAddrV6::new(ip, *p, 0, 0)), Family::V6),
                            None => continue,
                        },
                        _ => continue,
                    }
                }
            };
            // 获取 / 创建经网卡的上游 socket（按真实目标缓存复用）
            let up = if let Some(u) = upstreams.get(&real_dst) {
                u.clone()
            } else {
                let u = match udp_bind_via_nic(&nic, family).await {
                    Ok(s) => Arc::new(s),
                    Err(e) => {
                        eng.log(if eng.zh {
                            format!("[UDP关联] 网卡 {} 创建上游 UDP 失败: {}", nic.name, e)
                        } else {
                            format!("[UDP associate] adapter {} upstream UDP failed: {}", nic.name, e)
                        });
                        continue;
                    }
                };
                upstreams.insert(real_dst, u.clone());
                // 下行转发任务：上游响应 -> 封回 SOCKS5 UDP 头 -> 回送客户端
                let relay_send = relay_recv.clone();
                let ca = client_addr.clone();
                let hdr_target = target.clone();
                let up_task = u.clone();
                let dtoken = pump_cancel.clone();
                tauri::async_runtime::spawn(async move {
                    let mut dbuf = vec![0u8; 65536];
                    loop {
                        let m = tokio::select! {
                            _ = dtoken.cancelled() => break,
                            r = up_task.recv_from(&mut dbuf) => match r {
                                Ok((m, _from)) => m,
                                Err(_) => break,
                            },
                        };
                        let mut framed = build_socks_udp_header(&hdr_target);
                        framed.extend_from_slice(&dbuf[..m]);
                        let dst_client = ca.lock().ok().and_then(|g| *g);
                        if let Some(c) = dst_client {
                            let _ = relay_send.send_to(&framed, c).await;
                        }
                    }
                });
                u
            };
            // 上行：把去掉 SOCKS5 UDP 头后的载荷发往真实目标
            let _ = up.send_to(&buf[off..n], real_dst).await;
        }
    };

    tokio::select! {
        _ = ctrl => {}
        _ = pump => {}
    }
    cancel.cancel();
    Ok(())
}

/// Happy-Eyeballs 式双栈拨号：按 `pick_family(pref, ...)` 给出的地址族顺序依次尝试连接，
/// 首选族在 `timeout` 内失败（连接错误或超时）且存在备选族时，记录一条可读日志并回退到
/// 另一地址族。仅当所有候选族均失败才返回最后一次错误。
///
/// 关键不变量：当目标只有 IPv4 地址时 `pick_family` 只返回 `[V4]`，本函数只尝试 IPv4，
/// 绝不触发任何 IPv6 分支——与既有纯 IPv4 行为保持一致。无全局 IPv6 源地址时，
/// `connect_via_nic` 的 IPv6 分支返回 `AddrNotAvailable`，本函数据此回退 IPv4。
///
/// 说明：形参保留设计文档约定的 `pref`/`timeout`，并额外接收 `engine` 仅用于按当前
/// 界面语言输出可读的回退日志（Requirement 1.3）。
async fn dial_dual(
    engine: &Engine,
    nic: &NicRuntime,
    addrs: &ResolvedAddrs,
    port: u16,
    pref: &str,
    timeout: std::time::Duration,
) -> std::io::Result<TcpStream> {
    let families = pick_family(pref, addrs.v4.is_some(), addrs.v6.is_some());
    let total = families.len();
    let mut last_err = std::io::Error::new(
        std::io::ErrorKind::AddrNotAvailable,
        "目标无可用地址族 / no usable address family for target",
    );

    for (i, fam) in families.iter().enumerate() {
        // 依据地址族构造目标 SocketAddr；对应地址缺失则跳过该族（理论上 pick_family 已保证存在）。
        let dst = match fam {
            Family::V4 => match addrs.v4 {
                Some(ip) => SocketAddr::V4(SocketAddrV4::new(ip, port)),
                None => continue,
            },
            Family::V6 => match addrs.v6 {
                Some(ip) => SocketAddr::V6(SocketAddrV6::new(ip, port, 0, 0)),
                None => continue,
            },
        };

        let attempt = tokio::time::timeout(timeout, connect_via_nic(nic, dst)).await;
        let err = match attempt {
            Ok(Ok(stream)) => return Ok(stream),
            Ok(Err(e)) => e,
            Err(_) => std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!("连接首选地址族超时 ({timeout:?})"),
            ),
        };

        // 首选族失败：若存在备选族则记录可读回退日志并继续尝试下一族。
        if i + 1 < total {
            let next = families[i + 1];
            engine.log(if engine.zh {
                format!(
                    "[双栈回退] 网卡 {} 经 {} 连接失败（{}），回退尝试 {}",
                    nic.name,
                    family_label(*fam),
                    err,
                    family_label(next),
                )
            } else {
                format!(
                    "[Dual-stack fallback] adapter {} failed over {} ({}), retrying {}",
                    nic.name,
                    family_label(*fam),
                    err,
                    family_label(next),
                )
            });
        }
        last_err = err;
    }

    Err(last_err)
}

/// 地址族的可读标签（日志用）。
fn family_label(f: Family) -> &'static str {
    match f {
        Family::V4 => "IPv4",
        Family::V6 => "IPv6",
    }
}

// ============================== SOCKS5 ==============================

async fn handle_socks(engine: Arc<Engine>, mut client: TcpStream) -> std::io::Result<()> {
    // 1) 握手：版本 + 方法列表（首个读加超时，回收连上不发数据的僵死连接）
    let mut head = [0u8; 2];
    match tokio::time::timeout(HANDSHAKE_TIMEOUT, client.read_exact(&mut head)).await {
        Ok(Ok(_)) => {}
        _ => return Ok(()),
    }
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
    // CMD 分派：0x01 CONNECT（既有路径，完全不变）；0x03 UDP ASSOCIATE（启用时处理，
    // 未启用回 REP=0x07）；其余命令回 REP=0x07。
    if req[1] == 0x03 {
        if !engine.udp_associate {
            // 未启用：以标准"命令不支持"应答拒绝（Req 4.3）
            client
                .write_all(&[0x05, 0x07, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                .await?;
            return Ok(());
        }
        // 读取并丢弃请求中的 DST.ADDR/DST.PORT（ASSOCIATE 中为客户端预期源，通常 0.0.0.0:0）
        match req[3] {
            0x01 => {
                let mut a = [0u8; 4 + 2];
                client.read_exact(&mut a).await?;
            }
            0x04 => {
                let mut a = [0u8; 16 + 2];
                client.read_exact(&mut a).await?;
            }
            0x03 => {
                let mut l = [0u8; 1];
                client.read_exact(&mut l).await?;
                let mut b = vec![0u8; l[0] as usize + 2];
                client.read_exact(&mut b).await?;
            }
            _ => {
                client
                    .write_all(&[0x05, 0x08, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                    .await?;
                return Ok(());
            }
        }
        return udp_associate(engine.clone(), client).await;
    }
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
    let mut literal_v6: Option<Ipv6Addr> = None;
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
        0x04 => {
            // IPv6 字面量目标：读取 16 字节地址
            let mut a = [0u8; 16];
            client.read_exact(&mut a).await?;
            literal_v6 = Some(Ipv6Addr::from(a));
        }
        _ => {
            // 未知地址类型
            client
                .write_all(&[0x05, 0x08, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                .await?;
            return Ok(());
        }
    }
    let mut port_buf = [0u8; 2];
    client.read_exact(&mut port_buf).await?;
    let port = u16::from_be_bytes(port_buf);

    // IPv6 字面量目标（ATYP=0x04）：平行于既有 IPv4 分流路径，直接构造 SocketAddr::V6
    if let Some(v6) = literal_v6 {
        let target_display = v6.to_string();
        let dst6 = SocketAddrV6::new(v6, port, 0, 0);

        // 白名单命中：走默认网关直连（字面 IPv6 通常不会命中域名规则，保持语义一致）
        if engine.is_bypass(&target_display) {
            let _ctg = engine.register_conn(
                format!("{target_display}:{port}"),
                "Direct".to_string(),
                "SOCKS",
            );
            engine.log(if engine.zh {
                format!("[直连] 白名单命中 -> 默认网关 | 目标: {target_display}:{port}")
            } else {
                format!("[Direct] bypass match -> default gateway | target: {target_display}:{port}")
            });
            match TcpStream::connect(SocketAddr::V6(dst6)).await {
                Ok(mut upstream) => {
                    let _ = upstream.set_nodelay(true);
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

        // 调度 + 物理绑定连接（复用既有网卡选择逻辑；proc_name=None，进程反查为运行时集成项）
        let nic = engine.pick_nic(&target_display, port, None);
        nic.active.fetch_add(1, Ordering::Relaxed);
        let _guard = ConnGuard(nic.clone());

        engine.log(if engine.zh {
            format!("[调度分配] 新连接 -> [{}] | 目标: {}:{}", nic.name, target_display, port)
        } else {
            format!("[Dispatch] new connection -> [{}] | target: {}:{}", nic.name, target_display, port)
        });
        let _ctg = engine.register_conn(format!("{target_display}:{port}"), nic.name.clone(), "SOCKS");

        let mut upstream = match connect_via_nic(&nic, SocketAddr::V6(dst6)).await {
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

        client
            .write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
            .await?;
        relay(&mut client, &mut upstream, nic.limiter.clone().or_else(|| engine.limiter.clone())).await;
        return Ok(());
    }

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

    // 调度 + 物理绑定连接（先查域名→网卡规则，否则按策略调度；proc_name=None）
    let nic = engine.pick_nic(&target_display, port, None);
    nic.active.fetch_add(1, Ordering::Relaxed);
    let _guard = ConnGuard(nic.clone());

    // 域名目标需先完成双栈解析（A + AAAA）：解析全失败时在登记连接前提前返回，与既有一致。
    // 字面 IPv4（ATYP=0x01）不做任何解析，保持既有纯 IPv4 路径。
    let dual_addrs: Option<ResolvedAddrs> = if literal_ip.is_some() {
        None
    } else {
        let addrs = engine.resolve_host_dual(&nic, &target_display, port).await;
        if addrs.v4.is_none() && addrs.v6.is_none() {
            engine.log(if engine.zh {
                format!("[DNS失败] 无法解析域名 {target_display}")
            } else {
                format!("[DNS failed] cannot resolve {target_display}")
            });
            client.write_all(&[0x05, 0x04, 0x00, 0x01, 0, 0, 0, 0, 0, 0]).await?;
            return Ok(());
        }
        Some(addrs)
    };

    engine.log(if engine.zh {
        format!("[调度分配] 新连接 -> [{}] | 目标: {}:{}", nic.name, target_display, port)
    } else {
        format!("[Dispatch] new connection -> [{}] | target: {}:{}", nic.name, target_display, port)
    });
    let _ctg = engine.register_conn(format!("{target_display}:{port}"), nic.name.clone(), "SOCKS");

    let mut upstream = if let Some(ip) = literal_ip {
        // ATYP=0x01 字面 IPv4：保持既有纯 IPv4 路径不变（直连 connect_via_nic，无双栈/超时包裹）
        match connect_via_nic(&nic, SocketAddr::V4(SocketAddrV4::new(ip, port))).await {
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
        }
    } else {
        // 域名目标：按 IP 版本偏好做 Happy-Eyeballs 双栈拨号与回退
        let addrs = dual_addrs.expect("域名路径必然已完成双栈解析");
        match dial_dual(&engine, &nic, &addrs, port, &engine.ip_version, DIAL_FAMILY_TIMEOUT).await {
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
    // 逐字节读取直到 \r\n\r\n（请求头），避免吞掉请求体。
    // 整个请求头读取加超时，回收连上却迟迟不发完整请求头的僵死连接。
    let read_header = async {
        let mut header = Vec::with_capacity(1024);
        let mut byte = [0u8; 1];
        loop {
            client.read_exact(&mut byte).await?;
            header.push(byte[0]);
            if header.len() >= 4 && &header[header.len() - 4..] == b"\r\n\r\n" {
                return std::io::Result::Ok((header, false));
            }
            if header.len() > MAX_HEADER_BYTES {
                return std::io::Result::Ok((header, true)); // 超长：交由外层回 431
            }
        }
    };
    let (header, oversized) = match tokio::time::timeout(HANDSHAKE_TIMEOUT, read_header).await {
        Ok(Ok(v)) => v,
        _ => return Ok(()), // 超时或读错误：放弃该连接
    };
    if oversized {
        let _ = client
            .write_all(b"HTTP/1.1 431 Request Header Fields Too Large\r\nConnection: close\r\n\r\n")
            .await;
        return Ok(());
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

    // 调度 + 物理绑定连接（先查域名→网卡规则，否则按策略调度；proc_name=None）
    let nic = engine.pick_nic(&dst_host, dst_port, None);
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

    let mut upstream = match connect_via_nic(&nic, SocketAddr::V4(dst)).await {
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
                    let ip_str = nic.ipv4.to_string();
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

        // 实时连接列表快照：按连接 ID（＝发生顺序）稳定排序后取最新 80 条，
        // 避免 HashMap 遍历顺序不确定导致前端列表每秒乱跳/闪烁。
        let snapshot: Vec<ConnInfo> = match conns.lock() {
            Ok(map) => {
                let mut v: Vec<ConnInfo> = map.values().cloned().collect();
                v.sort_by_key(|c| c.id);
                // 保留最新的 80 条（ID 越大越新）
                if v.len() > 80 {
                    v.drain(0..v.len() - 80);
                }
                v
            }
            Err(_) => Vec::new(),
        };
        let _ = app.emit("hmx-connections", snapshot);
    }

    // 停止后还原默认托盘提示
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_tooltip(Some("HypoMuxPlus · 多网卡带宽聚合工具"));
    }
}

// ============================= 属性测试 =============================

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        // Feature: network-capability-expansion, Property 1
        // SOCKS5 IPv6 请求头解析 round-trip：任意 IPv6 地址与端口编码为 ATYP=0x04 地址段后
        // 再解析，应还原出等价的 IPv6 地址与端口。
        // Validates: Requirements 1.1
        #![proptest_config(ProptestConfig { cases: 100, ..ProptestConfig::default() })]

        #[test]
        fn prop_socks5_v6_addr_roundtrip(
            segs in prop::array::uniform8(any::<u16>()),
            port in any::<u16>(),
        ) {
            let addr = Ipv6Addr::new(
                segs[0], segs[1], segs[2], segs[3], segs[4], segs[5], segs[6], segs[7],
            );
            let encoded = build_socks5_v6_addr(addr, port);
            let (decoded_addr, decoded_port) =
                parse_socks5_v6_addr(&encoded).expect("合法 ATYP=0x04 地址段应可解析");
            prop_assert_eq!(decoded_addr, addr);
            prop_assert_eq!(decoded_port, port);
        }
    }

    proptest! {
        // Feature: network-capability-expansion, Property 2
        // 双栈地址族选择（pick_family）综合正确性：
        //   - v4only 结果绝不含 V6；
        //   - 单族目标只含该族；
        //   - 双栈同在时含两族且首位由 pref 决定（v4first=>V4 首，v6first/auto=>V6 首）。
        // Validates: Requirements 1.3, 1.5, 1.6
        #![proptest_config(ProptestConfig { cases: 100, ..ProptestConfig::default() })]

        #[test]
        fn prop_pick_family_correctness(
            pref in prop::sample::select(vec!["auto", "v4first", "v6first", "v4only", "bogus"]),
            has_v4 in any::<bool>(),
            has_v6 in any::<bool>(),
        ) {
            let out = pick_family(pref, has_v4, has_v6);

            // v4only 绝不含 V6
            if pref == "v4only" {
                prop_assert!(!out.contains(&Family::V6));
            }

            match (has_v4, has_v6) {
                (false, false) => prop_assert!(out.is_empty()),
                (true, false) => prop_assert_eq!(out, vec![Family::V4]),
                (false, true) => {
                    // v4only 排除 V6，仅返回空；否则只含 V6
                    if pref == "v4only" {
                        prop_assert!(out.is_empty());
                    } else {
                        prop_assert_eq!(out, vec![Family::V6]);
                    }
                }
                (true, true) => {
                    if pref == "v4only" {
                        prop_assert_eq!(out, vec![Family::V4]);
                    } else {
                        // 双栈：含两族且首位由 pref 决定
                        prop_assert_eq!(out.len(), 2);
                        prop_assert!(out.contains(&Family::V4) && out.contains(&Family::V6));
                        let first = out[0];
                        if pref == "v4first" {
                            prop_assert_eq!(first, Family::V4);
                        } else {
                            // v6first / auto / 未知值 => V6 首
                            prop_assert_eq!(first, Family::V6);
                        }
                    }
                }
            }
        }
    }

    proptest! {
        // Feature: network-capability-expansion, Property 3
        // AAAA 查询/应答 round-trip：build_dns_query_type(host,28) 问题段可还原同一 host；
        // 任意 IPv6 地址构造的 AAAA 应答经 parse_dns_aaaa 还原该地址。
        // Validates: Requirements 1.4
        #![proptest_config(ProptestConfig { cases: 100, ..ProptestConfig::default() })]

        #[test]
        fn prop_aaaa_query_and_answer_roundtrip(
            labels in prop::collection::vec("[a-z][a-z0-9]{0,7}", 1..=4),
            segs in prop::array::uniform8(any::<u16>()),
        ) {
            let host = labels.join(".");
            // 1) 查询问题段可还原 host（qtype=28）
            let query = build_dns_query_type(&host, 28);
            // 解析问题段域名：从偏移 12 起逐标签读取
            let mut pos = 12usize;
            let mut got_labels: Vec<String> = Vec::new();
            loop {
                let len = query[pos] as usize;
                if len == 0 { break; }
                let s = String::from_utf8(query[pos + 1..pos + 1 + len].to_vec()).unwrap();
                got_labels.push(s);
                pos += 1 + len;
            }
            prop_assert_eq!(got_labels.join("."), host.clone());
            // qtype 字段应为 28（AAAA）
            let qtype = u16::from_be_bytes([query[pos + 1], query[pos + 2]]);
            prop_assert_eq!(qtype, 28u16);

            // 2) 构造一条 AAAA 应答并解析还原地址
            let addr = Ipv6Addr::new(
                segs[0], segs[1], segs[2], segs[3], segs[4], segs[5], segs[6], segs[7],
            );
            // 应答 = 查询头(QR=1) + 原问题段 + 一条 AAAA 记录（指向问题段的压缩指针 0xC00C）
            let mut resp: Vec<u8> = Vec::new();
            resp.extend_from_slice(&query[0..2]); // ID
            resp.extend_from_slice(&[0x81, 0x80]); // QR=1 RA=1
            resp.extend_from_slice(&[0x00, 0x01]); // QDCOUNT=1
            resp.extend_from_slice(&[0x00, 0x01]); // ANCOUNT=1
            resp.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // NS/AR=0
            resp.extend_from_slice(&query[12..pos + 5]); // 问题段（名称 + 0x00 + qtype + qclass）
            resp.extend_from_slice(&[0xC0, 0x0C]); // 名称压缩指针
            resp.extend_from_slice(&[0x00, 0x1C]); // TYPE = AAAA(28)
            resp.extend_from_slice(&[0x00, 0x01]); // CLASS = IN
            resp.extend_from_slice(&[0x00, 0x00, 0x00, 0x3C]); // TTL
            resp.extend_from_slice(&[0x00, 0x10]); // RDLENGTH = 16
            resp.extend_from_slice(&addr.octets());

            let parsed = parse_dns_aaaa(&resp);
            prop_assert_eq!(parsed, Some(addr));
        }
    }

    proptest! {
        // Feature: network-capability-expansion, Property 9
        // SOCKS5 UDP 请求头解析 round-trip：任意目标（IPv4/IPv6/域名）与端口经
        // build_socks_udp_header 封装 + 任意载荷后，parse_socks_udp_header 应还原等价目标
        // 且返回正确的载荷偏移（= 头长度）。
        // Validates: Requirements 4.2, 6.6
        #![proptest_config(ProptestConfig { cases: 100, ..ProptestConfig::default() })]

        #[test]
        fn prop_socks_udp_header_roundtrip(
            which in 0u8..3,
            v4 in any::<u32>(),
            segs in prop::array::uniform8(any::<u16>()),
            labels in prop::collection::vec("[a-z][a-z0-9]{0,7}", 1..=3),
            port in any::<u16>(),
            payload in prop::collection::vec(any::<u8>(), 0..64),
        ) {
            let target = match which {
                0 => SocksUdpTarget::V4(SocketAddrV4::new(Ipv4Addr::from(v4), port)),
                1 => {
                    let ip = Ipv6Addr::new(segs[0], segs[1], segs[2], segs[3], segs[4], segs[5], segs[6], segs[7]);
                    SocksUdpTarget::V6(SocketAddrV6::new(ip, port, 0, 0))
                }
                _ => SocksUdpTarget::Domain(labels.join("."), port),
            };
            let mut buf = build_socks_udp_header(&target);
            let header_len = buf.len();
            buf.extend_from_slice(&payload);

            let (parsed, off) = parse_socks_udp_header(&buf).expect("合法 UDP 头应可解析");
            prop_assert_eq!(off, header_len);
            prop_assert_eq!(&buf[off..], &payload[..]);
            prop_assert_eq!(parsed, target);
        }
    }

    proptest! {
        // Feature: network-capability-expansion, Property 10
        // 进程规则匹配大小写不敏感（match_proc_rule）：规则名与查询名忽略大小写相等时命中，
        // 否则返回 None。
        // Validates: Requirements 5.1
        #![proptest_config(ProptestConfig { cases: 100, ..ProptestConfig::default() })]

        #[test]
        fn prop_match_proc_rule_case_insensitive(
            name in "[a-zA-Z]{1,10}\\.exe",
            ifindex in any::<u32>(),
            query_upper in any::<bool>(),
            other in "[a-z]{1,6}\\.exe",
        ) {
            // 规则名存入时小写（与 start() 解析一致）
            let rules = vec![(name.to_lowercase(), RuleAction::Nic(ifindex))];
            // 查询名用任意大小写变体
            let q = if query_upper { name.to_uppercase() } else { name.clone() };
            prop_assert_eq!(match_proc_rule(&rules, &q), Some(RuleAction::Nic(ifindex)));
            // 不同名（确保确实不等）应未命中
            if other.to_lowercase() != name.to_lowercase() {
                prop_assert_eq!(match_proc_rule(&rules, &other), None);
            }
        }
    }

    proptest! {
        // Feature: network-capability-expansion, Property 11
        // 规则动作解析 round-trip（RuleAction）：合法动作串解析再回写等价；
        // nic:<n> 解析出的接口索引等于 n。
        // Validates: Requirements 5.2
        #![proptest_config(ProptestConfig { cases: 100, ..ProptestConfig::default() })]

        #[test]
        fn prop_rule_action_roundtrip(
            which in 0u8..3,
            n in any::<u32>(),
        ) {
            let action = match which {
                0 => RuleAction::Direct,
                1 => RuleAction::Aggregate,
                _ => RuleAction::Nic(n),
            };
            let s = rule_action_to_string(action);
            let parsed = parse_rule_action(&s);
            prop_assert_eq!(parsed, Some(action));
            if let RuleAction::Nic(idx) = action {
                prop_assert_eq!(idx, n);
                prop_assert_eq!(s, format!("nic:{}", n));
            }
        }
    }

    proptest! {
        // Feature: network-capability-expansion, Property 22
        // 延迟统计综合正确性（compute_latency_stats）。
        // Validates: Requirements 9.1, 9.2, 9.3, 9.5
        #![proptest_config(ProptestConfig { cases: 100, ..ProptestConfig::default() })]

        #[test]
        fn prop_compute_latency_stats(
            samples in prop::collection::vec(prop::option::of(0u64..2000), 0..20),
        ) {
            let stats = compute_latency_stats(&samples);
            let ok: Vec<u64> = samples.iter().filter_map(|s| *s).collect();
            let total = samples.len();
            let failed = total - ok.len();

            if total == 0 || ok.is_empty() {
                // 全失败 / 无样本：不可用标记
                prop_assert_eq!(stats.loss_pct, 1.0);
                prop_assert_eq!(stats.jitter, -1);
                prop_assert_eq!(stats.min, -1);
                prop_assert_eq!(stats.avg, -1);
            } else {
                // loss_pct = 失败/总数 ∈ [0,1]
                let expected_loss = failed as f64 / total as f64;
                prop_assert!((stats.loss_pct - expected_loss).abs() < 1e-9);
                prop_assert!(stats.loss_pct >= 0.0 && stats.loss_pct <= 1.0);
                // min <= avg
                prop_assert!(stats.min <= stats.avg);
                // jitter >= 0；成功样本全相等时 jitter = 0
                prop_assert!(stats.jitter >= 0);
                if ok.iter().all(|&x| x == ok[0]) {
                    prop_assert_eq!(stats.jitter, 0);
                }
                // min 为成功样本最小值
                prop_assert_eq!(stats.min as u64, *ok.iter().min().unwrap());
            }
        }
    }

    proptest! {
        // Feature: network-capability-expansion, Property 12
        // 进程规则优先级与无进程回退（decide_rule_action，即 pick_nic 的纯决策部分）：
        //   - 命中进程规则时结果等于该进程动作，无论是否同时命中域名规则；
        //   - proc_name=None 时结果与「无进程规则」路径（仅域名规则）一致。
        // Validates: Requirements 5.3, 5.6
        #![proptest_config(ProptestConfig { cases: 100, ..ProptestConfig::default() })]

        #[test]
        fn prop_decide_rule_action_priority_and_fallback(
            proc_name in "[a-z]{1,8}\\.exe",
            proc_ifindex in any::<u32>(),
            domain in "[a-z]{1,8}\\.[a-z]{2,4}",
            domain_ifindex in any::<u32>(),
            port in any::<u16>(),
            // 是否让域名规则也命中同一 host
            domain_matches in any::<bool>(),
        ) {
            let rules_proc = vec![(proc_name.to_lowercase(), RuleAction::Nic(proc_ifindex))];
            // 域名规则：命中场景用相同 domain，否则用一个不会匹配的模式
            let rules_nic = if domain_matches {
                vec![(domain.clone(), domain_ifindex)]
            } else {
                vec![("no-such-domain-zzz.invalid".to_string(), domain_ifindex)]
            };
            let host = domain.clone();

            // 1) 命中进程规则时，结果 = 进程动作（无论域名是否命中）
            let with_proc = decide_rule_action(&rules_proc, &rules_nic, Some(&proc_name), &host, port);
            prop_assert_eq!(with_proc, Some(RuleAction::Nic(proc_ifindex)));

            // 2) proc_name=None 时，结果 == 仅域名规则的决策
            let none_proc = decide_rule_action(&rules_proc, &rules_nic, None, &host, port);
            let domain_only = decide_rule_action(&[], &rules_nic, None, &host, port);
            prop_assert_eq!(none_proc, domain_only);
            if domain_matches {
                prop_assert_eq!(none_proc, Some(RuleAction::Nic(domain_ifindex)));
            } else {
                prop_assert_eq!(none_proc, None);
            }
        }
    }

    proptest! {
        // Feature: network-capability-expansion, Property 15
        // SWRR 加权轮询长期比例正确（swrr_pick_index）：对一组正权重，经整数个完整周期
        // （sum(weights) 次为一周期）的选择后，各下标被选次数恰为 cycles * weight_i；
        // 同时最少连接（least_conn_pick_pos）始终选出 活跃/权重 最小者。
        // Validates: Requirements 6.3
        #![proptest_config(ProptestConfig { cases: 100, ..ProptestConfig::default() })]

        #[test]
        fn prop_swrr_and_least_conn(
            weights in prop::collection::vec(1i64..10, 2..6),
            cycles in 1usize..20,
            active in prop::collection::vec(0i64..100, 2..6),
        ) {
            // ---- SWRR 长期比例 ----
            let n = weights.len();
            let pool: Vec<usize> = (0..n).collect();
            let sum: i64 = weights.iter().sum();
            let mut cur = vec![0i64; n];
            let mut counts = vec![0usize; n];
            let iters = cycles * sum as usize;
            for _ in 0..iters {
                let idx = swrr_pick_index(&pool, &weights, &mut cur);
                counts[idx] += 1;
            }
            // 整数个完整周期后，各下标次数恰为 cycles * weight_i
            for i in 0..n {
                prop_assert_eq!(counts[i], cycles * weights[i] as usize);
            }

            // ---- 最少连接 ----
            let wts: Vec<u32> = active.iter().map(|_| 1u32).collect(); // 权重均一时选活跃最小者
            let pos = least_conn_pick_pos(&active, &wts);
            let chosen = active[pos] as f64 / 1.0;
            for i in 0..active.len() {
                let v = active[i] as f64 / 1.0;
                prop_assert!(chosen <= v);
            }
        }
    }

    proptest! {
        // Feature: network-capability-expansion, Property 14
        // 令牌桶取用不变量（RateLimiter）：初始令牌=容量=速率；任意取用序列后令牌数
        // 始终不超过容量（补充受容量上限约束，不产生凭空额度）。
        // Validates: Requirements 6.3
        #![proptest_config(ProptestConfig { cases: 100, ..ProptestConfig::default() })]

        #[test]
        fn prop_rate_limiter_token_invariant(
            rate in 1u64..10_000_000,
            takes in prop::collection::vec(0.0f64..5000.0, 0..30),
        ) {
            let rl = RateLimiter::new(rate);
            let cap = rate as f64;
            // 初始：令牌 = 容量 = 速率
            {
                let t = *rl.tokens.lock().unwrap();
                prop_assert!((t - cap).abs() < 1e-6);
            }
            prop_assert!((rl.capacity - cap).abs() < 1e-6);
            // 任意取用后：令牌不超过容量（含极小刷新裕度），且不为负
            for w in takes {
                let _ = rl.try_take(w);
                let t = *rl.tokens.lock().unwrap();
                prop_assert!(t <= cap + 1.0);
                prop_assert!(t >= -1e-6);
            }
        }
    }

    proptest! {
        // Feature: network-capability-expansion, Property 4
        // 既有域名/端口规则匹配（pattern_match）不变：精确名与其任意子域命中；
        // 端口限定仅在端口相等（或查询端口为 0=未指定）时命中。
        // Validates: Requirements 1.7, 6.2
        #![proptest_config(ProptestConfig { cases: 100, ..ProptestConfig::default() })]

        #[test]
        fn prop_pattern_match(
            labels in prop::collection::vec("[a-z]{1,6}", 2..4),
            sub in "[a-z]{1,6}",
            pat_port in 1u16..65535,
            query_port in 0u16..65535,
        ) {
            let domain = labels.join(".");
            // 精确匹配（无端口限定）
            prop_assert!(pattern_match(&domain, &domain, 0));
            // 子域匹配
            let subdomain = format!("{sub}.{domain}");
            prop_assert!(pattern_match(&domain, &subdomain, 0));
            // 非子域不匹配（前缀无点）
            let glued = format!("{sub}{domain}");
            if glued != domain {
                prop_assert!(!pattern_match(&domain, &glued, 0));
            }
            // 端口限定：pattern "domain:pat_port"
            let pat = format!("{domain}:{pat_port}");
            // 查询端口为 0（未指定）时端口限定不生效 => 命中
            prop_assert!(pattern_match(&pat, &domain, 0));
            // 查询端口等于 pat_port => 命中；不等 => 不命中
            prop_assert!(pattern_match(&pat, &domain, pat_port));
            if query_port != 0 && query_port != pat_port {
                prop_assert!(!pattern_match(&pat, &domain, query_port));
            }
        }
    }

    // ---- 既有纯函数示例测试（11.1）：DNS / 头部 / 端口解析 / 调度策略解析 ----

    #[test]
    fn example_build_and_parse_dns_a_roundtrip() {
        // 构造 A 查询后，拼一条指向问题段的 A 应答，parse_dns_a 应还原该 IP
        let q = build_dns_query("example.com");
        // 定位问题段结束
        let mut pos = 12usize;
        loop {
            let len = q[pos] as usize;
            if len == 0 { break; }
            pos += 1 + len;
        }
        let qend = pos + 1 + 4; // 0x00 + qtype(2) + qclass(2)
        let mut resp: Vec<u8> = Vec::new();
        resp.extend_from_slice(&q[0..2]); // ID
        resp.extend_from_slice(&[0x81, 0x80]); // QR=1
        resp.extend_from_slice(&[0x00, 0x01]); // QD=1
        resp.extend_from_slice(&[0x00, 0x01]); // AN=1
        resp.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
        resp.extend_from_slice(&q[12..qend]); // 问题段
        resp.extend_from_slice(&[0xC0, 0x0C, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x3C, 0x00, 0x04]);
        resp.extend_from_slice(&[203, 0, 113, 7]);
        assert_eq!(parse_dns_a(&resp), Some(Ipv4Addr::new(203, 0, 113, 7)));
    }

    #[test]
    fn example_dns_skip_name() {
        // "\x03www\x00" 从 pos=0 跳过后应指向结尾（pos=5）
        let buf = [3u8, b'w', b'w', b'w', 0u8];
        assert_eq!(dns_skip_name(&buf, 0), Some(5));
        // 压缩指针：0xC0 0x0C 跳 2 字节
        let ptr = [0xC0u8, 0x0C];
        assert_eq!(dns_skip_name(&ptr, 0), Some(2));
    }

    #[test]
    fn example_split_host_port() {
        assert_eq!(split_host_port("example.com:8080", 80), ("example.com".to_string(), 8080));
        assert_eq!(split_host_port("example.com", 80), ("example.com".to_string(), 80));
        // 以 '[' 起始（IPv6 字面量形式）视为非 host:port，返回空
        assert_eq!(split_host_port("[::1]:80", 80), (String::new(), 0));
    }

    #[test]
    fn example_find_header_case_insensitive() {
        let lines = vec!["GET / HTTP/1.1", "Host: example.com", "X-Test: 1"];
        assert_eq!(find_header(&lines, "host"), "example.com");
        assert_eq!(find_header(&lines, "HOST"), "example.com");
        assert_eq!(find_header(&lines, "missing"), "");
    }

    #[test]
    fn example_build_origin_header_strips_hop_headers() {
        let lines = vec![
            "GET http://x/y HTTP/1.1",
            "Host: x",
            "Proxy-Connection: keep-alive",
            "Proxy-Authorization: Basic zzz",
            "Accept: */*",
        ];
        let out = build_origin_header("GET", "/y", "HTTP/1.1", &lines);
        let text = String::from_utf8(out).unwrap();
        assert!(text.starts_with("GET /y HTTP/1.1\r\n"));
        assert!(text.to_lowercase().contains("host: x"));
        assert!(text.contains("Accept: */*"));
        // hop-by-hop 头被剔除
        assert!(!text.to_lowercase().contains("proxy-connection"));
        assert!(!text.to_lowercase().contains("proxy-authorization"));
        assert!(text.ends_with("\r\n\r\n"));
    }

    #[test]
    fn example_strategy_parse() {
        // 以 super::Strategy 限定，避免与 proptest 预导入的 Strategy trait 命名冲突
        use super::Strategy as Sched;
        assert!(Sched::parse("least") == Sched::LeastConn);
        assert!(Sched::parse("weighted") == Sched::WeightedSpeed);
        assert!(Sched::parse("rr") == Sched::RoundRobin);
        assert!(Sched::parse("anything-else") == Sched::RoundRobin);
    }
}
