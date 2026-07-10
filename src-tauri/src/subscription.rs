//! 订阅式上游导入解析（Subscription_Importer，Req 4）。
//!
//! 本模块把一段 Import_Source（Clash 订阅 YAML / base64 编码正文 /
//! 节点分享链接集合）解析为一组 `UpstreamProxy` 候选。全部为**不依赖 IO 的纯函数**，
//! 可被 `proptest` 属性测试完全覆盖（Req 11.3）。
//!
//! 设计约束（见 design.md 关键设计决策「订阅解析不引入 YAML 重依赖」）：
//! - 仅识别并保留类型为 `socks5` / `http` 的节点（Req 4.1/4.3）。
//! - 非 `socks5` / `http` 协议（ss / vmess / trojan / hysteria 等）或畸形节点被
//!   跳过、不写入候选，并计入 `ignored_unsupported`（Req 4.4/4.8）。
//! - 对任意字节输入绝不 panic，无受支持节点返回空候选（Req 4.5/4.8）。
//! - 采用最小内联解析（Clash proxies 段行式解析 + 分享链接解析 + base64 预解码），
//!   不引入 YAML / base64 重依赖。

#![allow(dead_code)]

use crate::engine::UpstreamProxy;

/// 一次订阅导入的解析结果。
///
/// `candidates` 仅含类型为 `socks5` / `http` 的受支持上游候选；
/// `ignored_unsupported` 统计被识别为「不支持」或「畸形」而跳过的节点数量，供 UI 提示。
#[derive(Debug, Clone)]
pub struct ImportResult {
    /// 受支持的上游候选（仅 socks5 / http）。
    pub candidates: Vec<UpstreamProxy>,
    /// 被忽略（不支持协议或畸形）的节点计数。
    pub ignored_unsupported: usize,
}

impl ImportResult {
    /// 空结果（无候选、无忽略）。
    fn empty() -> Self {
        ImportResult { candidates: Vec::new(), ignored_unsupported: 0 }
    }
}

// `UpstreamProxy` 沿用 engine 定义，仅派生 `Debug/Clone/Deserialize`（无 `PartialEq`）。
// 为不改动 engine.rs，这里手写 `ImportResult` 的相等语义（按候选字段逐条比较）。
impl PartialEq for ImportResult {
    fn eq(&self, other: &Self) -> bool {
        self.ignored_unsupported == other.ignored_unsupported
            && self.candidates.len() == other.candidates.len()
            && self
                .candidates
                .iter()
                .zip(other.candidates.iter())
                .all(|(a, b)| proxy_eq(a, b))
    }
}

/// 两个 `UpstreamProxy` 是否字段等价（供 `ImportResult` 的 `PartialEq`）。
fn proxy_eq(a: &UpstreamProxy, b: &UpstreamProxy) -> bool {
    a.id == b.id
        && a.kind == b.kind
        && a.host == b.host
        && a.port == b.port
        && a.username == b.username
        && a.password == b.password
        && a.label == b.label
}

/// 顶层解析：自动识别 Clash YAML / base64 正文 / 分享链接集合（Req 4.1/4.2/4.5/4.8）。
///
/// 识别顺序：
/// 1. 若正文含 `proxies:` 段 => 按 Clash 订阅解析；
/// 2. 否则尝试 base64 解码，成功后对解码正文再按 Clash / 分享链接解析（Req 4.2）；
/// 3. 否则按明文分享链接集合逐行解析。
///
/// 对任意字节输入均不 panic；解析不出受支持节点时返回空候选。
pub fn parse_subscription(input: &str) -> ImportResult {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return ImportResult::empty();
    }

    // 1) 明文即为 Clash YAML。
    if looks_like_clash(trimmed) {
        return parse_clash_proxies(trimmed);
    }

    // 2) base64 正文预解码后再解析（Req 4.2）。
    if let Some(decoded) = try_base64_decode(trimmed) {
        if looks_like_clash(&decoded) {
            return parse_clash_proxies(&decoded);
        }
        return parse_share_links_block(&decoded);
    }

    // 3) 明文分享链接集合。
    parse_share_links_block(trimmed)
}

