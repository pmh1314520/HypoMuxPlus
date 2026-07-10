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

/// 上游代理条目（Upstream_Proxy，Req 1）。serde camelCase 与前端契约一致。
///
/// 一条上游代理节点：唯一标识 `id`、类型 `kind`（`socks5` / `http`）、
/// 主机地址 `host`（域名或 IP，≤253 字符）、端口，以及可选的认证凭据与备注名。
/// `username` / `password` / `label` 缺省，未配置认证时为 `None` / 空字符串。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct UpstreamProxy {
    /// Upstream_Id：同组唯一、稳定、不复用，供 UpstreamBinding 引用
    pub id: String,
    /// 上游类型："socks5" | "http"
    pub kind: String,
    /// 上游主机地址：域名或 IP（≤253 字符）
    pub host: String,
    /// 上游端口：1..=65535
    pub port: u16,
    /// 认证用户名（1..=255）；无认证则 None
    #[serde(default)]
    pub username: Option<String>,
    /// 认证密码（1..=255）；无认证则 None
    #[serde(default)]
    pub password: Option<String>,
    /// Upstream_Label（≤64），用于日志与 UI 展示
    #[serde(default)]
    pub label: String,
}

/// 网卡↔上游映射：一条 Upstream_Binding（Req 2）。
///
/// 一对一为 `upstream_ids.len() == 1`；一网卡多上游为 `len() > 1`；
/// 多张网卡共享同一 id 亦允许。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct UpstreamBinding {
    /// 网卡权威标识 IfIndex
    pub if_index: u32,
    /// 该网卡绑定的上游 id 列表（引用 UpstreamProxy.id）
    pub upstream_ids: Vec<String>,
}

// ==== 上游健康探测数据结构（Health_Prober，Req 1）[新增] ====
//
// 以下类型服务于「上游节点健康探测与故障熔断/自动恢复」能力。均为纯数据 /
// 配置载体，未启用（`HealthConfig.enabled == false`）时不参与任何既有分支，
// 既有直连聚合 / 上游链 / 调度 / 限速 / DNS 行为字节级不变（Req 13.1）。

/// 单个上游的健康状态（Health_Prober，Req 1）。
///
/// - `Healthy`：可作为优选候选。
/// - `CircuitOpen`：因连续探测/连接失败达阈值而被熔断，暂时排除出优选候选，
///   经冷却期后允许半开探测恢复。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum HealthState {
    Healthy,
    CircuitOpen,
}

/// 单个上游的健康度量（Upstream_Health，Req 1）。纯数据，可 Clone。
///
/// 记录当前可用性状态、连续失败计数、最近一次成功探测延迟样本，以及进入熔断
/// 的时间戳（毫秒 since epoch），供健康状态机与冷却期判定使用。
#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub(crate) struct UpstreamHealth {
    /// 可用性状态：Healthy / CircuitOpen
    pub state: HealthState,
    /// 连续失败计数（成功探测清零）
    pub consecutive_failures: u32,
    /// 最近一次成功探测的延迟样本（毫秒）；从未成功则 None
    pub last_latency_ms: Option<u64>,
    /// 进入熔断的时间戳（毫秒 since epoch）；非熔断则 None
    pub opened_at_ms: Option<u64>,
}

impl Default for UpstreamHealth {
    /// 初始视为健康：无失败、无延迟样本、未熔断。
    fn default() -> Self {
        UpstreamHealth {
            state: HealthState::Healthy,
            consecutive_failures: 0,
            last_latency_ms: None,
            opened_at_ms: None,
        }
    }
}

/// 健康探测配置（HealthConfig，Req 1）。均可配置，带缺省值。
///
/// `enabled` 默认 false（Req 1.7）：未启用时全部被引用上游视为 Healthy，
/// 按既有回退与调度逻辑处理连接（Req 1.6）。
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub(crate) struct HealthConfig {
    /// 是否启用后台健康探测（默认 false，Req 1.7）
    pub enabled: bool,
    /// 探测间隔（毫秒），缺省 30_000（Req 1.1）
    pub interval_ms: u64,
    /// 探测超时（毫秒），缺省 5_000（≤5s，Req 1.2）
    pub timeout_ms: u64,
    /// 熔断阈值：连续失败达此次数进入 CircuitOpen，缺省 3（Req 1.3）
    pub fail_threshold: u32,
    /// 冷却期（毫秒）：熔断后经此时长允许半开探测，缺省 60_000（Req 1.4/1.5）
    pub cooldown_ms: u64,
}

impl Default for HealthConfig {
    /// 缺省：未启用 + 30s 间隔 / 5s 超时 / 3 次阈值 / 60s 冷却（Req 1.1/1.2/1.3/1.4/1.7）。
    fn default() -> Self {
        HealthConfig {
            enabled: false,
            interval_ms: 30_000,
            timeout_ms: 5_000,
            fail_threshold: 3,
            cooldown_ms: 60_000,
        }
    }
}

/// 探测事件（喂给健康状态机的输入，Req 1）。
///
/// - `Success(latency_ms)`：一次探测在超时内完成握手，携带本次延迟样本。
/// - `Failure`：一次探测失败（超时 / 连接被拒 / 握手失败）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum ProbeEvent {
    Success(u64),
    Failure,
}

// ==== 健康状态机纯函数（Health_Prober，Req 1）[新增] ====
//
// 下列三个函数均为不依赖 IO 的纯函数：后台探测任务仅负责「按间隔拨号 + 记录
// 成功/失败/延迟」，把状态迁移与候选判定委托给这里，从而可被属性测试完全覆盖
// （Property 1/2）。对任意输入均不 panic（时间差用 saturating_sub 防下溢）。

/// 健康状态机迁移（Req 1.2/1.3/1.5）。给定当前健康度量、探测事件、配置与时刻，
/// 计算迁移后的新度量。语义：
///
/// - `Success(latency)` => `Healthy`，连续失败计数清零，更新最近延迟样本，
///   清除熔断时间戳（涵盖 `CircuitOpen` 下半开探测成功的 Circuit_Recovery）。
/// - `Failure` 且 `consecutive_failures + 1 >= fail_threshold` => `CircuitOpen`，
///   记录进入熔断的时间戳 `opened_at_ms = now_ms`。
/// - `Failure` 未达阈值 => 保持 `Healthy`，连续失败计数 +1。
///
/// 纯函数，无 IO，对任意输入不 panic（失败计数用 saturating_add 防溢出）。
#[allow(dead_code)]
pub(crate) fn health_transition(
    cur: UpstreamHealth,
    event: ProbeEvent,
    cfg: HealthConfig,
    now_ms: u64,
) -> UpstreamHealth {
    match event {
        // 探测成功：恒恢复为 Healthy，清零失败计数、更新延迟样本、清除熔断时间戳。
        ProbeEvent::Success(latency) => UpstreamHealth {
            state: HealthState::Healthy,
            consecutive_failures: 0,
            last_latency_ms: Some(latency),
            opened_at_ms: None,
        },
        // 探测失败：失败计数 +1；达到阈值则熔断并记录时间戳，否则保持原状态。
        ProbeEvent::Failure => {
            let failures = cur.consecutive_failures.saturating_add(1);
            // fail_threshold 为 0 时视为下限 1，避免「零阈值永不熔断」的歧义。
            let threshold = cfg.fail_threshold.max(1);
            if failures >= threshold {
                UpstreamHealth {
                    state: HealthState::CircuitOpen,
                    consecutive_failures: failures,
                    last_latency_ms: cur.last_latency_ms,
                    opened_at_ms: Some(now_ms),
                }
            } else {
                UpstreamHealth {
                    state: HealthState::Healthy,
                    consecutive_failures: failures,
                    last_latency_ms: cur.last_latency_ms,
                    opened_at_ms: cur.opened_at_ms,
                }
            }
        }
    }
}

/// 冷却期与半开判定（Req 1.4/1.5）。当上游处于 `CircuitOpen` 且距进入熔断已达
/// 冷却期（`now_ms - opened_at >= cooldown_ms`）时返回 true，表示应发起一次半开探测。
///
/// 非 `CircuitOpen` 或缺少熔断时间戳时恒为 false。时间差用 saturating_sub 防下溢，
/// 对任意输入不 panic。
#[allow(dead_code)]
pub(crate) fn should_half_open(h: &UpstreamHealth, cfg: HealthConfig, now_ms: u64) -> bool {
    match h.state {
        HealthState::CircuitOpen => match h.opened_at_ms {
            Some(opened) => now_ms.saturating_sub(opened) >= cfg.cooldown_ms,
            None => false,
        },
        HealthState::Healthy => false,
    }
}

/// 该上游当前是否可作为优选候选（Req 1.4/2.6）。
///
/// - `Healthy` 恒可选（true）。
/// - `CircuitOpen` 未过冷却期 => 不可选（false，排除出优选候选，Req 1.4）。
/// - `CircuitOpen` 已过冷却期 => 可选（true，允许半开纳入候选）。
///
/// 纯函数，对任意输入不 panic。
#[allow(dead_code)]
pub(crate) fn is_selectable(h: &UpstreamHealth, cfg: HealthConfig, now_ms: u64) -> bool {
    match h.state {
        HealthState::Healthy => true,
        HealthState::CircuitOpen => should_half_open(h, cfg, now_ms),
    }
}

// ==== 上游加权优选纯函数（Upstream_Selector，Req 2）[新增] ====
//
// 权重与取样均用整数运算，保证「无随机性、可复现」（同一输入恒得同一结果），
// 并对任意输入不 panic（除零/空切片/溢出均已防护）。加权取样只决定「首选哪个
// 上游」，实际建连与失败后续仍走既有 establish_target + next_fallback 循环。

/// 权重定标常量：weight_i = WEIGHT_SCALE / (latency_i + LATENCY_BASE_MS)。
/// 取足够大的定标值使不同延迟档位间产生可区分的整数权重。
const WEIGHT_SCALE: u64 = 1_000_000;
/// 延迟平滑基值（ms）：并入分母避免除零，并抑制超低延迟样本独占全部权重。
const LATENCY_BASE_MS: u64 = 50;
/// 缺失延迟样本（`None`）视为的较大延迟基值（ms），使未探测/无样本的上游权重偏低
/// 但仍保留被选中的机会（Req 2.1「None 视为较大延迟」）。
const MISSING_LATENCY_MS: u64 = 10_000;

/// 在候选上游中按延迟加权确定性优选（Req 2.1/2.6）。
///
/// - `candidates`：该网卡当前 `is_selectable` 的上游 id 有序列表（调用方已排除熔断）。
/// - `latencies`：与 `candidates` 位置对应的最近延迟样本；`None` 视为较大延迟基值。
///   该切片长度可能与 `candidates` 不一致——按下标安全取，缺失即按 `None` 处理。
/// - `sched_idx`：复用既有调度序，在加权分布上做确定性取样（无随机性、可复现）。
///
/// 语义与不变量：
///   - `candidates` 为空 => `None`；
///   - 返回值恒 ∈ `candidates`（不引入候选集合外元素）；
///   - 权重 ∝ 1/(latency + base)，延迟越低权重越高；遍历连续 `sched_idx` 的选择
///     分布偏向低延迟候选；
///   - 全部候选延迟极大导致权重整体为 0 时，退化为按 `sched_idx` 轮转的确定性选取，
///     仍保证返回值 ∈ `candidates`。
///
/// 纯函数，无 IO，对任意输入不 panic。
#[allow(dead_code)]
pub(crate) fn select_weighted_upstream(
    candidates: &[String],
    latencies: &[Option<u64>],
    sched_idx: usize,
) -> Option<String> {
    let len = candidates.len();
    if len == 0 {
        return None;
    }

    // 计算每个候选的整数权重：weight_i = WEIGHT_SCALE / (lat_i + LATENCY_BASE_MS)。
    // 分母恒 >= LATENCY_BASE_MS(>0) 故不会除零；延迟极大时 weight 退化为 0。
    let mut total: u64 = 0;
    let mut weights: Vec<u64> = Vec::with_capacity(len);
    for i in 0..len {
        // latencies 长度可能与 candidates 不一致：越界即视为 None（缺失样本）。
        let lat = latencies
            .get(i)
            .copied()
            .flatten()
            .unwrap_or(MISSING_LATENCY_MS);
        let denom = lat.saturating_add(LATENCY_BASE_MS);
        let w = WEIGHT_SCALE / denom; // denom >= 50，绝不为零
        weights.push(w);
        total = total.saturating_add(w);
    }

    // 全部权重为 0（延迟整体极大）：退化为按 sched_idx 的确定性轮转，返回值仍 ∈ candidates。
    if total == 0 {
        return Some(candidates[sched_idx % len].clone());
    }

    // 在 [0, total) 上以 sched_idx 确定性取样，落入各候选的权重区间即选中。
    let target = (sched_idx as u64) % total;
    let mut cumulative: u64 = 0;
    for i in 0..len {
        cumulative = cumulative.saturating_add(weights[i]);
        if target < cumulative {
            return Some(candidates[i].clone());
        }
    }

    // 理论不可达（target < total == sum(weights)）：兜底返回最后一个候选，保证 ∈ candidates。
    Some(candidates[len - 1].clone())
}

// ==== 每网卡 DNS/DoH 数据结构（Per_NIC_DNS，Req 7）[新增] ====

/// 每网卡独立 DNS 解析配置（Per_NIC_DNS，Req 7）。
///
/// 为某张参与聚合的网卡指定独立的解析路径：明文 DNS 服务器地址或 DoH 端点 URL。
/// 未配置该网卡时走既有全局解析路径（零回归，Req 7.3）。
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct PerNicDns {
    /// 解析类型：明文 UDP DNS / DoH
    pub kind: DnsKind,
    /// 端点：明文形如 "1.1.1.1"，DoH 形如 "https://dns.google/dns-query"
    pub endpoint: String,
}

/// 每网卡 DNS 类型（Req 7）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
#[allow(dead_code)]
pub(crate) enum DnsKind {
    /// 明文 UDP DNS 服务器（IPv4/IPv6 地址）
    Plain,
    /// DNS over HTTPS 端点（https URL）
    Doh,
}

/// 每网卡 DNS 配置的命令传输条目（Per_NIC_DNS，Req 7）。
///
/// 前端以 `PerNicDnsCfg { ifIndex, kind, endpoint }` 数组下发，`start_boost` 命令按
/// camelCase 反序列化为本结构，再由 `engine::start` 映射为 `HashMap<u32, PerNicDns>`。
/// 未下发（空数组）时全部网卡走既有全局解析路径（零回归，Req 7.3/13.1）。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct PerNicDnsEntry {
    /// 承载网卡的 Windows 接口索引（IfIndex）
    pub if_index: u32,
    /// 解析类型：明文 UDP DNS / DoH
    pub kind: DnsKind,
    /// 端点：明文形如 "1.1.1.1"，DoH 形如 "https://dns.google/dns-query"
    pub endpoint: String,
}

/// 校验单张网卡的 DNS 端点是否合法（Req 7.5，Property 6）。
///
/// 语义与前端 `src/lib/dnsvalidate.ts` 的 `validateDnsEndpoint` 严格一致：
///   - `Plain` 通过当且仅当 `endpoint` 是合法 IPv4 或 IPv6 地址；
///   - `Doh`   通过当且仅当 `endpoint` 是 `https://` 开头且主机段非空的 URL；
///   - 其余一律不通过。
///
/// 纯函数、无 IO，对任意输入不 panic。IPv4/IPv6 校验直接复用标准库解析器
/// （前端手写解析器亦以「与标准库 `from_str` 行为一致」为目标，故语义等价）。
#[allow(dead_code)]
pub(crate) fn validate_dns_endpoint(kind: DnsKind, endpoint: &str) -> bool {
    match kind {
        DnsKind::Plain => {
            endpoint.parse::<Ipv4Addr>().is_ok() || endpoint.parse::<Ipv6Addr>().is_ok()
        }
        DnsKind::Doh => is_https_url_with_host(endpoint),
    }
}

/// 判定字符串是否为 `https://` 开头且主机段非空的 URL（DoH 端点）。
///
/// 手写解析而非依赖内置 URL 解析器，逐字节镜像前端 `isHttpsUrlWithHost`：
/// 去掉 `https://` 前缀后，主机段为路径 `/`、查询 `?`、片段 `#` 之前的部分；
/// 若带用户信息 `user:pass@host` 取 `@` 之后的权威主机；去除端口（兼容 IPv6
/// 字面量 `[::1]:443`）后要求主机段非空。对任意输入不 panic。
fn is_https_url_with_host(input: &str) -> bool {
    const PREFIX: &str = "https://";
    // 长度必须严格大于前缀（与前端 `input.length <= prefix.length` 一致）。
    if input.len() <= PREFIX.len() {
        return false;
    }
    // 取前缀（用 `get` 避免非 UTF-8 边界切片 panic；非 ASCII 首段必然不等于纯 ASCII 前缀）。
    match input.get(..PREFIX.len()) {
        Some(head) if head.eq_ignore_ascii_case(PREFIX) => {}
        _ => return false,
    }
    let rest = &input[PREFIX.len()..];

    // 截断到主机段结束：路径 / 查询 / 片段之前。
    let mut end = rest.len();
    for ch in ['/', '?', '#'] {
        if let Some(idx) = rest.find(ch) {
            if idx < end {
                end = idx;
            }
        }
    }
    let mut authority = &rest[..end];

    // 去掉用户信息部分（user:pass@host）。
    if let Some(at) = authority.rfind('@') {
        authority = &authority[at + 1..];
    }

    // 去掉端口部分（host:port），兼容 IPv6 字面量 [::1]:443。
    let host: &str = if authority.starts_with('[') {
        match authority.find(']') {
            Some(close) => &authority[1..close],
            None => return false,
        }
    } else {
        match authority.find(':') {
            Some(colon) => &authority[..colon],
            None => authority,
        }
    };

    !host.is_empty()
}

/// 上游不可用时的回退策略（Req 6）。
///
/// `Direct`：该网卡绑定的全部上游均不可用时，经该网卡物理出口以直连聚合方式直连真实目标；
/// `Fail`：以标准 SOCKS5/HTTP 错误应答向入站客户端返回连接失败。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum FallbackPolicy {
    Direct,
    Fail,
}

/// 剔除悬空上游引用并构建网卡→上游 id 列表映射（Req 2.6）。
///
/// 对每条 `UpstreamBinding`，过滤掉不在 `upstreams` 中的 id（悬空引用剔除，
/// 被删除条目对应绑定视为未绑定）；同一 `if_index` 的多条 binding 合并到同一列表；
/// 过滤后为空的列表等价于「该网卡未绑定上游」。纯函数（无 IO），供 `engine::start`
/// 构建绑定表与后续出口决策复用。
pub(crate) fn sanitize_bindings(
    upstreams: &HashMap<String, UpstreamProxy>,
    bindings: &[UpstreamBinding],
) -> HashMap<u32, Vec<String>> {
    let mut out: HashMap<u32, Vec<String>> = HashMap::new();
    for b in bindings {
        let entry = out.entry(b.if_index).or_default();
        for id in &b.upstream_ids {
            if upstreams.contains_key(id) {
                entry.push(id.clone());
            }
        }
    }
    out
}

// ============================================================================
// SOCKS5 上游握手纯函数（Req 3.3/3.6/9）
//
// 以下函数均为不依赖 IO 的纯函数，供 Upstream_Client 在建立到上游的隧道时
// 构造/解析 SOCKS5 版本协商与用户名/密码子协商（RFC 1928 / RFC 1929）报文，
// 并可被 proptest round-trip 属性测试独立验证。所有解析函数对截断/畸形输入
// 一律返回 `None`，绝不 panic、绝不 unwrap 用户数据。
//
// 注意：这些函数不触碰既有 SOCKS5 入站 `handle_socks` 解析路径，仅供上游
// 客户端方向使用；在被后续任务（阶段 D `connect_via_upstream`）引用前，
// 以 `#[allow(dead_code)]` 抑制未使用告警。
// ============================================================================

/// 构建 SOCKS5 版本协商请求（RFC 1928）：`VER NMETHODS METHODS...`。
///
/// - 无认证（`with_auth == false`）：仅声明「无认证」方法 `0x00`，即 `[0x05, 0x01, 0x00]`。
/// - 有认证（`with_auth == true`）：同时声明「无认证 `0x00`」与「用户名/密码 `0x02`」
///   两种方法，即 `[0x05, 0x02, 0x00, 0x02]`（Req 3.3）。保留无认证方法以兼容不要求
///   认证的上游，同时声明用户名/密码方法以支持需要认证的上游。
#[allow(dead_code)]
pub(crate) fn build_socks5_greeting(with_auth: bool) -> Vec<u8> {
    if with_auth {
        vec![0x05, 0x02, 0x00, 0x02]
    } else {
        vec![0x05, 0x01, 0x00]
    }
}

/// 解析 SOCKS5 服务端方法选择应答（RFC 1928）：`VER METHOD` -> 选定方法字节。
///
/// 返回上游选定的认证方法字节（`0x00` 无认证 / `0x02` 用户名密码 / `0xFF` 无可接受方法等）。
/// 当长度不足 2 字节或 `VER != 0x05` 时返回 `None`（截断/畸形一律 None，绝不 panic，Req 3.6）。
#[allow(dead_code)]
pub(crate) fn parse_socks5_method_reply(buf: &[u8]) -> Option<u8> {
    if buf.len() < 2 || buf[0] != 0x05 {
        return None;
    }
    Some(buf[1])
}

/// 构建 SOCKS5 用户名/密码子协商请求（RFC 1929）：`VER(0x01) ULEN USER PLEN PASS`。
///
/// 用户名与密码各以单字节长度前缀声明后跟原始字节。上层（前端校验）保证
/// 用户名与密码长度均在 `1..=255`（Req 1.5）；此处以 `u8` 承载长度前缀，
/// 对超出 255 字节的输入长度前缀会被 `as u8` 截断，故调用方须传入合法长度。
#[allow(dead_code)]
pub(crate) fn build_socks5_userpass(username: &str, password: &str) -> Vec<u8> {
    let u = username.as_bytes();
    let p = password.as_bytes();
    let mut out = Vec::with_capacity(3 + u.len() + p.len());
    out.push(0x01);
    out.push(u.len() as u8);
    out.extend_from_slice(u);
    out.push(p.len() as u8);
    out.extend_from_slice(p);
    out
}