/// 是否疑似 Clash 订阅（存在以 `proxies:` 开头的行）。
fn looks_like_clash(text: &str) -> bool {
    text.lines().any(|l| l.trim_start().starts_with("proxies:"))
}

/// 逐行解析分享链接集合：受支持链接入候选，其余非空行计入忽略计数。
fn parse_share_links_block(text: &str) -> ImportResult {
    let mut candidates = Vec::new();
    let mut ignored = 0usize;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        match parse_share_link(line) {
            Some(p) => candidates.push(p),
            None => ignored += 1,
        }
    }
    ImportResult { candidates, ignored_unsupported: ignored }
}

/// base64 正文预解码（Req 4.2）。
///
/// 兼容标准（`+/`）与 URL-safe（`-_`）字母表，忽略空白与 `=` 补位，容忍缺省补位。
/// 解码失败、含非法字符、结果非合法 UTF-8 或结果为空时返回 `None`（交由后续按明文尝试）。
pub fn try_base64_decode(input: &str) -> Option<String> {
    let mut val: u32 = 0;
    let mut bits: u32 = 0;
    let mut out: Vec<u8> = Vec::new();
    let mut seen = false;
    for b in input.bytes() {
        if b.is_ascii_whitespace() {
            continue;
        }
        if b == b'=' {
            // 补位符：忽略，不参与解码。
            continue;
        }
        seen = true;
        let d: u32 = match b {
            b'A'..=b'Z' => (b - b'A') as u32,
            b'a'..=b'z' => (b - b'a' + 26) as u32,
            b'0'..=b'9' => (b - b'0' + 52) as u32,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None, // 非法 base64 字符 => 判定为非 base64 正文
        };
        val = (val << 6) | d;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((val >> bits) as u8);
        }
    }
    if !seen || out.is_empty() {
        return None;
    }
    String::from_utf8(out).ok()
}

/// Clash 订阅 `proxies:` 段最小提取（Req 4.3）。
///
/// 仅取 `type ∈ {socks5, http}` 的 `name` / `server` / `port` / `username` / `password`
/// 映射为候选；其他类型或畸形节点计入 `ignored_unsupported`。
pub fn parse_clash_proxies(yaml: &str) -> ImportResult {
    let mut candidates = Vec::new();
    let mut ignored = 0usize;
    for item in extract_proxy_items(yaml) {
        match proxy_from_map(&item) {
            Some(p) => candidates.push(p),
            None => ignored += 1,
        }
    }
    ImportResult { candidates, ignored_unsupported: ignored }
}

/// 从 Clash YAML 的 `proxies:` 段抽取每个节点的键值对列表。
///
/// 支持块状（多行缩进 `key: value`）与流式（`- {name: x, type: socks5, ...}`）两种写法。
fn extract_proxy_items(yaml: &str) -> Vec<Vec<(String, String)>> {
    let mut items: Vec<Vec<(String, String)>> = Vec::new();
    let mut in_proxies = false;
    let mut proxies_indent = 0usize;
    let mut current: Option<Vec<(String, String)>> = None;

    for raw in yaml.lines() {
        let indent = raw.len() - raw.trim_start().len();
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if !in_proxies {
            if trimmed == "proxies:" {
                in_proxies = true;
                proxies_indent = indent;
            }
            continue;
        }

        // 已进入 proxies 段：遇到同级或更浅的普通键 => 退出该段。
        if indent <= proxies_indent && !trimmed.starts_with('-') {
            if let Some(item) = current.take() {
                items.push(item);
            }
            in_proxies = false;
            continue;
        }

        if trimmed.starts_with('-') {
            // 新列表项：先落盘上一项。
            if let Some(item) = current.take() {
                items.push(item);
            }
            let rest = trimmed[1..].trim();
            if rest.starts_with('{') {
                // 流式：单行完整节点。
                let mut map = Vec::new();
                parse_flow_map(rest, &mut map);
                items.push(map);
                current = None;
            } else {
                // 块状：`- key: value` 的首键。
                let mut map = Vec::new();
                if let Some(kv) = parse_kv(rest) {
                    map.push(kv);
                }
                current = Some(map);
            }
        } else if let Some(ref mut map) = current {
            // 块状项的后续键。
            if let Some(kv) = parse_kv(trimmed) {
                map.push(kv);
            }
        }
    }

    if let Some(item) = current.take() {
        items.push(item);
    }
    items
}

/// 解析流式内联映射 `{k1: v1, k2: v2, ...}` 为键值对。
fn parse_flow_map(s: &str, out: &mut Vec<(String, String)>) {
    let inner = s
        .trim()
        .trim_start_matches('{')
        .trim_end_matches('}')
        .trim();
    for part in inner.split(',') {
        if let Some(kv) = parse_kv(part.trim()) {
            out.push(kv);
        }
    }
}

/// 解析单个 `key: value`，键小写化、值去引号去空白。
fn parse_kv(s: &str) -> Option<(String, String)> {
    let (k, v) = s.split_once(':')?;
    let key = k.trim().to_ascii_lowercase();
    if key.is_empty() {
        return None;
    }
    let value = unquote(v.trim());
    Some((key, value))
}

/// 去除首尾成对的单/双引号并去空白。
fn unquote(s: &str) -> String {
    let t = s.trim();
    let bytes = t.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return t[1..t.len() - 1].to_string();
        }
    }
    t.to_string()
}

/// 在键值对列表中取值（键已小写）。
fn map_get<'a>(map: &'a [(String, String)], key: &str) -> Option<&'a str> {
    map.iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

/// 将一个 Clash 节点的键值对映射为受支持候选；不支持/畸形返回 `None`。
fn proxy_from_map(map: &[(String, String)]) -> Option<UpstreamProxy> {
    let kind = normalize_kind(map_get(map, "type")?)?;
    let host = map_get(map, "server").unwrap_or("").trim();
    if host.is_empty() {
        return None;
    }
    let port = parse_port(map_get(map, "port").unwrap_or(""))?;
    let username = opt(map_get(map, "username").unwrap_or(""));
    let password = opt(map_get(map, "password").unwrap_or(""));
    let label = map_get(map, "name").unwrap_or("").to_string();
    Some(build_proxy(kind, host, port, username, password, label))
}

/// 单条分享链接解析（Req 4.4）。
///
/// `socks5://[user:pass@]host:port#name`、`http(s)://[user:pass@]host:port#name`
/// 映射为候选；`ss` / `vmess` / `trojan` / `hysteria` 等其他协议或畸形链接返回 `None`。
pub fn parse_share_link(line: &str) -> Option<UpstreamProxy> {
    let line = line.trim();
    let (scheme, rest) = line.split_once("://")?;
    let kind = normalize_kind(scheme)?;

    // 去除锚点 `#name` 作为 label。
    let (main, frag) = match rest.split_once('#') {
        Some((m, f)) => (m, Some(f)),
        None => (rest, None),
    };
    // 去除 host:port 之后的路径 / 查询串。
    let main = main
        .split(|c| c == '/' || c == '?')
        .next()
        .unwrap_or(main);

    // 认证段（最后一个 `@` 之前）。
    let (auth, hostport) = match main.rsplit_once('@') {
        Some((a, hp)) => (Some(a), hp),
        None => (None, main),
    };

    // host:port（从右侧切分，兼容 IPv6 的 `[::1]:port`）。
    let (host_raw, port_str) = hostport.rsplit_once(':')?;
    let port = parse_port(port_str)?;
    let host = strip_ipv6_brackets(host_raw);
    if host.is_empty() {
        return None;
    }

    let (username, password) = match auth {
        Some(a) => match a.split_once(':') {
            Some((u, p)) => (opt(u), opt(p)),
            None => (opt(a), None),
        },
        None => (None, None),
    };

    let label = frag.unwrap_or("").to_string();
    Some(build_proxy(kind, &host, port, username, password, label))
}