/// 解析 SOCKS5 用户名/密码子协商应答（RFC 1929）：`VER STATUS` -> STATUS。
///
/// 返回 STATUS 字节：`0x00` 表示认证成功，任何非零值表示认证失败。
/// 当长度不足 2 字节时返回 `None`（截断/畸形一律 None，绝不 panic，Req 3.6）。
#[allow(dead_code)]
pub(crate) fn parse_socks5_userpass_reply(buf: &[u8]) -> Option<u8> {
    if buf.len() < 2 {
        return None;
    }
    Some(buf[1])
}

// ============================================================================
// SOCKS5 CONNECT 请求/应答纯函数（Req 3.1/3.5/3.6/9.1/9.2）
//
// 供 Upstream_Client 在 SOCKS5 版本协商（含可选认证）完成后，向上游发起
// CONNECT 请求以建立到真实目标的隧道。构造与解析互逆，可被 proptest round-trip
// 属性测试独立验证。所有解析函数对截断/畸形/非 UTF-8 输入一律返回 `None`，
// 绝不 panic。仅供上游客户端方向，不触碰既有入站 `handle_socks` 解析路径。
// ============================================================================

/// SOCKS5 CONNECT 目标地址（复用既有 ATYP 语义）。
///
/// - `V4`：ATYP=0x01，IPv4 字面地址 + 端口。
/// - `V6`：ATYP=0x04，IPv6 字面地址 + 端口。
/// - `Domain`：ATYP=0x03，域名交由上游解析（Req 3.5）+ 端口。
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum ConnectTarget {
    V4(Ipv4Addr, u16),
    V6(Ipv6Addr, u16),
    Domain(String, u16),
}

/// 构建 SOCKS5 CONNECT 请求（RFC 1928）：`VER(0x05) CMD(0x01) RSV(0x00) ATYP ADDR PORT`。
///
/// - ATYP=0x01（`V4`）：4 字节 IPv4 地址；
/// - ATYP=0x04（`V6`）：16 字节 IPv6 地址；
/// - ATYP=0x03（`Domain`）：1 字节长度前缀 + 域名原始字节（Req 3.5）。
///
/// PORT 以大端（网络字节序）两字节写入。域名长度前缀以 `u8` 承载，调用方
/// （前端校验）保证域名 ≤253 字符（Req 1.1），故不会溢出单字节长度。
#[allow(dead_code)]
pub(crate) fn build_socks5_connect_req(target: &ConnectTarget) -> Vec<u8> {
    let mut out = Vec::new();
    out.push(0x05); // VER
    out.push(0x01); // CMD = CONNECT
    out.push(0x00); // RSV
    match target {
        ConnectTarget::V4(ip, port) => {
            out.push(0x01);
            out.extend_from_slice(&ip.octets());
            out.extend_from_slice(&port.to_be_bytes());
        }
        ConnectTarget::V6(ip, port) => {
            out.push(0x04);
            out.extend_from_slice(&ip.octets());
            out.extend_from_slice(&port.to_be_bytes());
        }
        ConnectTarget::Domain(host, port) => {
            out.push(0x03);
            let bytes = host.as_bytes();
            out.push(bytes.len() as u8);
            out.extend_from_slice(bytes);
            out.extend_from_slice(&port.to_be_bytes());
        }
    }
    out
}

/// 解析 SOCKS5 CONNECT 请求（`build_socks5_connect_req` 的互逆函数）。
///
/// 校验 `VER==0x05 && CMD==0x01 && RSV==0x00` 及地址段长度与 build 完全匹配
/// （无多余尾随字节），据 ATYP 还原为 `ConnectTarget`。任何长度不足、字段非法、
/// ATYP 未知或域名段非 UTF-8 的输入一律返回 `None`（绝不 panic，Req 3.6）。
#[allow(dead_code)]
pub(crate) fn parse_socks5_connect_req(buf: &[u8]) -> Option<ConnectTarget> {
    if buf.len() < 4 {
        return None;
    }
    if buf[0] != 0x05 || buf[1] != 0x01 || buf[2] != 0x00 {
        return None;
    }
    match buf[3] {
        0x01 => {
            if buf.len() != 4 + 4 + 2 {
                return None;
            }
            let ip = Ipv4Addr::new(buf[4], buf[5], buf[6], buf[7]);
            let port = u16::from_be_bytes([buf[8], buf[9]]);
            Some(ConnectTarget::V4(ip, port))
        }
        0x04 => {
            if buf.len() != 4 + 16 + 2 {
                return None;
            }
            let mut octets = [0u8; 16];
            octets.copy_from_slice(&buf[4..20]);
            let port = u16::from_be_bytes([buf[20], buf[21]]);
            Some(ConnectTarget::V6(Ipv6Addr::from(octets), port))
        }
        0x03 => {
            let dlen = buf[4] as usize;
            if buf.len() != 5 + dlen + 2 {
                return None;
            }
            let host = std::str::from_utf8(&buf[5..5 + dlen]).ok()?.to_string();
            let port = u16::from_be_bytes([buf[5 + dlen], buf[5 + dlen + 1]]);
            Some(ConnectTarget::Domain(host, port))
        }
        _ => None,
    }
}

/// 解析 SOCKS5 CONNECT 应答（RFC 1928）：`VER REP RSV ATYP BND.ADDR BND.PORT`。
///
/// 返回 `(rep, consumed)`：`rep` 为应答状态字段（调用方据 `rep == 0x00` 判定隧道
/// 建立成功，Req 3.1），`consumed` 为该应答在缓冲区中占用的总字节数（便于调用方
/// 从流中剥离应答后开始转发）。消费长度按 ATYP 计算：0x01 => 4+4+2、0x04 => 4+16+2、
/// 0x03 => 4+1+len+2。当 `VER != 0x05`、长度不足或 ATYP 未知时返回 `None`
/// （绝不误判成功、绝不 panic，Req 3.6）。
#[allow(dead_code)]
pub(crate) fn parse_socks5_connect_reply(buf: &[u8]) -> Option<(u8, usize)> {
    if buf.len() < 4 || buf[0] != 0x05 {
        return None;
    }
    let rep = buf[1];
    let consumed = match buf[3] {
        0x01 => 4 + 4 + 2,
        0x04 => 4 + 16 + 2,
        0x03 => {
            if buf.len() < 5 {
                return None;
            }
            4 + 1 + buf[4] as usize + 2
        }
        _ => return None,
    };
    if buf.len() < consumed {
        return None;
    }
    Some((rep, consumed))
}

// ============================================================================
// HTTP CONNECT 请求行/状态行/Basic 认证纯函数（Req 3.2/3.4/3.5/3.6/9.1）
//
// 供 Upstream_Client 对 `http` 类型上游发起 CONNECT 隧道并解析响应状态行。
// 纯函数（无 IO），可被 proptest round-trip 属性测试独立验证；不触碰既有入站
// `handle_http` 解析路径。Base64 采用内联的 RFC 4648 标准实现（不新增依赖），
// 输出可被任何标准 Base64 解码器还原。
// ============================================================================

/// 构建 HTTP CONNECT 请求报文（Req 3.2/3.4/3.5）。
///
/// 生成：`CONNECT <host>:<port> HTTP/1.1\r\nHost: <host>:<port>\r\n`，
/// 当提供 `auth` 时追加一行 `Proxy-Authorization: Basic <b64>\r\n`（`<b64>` 为
/// `basic_auth_b64(user, pass)` 的结果），最后以空行 `\r\n` 结束请求头。
#[allow(dead_code)]
pub(crate) fn build_http_connect_req(host: &str, port: u16, auth: Option<(&str, &str)>) -> Vec<u8> {
    let mut s = String::new();
    s.push_str(&format!("CONNECT {}:{} HTTP/1.1\r\n", host, port));
    s.push_str(&format!("Host: {}:{}\r\n", host, port));
    if let Some((user, pass)) = auth {
        s.push_str(&format!(
            "Proxy-Authorization: Basic {}\r\n",
            basic_auth_b64(user, pass)
        ));
    }
    s.push_str("\r\n");
    s.into_bytes()
}

/// 解析 HTTP 响应状态行 `HTTP/1.x <code> <reason>` -> 状态码 `u16`（Req 3.2/3.6）。
///
/// 仅接受 `HTTP/1.0` 或 `HTTP/1.1` 版本前缀、恰好三位数字的状态码；`reason` 短语
/// 可有可无。调用方据状态码 ∈ [200,299] 判定隧道成功。版本前缀非法、状态码非三位
/// 数字或缺失等畸形格式一律返回 `None`（不误判成功、绝不 panic）。
#[allow(dead_code)]
pub(crate) fn parse_http_status_line(line: &str) -> Option<u16> {
    let line = line.trim_end_matches(['\r', '\n']);
    let mut parts = line.splitn(3, ' ');
    let version = parts.next()?;
    let suffix = version.strip_prefix("HTTP/1.")?;
    if suffix.len() != 1 || !suffix.as_bytes()[0].is_ascii_digit() {
        return None;
    }
    let code = parts.next()?;
    if code.len() != 3 || !code.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    code.parse::<u16>().ok()
}

/// HTTP Basic 认证凭据编码：`Base64("<user>:<pass>")`（Req 3.4）。
///
/// 输出为标准 Base64（RFC 4648，含 `=` 补位），可被任意标准 Base64 解码器还原为
/// 字节串 `"<user>:<pass>"`。
#[allow(dead_code)]
pub(crate) fn basic_auth_b64(username: &str, password: &str) -> String {
    let raw = format!("{}:{}", username, password);
    base64_encode_std(raw.as_bytes())
}

/// 最小内联标准 Base64 编码（RFC 4648 标准字母表 + `=` 补位）。
///
/// 不引入外部依赖；每 3 字节输入编码为 4 个 Base64 字符，末尾按 1/2 字节余量
/// 补 1/2 个 `=`。输出可被任何标准 Base64 解码器还原（round-trip 保证，Property 6）。
#[allow(dead_code)]
fn base64_encode_std(input: &[u8]) -> String {
    const TABLE: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((input.len() + 2) / 3 * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((n >> 18) & 0x3F) as usize] as char);
        out.push(TABLE[((n >> 12) & 0x3F) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((n >> 6) & 0x3F) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(n & 0x3F) as usize] as char
        } else {
            '='
        });
    }
    out
}

// ============================================================================
// 出口决策与一网卡多上游选择纯函数（Req 2.2/2.3/2.4/5.1/5.2/5.3/7.x/9.3）
//
// 在既有「选好网卡 → 连目标」之间插入的出口决策：判定一条连接走直连聚合还是
// 走上游，并在一网卡多上游时按调度序号轮转选择。均为纯函数（无 IO），与 IO
// 拨号解耦，不触碰既有 `pick_nic` / bypass / 调度 / 进程规则路径。
// ============================================================================

/// 出口决策结果（Req 5/7）。
///
/// - `Direct`：走既有直连聚合路径（Direct_Aggregate）。
/// - `ViaUpstream(id)`：经指定 `Upstream_Id` 的上游节点建立隧道。
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum Egress {
    Direct,
    ViaUpstream(String),
}

/// 一网卡多上游的调度选择（Req 2.2/2.3/2.4）。
///
/// - 该网卡无绑定或绑定列表为空 => `None`；
/// - 列表长度为 1 => 返回该唯一 id；
/// - 列表长度 > 1 => 按 `sched_idx % len` 轮转选取（复用既有调度序，多上游间轮转）。
///
/// 返回值（若有）必属于该网卡的绑定列表。纯函数，无 IO。
#[allow(dead_code)]
pub(crate) fn pick_upstream_for_nic(
    bindings: &HashMap<u32, Vec<String>>,
    if_index: u32,
    sched_idx: usize,
) -> Option<String> {
    let list = bindings.get(&if_index)?;
    if list.is_empty() {
        return None;
    }
    if list.len() == 1 {
        return Some(list[0].clone());
    }
    Some(list[sched_idx % list.len()].clone())
}

/// 出口决策：判定一条连接走直连聚合还是走上游（Req 5.1/5.2/5.3/7.1/7.3/7.4）。
///
/// 优先级：
///   1. 总开关 `upstream_chain == false` => `Direct`（零回归，Req 5.1）；
///   2. `is_bypass == true` => `Direct`（bypass 最高优先，绝不走上游，Req 7.1）；
///   3. 该网卡无绑定 / 绑定为空 => `Direct`（Req 7.3）；
///   4. 否则 => `ViaUpstream(pick_upstream_for_nic 选出的上游 id)`（Req 7.4）。
///
/// 纯函数，无 IO；返回 `ViaUpstream(id)` 时 `id` 必属于该网卡绑定集合。
#[allow(dead_code)]
pub(crate) fn decide_egress(
    upstream_chain: bool,
    if_index: u32,
    bindings: &HashMap<u32, Vec<String>>,
    is_bypass: bool,
    sched_idx: usize,
) -> Egress {
    if !upstream_chain || is_bypass {
        return Egress::Direct;
    }
    match pick_upstream_for_nic(bindings, if_index, sched_idx) {
        Some(id) => Egress::ViaUpstream(id),
        None => Egress::Direct,
    }
}

// ============================================================================
// 回退决策状态机纯函数（Req 6.2/6.3/6.4/9.4）
//
// 在同一网卡绑定的上游集合内驱动逐个尝试；全部试尽后按回退策略给出「回退直连」
// 或「失败」。纯函数（无 IO），与 IO 拨号解耦，不影响既有直连聚合 / 调度路径。
// ============================================================================

/// 回退状态机的一步动作（Req 6）。
///
/// - `TryUpstream(id)`：尝试该 `Upstream_Id`；
/// - `Direct`：回退直连（policy=Direct 且上游全试尽）；
/// - `Fail`：返回失败（policy=Fail 且上游全试尽）。
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum FallbackStep {
    TryUpstream(String),
    Direct,
    Fail,
}

/// 回退决策（Req 6.2/6.3/6.4）。
///
/// 给定「已尝试过的上游 id 集合 `tried`」「该网卡全部上游 id 有序列表 `nic_upstreams`」
/// 与回退策略 `policy`：
///   - `nic_upstreams` 中存在尚未出现在 `tried` 的 id => `TryUpstream(首个未试 id)`（Req 6.2）；
///   - 全部上游已试尽 且 `policy == Direct` => `Direct`（Req 6.3）；
///   - 全部上游已试尽 且 `policy == Fail` => `Fail`（Req 6.4）。
///
/// 纯函数，无 IO。
#[allow(dead_code)]
pub(crate) fn next_fallback(
    tried: &[String],
    nic_upstreams: &[String],
    policy: FallbackPolicy,
) -> FallbackStep {
    for id in nic_upstreams {
        if !tried.contains(id) {
            return FallbackStep::TryUpstream(id.clone());
        }
    }
    match policy {
        FallbackPolicy::Direct => FallbackStep::Direct,
        FallbackPolicy::Fail => FallbackStep::Fail,
    }
}

// ============================================================================
// 分流决策纯函数（Route_Decision / Route_Simulator，Req 3.1/3.2/3.3/3.4/3.6）
//
// 供分流决策可视化模拟器以纯函数复算一条连接的判定路径，语义与既有
// `decide_rule_action` / `decide_egress` / `pick_upstream_for_nic` / `pattern_match`
// 严格一致：优先级 bypass 最高 > 进程规则 > 域名规则 > 调度回退；「走上游 vs 直连」
// 与 `decide_egress` 对同一输入的结果一致。纯函数，无 IO，不发起真实连接、不改引擎状态。
// 前端等价实现见 `src/lib/routesim.ts` 的 `computeRouteDecision`。
// ============================================================================

/// 命中的规则类别（Route_Decision 的组成部分，Req 3.3）。
///
/// - `Process(name)`：命中进程规则，`name` 为命中的进程可执行文件名（已小写）。
/// - `Domain(pattern)`：命中域名规则，`pattern` 为命中的域名 pattern（已小写）。
/// - `None`：未命中任何「钉死承载网卡」的规则，回退调度策略预选的承载网卡。
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum MatchedRule {
    Process(String),
    Domain(String),
    None,
}

/// 分流决策模拟输出（Route_Decision，Req 3.2/3.3/3.4）。
///
/// - `bypass_hit`：是否命中 bypass 直连白名单（命中则直连、不展示承载上游）。
/// - `matched_rule`：命中的规则（进程 / 域名 / 无规则回退调度）。
/// - `nic_if_index`：承载网卡 IfIndex；命中 bypass 时为 `None`。
/// - `via_upstream`：走上游时选中的 `Upstream_Id`；直连为 `None`。
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) struct RouteDecision {
    pub bypass_hit: bool,
    pub matched_rule: MatchedRule,
    pub nic_if_index: Option<u32>,
    pub via_upstream: Option<String>,
}

/// 依据当前 bypass / 进程规则 / 域名规则 / 上游映射计算一条 Route_Decision
/// （Req 3.1/3.2/3.3/3.4/3.6）。
///
/// 优先级与 Route_Resolver 严格一致：
///   1. bypass 最高（镜像 `is_bypass`：仅按 host 匹配、端口忽略 => 传入 0）：命中则
///      `bypass_hit=true`、`matched_rule=None`、`nic_if_index=None`、`via_upstream=None`（Req 3.2）。
///   2. 进程规则优先于域名规则（镜像 `decide_rule_action`）：进程规则命中即以其动作为准，
///      不再查域名规则。
///   3. 仅 `RuleAction::Nic` 动作会钉死承载网卡（镜像 `pick_nic`）；进程规则的
///      `Direct`/`Aggregate` 动作不改变承载网卡、回退 `chosen_if_index`，此时 `matched_rule=None`。
///   4. 出口决策镜像 `decide_egress`：总开关关 / 命中 bypass / 该网卡无绑定 => 直连
///      （`via_upstream=None`）；否则走上游，`via_upstream` 为 `pick_upstream_for_nic`
///      选出的 id（必属于该网卡绑定集合，Req 3.4/3.6）。
///
/// 纯函数，无 IO，对任意输入不 panic，不发起真实连接、不改引擎状态。
#[allow(dead_code)]
pub(crate) fn compute_route_decision(
    upstream_chain: bool,
    bypass: &[String],
    rules_proc: &[(String, RuleAction)],
    rules_nic: &[(String, u32)],
    bindings: &HashMap<u32, Vec<String>>,
    host: &str,
    port: u16,
    proc_name: Option<&str>,
    chosen_if_index: u32,
    sched_idx: usize,
) -> RouteDecision {
    let h = host.to_lowercase();

    // 1) bypass 最高优先（镜像 `is_bypass`：仅按 host 匹配，端口忽略 => 传入 0）。
    let bypass_hit = bypass.iter().any(|b| pattern_match(b, &h, 0));
    if bypass_hit {
        return RouteDecision {
            bypass_hit: true,
            matched_rule: MatchedRule::None,
            nic_if_index: None,
            via_upstream: None,
        };
    }

    // 2) 规则决策（镜像 `decide_rule_action`）：进程规则优先于域名规则。
    //    仅 `Nic` 动作钉死承载网卡（镜像 `pick_nic`），其余回退 `chosen_if_index`。
    let mut carrier_if_index = chosen_if_index;
    let mut matched_rule = MatchedRule::None;

    if let Some(name) = proc_name {
        if let Some(action) = match_proc_rule(rules_proc, name) {
            // 进程规则命中：以其动作为准，不再查域名规则。
            if let RuleAction::Nic(ifindex) = action {
                carrier_if_index = ifindex;
                matched_rule = MatchedRule::Process(name.to_lowercase());
            }
            return decide_route_egress(
                upstream_chain,
                bindings,
                carrier_if_index,
                sched_idx,
                matched_rule,
            );
        }
    }

    // 未命中进程规则：按域名 `rules_nic` 首个匹配（镜像 `decide_rule_action`）。
    for (pat, ifindex) in rules_nic {
        if pattern_match(pat, &h, port) {
            carrier_if_index = *ifindex;
            matched_rule = MatchedRule::Domain(pat.clone());
            break;
        }
    }

    decide_route_egress(
        upstream_chain,
        bindings,
        carrier_if_index,
        sched_idx,
        matched_rule,
    )
}