/// 归一化上游类型：仅接受 socks5 / http 家族，其余（含空）返回 `None`。
fn normalize_kind(raw: &str) -> Option<&'static str> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "socks5" | "socks" | "socks5h" => Some("socks5"),
        "http" | "https" => Some("http"),
        _ => None,
    }
}

/// 解析端口：必须为 1..=65535。
fn parse_port(s: &str) -> Option<u16> {
    let p: u16 = s.trim().parse().ok()?;
    if p == 0 {
        None
    } else {
        Some(p)
    }
}

/// 去除 IPv6 字面量的方括号。
fn strip_ipv6_brackets(host: &str) -> String {
    let h = host.trim();
    if h.starts_with('[') && h.ends_with(']') && h.len() >= 2 {
        h[1..h.len() - 1].to_string()
    } else {
        h.to_string()
    }
}

/// 空字符串归一为 `None`，否则 `Some`。
fn opt(s: &str) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

/// 构造候选，生成稳定唯一 id（内容哈希，纯确定性）。
fn build_proxy(
    kind: &'static str,
    host: &str,
    port: u16,
    username: Option<String>,
    password: Option<String>,
    label: String,
) -> UpstreamProxy {
    let id = stable_id(kind, host, port, username.as_deref(), password.as_deref(), &label);
    UpstreamProxy {
        id,
        kind: kind.to_string(),
        host: host.to_string(),
        port,
        username,
        password,
        label,
    }
}

/// 基于节点内容生成稳定唯一 id（FNV-1a 64 位哈希，纯函数、可复现）。
fn stable_id(
    kind: &str,
    host: &str,
    port: u16,
    username: Option<&str>,
    password: Option<&str>,
    label: &str,
) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut mix = |bytes: &[u8]| {
        for &b in bytes {
            hash ^= b as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        // 字段分隔符，避免不同拼接产生同一序列。
        hash ^= 0x1f;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    };
    mix(kind.as_bytes());
    mix(host.as_bytes());
    mix(&port.to_le_bytes());
    mix(username.unwrap_or("").as_bytes());
    mix(password.unwrap_or("").as_bytes());
    mix(label.as_bytes());
    format!("sub-{:016x}", hash)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// 受支持协议方案（round-trip 用），归一化后 kind 与自身一致。
    fn supported_scheme() -> impl Strategy<Value = &'static str> {
        prop_oneof![Just("socks5"), Just("http")]
    }

    /// 不受支持协议方案：解析应返回 None 并计入 ignored。
    fn unsupported_scheme() -> impl Strategy<Value = &'static str> {
        prop_oneof![
            Just("ss"),
            Just("vmess"),
            Just("trojan"),
            Just("hysteria"),
            Just("hysteria2"),
            Just("vless"),
            Just("ssr"),
            Just("socks4"),
        ]
    }

    // 认证段：None=无认证；Some((user, None))=仅用户名；Some((user, Some(pass)))=用户名+密码。
    fn auth_strategy() -> impl Strategy<Value = Option<(String, Option<String>)>> {
        let user = "[a-zA-Z0-9]{1,15}";
        let pass = "[a-zA-Z0-9]{1,15}";
        prop_oneof![
            Just(None),
            user.prop_map(|u| Some((u, None))),
            (user, pass).prop_map(|(u, p)| Some((u, Some(p)))),
        ]
    }

    proptest! {
        // Feature: pro-differentiation-and-hardening, Property 4
        // 健壮性：对任意字符串输入，parse_subscription 与 parse_share_link 均不 panic；
        // 解析出的候选 kind 恒 ∈ {socks5, http}；候选数 + 忽略数不溢出（usize 语义天然 ≥0）。
        // Validates: Requirements 4.1, 4.5, 4.8
        #[test]
        fn prop_parse_subscription_robust(input in ".*") {
            let res = parse_subscription(&input);
            // 全部候选仅可能为受支持类型。
            for p in &res.candidates {
                prop_assert!(p.kind == "socks5" || p.kind == "http");
            }
            // parse_share_link 对任意行输入亦不得 panic。
            let _ = parse_share_link(&input);
        }
    }

    proptest! {
        // Feature: pro-differentiation-and-hardening, Property 4
        // round-trip：由受支持（socks5/http）节点构造的合法分享链接（含可选认证）
        // 经 parse_share_link 应还原出等价的 kind/host/port/username/password/label；
        // 且经 parse_subscription（单行）应得到恰好 1 个候选、0 个忽略。
        // Validates: Requirements 4.1, 4.4
        #[test]
        fn prop_share_link_round_trip(
            scheme in supported_scheme(),
            host in "[a-z][a-z0-9.-]{0,20}",
            port in 1u16..=65535,
            auth in auth_strategy(),
            label in "[a-zA-Z0-9]{0,10}",
        ) {
            // 构造认证前缀。
            let auth_prefix = match &auth {
                None => String::new(),
                Some((u, None)) => format!("{}@", u),
                Some((u, Some(p))) => format!("{}:{}@", u, p),
            };
            let link = format!("{}://{}{}:{}#{}", scheme, auth_prefix, host, port, label);

            // parse_share_link 应还原等价字段。
            let parsed = parse_share_link(&link);
            prop_assert!(parsed.is_some(), "link should parse: {}", link);
            let p = parsed.unwrap();
            prop_assert_eq!(&p.kind, scheme);
            prop_assert_eq!(&p.host, &host);
            prop_assert_eq!(p.port, port);
            prop_assert_eq!(&p.label, &label);

            let (exp_user, exp_pass) = match &auth {
                None => (None, None),
                Some((u, None)) => (Some(u.clone()), None),
                Some((u, Some(pw))) => (Some(u.clone()), Some(pw.clone())),
            };
            prop_assert_eq!(&p.username, &exp_user);
            prop_assert_eq!(&p.password, &exp_pass);

            // parse_subscription 单行：恰好 1 个候选、0 个忽略，且字段一致。
            let res = parse_subscription(&link);
            prop_assert_eq!(res.candidates.len(), 1);
            prop_assert_eq!(res.ignored_unsupported, 0);
            let c = &res.candidates[0];
            prop_assert_eq!(&c.kind, scheme);
            prop_assert_eq!(&c.host, &host);
            prop_assert_eq!(c.port, port);
            prop_assert_eq!(&c.username, &exp_user);
            prop_assert_eq!(&c.password, &exp_pass);
        }
    }

    proptest! {
        // Feature: pro-differentiation-and-hardening, Property 4
        // 非支持协议：ss/vmess/trojan/hysteria 等分享链接 parse_share_link 返回 None，
        // 且在 parse_subscription 中不写入候选、计入 ignored_unsupported。
        // Validates: Requirements 4.2, 4.3, 4.4, 4.8
        #[test]
        fn prop_unsupported_scheme_ignored(
            scheme in unsupported_scheme(),
            host in "[a-z][a-z0-9.-]{0,20}",
            port in 1u16..=65535,
        ) {
            let link = format!("{}://{}:{}", scheme, host, port);

            // 单条解析应为 None（不受支持）。
            prop_assert!(parse_share_link(&link).is_none());

            // 顶层解析：无受支持候选、忽略计数为 1。
            let res = parse_subscription(&link);
            prop_assert!(res.candidates.is_empty());
            prop_assert_eq!(res.ignored_unsupported, 1);
        }
    }
}