/// `compute_route_decision` 的出口决策辅助（镜像 `decide_egress`）：
/// 总开关关 / 该网卡无绑定 => 直连；否则走 `pick_upstream_for_nic` 选出的上游。
/// bypass 命中已在调用方提前返回，故此处 `is_bypass` 恒为 false。
#[allow(dead_code)]
fn decide_route_egress(
    upstream_chain: bool,
    bindings: &HashMap<u32, Vec<String>>,
    carrier_if_index: u32,
    sched_idx: usize,
    matched_rule: MatchedRule,
) -> RouteDecision {
    let via_upstream = match decide_egress(upstream_chain, carrier_if_index, bindings, false, sched_idx) {
        Egress::ViaUpstream(id) => Some(id),
        Egress::Direct => None,
    };
    RouteDecision {
        bypass_hit: false,
        matched_rule,
        nic_if_index: Some(carrier_if_index),
        via_upstream,
    }
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
    // 每方向复用缓冲的固定尺寸（64 KiB）。语义固化：
    // 1) 每个方向（down/up）各一次性分配一块该尺寸的复用缓冲，循环内复用、不重复分配；
    // 2) 任一方向读到 0（EOF）即跳出循环并对对端写半执行 flush + shutdown（半关闭）；
    // 3) 双向 future 经 tokio::join! 全部结束后，缓冲与 split 借用随作用域一并释放。
    // 该常量仅提取魔法数字，不改变任何读写/限速/统计行为，保持逐字节等价。
    const RELAY_BUF_BYTES: usize = 65536;

    let (mut cr, mut cw) = client.split();
    let (mut ur, mut uw) = upstream.split();

    // 下行：上游 -> 客户端（limiter 存在时限速）
    let down = async {
        let mut buf = vec![0u8; RELAY_BUF_BYTES];
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
        let mut buf = vec![0u8; RELAY_BUF_BYTES];
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
    /// Tauri 应用句柄：生产路径恒为 `Some`（既有 emit / 日志行为逐字节不变）；
    /// 仅在 `#[cfg(test)]` 端到端集成测试中为 `None`（无 GUI 运行时），此时 emit /
    /// 日志降级为无操作，不影响被测的上游握手 / 隧道转发逻辑（Req 10.7/10.8）。
    app: Option<AppHandle>,
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
    /// 上游代理链总开关（默认 false，未启用时不影响任何既有分支，Req 5.5）
    #[allow(dead_code)]
    upstream_chain: bool,
    /// 上游条目表：id -> UpstreamProxy（start 阶段由 Vec 构建，便于 O(1) 引用，Req 1）
    #[allow(dead_code)]
    upstreams: HashMap<String, UpstreamProxy>,
    /// 网卡→上游 id 列表：if_index -> Vec<UpstreamId>（已剔除悬空引用，Req 2.6）
    #[allow(dead_code)]
    upstream_bindings: HashMap<u32, Vec<String>>,
    /// 回退策略（默认 Direct，Req 6）
    #[allow(dead_code)]
    upstream_fallback: FallbackPolicy,
    /// 上游超时（缺省 ≤10s，Req 6.1）
    #[allow(dead_code)]
    upstream_timeout: std::time::Duration,
    // ==== 本次新增字段（专业化差异化与稳定性加固）====
    // 均以「默认关闭 / 默认旁路」引入：未启用时既有分支不引用这些字段，
    // 既有直连聚合 / 上游链 / IPv4·IPv6 / DNS / 限速 / 调度 / 进程规则 /
    // fake-ip 路径行为字节级不变（Req 13.1）。
    /// [新增] 健康探测配置（enabled 默认 false，Req 1.7）
    #[allow(dead_code)]
    health_cfg: HealthConfig,
    /// [新增] 上游健康表：Upstream_Id -> UpstreamHealth（后台探测任务维护，Req 1）
    #[allow(dead_code)]
    upstream_health: Arc<Mutex<HashMap<String, UpstreamHealth>>>,
    /// [新增] 每网卡 DNS 配置：if_index -> PerNicDns（空表示未配置，走全局解析，Req 7）
    #[allow(dead_code)]
    per_nic_dns: HashMap<u32, PerNicDns>,
    /// [新增] Connection_Cap：活跃中继连接数上限（Req 8.2/8.6）
    #[allow(dead_code)]
    conn_cap: usize,
    /// [新增] Task_Cap：后台任务并发上限（Req 8.3/8.6）
    #[allow(dead_code)]
    task_cap: usize,
    /// [新增] 活跃连接计数（Connection_Cap 判定与遥测复用，Req 8.2）
    #[allow(dead_code)]
    active_conns: Arc<AtomicI64>,
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
        // [新增] 每网卡 DNS 旁路（Per_NIC_DNS，Req 7.2/7.4）：仅当该承载网卡配置了
        // Per_NIC_DNS 时才启用；经该网卡出口用指定 DNS/DoH 解析，成功则采用并写入
        // 既有共享缓存，失败/超时记可读日志后回退既有全局解析路径。未配置该网卡时
        // `get` 返回 None，本分支被整体跳过，既有全局路径逐字节不变（零回归，Req 7.3/7.6）。
        if let Some(dns) = self.per_nic_dns.get(&nic.if_index) {
            // 命中近期缓存直接复用（与全局路径共享 60s 缓存，避免重复查询）。
            if let Ok(cache) = self.dns_cache.lock() {
                if let Some((ip, t)) = cache.get(host) {
                    if t.elapsed() < std::time::Duration::from_secs(60) {
                        return Some(SocketAddrV4::new(*ip, port));
                    }
                }
            }
            match resolve_host_via_per_nic_dns(nic, host, dns).await {
                Some(ip) => {
                    if let Ok(mut cache) = self.dns_cache.lock() {
                        cache.insert(host.to_string(), (ip, std::time::Instant::now()));
                    }
                    return Some(SocketAddrV4::new(ip, port));
                }
                None => {
                    self.log(format!(
                        "[Per_NIC_DNS] 网卡 if_index={} 经自定义 DNS({}) 解析 {} 失败/超时，回退全局解析",
                        nic.if_index, dns.endpoint, host
                    ));
                }
            }
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
        // 域名：复用既有 A 记录解析（含缓存/DoH/UDP/系统回退），并追加 AAAA 平行路径。
        // A 记录经 `resolve_host` 已内建每网卡 DNS 旁路与回退（Per_NIC_DNS，Req 7.2/7.4）；
        // AAAA 仍走既有全局 UDP 平行路径，per_nic_dns 为空时二者均逐字节不变（Req 7.3/7.6）。
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
        // 统一日志入口：既有 emit("hmx-log") 行为不变，附加写入本地滚动日志文件。
        // app 为 None（仅端到端测试路径）时降级为无操作，不触达 GUI 运行时。
        if let Some(app) = &self.app {
            crate::hmx_log(app, crate::logger::LogLevel::Info, &msg.into());
        }
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
    app: Option<AppHandle>,
    info: ConnInfo,
}
impl Drop for ConnTableGuard {
    fn drop(&mut self) {
        if let Ok(mut map) = self.conns.lock() {
            map.remove(&self.id);
        }
        if let Some(app) = &self.app {
            let _ = app.emit("hmx-conn-closed", self.info.clone());
        }
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
    // 上游代理链配置（Req 1/2/5）：默认空列表 + 总开关关时构建结果为空映射，既有路径零回归。
    upstreams: Vec<UpstreamProxy>,
    upstream_bindings: Vec<UpstreamBinding>,
    upstream_chain: bool,
    upstream_fallback: String,
    // 本次新增能力配置（Req 1/7/8/13）：默认关闭 / 默认旁路，未提供时取默认值，
    // 此时构建结果与升级前等价，既有启动路径行为不变（Req 13.1/13.2）。
    // 健康探测配置（HealthConfig，Req 1）；`enabled=false` 时全部上游视为 Healthy。
    health_cfg: HealthConfig,
    // 每网卡独立 DNS/DoH 映射条目（Per_NIC_DNS，Req 7）；空表示全部网卡走既有全局解析。
    per_nic_dns: Vec<(u32, PerNicDns)>,
    // 活跃中继连接数上限（Connection_Cap，Req 8.2/8.6）。
    conn_cap: usize,
    // 后台任务并发上限（Task_Cap，Req 8.3/8.6）。
    task_cap: usize,
    // 系统代理防泄漏看门狗开关（Proxy_Guardian，Req 5）。看门狗生命周期由 lib.rs
    // 与 proxyguardian 模块驱动（任务 6.x），引擎侧当前不据此改变连接处理路径。
    _proxy_guardian: bool,
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

    // 构建上游条目表：Vec -> HashMap<id, UpstreamProxy>（便于 O(1) 引用，Req 1）。
    // 同 id 以后者覆盖前者（前端保证 id 同组唯一，此处仅作稳健处理）。
    let upstreams: HashMap<String, UpstreamProxy> = upstreams
        .into_iter()
        .map(|u| (u.id.clone(), u))
        .collect();
    // 构建网卡→上游 id 列表映射并剔除悬空引用（引用已删除条目的绑定视为未绑定，Req 2.6）。
    let upstream_bindings = sanitize_bindings(&upstreams, &upstream_bindings);
    // 回退策略解析："fail" => Fail；其它（含 "direct" 与未知值）=> Direct（Req 6）。
    let upstream_fallback = match upstream_fallback.trim().to_ascii_lowercase().as_str() {
        "fail" => FallbackPolicy::Fail,
        _ => FallbackPolicy::Direct,
    };

    let cancel = CancellationToken::new();
    let engine = Arc::new(Engine {
        nics: nics.clone(),
        strategy,
        wrr: Mutex::new(vec![0i64; nics.len()]),
        conns: Arc::new(Mutex::new(HashMap::new())),
        conn_id: AtomicU64::new(0),
        app: Some(app.clone()),
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
        // 上游代理链配置（Req 1/2/5/6）：由 start 参数构建，未启用（upstream_chain=false）
        // 且无上游配置时为空映射，既有直连聚合路径行为不变。
        upstream_chain,
        upstreams,
        upstream_bindings,
        upstream_fallback,
        upstream_timeout: std::time::Duration::from_secs(10),
        // 本次新增能力配置（Req 1/7/8/13）：由 start 参数正式透传。未提供新配置
        // （health 关闭 / 空 DNS / 默认上限）时构建结果与升级前等价，既有启动路径
        // 行为不变（Req 13.1/13.2）。
        health_cfg,
        // 上游健康表初始为空；后台探测任务（任务 2.6）启用后填充维护（Req 1）。
        upstream_health: Arc::new(Mutex::new(HashMap::new())),
        // 每网卡 DNS：Vec<(if_index, PerNicDns)> → HashMap；空 Vec 得空表，
        // 全部网卡走既有全局解析路径（零回归，Req 7.3）。
        per_nic_dns: per_nic_dns.into_iter().collect(),
        // 稳定性上限（Req 8.2/8.3/8.6）：0 视为未指定，回退到合理默认，避免退化为零上限阻断连接。
        conn_cap: if conn_cap == 0 { 4096 } else { conn_cap },
        task_cap: if task_cap == 0 { 64 } else { task_cap },
        active_conns: Arc::new(AtomicI64::new(0)),
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

    // [新增] 后台健康探测任务（Health_Prober，Req 1）：仅当 health_cfg.enabled 时 spawn。
    // 未启用时绝不派发探测任务、绝不改变选路（零回归，Req 1.6/1.7/13.2）。随 cancel 取消结束。
    if engine.health_cfg.enabled {
        let eng = engine.clone();
        let c = cancel.clone();
        tauri::async_runtime::spawn(async move {
            health_prober_loop(eng, c).await;
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
                    Ok((stream, peer)) => {
                        // Connection_Cap（Stability_Guard，Req 8.2）：活跃中继连接达到上限时
                        // 拒绝该新入站连接——不 spawn、直接 drop 关闭，既有活跃连接不受影响。
                        // 计数低于上限时不额外改变任何处理路径（零回归，Req 8.5）。
                        if engine.active_conns.load(Ordering::Relaxed) >= engine.conn_cap as i64 {
                            let pfx = if engine.zh {
                                "[连接上限] 活跃连接已达上限，拒绝新入站连接 "
                            } else {
                                "[Connection cap] active connections reached cap, rejecting inbound "
                            };
                            engine.log(format!("{pfx}{peer}"));
                            drop(stream);
                            continue;
                        }
                        let _ = stream.set_nodelay(true);
                        let eng = engine.clone();
                        let c = cancel.clone();
                        // 活跃连接计数 +1；RAII 守卫随任务任意退出路径（正常 / 取消 / panic）
                        // 在 drop 时 -1，保证计数不泄漏（Req 8.2）。守卫先于 spawn 构建并移入任务，
                        // 即便任务从未被轮询也会在 drop 时正确回收。
                        let cap_guard = ActiveConnGuard::new(engine.active_conns.clone());
                        tauri::async_runtime::spawn(async move {
                            let _cap_guard = cap_guard;
                            handle_connection_isolated(eng, stream, peer, proto, c).await;
                        });
                    }
                    Err(_) => break,
                }
            }
        }
    }
}

/// 活跃连接计数 RAII 守卫（Connection_Cap，Req 8.2）。
///
/// 构造时对 `active_conns` +1、drop 时 -1。drop 覆盖任务的任意退出路径（正常结束、
/// 被取消、或 panic 展开），保证活跃连接计数与 Connection_Cap 判定不泄漏。
struct ActiveConnGuard(Arc<AtomicI64>);
impl ActiveConnGuard {
    fn new(counter: Arc<AtomicI64>) -> Self {
        counter.fetch_add(1, Ordering::Relaxed);
        Self(counter)
    }
}
impl Drop for ActiveConnGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

/// 单连接处理 + panic 隔离（Stability_Guard，Req 8.1/8.4）。
///
/// 将实际的 `handle_socks` / `handle_http` 放入独立子任务运行：依赖 release
/// `panic = "unwind"`，子任务内发生的 panic 仅结束该子任务、被 tokio 兜底展开而不
/// 波及引擎与其他连接（Req 8.1）；随后经子任务 `JoinHandle` 捕获 panic 负载并记一条
/// 结构化日志（含连接标识 `peer` 与失败位置：所属协议处理器 + panic 消息，Req 8.4），
/// 仅释放本连接资源。引擎停止（cancel）时主动 abort 子任务，与既有
/// `tokio::select! { cancelled }` 的取消语义保持一致（零回归，Req 8.5）。
async fn handle_connection_isolated(
    eng: Arc<Engine>,
    stream: TcpStream,
    peer: SocketAddr,
    proto: Protocol_,
    cancel: CancellationToken,
) {
    let eng_inner = eng.clone();
    let mut inner = tokio::task::spawn(async move {
        match proto {
            Protocol_::Socks => handle_socks(eng_inner, stream).await,
            Protocol_::Http => handle_http(eng_inner, stream).await,
        }
    });
    tokio::select! {
        _ = cancel.cancelled() => {
            // 引擎停止：主动 abort 子任务，等价于既有取消时丢弃处理 future 的行为。
            inner.abort();
        }
        res = &mut inner => {
            match res {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    // 既有连接错误日志路径（保持不变）：仅记录非空错误，常见连接重置忽略。
                    let s = e.to_string();
                    if !s.is_empty() {
                        let pfx = if eng.zh { "[连接异常] " } else { "[Connection error] " };
                        eng.log(format!("{pfx}{s}"));
                    }
                }
                Err(join_err) if join_err.is_panic() => {
                    // 单连接 panic 被隔离：记结构化日志（连接标识 + 失败位置 + panic 消息），
                    // 仅本连接受影响，引擎与其他连接继续存活（Req 8.1/8.4）。
                    let location = match proto {
                        Protocol_::Socks => "handle_socks",
                        Protocol_::Http => "handle_http",
                    };
                    let payload = panic_payload_message(join_err.into_panic());
                    let msg = format!("conn={peer} at={location} panic={payload}");
                    if let Some(app) = &eng.app {
                        crate::hmx_log_structured(
                            app,
                            crate::logger::LogLevel::Error,
                            "Stability_Guard",
                            &msg,
                        );
                    }
                }
                Err(_) => {
                    // 非 panic 的 JoinError（如取消）无需处理。
                }
            }
        }
    }
}

/// 从 panic 负载中尽力提取可读消息（`&str` / `String`，否则给出占位符）。
/// 供单连接 panic 隔离日志复用，绝不 panic。
fn panic_payload_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic payload".to_string()
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

// ==== 每网卡 DNS/DoH 解析（Per_NIC_DNS，Req 7）[新增] ====
//
// 复用既有 UDP 拨号骨架（IP_UNICAST_IF / IPV6_UNICAST_IF egress binding +
// build_dns_query / parse_dns_a）与既有 DoH 拨号骨架（connect_via_nic + TLS +
// POST dns-message），仅将「目标 DNS 服务器 / DoH 端点」替换为用户为该网卡配置的值。
// 任一步失败均返回 None，由调用方 `resolve_host` 记日志并回退既有全局解析路径。

/// 经指定网卡用该网卡配置的 Per_NIC_DNS 解析域名的 A 记录（IPv4）。
///
/// - `Plain`：`endpoint` 为明文 DNS 服务器地址（IPv4/IPv6），经该网卡出口发 UDP A 查询；
/// - `Doh`：`endpoint` 为 `https://` DoH 端点 URL，经该网卡出口发 DoH POST 查询。
///
/// 字面 IPv4 目标直接返回（与既有解析入口一致）。对任意输入不 panic。
async fn resolve_host_via_per_nic_dns(
    nic: &NicRuntime,
    host: &str,
    dns: &PerNicDns,
) -> Option<Ipv4Addr> {
    if let Ok(ip) = host.parse::<Ipv4Addr>() {
        return Some(ip);
    }
    match dns.kind {
        DnsKind::Plain => {
            // 明文 DNS 服务器地址：IPv4 走 IPv4 UDP 出口，IPv6 走 IPv6 UDP 出口。
            let server: IpAddr = dns.endpoint.parse().ok()?;
            resolve_via_nic_plain(nic, host, server).await
        }
        DnsKind::Doh => resolve_via_nic_doh(nic, host, &dns.endpoint).await,
    }
}

/// 经指定网卡向「自定义明文 DNS 服务器」发起 A 查询解析域名（Per_NIC_DNS Plain）。
///
/// 与既有 `resolve_via_nic` 平行：仅把固定的 223.5.5.5 换为传入的 `server`，
/// 并按服务器地址族选择 IPv4 / IPv6 出口的 Egress_Binding。IPv6 服务器要求该网卡
/// 具备可用 IPv6 源地址，否则返回 None 交由上层回退。
async fn resolve_via_nic_plain(nic: &NicRuntime, host: &str, server: IpAddr) -> Option<Ipv4Addr> {
    let (domain, if_level, if_optname, bind_ip, server_addr) = match server {
        IpAddr::V4(s) => (
            Domain::IPV4,
            IPPROTO_IP,
            IP_UNICAST_IF,
            IpAddr::V4(nic.ipv4),
            SocketAddr::V4(SocketAddrV4::new(s, 53)),
        ),
        IpAddr::V6(s) => {
            let v6 = nic.ipv6?;
            (
                Domain::IPV6,
                IPPROTO_IPV6,
                IPV6_UNICAST_IF,
                IpAddr::V6(v6),
                SocketAddr::V6(SocketAddrV6::new(s, 53, 0, 0)),
            )
        }
    };
    let socket = Socket::new(domain, Type::DGRAM, Some(Protocol::UDP)).ok()?;
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
            return None;
        }
    }
    #[cfg(not(windows))]
    let _ = (if_level, if_optname);
    let bind_addr: socket2::SockAddr = SocketAddr::new(bind_ip, 0).into();
    socket.bind(&bind_addr).ok()?;
    socket.set_nonblocking(true).ok()?;
    let std_udp: std::net::UdpSocket = socket.into();
    let udp = tokio::net::UdpSocket::from_std(std_udp).ok()?;
    let query = build_dns_query(host);
    udp.send_to(&query, server_addr).await.ok()?;
    // 循环接收直到「来源正确 + 事务 ID 匹配」的响应或超时（与 resolve_via_nic 一致）。
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let mut buf = [0u8; 512];
    loop {
        let remaining = deadline.checked_duration_since(std::time::Instant::now())?;
        let (n, from) = tokio::time::timeout(remaining, udp.recv_from(&mut buf))
            .await
            .ok()?
            .ok()?;
        if from != server_addr {
            continue;
        }
        if n < 2 || buf[0] != 0x12 || buf[1] != 0x34 {
            continue;
        }
        if let Some(ip) = parse_dns_a(&buf[..n]) {
            return Some(ip);
        }
        return None;
    }
}

/// 经指定网卡用「自定义 DoH 端点」解析域名的 A 记录（Per_NIC_DNS Doh）。
///
/// 与既有 `resolve_via_doh` 平行：把固定的 `DOH_RESOLVERS` 换为用户配置的端点。
/// DoH 主机若为字面 IPv4 直接连接，否则经该网卡先解析其 A 记录（`resolve_via_nic`）。
async fn resolve_via_nic_doh(nic: &NicRuntime, host: &str, endpoint: &str) -> Option<Ipv4Addr> {
    if let Ok(ip) = host.parse::<Ipv4Addr>() {
        return Some(ip);
    }
    let (doh_host, doh_port, doh_path) = parse_doh_endpoint(endpoint)?;
    // 确定 DoH 服务器连接 IP：字面 IPv4 直接用，否则经该网卡解析其 A 记录。
    let server_ip: Ipv4Addr = match doh_host.parse::<Ipv4Addr>() {
        Ok(ip) => ip,
        Err(_) => resolve_via_nic(nic, &doh_host).await?,
    };
    let query = build_dns_query(host);
    let dst = SocketAddrV4::new(server_ip, doh_port);
    let fut = async {
        let tcp = connect_via_nic(nic, SocketAddr::V4(dst)).await.ok()?;
        let connector = tls_connector();
        let server_name =
            tokio_rustls::rustls::pki_types::ServerName::try_from(doh_host.clone()).ok()?;
        let mut tls = connector.connect(server_name, tcp).await.ok()?;
        let head = format!(
            "POST {doh_path} HTTP/1.1\r\nHost: {doh_host}\r\nAccept: application/dns-message\r\nContent-Type: application/dns-message\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            query.len()
        );
        tls.write_all(head.as_bytes()).await.ok()?;
        tls.write_all(&query).await.ok()?;
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
        // 兼容分块传输：去掉首个 chunk-size 行。
        if head_str.to_ascii_lowercase().contains("transfer-encoding: chunked") {
            if let Some(p) = body.windows(2).position(|w| w == b"\r\n") {
                body = &body[p + 2..];
            }
        }
        parse_dns_a(body)
    };
    match tokio::time::timeout(std::time::Duration::from_secs(5), fut).await {
        Ok(opt) => opt,
        Err(_) => None,
    }
}

/// 从 DoH 端点 URL 提取 `(host, port, path)`：仅接受 `https://`、主机段非空；
/// 缺省端口 443、缺省路径 `/dns-query`。对畸形输入返回 None，绝不 panic。
fn parse_doh_endpoint(endpoint: &str) -> Option<(String, u16, String)> {
    const PREFIX: &str = "https://";
    let head = endpoint.get(..PREFIX.len())?;
    if !head.eq_ignore_ascii_case(PREFIX) {
        return None;
    }
    let rest = &endpoint[PREFIX.len()..];
    // 分离 authority 与 path：以首个 '/' 为界，无 '/' 时截断 query/fragment。
    let (authority, path): (&str, String) = match rest.find('/') {
        Some(idx) => (&rest[..idx], rest[idx..].to_string()),
        None => {
            let mut end = rest.len();
            for ch in ['?', '#'] {
                if let Some(i) = rest.find(ch) {
                    if i < end {
                        end = i;
                    }
                }
            }
            (&rest[..end], "/dns-query".to_string())
        }
    };
    // 去用户信息。
    let authority = match authority.rfind('@') {
        Some(at) => &authority[at + 1..],
        None => authority,
    };
    // host[:port]，兼容 IPv6 字面量 [::1]:443。
    let (host, port) = if authority.starts_with('[') {
        let close = authority.find(']')?;
        let host = authority[1..close].to_string();
        let after = &authority[close + 1..];
        let port = match after.strip_prefix(':') {
            Some(p) => p.parse::<u16>().ok()?,
            None => 443,
        };
        (host, port)
    } else {
        match authority.find(':') {
            Some(colon) => {
                let host = authority[..colon].to_string();
                let port = authority[colon + 1..].parse::<u16>().ok()?;
                (host, port)
            }
            None => (authority.to_string(), 443),
        }
    };
    if host.is_empty() {
        return None;
    }
    Some((host, port, path))
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

// ============================================================================
// Upstream_Client：经网卡物理出口连接上游并建立到真实目标的隧道（Req 3/4/6.1）
//
// 组装薄封装：复用 `resolve_host_dual` + `dial_dual`/`connect_via_nic`（Egress_Binding）
// + 阶段 B 握手纯函数（SOCKS5 / HTTP CONNECT）。整体受 `engine.upstream_timeout`
// 约束；任一步失败/超时记录含上游标签与原因的可读日志后返回 Err，交由后续回退循环
// （任务 6.1）处理。不改动 `connect_via_nic`/`resolve_host_dual`/`dial_dual` 底层实现。
// ============================================================================

/// 上游日志标签：优先用 `label`，为空时回退到 `host:port`。
fn upstream_label(u: &UpstreamProxy) -> String {
    if u.label.trim().is_empty() {
        format!("{}:{}", u.host, u.port)
    } else {
        u.label.clone()
    }
}

/// 上游认证判定与凭据提取（Req 3.3/3.4）。
///
/// `username`/`password` 任一为 `Some` 且非空即视为"有认证"，返回 `(user, pass)`
/// （缺失的一侧以空串补齐）；均缺失/为空则返回 `None`（无认证）。
fn upstream_credential(u: &UpstreamProxy) -> Option<(&str, &str)> {
    let user = u.username.as_deref().unwrap_or("");
    let pass = u.password.as_deref().unwrap_or("");
    if user.is_empty() && pass.is_empty() {
        None
    } else {
        Some((user, pass))
    }
}

/// 将真实目标（域名或字面 IP）映射为 SOCKS5 CONNECT 目标类型（Req 3.5）。
///
/// 字面 IPv4/IPv6 分别用 `V4`/`V6`；域名用 `Domain`（ATYP=0x03）交由上游解析。
fn connect_target_of(host: &str, port: u16) -> ConnectTarget {
    if let Ok(ip) = host.parse::<Ipv4Addr>() {
        ConnectTarget::V4(ip, port)
    } else if let Ok(ip) = host.parse::<Ipv6Addr>() {
        ConnectTarget::V6(ip, port)
    } else {
        ConnectTarget::Domain(host.to_string(), port)
    }
}

/// 读取 HTTP 响应头块（直到首个空行 `\r\n\r\n`）。
///
/// 逐字节读取，遇到 `\r\n\r\n` 终止并返回已读全部字节（含终止序列）。这样既能取到
/// 状态行判定，又能把上游 CONNECT 响应的全部响应头从流中剥离，避免残留字节混入隧道
/// 数据。EOF 提前到达返回 `UnexpectedEof`；头块超过 8KiB 返回 `InvalidData`（防御畸形上游）。
async fn read_http_response_head(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let mut buf: Vec<u8> = Vec::with_capacity(256);
    let mut byte = [0u8; 1];
    loop {
        let n = stream.read(&mut byte).await?;
        if n == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "上游在 HTTP 响应头结束前关闭连接 / upstream closed before end of HTTP response head",
            ));
        }
        buf.push(byte[0]);
        if buf.ends_with(b"\r\n\r\n") {
            return Ok(buf);
        }
        if buf.len() > 8192 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "HTTP 响应头过大 / HTTP response head too large",
            ));
        }
    }
}

/// SOCKS5 上游握手：版本协商（含可选用户名/密码认证）+ CONNECT 请求/应答（Req 3.1/3.3/3.5）。
///
/// 当且仅当 CONNECT 应答 `REP == 0x00` 时返回 `Ok`（Req 3.1）；方法被拒绝、认证失败、
/// 应答格式非法或 `REP != 0x00` 均返回 `Err`。应答被完整读出（含 BND.ADDR/BND.PORT），
/// 隧道就绪后 `stream` 恰好定位在真实目标数据流起点。
async fn socks5_upstream_handshake(
    stream: &mut TcpStream,
    upstream: &UpstreamProxy,
    target: &ConnectTarget,
) -> std::io::Result<()> {
    let cred = upstream_credential(upstream);

    // 1) 版本协商：按是否有认证声明方法列表
    stream.write_all(&build_socks5_greeting(cred.is_some())).await?;
    let mut method_buf = [0u8; 2];
    stream.read_exact(&mut method_buf).await?;
    let method = parse_socks5_method_reply(&method_buf).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "SOCKS5 方法应答非法 / invalid SOCKS5 method reply",
        )
    })?;

    // 2) 认证子协商（仅当上游选定用户名/密码方法 0x02）
    match method {
        0x02 => {
            let (user, pass) = cred.ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "上游要求认证但未配置凭据 / upstream requires auth but no credentials configured",
                )
            })?;
            stream.write_all(&build_socks5_userpass(user, pass)).await?;
            let mut auth_buf = [0u8; 2];
            stream.read_exact(&mut auth_buf).await?;
            let status = parse_socks5_userpass_reply(&auth_buf).ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "SOCKS5 认证应答非法 / invalid SOCKS5 auth reply",
                )
            })?;
            if status != 0x00 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    format!("SOCKS5 上游认证失败 status={status:#04x} / SOCKS5 upstream auth failed"),
                ));
            }
        }
        0x00 => {}
        other => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!("上游拒绝认证方法 method={other:#04x} / upstream rejected auth methods"),
            ));
        }
    }

    // 3) CONNECT 请求
    stream.write_all(&build_socks5_connect_req(target)).await?;

    // 4) 读取 CONNECT 应答：先读固定 4 字节头判 ATYP，再按类型读齐地址 + 端口
    let mut reply: Vec<u8> = vec![0u8; 4];
    stream.read_exact(&mut reply).await?;
    match reply[3] {
        0x01 => {
            let mut rest = [0u8; 4 + 2];
            stream.read_exact(&mut rest).await?;
            reply.extend_from_slice(&rest);
        }
        0x04 => {
            let mut rest = [0u8; 16 + 2];
            stream.read_exact(&mut rest).await?;
            reply.extend_from_slice(&rest);
        }
        0x03 => {
            let mut dlen = [0u8; 1];
            stream.read_exact(&mut dlen).await?;
            reply.push(dlen[0]);
            let mut rest = vec![0u8; dlen[0] as usize + 2];
            stream.read_exact(&mut rest).await?;
            reply.extend_from_slice(&rest);
        }
        other => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("SOCKS5 应答 ATYP 非法 atyp={other:#04x} / invalid SOCKS5 reply ATYP"),
            ));
        }
    }
    let (rep, _consumed) = parse_socks5_connect_reply(&reply).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "SOCKS5 CONNECT 应答非法 / invalid SOCKS5 CONNECT reply",
        )
    })?;
    if rep != 0x00 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            format!("SOCKS5 上游 CONNECT 失败 rep={rep:#04x} / SOCKS5 upstream CONNECT failed"),
        ));
    }
    Ok(())
}

/// HTTP 上游握手：`CONNECT host:port` 请求（含可选 Basic 认证）+ 2xx 状态行判定（Req 3.2/3.4/3.5）。
///
/// 当且仅当响应状态码 ∈ [200,299] 时返回 `Ok`（Req 3.2）；解析失败或非 2xx 返回 `Err`。
/// 完整读出响应头块（直到 `\r\n\r\n`），隧道就绪后 `stream` 定位在真实目标数据流起点。
async fn http_upstream_handshake(
    stream: &mut TcpStream,
    upstream: &UpstreamProxy,
    host: &str,
    port: u16,
) -> std::io::Result<()> {
    let cred = upstream_credential(upstream);
    let req = build_http_connect_req(host, port, cred);
    stream.write_all(&req).await?;

    let head = read_http_response_head(stream).await?;
    // 首个 \r\n 之前即状态行
    let line_end = head
        .windows(2)
        .position(|w| w == b"\r\n")
        .unwrap_or(head.len());
    let status_line = String::from_utf8_lossy(&head[..line_end]);
    let code = parse_http_status_line(&status_line).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "HTTP 状态行非法 / invalid HTTP status line",
        )
    })?;
    if !(200..=299).contains(&code) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            format!("HTTP 上游 CONNECT 失败 status={code} / HTTP upstream CONNECT failed"),
        ));
    }
    Ok(())
}

/// 经 `nic` 物理出口连接上游、完成握手，得到到真实目标的隧道流（不含超时包裹）。
///
/// 步骤：① 经网卡解析上游 host（`resolve_host_dual`，Req 4.2）；② 字面 IP 用
/// `connect_via_nic`、域名/双栈用 `dial_dual` 经网卡物理出口连上游（Egress_Binding，
/// Req 4.1/4.5）；③ 按 `upstream.kind` 执行 SOCKS5 / HTTP CONNECT 握手（Req 3）。
async fn connect_via_upstream_inner(
    engine: &Engine,
    nic: &NicRuntime,
    upstream: &UpstreamProxy,
    host: &str,
    port: u16,
) -> std::io::Result<TcpStream> {
    // 1) 经网卡出口解析上游 host（字面 IP 直接填入对应族，域名走该网卡解析，Req 4.2）
    let addrs = engine
        .resolve_host_dual(nic, &upstream.host, upstream.port)
        .await;

    // 2) 经该网卡物理出口连上游地址:端口（复用 Egress_Binding，Req 4.1/4.5）
    let mut stream = if let Ok(ip) = upstream.host.parse::<Ipv4Addr>() {
        connect_via_nic(nic, SocketAddr::V4(SocketAddrV4::new(ip, upstream.port))).await?
    } else if let Ok(ip) = upstream.host.parse::<Ipv6Addr>() {
        connect_via_nic(nic, SocketAddr::V6(SocketAddrV6::new(ip, upstream.port, 0, 0))).await?
    } else {
        // 域名/双栈：无任何可用地址族即失败（Req 4.4 之一）
        if addrs.v4.is_none() && addrs.v6.is_none() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AddrNotAvailable,
                format!(
                    "网卡 {} 经出口解析上游 {} 无可用地址族 / no usable address family for upstream via adapter",
                    nic.name, upstream.host
                ),
            ));
        }
        dial_dual(
            engine,
            nic,
            &addrs,
            upstream.port,
            engine.ip_pref(),
            engine.upstream_timeout,
        )
        .await?
    };

    // 3) 按上游类型握手，建立到真实目标的隧道（Req 3）
    let target = connect_target_of(host, port);
    match upstream.kind.as_str() {
        "socks5" => socks5_upstream_handshake(&mut stream, upstream, &target).await?,
        "http" => http_upstream_handshake(&mut stream, upstream, host, port).await?,
        other => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("未知上游类型 kind={other} / unknown upstream kind"),
            ));
        }
    }
    Ok(stream)
}

/// 建立一条"经 nic 物理出口 → 上游 → 真实目标"的隧道（Req 3/4/6.1）。
///
/// 对 `connect_via_upstream_inner`（解析 + 建连 + 握手）整体以 `engine.upstream_timeout`
/// 包裹（缺省 ≤10s，Req 6.1）；超时或任一步失败返回 `Err`，并记录一条含上游标签与失败
/// 原因的可读日志（中英随 `engine.zh`），交由后续回退循环（任务 6.1）处理。
///
/// 说明：仅在总开关启用且该连接被判定走上游时由调用点（任务 6.1）触发；未启用时既有
/// 直连聚合 / IPv4 / IPv6 / DNS 路径完全不经此函数。
async fn connect_via_upstream(
    engine: &Engine,
    nic: &NicRuntime,
    upstream: &UpstreamProxy,
    host: &str,
    port: u16,
) -> std::io::Result<TcpStream> {
    let result = match tokio::time::timeout(
        engine.upstream_timeout,
        connect_via_upstream_inner(engine, nic, upstream, host, port),
    )
    .await
    {
        Ok(r) => r,
        Err(_) => Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            format!(
                "经上游建立隧道超时（{:?}）/ upstream tunnel timed out",
                engine.upstream_timeout
            ),
        )),
    };

    if let Err(ref e) = result {
        let label = upstream_label(upstream);
        engine.log(if engine.zh {
            format!(
                "[上游] 网卡 {} 经上游 {} 连接 {}:{} 失败：{}",
                nic.name, label, host, port, e
            )
        } else {
            format!(
                "[Upstream] adapter {} via upstream {} to {}:{} failed: {}",
                nic.name, label, host, port, e
            )
        });
    }
    result
}

// ============================================================================
// 出口分派 + 回退循环（阶段 E 路由集成，Req 5/6/7）
//
// 在既有「选好网卡 → 连真实目标」之间插入统一的出口决策分派：先以 `decide_egress`
// 判定走直连聚合还是走上游；走上游时以 `next_fallback` 驱动在该网卡上游集合内逐个
// 尝试，全部试尽后按 `upstream_fallback` 回退直连或返回错误。总开关未启用 / 命中
// bypass / 网卡无绑定时 `decide_egress` 恒返回 `Direct`，字节流与既有完全一致（零回归）。
// ============================================================================

/// 既有直连聚合路径：按目标形态选择既有 IO——字面 IPv4/IPv6 走 `connect_via_nic`
/// （与既有字面路径一致，无双栈/超时包裹）；域名走 `dial_dual`（Happy-Eyeballs 双栈拨号）。
///
/// 三者互斥且行为与升级前完全一致（Req 5.1/5.3），供 `establish_target` 的 `Direct`
/// 分支与上游全试尽后的「回退直连」分支共用。
async fn connect_target_direct(
    engine: &Engine,
    nic: &NicRuntime,
    port: u16,
    literal_ip: Option<Ipv4Addr>,
    literal_v6: Option<Ipv6Addr>,
    dual_addrs: Option<&ResolvedAddrs>,
) -> std::io::Result<TcpStream> {
    if let Some(ip) = literal_ip {
        // ATYP=0x01 字面 IPv4：既有纯 IPv4 直连路径（无双栈/超时包裹）
        connect_via_nic(nic, SocketAddr::V4(SocketAddrV4::new(ip, port))).await
    } else if let Some(v6) = literal_v6 {
        // ATYP=0x04 字面 IPv6：既有 IPv6 直连路径
        connect_via_nic(nic, SocketAddr::V6(SocketAddrV6::new(v6, port, 0, 0))).await
    } else if let Some(addrs) = dual_addrs {
        // 域名目标：按 IP 版本偏好做 Happy-Eyeballs 双栈拨号与回退
        dial_dual(engine, nic, addrs, port, engine.ip_pref(), DIAL_FAMILY_TIMEOUT).await
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "establish_target 缺少目标地址信息 / no target address provided",
        ))
    }
}

/// 出口分派 + 回退循环：根据 `decide_egress` 结果返回「到真实目标的可转发流」（Req 5/6/7）。
///
/// - `Direct`：走 `connect_target_direct`（既有 `connect_via_nic`/`dial_dual`，字节流与
///   现状完全一致，Req 5.1/5.3）。
/// - `ViaUpstream`：以 `next_fallback` 驱动，在该网卡上游集合内逐个 `connect_via_upstream`，
///   维护 `tried` 集合；全部试尽后按 `upstream_fallback`——`Direct` 回退既有直连（Req 6.3）、
///   `Fail` 返回 `Err` 供调用方回标准错误应答（Req 6.4）。每次回退记录含原因的可读日志（Req 6.5）。
///
/// `sched_idx` 复用该网卡既有连接计数（`nic.active`），保证一网卡多上游间轮转（Req 2.3）。
/// `is_bypass` 用于满足 Req 7.1（bypass 绝不走上游）；调用点在既有 bypass 分支拦截之后，
/// 故实际传入恒为 `false`。
async fn establish_target(
    engine: &Engine,
    nic: &NicRuntime,
    host: &str,
    port: u16,
    literal_ip: Option<Ipv4Addr>,
    literal_v6: Option<Ipv6Addr>,
    dual_addrs: Option<&ResolvedAddrs>,
    is_bypass: bool,
) -> std::io::Result<TcpStream> {
    // 复用该网卡既有连接计数作为调度序（取模在 pick_upstream_for_nic 内做）
    let sched_idx = nic.active.load(Ordering::Relaxed).max(0) as usize;

    match decide_egress(
        engine.upstream_chain,
        nic.if_index,
        &engine.upstream_bindings,
        is_bypass,
        sched_idx,
    ) {
        Egress::Direct => {
            connect_target_direct(engine, nic, port, literal_ip, literal_v6, dual_addrs).await
        }
        Egress::ViaUpstream(_) => {
            let nic_upstreams = engine
                .upstream_bindings
                .get(&nic.if_index)
                .cloned()
                .unwrap_or_default();
            // [新增] Health_Prober 启用时：用「is_selectable 过滤候选 + select_weighted_upstream
            // （以 upstream_health 的 last_latency_ms 作延迟样本）」选出首选上游并置于回退列表
            // 首位（Req 2.1）；未启用时保持既有顺序不变——此时 next_fallback 回退循环、tried
            // 集合、回退直连/失败逻辑与上游代理链现状逐字节一致（零回归，Req 2.4/13.2）。
            let nic_upstreams = if engine.health_cfg.enabled && nic_upstreams.len() > 1 {
                preferred_order_by_health(engine, &nic_upstreams, sched_idx)
            } else {
                nic_upstreams
            };
            let mut tried: Vec<String> = Vec::new();
            loop {
                match next_fallback(&tried, &nic_upstreams, engine.upstream_fallback) {
                    FallbackStep::TryUpstream(id) => {
                        match engine.upstreams.get(&id) {
                            Some(up) => {
                                match connect_via_upstream(engine, nic, up, host, port).await {
                                    Ok(s) => return Ok(s),
                                    // 失败日志已在 connect_via_upstream 内记录（含上游标签）
                                    Err(_) => {
                                        tried.push(id);
                                    }
                                }
                            }
                            None => {
                                // 悬空引用（理论上 sanitize_bindings 已剔除）：跳过该 id
                                tried.push(id);
                            }
                        }
                    }
                    FallbackStep::Direct => {
                        engine.log(if engine.zh {
                            format!(
                                "[上游] 网卡 {} 绑定上游均不可用，回退直连 -> {}:{}",
                                nic.name, host, port
                            )
                        } else {
                            format!(
                                "[Upstream] adapter {} all bound upstreams unavailable, falling back to direct -> {}:{}",
                                nic.name, host, port
                            )
                        });
                        return connect_target_direct(
                            engine, nic, port, literal_ip, literal_v6, dual_addrs,
                        )
                        .await;
                    }
                    FallbackStep::Fail => {
                        engine.log(if engine.zh {
                            format!(
                                "[上游] 网卡 {} 绑定上游均不可用，策略=失败，返回错误 -> {}:{}",
                                nic.name, host, port
                            )
                        } else {
                            format!(
                                "[Upstream] adapter {} all bound upstreams unavailable, policy=fail -> {}:{}",
                                nic.name, host, port
                            )
                        });
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::ConnectionRefused,
                            "所有上游均不可用且回退策略为失败 / all upstreams unavailable, fallback policy is fail",
                        ));
                    }
                }
            }
        }
    }
}

// ============================================================================
// 后台健康探测任务 + 上游首选优选（Health_Prober / Upstream_Selector，Req 1/2）
//
// 仅当 `health_cfg.enabled` 时由 `start` spawn 探测任务；未启用时以下逻辑完全不触达，
// `establish_target` 保持既有 `pick_upstream_for_nic` 顺序，上游代理链行为字节级不变
// （零回归，Req 1.6/2.4/13.2）。探测经承载网卡 Egress_Binding 发起，对任意错误不 panic，
// 仅记日志；并发受 `task_cap` 信号量限制；随引擎 `cancel` 取消而结束。
// ============================================================================

/// 取当前 Unix 纪元毫秒（供健康状态机时间戳）。系统时钟异常时回退 0，绝不 panic。
fn now_epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// `establish_target` 的上游首选排序（Upstream_Selector，Req 2.1）。
///
/// 读取 `upstream_health` 快照，用 `is_selectable` 过滤该网卡候选（排除未过冷却期的
/// `CircuitOpen`，Req 1.4），再以 `select_weighted_upstream`（以各候选 `last_latency_ms`
/// 为延迟样本）选出首选上游并置于回退列表首位；其余上游（含被熔断排除者）按原相对顺序
/// 追加于其后，以便 `next_fallback` 在首选失败后仍可继续既有回退（Req 2.2/2.3）。
///
/// 仅读取健康快照、不改变引擎状态。全部候选均被熔断排除或加权取样无结果时，返回原顺序，
/// 交既有 `next_fallback` 回退循环处理（回退直连/失败语义完全不变，Req 2.2）。
fn preferred_order_by_health(
    engine: &Engine,
    nic_upstreams: &[String],
    sched_idx: usize,
) -> Vec<String> {
    let cfg = engine.health_cfg;
    let now = now_epoch_ms();

    // 过滤出当前可选候选（Healthy 或已过冷却期的 CircuitOpen），并取其延迟样本。
    let mut candidates: Vec<String> = Vec::new();
    let mut latencies: Vec<Option<u64>> = Vec::new();
    {
        let health = match engine.upstream_health.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        for id in nic_upstreams {
            let h = health.get(id).cloned().unwrap_or_default();
            if is_selectable(&h, cfg, now) {
                candidates.push(id.clone());
                latencies.push(h.last_latency_ms);
            }
        }
    }

    // 无可选候选（全部熔断且未到冷却）：保持原顺序，交既有 next_fallback 回退处理（Req 2.2）。
    if candidates.is_empty() {
        return nic_upstreams.to_vec();
    }

    // 加权优选首选上游；取样无结果时同样回退原顺序（稳健兜底）。
    let preferred = match select_weighted_upstream(&candidates, &latencies, sched_idx) {
        Some(p) => p,
        None => return nic_upstreams.to_vec(),
    };

    // 首选置于队首，其余上游（含被排除者）按原相对顺序追加，保留完整回退链（Req 2.2/2.3）。
    let mut ordered: Vec<String> = Vec::with_capacity(nic_upstreams.len());
    ordered.push(preferred.clone());
    for id in nic_upstreams {
        if *id != preferred {
            ordered.push(id.clone());
        }
    }
    ordered
}

/// SOCKS5 轻量握手探测：仅版本协商 + 可选用户名/密码认证子协商（不发 CONNECT）。
///
/// 足以验证「经承载网卡可达该上游且认证有效」，且不依赖任意外部目标的可达性——
/// 从而把「上游健康」与「目标可达」解耦。方法应答非法 / 认证失败 / 拒绝方法均返回 `Err`。
async fn probe_socks5_greeting(
    stream: &mut TcpStream,
    upstream: &UpstreamProxy,
) -> std::io::Result<()> {
    let cred = upstream_credential(upstream);

    // 版本协商：按是否有认证声明方法列表（复用既有构造/解析纯函数）。
    stream.write_all(&build_socks5_greeting(cred.is_some())).await?;
    let mut method_buf = [0u8; 2];
    stream.read_exact(&mut method_buf).await?;
    let method = parse_socks5_method_reply(&method_buf).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "探测：SOCKS5 方法应答非法 / probe: invalid SOCKS5 method reply",
        )
    })?;

    match method {
        // 用户名/密码认证子协商
        0x02 => {
            let (user, pass) = cred.ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "探测：上游要求认证但未配置凭据 / probe: auth required but no credentials",
                )
            })?;
            stream.write_all(&build_socks5_userpass(user, pass)).await?;
            let mut auth_buf = [0u8; 2];
            stream.read_exact(&mut auth_buf).await?;
            let status = parse_socks5_userpass_reply(&auth_buf).ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "探测：SOCKS5 认证应答非法 / probe: invalid SOCKS5 auth reply",
                )
            })?;
            if status != 0x00 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    format!("探测：SOCKS5 认证失败 status={status:#04x} / probe: SOCKS5 auth failed"),
                ));
            }
        }
        // 无需认证
        0x00 => {}
        other => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!("探测：上游拒绝认证方法 method={other:#04x} / probe: upstream rejected auth methods"),
            ));
        }
    }
    Ok(())
}

/// 对单个上游经其所属网卡出口发起一次轻量连通性探测（Health_Prober，Req 1.1/1.2）。
///
/// 复用既有 Egress_Binding 拨号骨架经 `nic` 物理出口连接上游，再执行「轻量握手」：
///   - `socks5`：完成版本协商（含可选认证），见 [`probe_socks5_greeting`]；
///   - `http`：以 TCP 建连成功为连通判定（HTTP 代理无 CONNECT 前置握手）。
///
/// 返回 `Ok(latency_ms)`（自拨号起至握手完成的耗时）或 `Err`。对任意错误不 panic，
/// 由调用方转为 `ProbeEvent` 喂给状态机。整体由调用方以 `timeout_ms` 包裹。
async fn probe_upstream_once(
    engine: &Engine,
    nic: &NicRuntime,
    upstream: &UpstreamProxy,
) -> std::io::Result<u64> {
    let start = std::time::Instant::now();

    // 1) 经网卡出口解析上游 host（复用既有双栈解析骨架，Req 4.2）。
    let addrs = engine
        .resolve_host_dual(nic, &upstream.host, upstream.port)
        .await;

    // 2) 经该网卡物理出口连上游（复用 Egress_Binding，Req 4.1/4.5）。
    let mut stream = if let Ok(ip) = upstream.host.parse::<Ipv4Addr>() {
        connect_via_nic(nic, SocketAddr::V4(SocketAddrV4::new(ip, upstream.port))).await?
    } else if let Ok(ip) = upstream.host.parse::<Ipv6Addr>() {
        connect_via_nic(nic, SocketAddr::V6(SocketAddrV6::new(ip, upstream.port, 0, 0))).await?
    } else {
        if addrs.v4.is_none() && addrs.v6.is_none() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AddrNotAvailable,
                "探测：经出口解析上游无可用地址族 / probe: no usable address family",
            ));
        }
        dial_dual(
            engine,
            nic,
            &addrs,
            upstream.port,
            engine.ip_pref(),
            engine.upstream_timeout,
        )
        .await?
    };

    // 3) 轻量握手：socks5 版本/认证协商；http 仅 TCP 连通即可。
    match upstream.kind.as_str() {
        "socks5" => probe_socks5_greeting(&mut stream, upstream).await?,
        "http" => { /* TCP 建连成功即视为连通 */ }
        other => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("探测：未知上游类型 kind={other} / probe: unknown upstream kind"),
            ));
        }
    }

    Ok(start.elapsed().as_millis() as u64)
}

/// 探测单个上游并据结果更新健康表（Health_Prober，Req 1.2/1.3/1.5/1.8）。
///
/// 以 `timeout_ms` 包裹一次轻量探测：成功喂 `ProbeEvent::Success(latency)`，失败/超时喂
/// `ProbeEvent::Failure`；经 `health_transition` 计算新状态写回 `upstream_health`。当状态
/// 发生 Healthy↔CircuitOpen 变化时记录一条含上游标签、新状态与原因的可读日志（Req 1.8）。
/// 对任意错误不 panic。
async fn probe_and_update(engine: &Engine, nic: &NicRuntime, upstream: &UpstreamProxy) {
    let cfg = engine.health_cfg;
    let timeout = std::time::Duration::from_millis(cfg.timeout_ms.max(1));

    let (event, err_reason) =
        match tokio::time::timeout(timeout, probe_upstream_once(engine, nic, upstream)).await {
            Ok(Ok(latency)) => (ProbeEvent::Success(latency), None),
            Ok(Err(e)) => (ProbeEvent::Failure, Some(e.to_string())),
            Err(_) => (
                ProbeEvent::Failure,
                Some(format!("探测超时 {timeout:?} / probe timed out")),
            ),
        };

    let now = now_epoch_ms();
    let label = upstream_label(upstream);

    // 读改写健康表，检测状态变化（Healthy ↔ CircuitOpen）。
    let (old_state, new_health) = {
        let mut guard = match engine.upstream_health.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        let cur = guard.get(&upstream.id).cloned().unwrap_or_default();
        let old_state = cur.state;
        let new_health = health_transition(cur, event, cfg, now);
        guard.insert(upstream.id.clone(), new_health.clone());
        (old_state, new_health)
    };

    // 状态变化记可读日志（含上游标签、新状态、失败原因，Req 1.8）。
    if old_state != new_health.state {
        let reason = err_reason.unwrap_or_else(|| "探测成功 / probe succeeded".to_string());
        engine.log(match new_health.state {
            HealthState::CircuitOpen => {
                if engine.zh {
                    format!(
                        "[健康探测] 上游 {} 熔断（连续失败 {} 次）：{}",
                        label, new_health.consecutive_failures, reason
                    )
                } else {
                    format!(
                        "[Health] upstream {} circuit opened (after {} consecutive failures): {}",
                        label, new_health.consecutive_failures, reason
                    )
                }
            }
            HealthState::Healthy => {
                if engine.zh {
                    format!("[健康探测] 上游 {label} 恢复健康：{reason}")
                } else {
                    format!("[Health] upstream {label} recovered to healthy: {reason}")
                }
            }
        });
    }
}

/// 单轮健康探测（Health_Prober，Req 1.1）。
///
/// 收集「被任意 Upstream_Binding 引用」的上游（未被引用者不探测，Req 边界），并为每个
/// 上游选取一张承载其绑定的网卡（首个引用它的网卡）作为探测出口。对 `Healthy` 上游每轮
/// 探测；对 `CircuitOpen` 上游仅当 `should_half_open` 为真（超过冷却期）时发起半开探测
/// （Req 1.5），未过冷却期则跳过以保持熔断（Req 1.4）。各探测并发受 `task_cap` 信号量限制
/// （Req 8.3），任务结束后统一 join；单个探测任务的异常不影响整体。
async fn health_probe_round(engine: &Arc<Engine>, sem: &Arc<tokio::sync::Semaphore>) {
    let cfg = engine.health_cfg;
    let now = now_epoch_ms();

    // 为每个被引用的上游选取承载网卡（首个引用它的网卡）；去重保证每个上游每轮至多一次探测。
    let mut targets: Vec<(String, u32)> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (if_index, ids) in engine.upstream_bindings.iter() {
        for id in ids {
            if seen.insert(id.clone()) {
                targets.push((id.clone(), *if_index));
            }
        }
    }

    let mut handles = Vec::new();
    for (id, if_index) in targets {
        // 读取当前健康度量，决定本轮是否探测该上游（半开判定，Req 1.4/1.5）。
        let cur = {
            let guard = match engine.upstream_health.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            guard.get(&id).cloned().unwrap_or_default()
        };
        let should_probe = match cur.state {
            HealthState::Healthy => true,
            HealthState::CircuitOpen => should_half_open(&cur, cfg, now),
        };
        if !should_probe {
            continue;
        }

        // 定位承载网卡与上游条目；缺失则跳过（sanitize_bindings 通常已剔除悬空引用）。
        let nic = match engine.nics.iter().find(|n| n.if_index == if_index) {
            Some(n) => n.clone(),
            None => continue,
        };
        let upstream = match engine.upstreams.get(&id) {
            Some(u) => u.clone(),
            None => continue,
        };

        let eng = engine.clone();
        let sem = sem.clone();
        handles.push(tauri::async_runtime::spawn(async move {
            // Task_Cap：并发受信号量限制（Req 8.3）；permit 随任务结束释放。
            let _permit = match sem.acquire().await {
                Ok(p) => p,
                Err(_) => return, // 信号量已关闭：放弃本次探测
            };
            probe_and_update(&eng, &nic, &upstream).await;
        }));
    }

    for h in handles {
        let _ = h.await; // 单个探测任务异常不影响整体
    }
}

/// 后台健康探测任务主循环（Health_Prober，Req 1.1）。
///
/// 仅当 `health_cfg.enabled` 时由 `start` spawn。启动即先探测一轮，随后按 `interval_ms`
/// 周期性探测；任一等待/探测期间收到 `cancel` 立即结束（随引擎停止而取消）。并发受
/// `task_cap` 信号量限制。对任意错误不 panic，仅记日志。
async fn health_prober_loop(engine: Arc<Engine>, cancel: CancellationToken) {
    let cfg = engine.health_cfg;
    // 间隔下限保护，避免 0 / 极小间隔导致忙循环。
    let interval = std::time::Duration::from_millis(cfg.interval_ms.max(1_000));
    // Task_Cap 信号量：限制单轮内并发探测数（Req 8.3）。
    let sem = Arc::new(tokio::sync::Semaphore::new(engine.task_cap.max(1)));

    loop {
        // 探测一轮（可被 cancel 中途打断）。
        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = health_probe_round(&engine, &sem) => {}
        }
        // 等待下一个探测间隔（可被 cancel 打断）。
        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = tokio::time::sleep(interval) => {}
        }
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

        let _ = dst6; // 字面 IPv6 目标经 establish_target 统一分派（Direct 分支与既有 connect_via_nic 一致）
        let mut upstream = match establish_target(
            &engine,
            &nic,
            &target_display,
            port,
            None,
            Some(v6),
            None,
            false,
        )
        .await
        {
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
        // ATYP=0x01 字面 IPv4：经 establish_target 统一分派（Direct 分支保持既有纯 IPv4 路径不变）
        match establish_target(&engine, &nic, &target_display, port, Some(ip), None, None, false).await {
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
        // 域名目标：经 establish_target 统一分派（Direct 分支走既有 Happy-Eyeballs 双栈拨号）
        let addrs = dual_addrs.expect("域名路径必然已完成双栈解析");
        match establish_target(&engine, &nic, &target_display, port, None, None, Some(&addrs), false).await {
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

    // HTTP 路径既有解析为单一 IPv4 后 connect_via_nic，作字面 IPv4 传入 establish_target
    // （Direct 分支行为与既有一致）；上游全失败且策略 Fail 时回 502。
    let mut upstream = match establish_target(
        &engine,
        &nic,
        &dst_host,
        dst_port,
        Some(*dst.ip()),
        None,
        None,
        false,
    )
    .await
    {
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
        // Feature: nic-upstream-proxy-chain, Property 3
        // SOCKS5 用户名/密码子协商 round-trip 与认证声明：
        //   - 任意用户名/密码（长度 1..=255）经 build_socks5_userpass 封装后，
        //     字节布局为 VER(0x01) ULEN USER PLEN PASS，可原样还原用户名与密码；
        //   - parse_socks5_userpass_reply 对 STATUS==0x00 判成功、非零判失败、截断返回 None；
        //   - build_socks5_greeting(true) 声明含用户名/密码方法 0x02，
        //     build_socks5_greeting(false) 不含 0x02。
        // Validates: Requirements 3.3, 9.2
        #![proptest_config(ProptestConfig { cases: 100, ..ProptestConfig::default() })]

        #[test]
        fn prop_socks5_userpass_roundtrip_and_auth(
            // 可打印 ASCII（单字节字符），保证「字符长度 == 字节长度」落在 RFC 1929
            // 的 ULEN/PLEN 单字节前缀可表示范围（1..=255）内，与凭据的现实取值域一致。
            username in "[\\x20-\\x7e]{1,255}",
            password in "[\\x20-\\x7e]{1,255}",
            status in any::<u8>(),
        ) {
            // 1) 子协商请求字节布局可还原用户名/密码（VER ULEN USER PLEN PASS）。
            let u_bytes = username.as_bytes();
            let p_bytes = password.as_bytes();
            let req = build_socks5_userpass(&username, &password);

            // VER 恒为 0x01
            prop_assert_eq!(req[0], 0x01u8);
            // ULEN 等于用户名字节长度
            let ulen = req[1] as usize;
            prop_assert_eq!(ulen, u_bytes.len());
            // USER 段与输入一致
            let user_seg = &req[2..2 + ulen];
            prop_assert_eq!(user_seg, u_bytes);
            // PLEN 等于密码字节长度
            let plen_pos = 2 + ulen;
            let plen = req[plen_pos] as usize;
            prop_assert_eq!(plen, p_bytes.len());
            // PASS 段与输入一致
            let pass_seg = &req[plen_pos + 1..plen_pos + 1 + plen];
            prop_assert_eq!(pass_seg, p_bytes);
            // 总长度恰为 3 + ULEN + PLEN
            prop_assert_eq!(req.len(), 3 + ulen + plen);

            // 2) 子协商应答 STATUS 判定：0x00 成功、非零失败。
            let reply = [0x01u8, status];
            let parsed = parse_socks5_userpass_reply(&reply);
            prop_assert_eq!(parsed, Some(status));
            let success = parsed == Some(0x00);
            prop_assert_eq!(success, status == 0x00);

            // 截断应答（<2 字节）返回 None，绝不 panic。
            prop_assert_eq!(parse_socks5_userpass_reply(&[]), None);
            prop_assert_eq!(parse_socks5_userpass_reply(&[0x01]), None);

            // 3) 版本协商方法声明：有认证含 0x02、无认证不含 0x02。
            let greeting_auth = build_socks5_greeting(true);
            let greeting_noauth = build_socks5_greeting(false);
            prop_assert!(greeting_auth.contains(&0x02u8));
            prop_assert!(!greeting_noauth.contains(&0x02u8));
        }
    }

    // ---- nic-upstream-proxy-chain 属性测试公共辅助 ----

    /// 构造仅供 `sanitize_bindings` 判定「id 是否存在」用的最小 UpstreamProxy。
    fn mk_upstream(id: &str) -> UpstreamProxy {
        UpstreamProxy {
            id: id.to_string(),
            kind: "socks5".to_string(),
            host: "127.0.0.1".to_string(),
            port: 1080,
            username: None,
            password: None,
            label: String::new(),
        }
    }

    /// 最小标准 Base64 解码器（RFC 4648，忽略 `=` 补位），用于 Property 6
    /// 验证 `basic_auth_b64` 的输出可被标准解码器原样还原。
    fn b64_decode_std(s: &str) -> Vec<u8> {
        fn val(c: u8) -> u32 {
            match c {
                b'A'..=b'Z' => (c - b'A') as u32,
                b'a'..=b'z' => (c - b'a' + 26) as u32,
                b'0'..=b'9' => (c - b'0' + 52) as u32,
                b'+' => 62,
                b'/' => 63,
                _ => 0,
            }
        }
        let filtered: Vec<u8> = s.bytes().filter(|&b| b != b'=').collect();
        let mut out = Vec::new();
        for chunk in filtered.chunks(4) {
            let mut n = 0u32;
            for &c in chunk {
                n = (n << 6) | val(c);
            }
            let cnt = chunk.len();
            n <<= 6 * (4 - cnt);
            let bytes_out = match cnt {
                2 => 1,
                3 => 2,
                4 => 3,
                _ => 0,
            };
            for i in 0..bytes_out {
                out.push(((n >> (16 - 8 * i)) & 0xFF) as u8);
            }
        }
        out
    }

    proptest! {
        // Feature: nic-upstream-proxy-chain, Property 1
        // SOCKS5 CONNECT 请求构造/解析 round-trip 与健壮性：
        //   - 任意 CONNECT 目标（IPv4 / IPv6 / 域名 + 端口）经 build_socks5_connect_req
        //     构造后再经 parse_socks5_connect_req 解析，应还原出等价目标；
        //   - 对任意字节序列 parse_socks5_connect_req 绝不 panic（非法/截断返回 None）。
        // Validates: Requirements 3.1, 3.5, 3.6, 9.1, 9.2
        #![proptest_config(ProptestConfig { cases: 100, ..ProptestConfig::default() })]

        #[test]
        fn prop_socks5_connect_req_roundtrip(
            which in 0u8..3,
            v4 in prop::array::uniform4(any::<u8>()),
            v6 in prop::array::uniform8(any::<u16>()),
            // 可打印 ASCII（不含空格/控制符），字节长度 == 字符长度 ≤ 60，落在单字节长度前缀域内
            host in "[\\x21-\\x7e]{1,60}",
            port in any::<u16>(),
            junk in prop::collection::vec(any::<u8>(), 0..48),
        ) {
            let target = match which {
                0 => ConnectTarget::V4(Ipv4Addr::from(v4), port),
                1 => ConnectTarget::V6(Ipv6Addr::from(v6), port),
                _ => ConnectTarget::Domain(host.clone(), port),
            };
            let built = build_socks5_connect_req(&target);
            let parsed = parse_socks5_connect_req(&built);
            prop_assert_eq!(parsed.as_ref(), Some(&target));

            // 任意字节序列绝不 panic（不校验结果，仅确认无 panic）。
            let _ = parse_socks5_connect_req(&junk);
        }
    }

    proptest! {
        // Feature: nic-upstream-proxy-chain, Property 2
        // SOCKS5 CONNECT 应答 REP 判定：
        //   - 任意合法应答（VER REP RSV ATYP BND.ADDR BND.PORT），parse_socks5_connect_reply
        //     返回的 rep 等于应答 REP 字段，「隧道成功」当且仅当 rep == 0x00；
        //   - 截断/非法应答返回 None（不误判成功、绝不 panic）。
        // Validates: Requirements 3.1, 3.6
        #![proptest_config(ProptestConfig { cases: 100, ..ProptestConfig::default() })]

        #[test]
        fn prop_socks5_connect_reply_rep(
            rep in any::<u8>(),
            which in 0u8..3,
            v4 in prop::array::uniform4(any::<u8>()),
            v6 in prop::array::uniform8(any::<u16>()),
            dlen in 0usize..20,
            port in any::<u16>(),
            junk in prop::collection::vec(any::<u8>(), 0..48),
        ) {
            let mut reply = vec![0x05u8, rep, 0x00];
            let consumed_expected: usize = match which {
                0 => {
                    reply.push(0x01);
                    reply.extend_from_slice(&v4);
                    reply.extend_from_slice(&port.to_be_bytes());
                    4 + 4 + 2
                }
                1 => {
                    reply.push(0x04);
                    for s in &v6 {
                        reply.extend_from_slice(&s.to_be_bytes());
                    }
                    reply.extend_from_slice(&port.to_be_bytes());
                    4 + 16 + 2
                }
                _ => {
                    reply.push(0x03);
                    reply.push(dlen as u8);
                    reply.extend(std::iter::repeat(0u8).take(dlen));
                    reply.extend_from_slice(&port.to_be_bytes());
                    4 + 1 + dlen + 2
                }
            };
            let parsed = parse_socks5_connect_reply(&reply);
            prop_assert_eq!(parsed, Some((rep, consumed_expected)));

            let success = parsed.map(|(r, _)| r == 0x00).unwrap_or(false);
            prop_assert_eq!(success, rep == 0x00);

            // 截断应答（<4 字节）返回 None，绝不误判成功。
            let trunc = &reply[..reply.len().min(3)];
            prop_assert_eq!(parse_socks5_connect_reply(trunc), None);

            // 任意字节序列绝不 panic。
            let _ = parse_socks5_connect_reply(&junk);
        }
    }

    proptest! {
        // Feature: nic-upstream-proxy-chain, Property 4
        // HTTP CONNECT 请求行构造 round-trip：
        //   - 任意 host/port（及可选认证），build_http_connect_req 首行形如
        //     `CONNECT <host>:<port> HTTP/1.1`，解析出的 host/port 等于输入；
        //   - 提供认证时恰含一行 `Proxy-Authorization: Basic <b64>`，未提供时不含该头。
        // Validates: Requirements 3.2, 3.5, 9.1, 9.2
        #![proptest_config(ProptestConfig { cases: 100, ..ProptestConfig::default() })]

        #[test]
        fn prop_http_connect_req_roundtrip(
            // host 不含冒号/空格，便于从请求行原样解析回来
            host in "[a-zA-Z0-9.\\-]{1,60}",
            port in any::<u16>(),
            with_auth in any::<bool>(),
            user in "[\\x21-\\x7e]{1,20}",
            pass in "[\\x21-\\x7e]{1,20}",
        ) {
            let auth = if with_auth {
                Some((user.as_str(), pass.as_str()))
            } else {
                None
            };
            let bytes = build_http_connect_req(&host, port, auth);
            let text = String::from_utf8(bytes).expect("请求报文应为合法 UTF-8");

            // 首行 round-trip
            let first = text.split("\r\n").next().expect("至少有一行");
            let expected_first = format!("CONNECT {}:{} HTTP/1.1", host, port);
            prop_assert_eq!(first, expected_first.as_str());

            let mid = first
                .strip_prefix("CONNECT ")
                .and_then(|s| s.strip_suffix(" HTTP/1.1"))
                .expect("首行前后缀匹配");
            let (h, p) = mid.rsplit_once(':').expect("host:port 形式");
            prop_assert_eq!(h, host.as_str());
            prop_assert_eq!(p.parse::<u16>().expect("端口为数字"), port);

            // Proxy-Authorization 头计数与内容
            let auth_lines: Vec<&str> = text
                .split("\r\n")
                .filter(|l| l.starts_with("Proxy-Authorization:"))
                .collect();
            if with_auth {
                prop_assert_eq!(auth_lines.len(), 1);
                let expected = format!("Proxy-Authorization: Basic {}", basic_auth_b64(&user, &pass));
                prop_assert_eq!(auth_lines[0], expected.as_str());
            } else {
                prop_assert_eq!(auth_lines.len(), 0);
            }
        }
    }

    proptest! {
        // Feature: nic-upstream-proxy-chain, Property 5
        // HTTP 状态行解析与 2xx 判定：
        //   - 任意合法状态行 `HTTP/1.x <code> <reason>`，parse_http_status_line 返回的
        //     状态码等于 <code>，「隧道成功」当且仅当状态码 ∈ [200,299]；
        //   - 任意非法/畸形状态行返回 None（不误判成功、绝不 panic）。
        // Validates: Requirements 3.2, 3.6
        #![proptest_config(ProptestConfig { cases: 100, ..ProptestConfig::default() })]

        #[test]
        fn prop_http_status_line_2xx(
            minor in 0u8..=1,
            code in 100u16..=999,
            with_reason in any::<bool>(),
            reason in "[a-zA-Z ]{0,20}",
            junk in ".*",
        ) {
            let line = if with_reason {
                format!("HTTP/1.{} {} {}", minor, code, reason)
            } else {
                format!("HTTP/1.{} {}", minor, code)
            };
            let parsed = parse_http_status_line(&line);
            prop_assert_eq!(parsed, Some(code));

            let success = parsed.map(|c| (200..=299).contains(&c)).unwrap_or(false);
            prop_assert_eq!(success, (200..=299).contains(&code));

            // 任意字符串绝不 panic。
            let _ = parse_http_status_line(&junk);
        }
    }

    proptest! {
        // Feature: nic-upstream-proxy-chain, Property 6
        // HTTP Basic 认证 Base64 round-trip：
        //   任意用户名/密码，basic_auth_b64(user, pass) 的输出经标准 Base64 解码后
        //   等于字节串 `<user>:<pass>`。
        // Validates: Requirements 3.4
        #![proptest_config(ProptestConfig { cases: 100, ..ProptestConfig::default() })]

        #[test]
        fn prop_basic_auth_b64_roundtrip(
            user in "[\\x20-\\x7e]{0,30}",
            pass in "[\\x20-\\x7e]{0,30}",
        ) {
            let encoded = basic_auth_b64(&user, &pass);
            let decoded = b64_decode_std(&encoded);
            let expected = format!("{}:{}", user, pass).into_bytes();
            prop_assert_eq!(decoded, expected);
        }
    }

    proptest! {
        // Feature: nic-upstream-proxy-chain, Property 7
        // 一网卡上游选择综合正确性（pick_upstream_for_nic）：
        //   - 无绑定 / 空列表 => None；
        //   - 非空列表 => 返回值 ∈ 列表；长度为 1 => 恒返回唯一 id；
        //   - 长度 > 1 => 连续 sched_idx 轮转覆盖列表全部 id；
        //   - 不同 if_index 共享同一列表均能各自正确选出。
        // Validates: Requirements 2.2, 2.3, 2.4, 9.3
        #![proptest_config(ProptestConfig { cases: 100, ..ProptestConfig::default() })]

        #[test]
        fn prop_pick_upstream_for_nic(
            n in 1usize..=8,
            if_index in any::<u32>(),
            sched_base in any::<usize>(),
            other_if in any::<u32>(),
        ) {
            // 构造去重、非空的上游 id 列表
            let list: Vec<String> = (0..n).map(|i| format!("u{}", i)).collect();
            let mut bindings: HashMap<u32, Vec<String>> = HashMap::new();
            bindings.insert(if_index, list.clone());

            // 未绑定网卡 => None
            let absent = if_index.wrapping_add(1);
            if !bindings.contains_key(&absent) {
                prop_assert_eq!(pick_upstream_for_nic(&bindings, absent, sched_base), None);
            }

            // 空列表 => None
            let mut empty_bindings = bindings.clone();
            empty_bindings.insert(if_index, Vec::new());
            prop_assert_eq!(pick_upstream_for_nic(&empty_bindings, if_index, sched_base), None);

            // 非空 => 返回值 ∈ 列表
            let got = pick_upstream_for_nic(&bindings, if_index, sched_base)
                .expect("非空列表应有选择");
            prop_assert!(list.contains(&got));

            // 长度为 1 => 恒返回唯一 id
            if n == 1 {
                prop_assert_eq!(&got, &list[0]);
            }

            // 轮转全覆盖：连续 n 个 sched_idx 覆盖列表全部 id
            let mut seen = std::collections::HashSet::new();
            for k in 0..n {
                let pick = pick_upstream_for_nic(&bindings, if_index, sched_base.wrapping_add(k))
                    .expect("非空列表应有选择");
                seen.insert(pick);
            }
            prop_assert_eq!(seen.len(), n);

            // 共享映射：另一 if_index 指向同一列表亦正确选出该列表内元素
            if other_if != if_index {
                let mut shared = bindings.clone();
                shared.insert(other_if, list.clone());
                let g2 = pick_upstream_for_nic(&shared, other_if, sched_base)
                    .expect("共享列表应有选择");
                prop_assert!(list.contains(&g2));
            }
        }
    }

    proptest! {
        // Feature: nic-upstream-proxy-chain, Property 8
        // 悬空上游引用剔除（sanitize_bindings）：
        //   - 构建后的映射不含任何不属于上游全集的 id（悬空引用被剔除）；
        //   - 所有引用了存在条目的绑定均被保留（按 if_index 合并计数一致）。
        // Validates: Requirements 2.6
        #![proptest_config(ProptestConfig { cases: 100, ..ProptestConfig::default() })]

        #[test]
        fn prop_sanitize_bindings(
            // 上游全集 id（如 "id3"）
            universe in prop::collection::vec("id[0-9]", 0..6),
            // 绑定引用：(if_index, 引用 id 列表)，id 取值域与全集有交集也有差集
            raw in prop::collection::vec((any::<u32>(), prop::collection::vec("id[0-9]", 0..6)), 0..6),
        ) {
            let mut upstreams: HashMap<String, UpstreamProxy> = HashMap::new();
            for id in &universe {
                upstreams.insert(id.clone(), mk_upstream(id));
            }
            let bindings: Vec<UpstreamBinding> = raw
                .iter()
                .map(|(ifx, ids)| UpstreamBinding {
                    if_index: *ifx,
                    upstream_ids: ids.clone(),
                })
                .collect();

            let out = sanitize_bindings(&upstreams, &bindings);

            // 无悬空引用：输出所有 id 均属于全集
            for ids in out.values() {
                for id in ids {
                    prop_assert!(upstreams.contains_key(id));
                }
            }

            // 保留计数一致：每个 if_index 保留的 id 数 == 其绑定引用中属于全集的 id 数之和
            let mut expected: HashMap<u32, usize> = HashMap::new();
            for (ifx, ids) in &raw {
                let c = ids.iter().filter(|id| upstreams.contains_key(*id)).count();
                *expected.entry(*ifx).or_insert(0) += c;
            }
            for (ifx, cnt) in &expected {
                let got = out.get(ifx).map(|v| v.len()).unwrap_or(0);
                prop_assert_eq!(got, *cnt);
            }
        }
    }

    proptest! {
        // Feature: nic-upstream-proxy-chain, Property 9
        // 出口决策综合正确性（decide_egress）：
        //   - 总开关 false => 恒 Direct（零回归）；
        //   - is_bypass => 恒 Direct（bypass 最高优先）；
        //   - 开关 true、非 bypass、有非空绑定 => ViaUpstream(id) 且 id ∈ 绑定集合；
        //   - 开关 true、非 bypass、无绑定/空绑定 => Direct。
        // Validates: Requirements 5.1, 5.2, 5.3, 7.1, 7.2, 7.3, 7.4
        #![proptest_config(ProptestConfig { cases: 100, ..ProptestConfig::default() })]

        #[test]
        fn prop_decide_egress(
            upstream_chain in any::<bool>(),
            is_bypass in any::<bool>(),
            if_index in any::<u32>(),
            n in 0usize..=5,
            sched in any::<usize>(),
        ) {
            let list: Vec<String> = (0..n).map(|i| format!("u{}", i)).collect();
            let mut bindings: HashMap<u32, Vec<String>> = HashMap::new();
            if n > 0 {
                bindings.insert(if_index, list.clone());
            }

            let e = decide_egress(upstream_chain, if_index, &bindings, is_bypass, sched);
            if !upstream_chain || is_bypass {
                prop_assert_eq!(e, Egress::Direct);
            } else if n == 0 {
                prop_assert_eq!(e, Egress::Direct);
            } else {
                match e {
                    Egress::ViaUpstream(id) => prop_assert!(list.contains(&id)),
                    Egress::Direct => prop_assert!(false, "有非空绑定且非 bypass 应走上游"),
                }
            }

            // 空绑定列表在开关开、非 bypass 时亦回退 Direct
            let mut empty_bindings: HashMap<u32, Vec<String>> = HashMap::new();
            empty_bindings.insert(if_index, Vec::new());
            prop_assert_eq!(
                decide_egress(true, if_index, &empty_bindings, false, sched),
                Egress::Direct
            );
        }
    }

    proptest! {
        // Feature: nic-upstream-proxy-chain, Property 10
        // 回退决策综合正确性（next_fallback）：
        //   - nic_upstreams 存在未试 id => TryUpstream(首个未试 id)；
        //   - 全部试尽 且 policy == Direct => Direct；
        //   - 全部试尽 且 policy == Fail => Fail。
        // Validates: Requirements 6.2, 6.3, 6.4, 9.4
        #![proptest_config(ProptestConfig { cases: 100, ..ProptestConfig::default() })]

        #[test]
        fn prop_next_fallback(
            nic in prop::collection::vec("u[0-9]", 0..6),
            tried in prop::collection::vec("u[0-9]", 0..6),
            use_direct in any::<bool>(),
        ) {
            let policy = if use_direct {
                FallbackPolicy::Direct
            } else {
                FallbackPolicy::Fail
            };
            let step = next_fallback(&tried, &nic, policy);

            let first_untried = nic.iter().find(|id| !tried.contains(id)).cloned();
            match first_untried {
                Some(id) => prop_assert_eq!(step, FallbackStep::TryUpstream(id)),
                None => {
                    let expected = if use_direct {
                        FallbackStep::Direct
                    } else {
                        FallbackStep::Fail
                    };
                    prop_assert_eq!(step, expected);
                }
            }
        }
    }

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

    proptest! {
        // Feature: pro-differentiation-and-hardening, Property 1
        // 健康状态机迁移正确性（health_transition）：
        //   - Success(latency) 恒使状态为 Healthy、连续失败计数归零、last_latency_ms=Some(latency)、
        //     清除熔断时间戳（涵盖 CircuitOpen 下半开探测成功的恢复）；
        //   - Failure 使连续失败计数 +1（saturating_add 防溢出），当且仅当新计数达到
        //     fail_threshold（下限 1）时进入 CircuitOpen 并记录 opened_at_ms=now_ms，
        //     否则保持 Healthy 且延迟样本/熔断时间戳沿用旧值；
        //   - CircuitOpen 下的 Success（半开探测成功）恒恢复为 Healthy；
        //   - 对任意输入不 panic。
        // Validates: Requirements 1.2, 1.3, 1.5
        #![proptest_config(ProptestConfig { cases: 100, ..ProptestConfig::default() })]

        #[test]
        fn prop_health_transition_correctness(
            // 起始态覆盖 Healthy 与 CircuitOpen；初始不同 consecutive_failures 与 opened_at。
            start_open in any::<bool>(),
            cur_failures in 0u32..8,
            cur_latency in proptest::option::of(any::<u64>()),
            cur_opened in proptest::option::of(any::<u64>()),
            // fail_threshold 覆盖边界：含 1 与更大值。
            fail_threshold in 1u32..6,
            // 事件：Success(latency) 或 Failure。
            is_success in any::<bool>(),
            latency in any::<u64>(),
            now_ms in any::<u64>(),
        ) {
            // CircuitOpen 起始态保证带熔断时间戳；Healthy 起始态无时间戳。
            let cur = UpstreamHealth {
                state: if start_open { HealthState::CircuitOpen } else { HealthState::Healthy },
                consecutive_failures: cur_failures,
                last_latency_ms: cur_latency,
                opened_at_ms: if start_open { cur_opened.or(Some(0)) } else { None },
            };
            let cfg = HealthConfig {
                enabled: true,
                interval_ms: 30_000,
                timeout_ms: 5_000,
                fail_threshold,
                cooldown_ms: 60_000,
            };
            let event = if is_success {
                ProbeEvent::Success(latency)
            } else {
                ProbeEvent::Failure
            };

            // 对任意输入不 panic（能返回即证不 panic）。
            let next = health_transition(cur.clone(), event, cfg, now_ms);

            if is_success {
                // Success 恒恢复 Healthy、清零失败计数、更新延迟样本、清除熔断时间戳
                // （无论起始为 Healthy 还是 CircuitOpen 的半开成功恢复）。
                prop_assert_eq!(next.state, HealthState::Healthy);
                prop_assert_eq!(next.consecutive_failures, 0);
                prop_assert_eq!(next.last_latency_ms, Some(latency));
                prop_assert_eq!(next.opened_at_ms, None);
            } else {
                // Failure：连续失败计数 +1（saturating）。
                let expected = cur.consecutive_failures.saturating_add(1);
                prop_assert_eq!(next.consecutive_failures, expected);
                let threshold = fail_threshold.max(1);
                if expected >= threshold {
                    // 当且仅当达到阈值 => 熔断并记录 opened_at_ms=now_ms。
                    prop_assert_eq!(next.state, HealthState::CircuitOpen);
                    prop_assert_eq!(next.opened_at_ms, Some(now_ms));
                    // 延迟样本沿用旧值（失败不产生新延迟样本）。
                    prop_assert_eq!(next.last_latency_ms, cur.last_latency_ms);
                } else {
                    // 未达阈值 => 保持 Healthy，延迟样本与熔断时间戳沿用旧值。
                    prop_assert_eq!(next.state, HealthState::Healthy);
                    prop_assert_eq!(next.last_latency_ms, cur.last_latency_ms);
                    prop_assert_eq!(next.opened_at_ms, cur.opened_at_ms);
                }
            }
        }
    }

    proptest! {
        // Feature: pro-differentiation-and-hardening, Property 2
        // 冷却期与半开/候选判定（should_half_open / is_selectable）：
        //   - should_half_open 为真当且仅当 state==CircuitOpen 且 opened_at 存在
        //     且 now_ms - opened_at（saturating_sub 语义）>= cooldown_ms；
        //   - is_selectable：Healthy 恒为 true；未过冷却期的 CircuitOpen 恒为 false；
        //     已过冷却期的 CircuitOpen 为 true（允许半开纳入候选）；
        //   - 对任意输入不 panic。
        // Validates: Requirements 1.4, 2.6
        #![proptest_config(ProptestConfig { cases: 100, ..ProptestConfig::default() })]

        #[test]
        fn prop_cooldown_half_open_and_selectable(
            // 起始态覆盖 Healthy 与 CircuitOpen。
            start_open in any::<bool>(),
            // opened_at 覆盖 Some/None（含 CircuitOpen 缺时间戳的退化情形）。
            has_opened in any::<bool>(),
            opened in 0u64..1_000_000_000,
            cooldown_ms in 0u64..1_000_000_000,
            // now_offset 让 now_ms 落在「熔断时刻 + 冷却期」边界附近，
            // 覆盖 now-opened <、=、> cooldown 三种关系；当 cooldown+offset<0 时
            // 还覆盖 now<opened 的 saturating_sub 下溢情形。
            now_offset in -5i64..=5i64,
            cur_failures in 0u32..8,
            cur_latency in proptest::option::of(any::<u64>()),
        ) {
            let opened_at_ms = if has_opened { Some(opened) } else { None };
            let h = UpstreamHealth {
                state: if start_open { HealthState::CircuitOpen } else { HealthState::Healthy },
                consecutive_failures: cur_failures,
                last_latency_ms: cur_latency,
                opened_at_ms,
            };
            let cfg = HealthConfig {
                enabled: true,
                interval_ms: 30_000,
                timeout_ms: 5_000,
                fail_threshold: 3,
                cooldown_ms,
            };
            // now_ms 定位在「进入熔断时刻 + 冷却期」的边界附近。
            let boundary = opened.saturating_add(cooldown_ms) as i128;
            let now_ms = (boundary + now_offset as i128).max(0) as u64;

            // 对任意输入不 panic（能返回即证不 panic）。
            let half_open = should_half_open(&h, cfg, now_ms);
            let selectable = is_selectable(&h, cfg, now_ms);

            // 独立复算期望：should_half_open 当且仅当 CircuitOpen + 有时间戳 + 已过冷却期。
            let expected_half_open = start_open
                && opened_at_ms.is_some()
                && now_ms.saturating_sub(opened) >= cooldown_ms;
            prop_assert_eq!(half_open, expected_half_open);

            // is_selectable：Healthy 恒 true；CircuitOpen 与 should_half_open 判定一致。
            let expected_selectable = if start_open { expected_half_open } else { true };
            prop_assert_eq!(selectable, expected_selectable);

            // 结构断言：Healthy 恒可选；未过冷却期的 CircuitOpen 恒不可选。
            if !start_open {
                prop_assert!(selectable);
            } else if expected_half_open {
                prop_assert!(selectable);
            } else {
                prop_assert!(!selectable);
            }
        }
    }

    proptest! {
        // Feature: pro-differentiation-and-hardening, Property 3
        // 加权优选恒在候选集内且排除熔断（select_weighted_upstream）：
        //   - candidates 为空 => None；
        //   - candidates 非空 => 返回值必属于 candidates（不引入集合外元素）；
        //   - latencies 长度与 candidates 不一致时不 panic（越界样本视为 None）；
        //   - 延迟更低者长期权重更高：给定一低延迟一高延迟候选，遍历连续 sched_idx
        //     覆盖完整权重区间后，低延迟候选被选次数 >= 高延迟候选。
        // Validates: Requirements 2.1, 2.6
        #![proptest_config(ProptestConfig { cases: 100, ..ProptestConfig::default() })]

        // 恒在候选集内 + 空集为 None + 长度不一致不 panic。
        #[test]
        fn prop_weighted_select_in_candidates(
            // 1..6 个唯一字符串 id（HashSet 保证唯一）。
            cand_set in proptest::collection::hash_set("[a-z]{1,6}", 1..6),
            // latencies 长度可与 candidates 不同（0..8），元素为 Option<u64>。
            latencies in proptest::collection::vec(proptest::option::of(any::<u64>()), 0..8),
            sched_idx in any::<usize>(),
        ) {
            let candidates: Vec<String> = cand_set.into_iter().collect();

            // 空候选集恒返回 None（不依赖其余入参）。
            prop_assert_eq!(select_weighted_upstream(&[], &latencies, sched_idx), None);

            // 非空候选集：返回值必属于 candidates（长度不一致亦不 panic）。
            let picked = select_weighted_upstream(&candidates, &latencies, sched_idx);
            match picked {
                Some(ref id) => prop_assert!(candidates.contains(id)),
                None => prop_assert!(false, "非空候选集不应返回 None"),
            }
        }

        // 低延迟候选长期权重更高：遍历完整权重区间统计选择分布。
        #[test]
        fn prop_weighted_select_prefers_low_latency(
            // 低延迟严格小于高延迟，保证权重单调非增关系可区分。
            lat_low in 0u64..500,
            lat_high in 501u64..5_000,
        ) {
            let candidates = vec!["low".to_string(), "high".to_string()];
            let latencies = vec![Some(lat_low), Some(lat_high)];

            // 复算总权重：sched_idx 在 [0, total) 上遍历时每个残余值命中一次，
            // 各候选被选次数恰等于其权重，故低延迟（权重更高）被选次数 >= 高延迟。
            let denom_low = lat_low + 50; // LATENCY_BASE_MS
            let denom_high = lat_high + 50;
            let w_low = 1_000_000u64 / denom_low; // WEIGHT_SCALE
            let w_high = 1_000_000u64 / denom_high;
            let total = w_low + w_high;
            prop_assert!(total > 0);

            let mut low_count = 0u64;
            let mut high_count = 0u64;
            for sched_idx in 0..(total as usize) {
                match select_weighted_upstream(&candidates, &latencies, sched_idx) {
                    Some(ref id) if id == "low" => low_count += 1,
                    Some(ref id) if id == "high" => high_count += 1,
                    other => prop_assert!(false, "非预期选择: {:?}", other),
                }
            }

            // 遍历完整权重区间后，低延迟候选被选次数不少于高延迟候选。
            prop_assert!(low_count >= high_count);
            // 计数总和等于遍历次数（每次都选中某个候选）。
            prop_assert_eq!(low_count + high_count, total);
        }
    }

    proptest! {
        // Feature: pro-differentiation-and-hardening, Property 5
        // 分流决策与 Route_Resolver 语义一致（compute_route_decision）：
        //   对任意（上游总开关 / bypass / 进程规则 / 域名规则 / 上游绑定 / 目标 host:port /
        //   进程名 / 调度回退承载网卡 / 调度序号）输入，compute_route_decision 的结果
        //   恒等于「直接组合调用 decide_rule_action + decide_egress（并镜像 bypass 最高优先与
        //   命中来源 matched_rule 记账）」得到的等价决策：
        //     - 命中 bypass（仅按 host、忽略端口）=> bypass_hit=true 且 nic_if_index/via_upstream=None；
        //     - 未命中 bypass => 优先级严格「进程规则 > 域名规则 > 调度回退」，仅 Nic 动作钉死承载网卡；
        //     - 走上游 vs 直连与 decide_egress 对同一 (upstream_chain, carrier, bindings, sched_idx)
        //       结果一致，且 via_upstream（若有）必属于承载网卡绑定集合。
        //   对任意输入不 panic。
        // Validates: Requirements 3.2, 3.3, 3.4, 3.6
        #![proptest_config(ProptestConfig { cases: 100, ..ProptestConfig::default() })]

        #[test]
        fn prop_compute_route_decision_matches_resolver_semantics(
            upstream_chain in any::<bool>(),
            // bypass / 域名规则 / host 复用小字母表，令命中场景自然高频出现。
            bypass in proptest::collection::vec("[a-c]{1,3}\\.(com|net)", 0..3),
            rules_proc in proptest::collection::vec(
                ("[a-c]{1,4}\\.exe", prop_oneof![
                    Just(RuleAction::Direct),
                    Just(RuleAction::Aggregate),
                    (1u32..5).prop_map(RuleAction::Nic),
                ]),
                0..3,
            ),
            rules_nic in proptest::collection::vec(("[a-c]{1,3}\\.(com|net)", 1u32..5), 0..4),
            // 上游绑定：ifindex(1..5) -> 上游 id 列表（可为空，触发直连回退）。
            bindings_raw in proptest::collection::vec(
                (1u32..5, proptest::collection::vec("[u-z]{1,3}", 0..3)),
                0..5,
            ),
            host in "[a-c]{1,3}\\.(com|net)",
            port in any::<u16>(),
            proc_name_opt in proptest::option::of("[a-c]{1,4}\\.exe"),
            chosen_if_index in 1u32..6,
            sched_idx in any::<usize>(),
        ) {
            let bindings: HashMap<u32, Vec<String>> = bindings_raw.into_iter().collect();
            let proc_name = proc_name_opt.as_deref();
            let h = host.to_lowercase();

            // ---- 参考模型：组合已被单独验证的原语（decide_rule_action / decide_egress /
            //      match_proc_rule / pattern_match）复现 Route_Resolver 语义 ----
            let expected = if bypass.iter().any(|b| pattern_match(b, &h, 0)) {
                // bypass 最高优先（仅按 host 匹配，端口忽略 => 传入 0），Req 3.2。
                RouteDecision {
                    bypass_hit: true,
                    matched_rule: MatchedRule::None,
                    nic_if_index: None,
                    via_upstream: None,
                }
            } else {
                // decide_rule_action 作为规则决策权威（进程规则优先于域名规则）。
                let action = decide_rule_action(&rules_proc, &rules_nic, proc_name, &host, port);
                let (carrier, matched) = match action {
                    // 仅 Nic 动作钉死承载网卡；判定命中来源以构造 matched_rule（Req 3.3）。
                    Some(RuleAction::Nic(ifindex)) => {
                        let from_proc = proc_name
                            .and_then(|n| match_proc_rule(&rules_proc, n))
                            .is_some();
                        if from_proc {
                            (ifindex, MatchedRule::Process(proc_name.unwrap().to_lowercase()))
                        } else {
                            let pat = rules_nic
                                .iter()
                                .find(|(p, _)| pattern_match(p, &h, port))
                                .map(|(p, _)| p.clone())
                                .expect("Nic 动作非进程来源必有域名匹配");
                            (ifindex, MatchedRule::Domain(pat))
                        }
                    }
                    // Direct/Aggregate（仅来自进程规则）或 None：不钉死承载网卡，回退调度预选。
                    _ => (chosen_if_index, MatchedRule::None),
                };
                // 出口决策镜像 decide_egress（bypass 已提前返回 => is_bypass=false），Req 3.4/3.6。
                let via = match decide_egress(upstream_chain, carrier, &bindings, false, sched_idx) {
                    Egress::ViaUpstream(id) => Some(id),
                    Egress::Direct => None,
                };
                RouteDecision {
                    bypass_hit: false,
                    matched_rule: matched,
                    nic_if_index: Some(carrier),
                    via_upstream: via,
                }
            };

            // ---- 被测：compute_route_decision（对任意输入不 panic）----
            let got = compute_route_decision(
                upstream_chain,
                &bypass,
                &rules_proc,
                &rules_nic,
                &bindings,
                &host,
                port,
                proc_name,
                chosen_if_index,
                sched_idx,
            );

            // 语义一致：与组合参考模型逐字段相等。
            prop_assert_eq!(&got, &expected);

            // 附加不变量断言（强化 Req 3.2/3.4/3.6）。
            if got.bypass_hit {
                // 命中 bypass：直连、无承载网卡、无上游（Req 3.2）。
                prop_assert!(got.via_upstream.is_none());
                prop_assert!(got.nic_if_index.is_none());
            } else {
                prop_assert!(got.nic_if_index.is_some());
                // 总开关关 => 恒直连（Req 3.6）。
                if !upstream_chain {
                    prop_assert!(got.via_upstream.is_none());
                }
                // 走上游 => via_upstream 必属于承载网卡绑定集合（Req 3.4/3.6）。
                if let Some(ref id) = got.via_upstream {
                    let carrier = got.nic_if_index.unwrap();
                    prop_assert!(bindings.get(&carrier).is_some_and(|v| v.contains(id)));
                }
            }
        }
    }

    proptest! {
        // Feature: pro-differentiation-and-hardening, Property 6
        // DNS 端点校验正确性（validate_dns_endpoint）：
        //   对任意 kind 与 endpoint，
        //     - Plain 通过当且仅当 endpoint 是合法 IPv4 或 IPv6 地址；
        //     - Doh   通过当且仅当 endpoint 是 https:// 开头且主机段非空的 URL；
        //     - 其余一律不通过。
        //   以标准库 Ipv4Addr/Ipv6Addr::from_str 作为 Plain 的参考模型交叉验证，
        //   并覆盖 Doh 的正/反例（http:// 前缀、无主机段、空串等）。对任意输入不 panic。
        // Validates: Requirements 7.5
        #![proptest_config(ProptestConfig { cases: 100, ..ProptestConfig::default() })]

        // Plain：随机合法 IPv4 字面量恒通过；与标准库参考模型交叉验证。
        #[test]
        fn prop_validate_dns_plain_ipv4_literal_passes(
            a in any::<u8>(), b in any::<u8>(), c in any::<u8>(), d in any::<u8>(),
        ) {
            let endpoint = format!("{}.{}.{}.{}", a, b, c, d);
            // 参考模型：标准库解析成功即为合法。
            prop_assert!(endpoint.parse::<Ipv4Addr>().is_ok());
            prop_assert!(validate_dns_endpoint(DnsKind::Plain, &endpoint));
            // 合法 IPv4 并非 https URL，故 Doh 下不通过。
            prop_assert!(!validate_dns_endpoint(DnsKind::Doh, &endpoint));
        }

        // Plain：随机合法 IPv6 字面量恒通过；与标准库参考模型交叉验证。
        #[test]
        fn prop_validate_dns_plain_ipv6_literal_passes(
            segs in proptest::array::uniform8(any::<u16>()),
        ) {
            let addr = Ipv6Addr::new(
                segs[0], segs[1], segs[2], segs[3], segs[4], segs[5], segs[6], segs[7],
            );
            let endpoint = addr.to_string();
            prop_assert!(endpoint.parse::<Ipv6Addr>().is_ok());
            prop_assert!(validate_dns_endpoint(DnsKind::Plain, &endpoint));
        }

        // Plain：随机字符串的判定必与「标准库 IPv4 或 IPv6 解析成功」等价（含随机非法串必不通过）。
        #[test]
        fn prop_validate_dns_plain_matches_std_parser(
            endpoint in "[0-9a-fA-F:.gG xyz]{0,20}",
        ) {
            let expected =
                endpoint.parse::<Ipv4Addr>().is_ok() || endpoint.parse::<Ipv6Addr>().is_ok();
            prop_assert_eq!(validate_dns_endpoint(DnsKind::Plain, &endpoint), expected);
        }

        // Doh：随机 https://<host>[:port][/path...]（host 非空）恒通过。
        #[test]
        fn prop_validate_dns_doh_https_with_host_passes(
            host in "[a-zA-Z0-9][a-zA-Z0-9.-]{0,20}",
            port_opt in proptest::option::of(1u16..=65535),
            path in "(/[a-zA-Z0-9/_-]{0,15})?",
        ) {
            let mut endpoint = format!("https://{}", host);
            if let Some(p) = port_opt {
                endpoint.push_str(&format!(":{}", p));
            }
            endpoint.push_str(&path);
            prop_assert!(validate_dns_endpoint(DnsKind::Doh, &endpoint));
            // https URL 一般不是裸 IP 字面量，Plain 下不通过（host 含字母，parse 必失败）。
            prop_assert!(!validate_dns_endpoint(DnsKind::Plain, &endpoint));
        }

        // Doh：非 https 前缀（如 http://）一律不通过。
        #[test]
        fn prop_validate_dns_doh_non_https_prefix_fails(
            host in "[a-zA-Z0-9][a-zA-Z0-9.-]{0,20}",
            path in "(/[a-zA-Z0-9/_-]{0,15})?",
        ) {
            let endpoint = format!("http://{}{}", host, path);
            prop_assert!(!validate_dns_endpoint(DnsKind::Doh, &endpoint));
        }

        // Doh：https:// 前缀但主机段为空一律不通过。主机段结束于路径 `/`、查询 `?`、
        // 片段 `#` 之前，故当这三者之一紧随前缀时，主机段必为空（与 tail 内容无关）。
        #[test]
        fn prop_validate_dns_doh_empty_host_fails(
            sep in prop_oneof![Just("/"), Just("?"), Just("#")],
            tail in "[a-zA-Z0-9/_?=#@:-]{0,15}",
        ) {
            // 形如 "https:///path"、"https://?q"、"https://#f"：前缀后主机段为空。
            let endpoint = format!("https://{}{}", sep, tail);
            prop_assert!(!validate_dns_endpoint(DnsKind::Doh, &endpoint));
        }

        // Doh：https:// 前缀且长度恰等于前缀（"https://" 本身）主机段为空，不通过。
        #[test]
        fn prop_validate_dns_doh_bare_prefix_fails(_dummy in any::<bool>()) {
            prop_assert!(!validate_dns_endpoint(DnsKind::Doh, "https://"));
        }

        // 全域不 panic：对任意 kind 与任意字节内容构造的字符串，两种 kind 均安全返回布尔。
        #[test]
        fn prop_validate_dns_never_panics(
            endpoint in ".{0,40}",
        ) {
            let _ = validate_dns_endpoint(DnsKind::Plain, &endpoint);
            let _ = validate_dns_endpoint(DnsKind::Doh, &endpoint);
        }
    }

    // ========================================================================
    // 端到端本地 mock 测试基建（Mock_Upstream + Echo_Target，Req 10.1/10.2/10.4）
    //
    // 供阶段 J（任务 10.2/10.3）的 connect_via_upstream / establish_target 端到端
    // 集成测试复用。全部只监听 127.0.0.1:0（随机端口），绝不触达真实公网 / 真实网卡
    // （Req 10.7/10.8）；均通过 CancellationToken 优雅关闭，避免测试进程遗留后台任务。
    //
    // Mock_Upstream 严格按 engine.rs 既有上游客户端握手报文格式（build_socks5_greeting /
    // build_socks5_userpass / build_socks5_connect_req / build_http_connect_req 的互逆）
    // 作出响应，从而 connect_via_upstream 能正确完成握手并经隧道转发到目标。
    // ========================================================================

    use tokio::task::JoinHandle;

    /// Mock_Upstream 支持的上游协议模式（与 UpstreamProxy.kind 对应）。
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[allow(dead_code)]
    enum MockUpstreamKind {
        Socks5,
        Http,
    }

    /// Mock_Upstream 的认证策略。
    ///
    /// - `NoAuth`：不要求认证，任何客户端握手直接放行。
    /// - `RequireAuth`：要求认证。仅当客户端提供的凭据与配置逐字节一致时握手成功；
    ///   客户端提供错误凭据时握手失败；**客户端未提供任何凭据时视为一次有效的认证
    ///   尝试并允许握手成功**（Req 10.4 的澄清语义）。
    #[derive(Debug, Clone)]
    #[allow(dead_code)]
    enum MockAuth {
        NoAuth,
        RequireAuth { username: String, password: String },
    }

    impl MockAuth {
        /// 便捷构造要求认证的策略。
        #[allow(dead_code)]
        fn require(username: &str, password: &str) -> Self {
            MockAuth::RequireAuth {
                username: username.to_string(),
                password: password.to_string(),
            }
        }
    }

    /// 一个运行于 `127.0.0.1` 的本地 mock 上游代理句柄。
    ///
    /// 建连后在后台 accept 循环中按 `kind` 分派 SOCKS5 / HTTP CONNECT 握手，握手成功
    /// 即向 CONNECT 请求中的真实目标（本地 Echo_Target）建连并双向转发字节，形成完整隧道。
    #[allow(dead_code)]
    struct MockUpstream {
        addr: SocketAddr,
        kind: MockUpstreamKind,
        cancel: CancellationToken,
        handle: JoinHandle<()>,
    }

    #[allow(dead_code)]
    impl MockUpstream {
        /// 监听地址（含实际随机端口）。
        fn addr(&self) -> SocketAddr {
            self.addr
        }

        /// 监听主机字符串（恒为 "127.0.0.1"）。
        fn host(&self) -> String {
            self.addr.ip().to_string()
        }

        /// 监听端口（实际绑定的随机端口）。
        fn port(&self) -> u16 {
            self.addr.port()
        }

        /// 构造一个引用本 mock 的 `UpstreamProxy`，供 `connect_via_upstream` 使用。
        ///
        /// `cred` 为客户端将向上游出示的凭据：`Some((user, pass))` 使客户端以
        /// 用户名/密码方法握手；`None` 使客户端不提供任何凭据（无认证握手）。
        /// `kind` 恒与 mock 自身协议模式一致。
        fn as_upstream(&self, id: &str, cred: Option<(&str, &str)>) -> UpstreamProxy {
            UpstreamProxy {
                id: id.to_string(),
                kind: match self.kind {
                    MockUpstreamKind::Socks5 => "socks5",
                    MockUpstreamKind::Http => "http",
                }
                .to_string(),
                host: self.host(),
                port: self.port(),
                username: cred.map(|(u, _)| u.to_string()),
                password: cred.map(|(_, p)| p.to_string()),
                label: id.to_string(),
            }
        }

        /// 优雅关闭：取消 accept 循环并等待后台任务结束。
        async fn shutdown(self) {
            self.cancel.cancel();
            let _ = self.handle.await;
        }
    }

    /// 一个运行于 `127.0.0.1` 的本地回显目标句柄：把隧道内收到的字节原样回写（Req 10.2）。
    #[allow(dead_code)]
    struct EchoTarget {
        addr: SocketAddr,
        cancel: CancellationToken,
        handle: JoinHandle<()>,
    }

    #[allow(dead_code)]
    impl EchoTarget {
        /// 监听地址（含实际随机端口）。
        fn addr(&self) -> SocketAddr {
            self.addr
        }

        /// 监听主机字符串（恒为 "127.0.0.1"）。
        fn host(&self) -> String {
            self.addr.ip().to_string()
        }

        /// 监听端口（实际绑定的随机端口）。
        fn port(&self) -> u16 {
            self.addr.port()
        }

        /// 优雅关闭：取消 accept 循环并等待后台任务结束。
        async fn shutdown(self) {
            self.cancel.cancel();
            let _ = self.handle.await;
        }
    }

    /// 把 `ConnectTarget` 还原为可供 `TcpStream::connect` 使用的 "host:port" 字符串。
    /// IPv6 字面地址加方括号。仅用于 mock 内部把隧道请求转发到真实目标（本地 Echo_Target）。
    #[allow(dead_code)]
    fn connect_target_to_addr_string(target: &ConnectTarget) -> String {
        match target {
            ConnectTarget::V4(ip, port) => format!("{}:{}", ip, port),
            ConnectTarget::V6(ip, port) => format!("[{}]:{}", ip, port),
            ConnectTarget::Domain(host, port) => format!("{}:{}", host, port),
        }
    }

    /// 读取 HTTP 请求头块，直到遇到空行分隔符 `\r\n\r\n`（含）。带上限保护，绝不无限增长。
    #[allow(dead_code)]
    async fn read_http_head_until_blank_line(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
        let mut buf: Vec<u8> = Vec::with_capacity(256);
        let mut byte = [0u8; 1];
        loop {
            stream.read_exact(&mut byte).await?;
            buf.push(byte[0]);
            if buf.len() >= 4 && &buf[buf.len() - 4..] == b"\r\n\r\n" {
                break;
            }
            if buf.len() > MAX_HEADER_BYTES {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "mock HTTP 请求头过大 / mock HTTP request head too large",
                ));
            }
        }
        Ok(buf)
    }

    /// 启动一个本地 Mock_Upstream，监听 `127.0.0.1:0` 并返回其句柄（含实际端口）。
    #[allow(dead_code)]
    async fn spawn_mock_upstream(
        kind: MockUpstreamKind,
        auth: MockAuth,
    ) -> std::io::Result<MockUpstream> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let addr = listener.local_addr()?;
        let cancel = CancellationToken::new();
        let child = cancel.clone();
        let handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = child.cancelled() => break,
                    accepted = listener.accept() => {
                        let client = match accepted {
                            Ok((c, _peer)) => c,
                            Err(_) => continue,
                        };
                        let conn_auth = auth.clone();
                        tokio::spawn(async move {
                            let _ = match kind {
                                MockUpstreamKind::Socks5 => {
                                    mock_handle_socks5(client, conn_auth).await
                                }
                                MockUpstreamKind::Http => {
                                    mock_handle_http(client, conn_auth).await
                                }
                            };
                        });
                    }
                }
            }
        });
        Ok(MockUpstream {
            addr,
            kind,
            cancel,
            handle,
        })
    }

    /// SOCKS5 模式 mock 上游的单连接处理：版本协商（含可选用户名/密码认证）+ CONNECT
    /// 请求解析 + 应答，握手成功后连接真实目标并双向转发（RFC 1928 / 1929）。
    #[allow(dead_code)]
    async fn mock_handle_socks5(mut client: TcpStream, auth: MockAuth) -> std::io::Result<()> {
        // 1) 版本协商：读 VER NMETHODS，再读 METHODS...
        let mut head = [0u8; 2];
        client.read_exact(&mut head).await?;
        if head[0] != 0x05 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "mock SOCKS5 版本非 0x05 / bad SOCKS5 version",
            ));
        }
        let nmethods = head[1] as usize;
        let mut methods = vec![0u8; nmethods];
        client.read_exact(&mut methods).await?;

        let require_auth = matches!(auth, MockAuth::RequireAuth { .. });
        let client_offers_userpass = methods.contains(&0x02);

        // 选择认证方法：要求认证且客户端声明了 0x02 时走用户名/密码子协商；否则一律以
        // 无认证方法 0x00 放行——这既覆盖「不要求认证」，也覆盖「要求认证但客户端未提供
        // 任何凭据（仅声明 0x00）」这一被视为有效认证尝试的情形（Req 10.4）。
        let selected: u8 = if require_auth && client_offers_userpass {
            0x02
        } else {
            0x00
        };
        client.write_all(&[0x05, selected]).await?;

        // 2) 用户名/密码子协商（仅当选定 0x02）
        if selected == 0x02 {
            let mut ver = [0u8; 1];
            client.read_exact(&mut ver).await?; // 子协商版本 0x01
            let mut ulen = [0u8; 1];
            client.read_exact(&mut ulen).await?;
            let mut user = vec![0u8; ulen[0] as usize];
            client.read_exact(&mut user).await?;
            let mut plen = [0u8; 1];
            client.read_exact(&mut plen).await?;
            let mut pass = vec![0u8; plen[0] as usize];
            client.read_exact(&mut pass).await?;

            let ok = match &auth {
                MockAuth::RequireAuth { username, password } => {
                    user == username.as_bytes() && pass == password.as_bytes()
                }
                MockAuth::NoAuth => true,
            };
            let status = if ok { 0x00u8 } else { 0x01u8 };
            client.write_all(&[0x01, status]).await?;
            if !ok {
                // 认证失败：关闭连接，客户端 parse_socks5_userpass_reply 得非零 => 握手失败。
                return Ok(());
            }
        }

        // 3) 读 CONNECT 请求：先读 4 字节固定头判 ATYP，再按类型读齐地址 + 端口。
        let mut hdr = [0u8; 4];
        client.read_exact(&mut hdr).await?;
        let mut req = hdr.to_vec();
        match hdr[3] {
            0x01 => {
                let mut rest = [0u8; 4 + 2];
                client.read_exact(&mut rest).await?;
                req.extend_from_slice(&rest);
            }
            0x04 => {
                let mut rest = [0u8; 16 + 2];
                client.read_exact(&mut rest).await?;
                req.extend_from_slice(&rest);
            }
            0x03 => {
                let mut dlen = [0u8; 1];
                client.read_exact(&mut dlen).await?;
                req.push(dlen[0]);
                let mut rest = vec![0u8; dlen[0] as usize + 2];
                client.read_exact(&mut rest).await?;
                req.extend_from_slice(&rest);
            }
            _ => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "mock SOCKS5 CONNECT ATYP 非法 / invalid ATYP",
                ));
            }
        }
        let target = parse_socks5_connect_req(&req).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "mock SOCKS5 CONNECT 请求非法 / invalid CONNECT request",
            )
        })?;
        let target_addr = connect_target_to_addr_string(&target);

        // 4) 连接真实目标并应答。BND.ADDR/BND.PORT 以 ATYP=0x01 0.0.0.0:0 占位。
        match TcpStream::connect(&target_addr).await {
            Ok(mut upstream_to_target) => {
                client
                    .write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                    .await?; // REP=0x00 成功
                let _ =
                    tokio::io::copy_bidirectional(&mut client, &mut upstream_to_target).await;
            }
            Err(_) => {
                // REP=0x01 general failure：客户端 parse_socks5_connect_reply 得非零 => 失败。
                client
                    .write_all(&[0x05, 0x01, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                    .await?;
            }
        }
        Ok(())
    }

    /// HTTP 模式 mock 上游的单连接处理：解析 `CONNECT host:port` 请求（含可选 Basic 认证）+
    /// 2xx/407 应答，握手成功后连接真实目标并双向转发。
    #[allow(dead_code)]
    async fn mock_handle_http(mut client: TcpStream, auth: MockAuth) -> std::io::Result<()> {
        let head = read_http_head_until_blank_line(&mut client).await?;
        let text = String::from_utf8_lossy(&head);

        // 请求行：CONNECT host:port HTTP/1.1
        let first_line = text.lines().next().unwrap_or("");
        let mut parts = first_line.split_whitespace();
        let method = parts.next().unwrap_or("");
        let target_authority = parts.next().unwrap_or("");
        if method != "CONNECT" || target_authority.is_empty() {
            client
                .write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n")
                .await?;
            return Ok(());
        }

        // 认证判定：要求认证时校验 Proxy-Authorization: Basic <b64>；未提供凭据视为
        // 有效认证尝试并成功（Req 10.4）；凭据错误则 407。
        let ok = match &auth {
            MockAuth::NoAuth => true,
            MockAuth::RequireAuth { username, password } => {
                let provided = text.lines().find_map(|line| {
                    let lower = line.to_ascii_lowercase();
                    if lower.starts_with("proxy-authorization:") {
                        line.split_once(':')
                            .map(|(_, v)| v.trim())
                            .and_then(|v| v.strip_prefix("Basic "))
                            .map(|b| b.trim().to_string())
                    } else {
                        None
                    }
                });
                match provided {
                    Some(b64) => b64 == basic_auth_b64(username, password),
                    None => true,
                }
            }
        };
        if !ok {
            client
                .write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n")
                .await?;
            return Ok(());
        }

        match TcpStream::connect(target_authority).await {
            Ok(mut upstream_to_target) => {
                client
                    .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                    .await?;
                let _ =
                    tokio::io::copy_bidirectional(&mut client, &mut upstream_to_target).await;
            }
            Err(_) => {
                client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await?;
            }
        }
        Ok(())
    }

    /// 启动一个本地 Echo_Target，监听 `127.0.0.1:0` 并把收到的字节原样回写（Req 10.2）。
    #[allow(dead_code)]
    async fn spawn_echo_target() -> std::io::Result<EchoTarget> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let addr = listener.local_addr()?;
        let cancel = CancellationToken::new();
        let child = cancel.clone();
        let handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = child.cancelled() => break,
                    accepted = listener.accept() => {
                        let mut conn = match accepted {
                            Ok((c, _peer)) => c,
                            Err(_) => continue,
                        };
                        tokio::spawn(async move {
                            let mut buf = [0u8; 8192];
                            loop {
                                match conn.read(&mut buf).await {
                                    Ok(0) => break, // 对端半关闭：结束回显
                                    Ok(n) => {
                                        if conn.write_all(&buf[..n]).await.is_err() {
                                            break;
                                        }
                                    }
                                    Err(_) => break,
                                }
                            }
                        });
                    }
                }
            }
        });
        Ok(EchoTarget {
            addr,
            cancel,
            handle,
        })
    }

    /// 冒烟自检：验证 Mock_Upstream（socks5/http）与 Echo_Target 均能成功绑定
    /// `127.0.0.1` 随机端口并可优雅关闭。仅校验监听端点为环回地址（Req 10.7/10.8：
    /// 不触达真实公网），不做协议级断言（协议级端到端断言属任务 10.2/10.3）。
    #[tokio::test]
    async fn mock_infra_binds_loopback_only() {
        let socks = spawn_mock_upstream(MockUpstreamKind::Socks5, MockAuth::NoAuth)
            .await
            .expect("bind socks5 mock upstream");
        let http = spawn_mock_upstream(
            MockUpstreamKind::Http,
            MockAuth::require("user", "pass"),
        )
        .await
        .expect("bind http mock upstream");
        let echo = spawn_echo_target().await.expect("bind echo target");

        // 仅绑定环回地址，端口为系统分配的非零随机端口。
        for a in [socks.addr(), http.addr(), echo.addr()] {
            assert!(a.ip().is_loopback(), "监听端点必须为环回地址: {a}");
            assert_ne!(a.port(), 0, "端口应为实际分配的随机端口");
        }
        // as_upstream 构造的 UpstreamProxy 指向本地 mock。
        let up = socks.as_upstream("mock-1", None);
        assert_eq!(up.kind, "socks5");
        assert_eq!(up.host, "127.0.0.1");
        assert_eq!(up.port, socks.port());

        socks.shutdown().await;
        http.shutdown().await;
        echo.shutdown().await;
    }

    // ========================================================================
    // connect_via_upstream 端到端集成测试（握手 + 隧道转发 + 认证三态，Req 10.3/10.4）
    //
    // 真实调用引擎的上游客户端 `connect_via_upstream`：经本地 Mock_Upstream（socks5 /
    // http，含 / 不含认证）建立到本地 Echo_Target 的隧道，写入伪随机字节并断言逐字节
    // 原样回读（握手 + 转发正确）；并覆盖认证三态：正确凭据成功、错误凭据失败、无凭据
    // 成功（Req 10.4 澄清语义）。
    //
    // 全部仅在 127.0.0.1 上运行：Engine（app=None，无 GUI 运行时）+ 环回网卡
    // （ipv4=127.0.0.1，if_index=0 即不做物理接口强绑定）+ 本地 mock / echo，绝不
    // 触达真实公网、真实网卡或 GUI（Req 10.7/10.8）；任何真实网络依赖都会使建连或
    // 断言失败，不以"碰巧可用"为通过。
    // ========================================================================

    /// 构造一个仅用于端到端集成测试的最小 `Engine`：`app=None`（emit / 日志降级为无
    /// 操作）、上游链总开关开启、其余字段取与 `start` 一致的缺省值。测试模块作为
    /// `engine` 的子模块，可直接访问 `Engine` 的私有字段构造该实例。
    fn test_loopback_engine() -> Engine {
        // 环回出口网卡：源地址钉在 127.0.0.1，if_index=0 表示不做 UNICAST_IF 物理强绑定
        // （setsockopt 值为 0 即"使用默认路由"），从而在无真实网卡的测试机上经环回连通。
        let nic = Arc::new(NicRuntime {
            name: "loopback-test".to_string(),
            ipv4: Ipv4Addr::LOCALHOST,
            ipv6: None,
            if_index: 0,
            active: AtomicI64::new(0),
            speed: AtomicU64::new(0),
            alive: AtomicBool::new(true),
            weight: 100,
            limiter: None,
        });
        let nics = vec![nic];
        Engine {
            nics: nics.clone(),
            // 显式路径消歧：proptest::prelude::* 亦导出名为 `Strategy` 的 trait。
            strategy: super::Strategy::RoundRobin,
            wrr: Mutex::new(vec![0i64; nics.len()]),
            conns: Arc::new(Mutex::new(HashMap::new())),
            conn_id: AtomicU64::new(0),
            app: None,
            zh: false,
            ip_version: "auto".to_string(),
            udp_associate: false,
            limiter: None,
            bypass: Vec::new(),
            rules_nic: Vec::new(),
            rules_proc: Vec::new(),
            dns_cache: Mutex::new(HashMap::new()),
            upstream_chain: true,
            upstreams: HashMap::new(),
            upstream_bindings: HashMap::new(),
            upstream_fallback: FallbackPolicy::Direct,
            upstream_timeout: std::time::Duration::from_secs(10),
            health_cfg: HealthConfig::default(),
            upstream_health: Arc::new(Mutex::new(HashMap::new())),
            per_nic_dns: HashMap::new(),
            conn_cap: 4096,
            task_cap: 64,
            active_conns: Arc::new(AtomicI64::new(0)),
        }
    }

    /// 取本引擎的（唯一）环回网卡引用，供 `connect_via_upstream` 调用。
    fn engine_nic(engine: &Engine) -> Arc<NicRuntime> {
        engine.nics[0].clone()
    }

    /// 生成 `len` 字节的伪随机负载：以当前时刻纳秒为种子的 xorshift64，避免每次运行相同，
    /// 同时不引入外部随机源依赖（保持 Req 10.7/10.8 的网络 / 环境独立性）。
    fn pseudo_random_bytes(len: usize) -> Vec<u8> {
        let mut state: u64 = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E37_79B9_7F4A_7C15)
            | 1; // 确保非零种子
        let mut out = Vec::with_capacity(len);
        for _ in 0..len {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            out.push((state & 0xFF) as u8);
        }
        out
    }

    /// 经 `connect_via_upstream` 建立到 `echo` 的隧道，写入伪随机字节并断言逐字节回读
    /// 相等；返回建连与转发均成功。分块写入 / 读取，覆盖多次读写而非单包往返。
    async fn tunnel_echo_roundtrip(
        engine: &Engine,
        nic: &NicRuntime,
        upstream: &UpstreamProxy,
        echo: &EchoTarget,
    ) {
        let host = echo.host();
        let port = echo.port();
        let mut stream = connect_via_upstream(engine, nic, upstream, &host, port)
            .await
            .expect("connect_via_upstream 应成功建立到 Echo_Target 的隧道");

        // 多批伪随机字节：验证握手后隧道对任意字节流的双向转发逐字节等价。
        for size in [1usize, 64, 1500, 8192] {
            let payload = pseudo_random_bytes(size);
            stream
                .write_all(&payload)
                .await
                .expect("向隧道写入负载应成功");
            let mut back = vec![0u8; size];
            stream
                .read_exact(&mut back)
                .await
                .expect("应能从隧道读回等长回显字节");
            assert_eq!(back, payload, "回读字节必须与写入逐字节相等（size={size}）");
        }
    }

    /// 断言经 `connect_via_upstream` 的上游握手失败（返回 Err），用于错误凭据用例。
    async fn expect_handshake_failure(
        engine: &Engine,
        nic: &NicRuntime,
        upstream: &UpstreamProxy,
        echo: &EchoTarget,
    ) {
        let host = echo.host();
        let port = echo.port();
        let result = connect_via_upstream(engine, nic, upstream, &host, port).await;
        assert!(
            result.is_err(),
            "错误凭据时上游握手应失败，但 connect_via_upstream 返回了 Ok"
        );
    }

    /// SOCKS5：不要求认证 + 客户端不提供凭据 => 握手成功且隧道逐字节回显（Req 10.3）。
    #[tokio::test]
    async fn e2e_socks5_noauth_tunnel_echo() {
        let engine = test_loopback_engine();
        let nic = engine_nic(&engine);
        let echo = spawn_echo_target().await.expect("bind echo target");
        let mock = spawn_mock_upstream(MockUpstreamKind::Socks5, MockAuth::NoAuth)
            .await
            .expect("bind socks5 mock upstream");

        let upstream = mock.as_upstream("s5-noauth", None);
        tunnel_echo_roundtrip(&engine, &nic, &upstream, &echo).await;

        mock.shutdown().await;
        echo.shutdown().await;
    }

    /// HTTP：不要求认证 + 客户端不提供凭据 => 握手成功且隧道逐字节回显（Req 10.3）。
    #[tokio::test]
    async fn e2e_http_noauth_tunnel_echo() {
        let engine = test_loopback_engine();
        let nic = engine_nic(&engine);
        let echo = spawn_echo_target().await.expect("bind echo target");
        let mock = spawn_mock_upstream(MockUpstreamKind::Http, MockAuth::NoAuth)
            .await
            .expect("bind http mock upstream");

        let upstream = mock.as_upstream("http-noauth", None);
        tunnel_echo_roundtrip(&engine, &nic, &upstream, &echo).await;

        mock.shutdown().await;
        echo.shutdown().await;
    }

    /// SOCKS5：要求认证 + 正确凭据 => 握手成功且隧道逐字节回显（Req 10.4）。
    #[tokio::test]
    async fn e2e_socks5_auth_correct_succeeds() {
        let engine = test_loopback_engine();
        let nic = engine_nic(&engine);
        let echo = spawn_echo_target().await.expect("bind echo target");
        let mock =
            spawn_mock_upstream(MockUpstreamKind::Socks5, MockAuth::require("alice", "secret"))
                .await
                .expect("bind socks5 mock upstream");

        let upstream = mock.as_upstream("s5-auth-ok", Some(("alice", "secret")));
        tunnel_echo_roundtrip(&engine, &nic, &upstream, &echo).await;

        mock.shutdown().await;
        echo.shutdown().await;
    }

    /// SOCKS5：要求认证 + 错误凭据 => 握手失败（Req 10.4）。
    #[tokio::test]
    async fn e2e_socks5_auth_wrong_fails() {
        let engine = test_loopback_engine();
        let nic = engine_nic(&engine);
        let echo = spawn_echo_target().await.expect("bind echo target");
        let mock =
            spawn_mock_upstream(MockUpstreamKind::Socks5, MockAuth::require("alice", "secret"))
                .await
                .expect("bind socks5 mock upstream");

        let upstream = mock.as_upstream("s5-auth-bad", Some(("alice", "wrong-pass")));
        expect_handshake_failure(&engine, &nic, &upstream, &echo).await;

        mock.shutdown().await;
        echo.shutdown().await;
    }

    /// SOCKS5：要求认证 + 客户端不提供任何凭据 => 视为有效认证尝试并成功（Req 10.4）。
    #[tokio::test]
    async fn e2e_socks5_auth_none_succeeds() {
        let engine = test_loopback_engine();
        let nic = engine_nic(&engine);
        let echo = spawn_echo_target().await.expect("bind echo target");
        let mock =
            spawn_mock_upstream(MockUpstreamKind::Socks5, MockAuth::require("alice", "secret"))
                .await
                .expect("bind socks5 mock upstream");

        let upstream = mock.as_upstream("s5-auth-none", None);
        tunnel_echo_roundtrip(&engine, &nic, &upstream, &echo).await;

        mock.shutdown().await;
        echo.shutdown().await;
    }

    /// HTTP：要求认证 + 正确凭据 => 握手成功且隧道逐字节回显（Req 10.4）。
    #[tokio::test]
    async fn e2e_http_auth_correct_succeeds() {
        let engine = test_loopback_engine();
        let nic = engine_nic(&engine);
        let echo = spawn_echo_target().await.expect("bind echo target");
        let mock =
            spawn_mock_upstream(MockUpstreamKind::Http, MockAuth::require("bob", "p@ss"))
                .await
                .expect("bind http mock upstream");

        let upstream = mock.as_upstream("http-auth-ok", Some(("bob", "p@ss")));
        tunnel_echo_roundtrip(&engine, &nic, &upstream, &echo).await;

        mock.shutdown().await;
        echo.shutdown().await;
    }

    /// HTTP：要求认证 + 错误凭据 => 握手失败（407，Req 10.4）。
    #[tokio::test]
    async fn e2e_http_auth_wrong_fails() {
        let engine = test_loopback_engine();
        let nic = engine_nic(&engine);
        let echo = spawn_echo_target().await.expect("bind echo target");
        let mock =
            spawn_mock_upstream(MockUpstreamKind::Http, MockAuth::require("bob", "p@ss"))
                .await
                .expect("bind http mock upstream");

        let upstream = mock.as_upstream("http-auth-bad", Some(("bob", "nope")));
        expect_handshake_failure(&engine, &nic, &upstream, &echo).await;

        mock.shutdown().await;
        echo.shutdown().await;
    }

    /// HTTP：要求认证 + 客户端不提供任何凭据 => 视为有效认证尝试并成功（Req 10.4）。
    #[tokio::test]
    async fn e2e_http_auth_none_succeeds() {
        let engine = test_loopback_engine();
        let nic = engine_nic(&engine);
        let echo = spawn_echo_target().await.expect("bind echo target");
        let mock =
            spawn_mock_upstream(MockUpstreamKind::Http, MockAuth::require("bob", "p@ss"))
                .await
                .expect("bind http mock upstream");

        let upstream = mock.as_upstream("http-auth-none", None);
        tunnel_echo_roundtrip(&engine, &nic, &upstream, &echo).await;

        mock.shutdown().await;
        echo.shutdown().await;
    }

    // ========================================================================
    // establish_target + next_fallback 回退端到端集成测试（Req 10.5/10.6/10.7）
    //
    // 覆盖：①首选拒连回退次选成功；②全部上游不可用 + 回退策略 Direct 直连成功；
    // ③全部上游不可用 + 回退策略 Fail 返回错误。全部仅绑定 127.0.0.1 mock，
    // 不触达真实公网 / 真实网卡（Req 10.7）。
    // ========================================================================

    /// 构造一个指向「已释放端口」的 SOCKS5 `Upstream_Proxy`：绑定 `127.0.0.1:0` 取得
    /// 系统分配端口后立即释放监听器，使后续对该端口的连接被内核拒绝（ConnectionRefused），
    /// 从而稳定模拟「不可达 / 拒绝连接的上游」。仅使用环回地址（Req 10.7）。
    async fn dead_socks5_upstream(id: &str) -> UpstreamProxy {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("bind dead upstream placeholder");
        let addr = listener.local_addr().expect("dead upstream local_addr");
        // 释放监听器：该端口不再有进程 accept，连接将被拒绝。
        drop(listener);
        UpstreamProxy {
            id: id.to_string(),
            kind: "socks5".to_string(),
            host: addr.ip().to_string(),
            port: addr.port(),
            username: None,
            password: None,
            label: id.to_string(),
        }
    }

    /// 对已建立的隧道流写入多批伪随机字节并断言逐字节回读相等（复用 Echo_Target 语义）。
    async fn assert_stream_echo_roundtrip(stream: &mut TcpStream) {
        for size in [1usize, 64, 1500, 8192] {
            let payload = pseudo_random_bytes(size);
            stream
                .write_all(&payload)
                .await
                .expect("向隧道写入负载应成功");
            let mut back = vec![0u8; size];
            stream
                .read_exact(&mut back)
                .await
                .expect("应能从隧道读回等长回显字节");
            assert_eq!(back, payload, "回读字节必须与写入逐字节相等（size={size}）");
        }
    }

    /// 回退到次选：首个上游拒连、次选上游可用 => `establish_target` 经 `next_fallback`
    /// 回退到次选并成功建立到 Echo_Target 的隧道、逐字节回显正确（Req 10.5）。
    #[tokio::test]
    async fn e2e_establish_target_falls_back_to_secondary() {
        let mut engine = test_loopback_engine();
        let nic = engine_nic(&engine);
        let echo = spawn_echo_target().await.expect("bind echo target");

        // 首选：指向已释放端口的死上游（连接被拒绝）。
        let dead = dead_socks5_upstream("s5-dead-primary").await;
        // 次选：可用的本地 SOCKS5 mock（无认证）。
        let mock = spawn_mock_upstream(MockUpstreamKind::Socks5, MockAuth::NoAuth)
            .await
            .expect("bind socks5 mock upstream");
        let good = mock.as_upstream("s5-good-secondary", None);

        // 两上游按「死上游在前、可用上游在后」绑定到承载网卡（if_index=0）。
        engine.upstreams.insert(dead.id.clone(), dead.clone());
        engine.upstreams.insert(good.id.clone(), good.clone());
        engine
            .upstream_bindings
            .insert(nic.if_index, vec![dead.id.clone(), good.id.clone()]);
        engine.upstream_fallback = FallbackPolicy::Direct;

        let mut stream = establish_target(
            &engine,
            &nic,
            &echo.host(),
            echo.port(),
            Some(Ipv4Addr::LOCALHOST),
            None,
            None,
            false,
        )
        .await
        .expect("首选拒连后应经 next_fallback 回退到次选并成功建立隧道");

        assert_stream_echo_roundtrip(&mut stream).await;

        mock.shutdown().await;
        echo.shutdown().await;
    }

    /// 全部上游不可用 + 回退策略 Direct：两上游均拒连 => `establish_target` 回退直连
    /// 本地 Echo_Target 成功、逐字节回显正确（Req 10.6）。
    #[tokio::test]
    async fn e2e_establish_target_all_upstreams_down_direct_fallback() {
        let mut engine = test_loopback_engine();
        let nic = engine_nic(&engine);
        let echo = spawn_echo_target().await.expect("bind echo target");

        let dead1 = dead_socks5_upstream("s5-dead-1").await;
        let dead2 = dead_socks5_upstream("s5-dead-2").await;
        engine.upstreams.insert(dead1.id.clone(), dead1.clone());
        engine.upstreams.insert(dead2.id.clone(), dead2.clone());
        engine
            .upstream_bindings
            .insert(nic.if_index, vec![dead1.id.clone(), dead2.id.clone()]);
        engine.upstream_fallback = FallbackPolicy::Direct;

        // literal_ip 指向 Echo_Target（环回），供上游全试尽后的「回退直连」分支直连。
        let mut stream = establish_target(
            &engine,
            &nic,
            &echo.host(),
            echo.port(),
            Some(Ipv4Addr::LOCALHOST),
            None,
            None,
            false,
        )
        .await
        .expect("全部上游不可用且策略=Direct 时应回退直连 Echo_Target 成功");

        assert_stream_echo_roundtrip(&mut stream).await;

        echo.shutdown().await;
    }

    /// 全部上游不可用 + 回退策略 Fail：两上游均拒连 => `establish_target` 返回错误（Req 10.6）。
    #[tokio::test]
    async fn e2e_establish_target_all_upstreams_down_fail_policy_errors() {
        let mut engine = test_loopback_engine();
        let nic = engine_nic(&engine);
        let echo = spawn_echo_target().await.expect("bind echo target");

        let dead1 = dead_socks5_upstream("s5-dead-a").await;
        let dead2 = dead_socks5_upstream("s5-dead-b").await;
        engine.upstreams.insert(dead1.id.clone(), dead1.clone());
        engine.upstreams.insert(dead2.id.clone(), dead2.clone());
        engine
            .upstream_bindings
            .insert(nic.if_index, vec![dead1.id.clone(), dead2.id.clone()]);
        engine.upstream_fallback = FallbackPolicy::Fail;

        let result = establish_target(
            &engine,
            &nic,
            &echo.host(),
            echo.port(),
            Some(Ipv4Addr::LOCALHOST),
            None,
            None,
            false,
        )
        .await;

        assert!(
            result.is_err(),
            "全部上游不可用且策略=Fail 时 establish_target 应返回错误，但返回了 Ok"
        );

        echo.shutdown().await;
    }
}
