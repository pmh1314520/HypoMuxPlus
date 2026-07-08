//! 本地日志落地子系统的纯函数部分（Logger）
//!
//! 本模块仅包含不依赖任何 IO 的纯函数与 [`LogLevel`] 枚举，供上层的滚动文件
//! sink（见 task 8.2 的 `Logger` 结构体）与属性测试复用：
//!
//! - [`format_log_line`]：把「时间戳 + 级别 + 消息」组装为单行日志文本。
//! - [`redact`]：对本机可标识信息脱敏（IPv4 后两段掩码、IPv6 前缀外掩码、
//!   `C:\Users\<name>\` 的用户名段替换为 `<USER>`）。
//! - [`files_to_prune`]：给定现有日志文件名列表与保留上限，返回应删除的较旧文件。
//!
//! 设计约束：只脱敏「本机可标识信息」（本机 IP、用户名路径），公网目标 IP 不强制
//! 脱敏以便排障；完整数据报 / 负载内容绝不进入日志（由调用方保证）。

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// 日志级别。`label()` 提供用于 [`format_log_line`] 的稳定文本标签。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LogLevel {
    Info,
    Warn,
    Error,
}

impl LogLevel {
    /// 返回该级别的大写文本标签（用于日志行渲染）。
    pub(crate) fn label(self) -> &'static str {
        match self {
            LogLevel::Info => "INFO",
            LogLevel::Warn => "WARN",
            LogLevel::Error => "ERROR",
        }
    }
}

/// 组装单行日志：`[ts] [LEVEL] msg`。
///
/// 输出保证同时包含传入的时间戳、级别标签与消息文本。此函数不做任何 IO，
/// 也不对 `msg` 脱敏（脱敏由调用方在传入前经 [`redact`] 完成）。
pub(crate) fn format_log_line(ts: &str, level: LogLevel, msg: &str) -> String {
    format!("[{}] [{}] {}", ts, level.label(), msg)
}

/// 对日志消息中的本机可标识信息脱敏。
///
/// 规则：
/// - `C:\Users\<name>\...`（分隔符 `\` 或 `/`，`Users` 大小写不敏感）的用户名段
///   替换为 `<USER>`。
/// - IPv6 地址保留前两段、其余掩码为 `s0:s1::*`。
/// - IPv4 地址保留前两段、后两段掩码为 `a.b.*.*`。
///
/// 处理顺序为「用户名路径 -> IPv6 -> IPv4」，以避免 IPv4-mapped IPv6 被重复处理。
pub(crate) fn redact(msg: &str) -> String {
    let s = redact_user_path(msg);
    let s = mask_ipv6(&s);
    mask_ipv4(&s)
}

/// 滚动裁剪决策：给定现有日志文件名列表与保留上限 `max_files`，
/// 返回应删除的较旧文件名列表（保留按字典序最新的 `max_files` 个）。
///
/// 日志文件通常以可排序的时间戳命名，按字典序升序排列后，较早（较小）者视为较旧。
/// 当文件数不超过上限时返回空列表；`max_files == 0` 时返回全部文件。
pub(crate) fn files_to_prune(existing: &[String], max_files: usize) -> Vec<String> {
    if existing.len() <= max_files {
        return Vec::new();
    }
    let mut sorted: Vec<String> = existing.to_vec();
    sorted.sort(); // 字典序升序：最旧在前
    let prune_count = sorted.len() - max_files;
    sorted.into_iter().take(prune_count).collect()
}

// ------------------------- 内部脱敏辅助（纯函数） -------------------------

/// 判断 `c` 忽略 ASCII 大小写后是否等于小写字符 `lower`。
fn ci_eq(c: char, lower: char) -> bool {
    c.to_ascii_lowercase() == lower
}

/// 将 `C:\Users\<name>\` / `C:/Users/<name>/` 中的用户名段替换为 `<USER>`。
fn redact_user_path(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < n {
        // 形如 <sep>Users<sep>，且其后至少还有一个字符作为用户名段
        let is_sep = |c: char| c == '\\' || c == '/';
        if is_sep(chars[i])
            && i + 6 < n
            && ci_eq(chars[i + 1], 'u')
            && ci_eq(chars[i + 2], 's')
            && ci_eq(chars[i + 3], 'e')
            && ci_eq(chars[i + 4], 'r')
            && ci_eq(chars[i + 5], 's')
            && is_sep(chars[i + 6])
        {
            out.push(chars[i]);
            for &c in &chars[i + 1..=i + 5] {
                out.push(c); // 保留 Users 原始大小写
            }
            out.push(chars[i + 6]);
            // 用户名段：直到下一个分隔符或结尾
            let mut j = i + 7;
            while j < n && !is_sep(chars[j]) {
                j += 1;
            }
            if j > i + 7 {
                out.push_str("<USER>");
            }
            i = j;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// 对输入中所有「连续匹配 `in_set` 的子串」应用 `transform`，
/// `transform` 返回 `Some` 则替换、返回 `None` 则原样保留。
fn transform_runs<P, F>(input: &str, in_set: P, transform: F) -> String
where
    P: Fn(char) -> bool,
    F: Fn(&str) -> Option<String>,
{
    let mut out = String::with_capacity(input.len());
    let mut run = String::new();
    for ch in input.chars() {
        if in_set(ch) {
            run.push(ch);
        } else {
            if !run.is_empty() {
                flush_run(&run, &transform, &mut out);
                run.clear();
            }
            out.push(ch);
        }
    }
    if !run.is_empty() {
        flush_run(&run, &transform, &mut out);
    }
    out
}

/// 将一个 run 经 `transform` 处理后写入 `out`（`None` 则原样写入）。
fn flush_run<F>(run: &str, transform: &F, out: &mut String)
where
    F: Fn(&str) -> Option<String>,
{
    match transform(run) {
        Some(replaced) => out.push_str(&replaced),
        None => out.push_str(run),
    }
}

/// 掩码所有可解析为 IPv6 地址的片段（保留前两段，其余掩码）。
fn mask_ipv6(input: &str) -> String {
    transform_runs(
        input,
        |c| c.is_ascii_hexdigit() || c == ':' || c == '.',
        mask_one_ipv6,
    )
}

/// 掩码所有可解析为 IPv4 地址的片段（保留前两段，后两段掩码）。
fn mask_ipv4(input: &str) -> String {
    transform_runs(input, |c| c.is_ascii_digit() || c == '.', mask_one_ipv4)
}

/// 尝试把 run 视为 IPv6 地址并掩码；保留 run 首尾的 `.`（若有）。
fn mask_one_ipv6(run: &str) -> Option<String> {
    let core = run.trim_matches('.');
    if core.is_empty() || !core.contains(':') {
        return None;
    }
    let ip = core.parse::<Ipv6Addr>().ok()?;
    let seg = ip.segments();
    let masked = format!("{:x}:{:x}::*", seg[0], seg[1]);
    Some(reattach_dots(run, core, &masked))
}

/// 尝试把 run 视为 IPv4 地址并掩码；保留 run 首尾的 `.`（若有）。
fn mask_one_ipv4(run: &str) -> Option<String> {
    let core = run.trim_matches('.');
    if core.is_empty() {
        return None;
    }
    let ip = core.parse::<Ipv4Addr>().ok()?;
    let o = ip.octets();
    let masked = format!("{}.{}.*.*", o[0], o[1]);
    Some(reattach_dots(run, core, &masked))
}

/// 把 `run` 中被 `trim_matches('.')` 去掉的首尾 `.` 重新拼回 `masked` 两侧。
fn reattach_dots(run: &str, core: &str, masked: &str) -> String {
    let lead = &run[..run.len() - run.trim_start_matches('.').len()];
    let trail = &run[run.len() - (run.len() - run.trim_end_matches('.').len())..];
    debug_assert!(run.contains(core));
    format!("{}{}{}", lead, masked, trail)
}

// ============================= Logger sink =============================
//
// 滚动文件日志 sink：在既有 `emit("hmx-log")` 前端日志面板之外，额外把脱敏后的
// 关键事件与错误落地到本地文件。所有文件操作均使用 `Result`/`Option` 静默降级——
// 写盘失败不 panic、不阻断主流程，仅退化为「仅前端输出」（Req 8.5/8.6）。

/// 活动日志文件名（滚动时归档为带时间戳的 `hmx-*.log`）。
const ACTIVE_LOG_NAME: &str = "hmx.log";

/// 本地滚动日志 sink。
///
/// - `dir`：日志目录（通常为 Tauri `app_log_dir`）。
/// - `file`：当前活动文件句柄（追加写）；`None` 表示尚未成功打开（降级为不落地）。
/// - `max_bytes`：单文件大小上限，达上限即滚动（建议 2MB）。
/// - `max_files`：保留的历史归档文件数上限（建议 5）。
/// - `current_bytes`：当前活动文件的近似字节数，用于触发滚动。
pub(crate) struct Logger {
    dir: PathBuf,
    file: Mutex<Option<File>>,
    max_bytes: u64,
    max_files: usize,
    current_bytes: AtomicU64,
}

impl Logger {
    /// 创建 sink 并尝试打开活动日志文件（失败静默降级）。
    pub(crate) fn new(dir: PathBuf, max_bytes: u64, max_files: usize) -> Self {
        let logger = Logger {
            dir,
            file: Mutex::new(None),
            max_bytes: max_bytes.max(1),
            max_files,
            current_bytes: AtomicU64::new(0),
        };
        logger.ensure_open();
        logger
    }

    /// 写入一条日志记录：先脱敏再组装为单行文本落盘，达上限触发滚动。
    ///
    /// 任何文件操作失败都会被静默吞掉，绝不 panic、不阻断调用方。
    pub(crate) fn write(&self, level: LogLevel, msg: &str) {
        let line = format_log_line(&now_timestamp(), level, &redact(msg));
        self.ensure_open();
        let written = {
            let mut guard = self.lock_file();
            let Some(file) = guard.as_mut() else {
                return; // 文件未打开：降级为不落地
            };
            let mut data = line;
            data.push('\n');
            if file.write_all(data.as_bytes()).is_err() {
                return;
            }
            let _ = file.flush();
            data.len() as u64
        };
        let total = self.current_bytes.fetch_add(written, Ordering::Relaxed) + written;
        if total >= self.max_bytes {
            self.rotate_if_needed();
        }
    }

    /// 达到大小上限则滚动：归档当前文件、按 [`files_to_prune`] 裁剪历史、重开活动文件。
    fn rotate_if_needed(&self) {
        if self.current_bytes.load(Ordering::Relaxed) < self.max_bytes {
            return;
        }
        let mut guard = self.lock_file();
        // 双检：加锁后可能已有其它线程完成滚动
        if self.current_bytes.load(Ordering::Relaxed) < self.max_bytes {
            return;
        }
        // 关闭当前句柄后再重命名，避免占用导致重命名失败
        *guard = None;
        let active = self.dir.join(ACTIVE_LOG_NAME);
        let archived = self.dir.join(rotated_name());
        let renamed = fs::rename(&active, &archived).is_ok();
        self.current_bytes.store(0, Ordering::Relaxed);
        drop(guard);
        if renamed {
            self.prune_archives();
        }
        // 无论重命名成功与否都重开活动文件（失败则维持降级）
        self.ensure_open();
    }

    /// 列出目录内历史归档并裁剪至 `max_files` 个（删除较旧者）。
    fn prune_archives(&self) {
        let entries = match fs::read_dir(&self.dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        let mut names: Vec<String> = Vec::new();
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if name.starts_with("hmx-") && name.ends_with(".log") {
                    names.push(name.to_string());
                }
            }
        }
        for name in files_to_prune(&names, self.max_files) {
            let _ = fs::remove_file(self.dir.join(name));
        }
    }

    /// 确保活动文件已打开（幂等）。创建目录 / 打开文件失败则保持 `None`（降级）。
    fn ensure_open(&self) {
        let mut guard = self.lock_file();
        if guard.is_some() {
            return;
        }
        if fs::create_dir_all(&self.dir).is_err() {
            return;
        }
        let path = self.dir.join(ACTIVE_LOG_NAME);
        if let Ok(file) = OpenOptions::new().create(true).append(true).open(&path) {
            let size = file.metadata().map(|m| m.len()).unwrap_or(0);
            self.current_bytes.store(size, Ordering::Relaxed);
            *guard = Some(file);
        }
    }

    /// 获取文件锁，中毒时降级取回内部值（日志不应因锁中毒而中断主流程）。
    fn lock_file(&self) -> std::sync::MutexGuard<'_, Option<File>> {
        match self.file.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

/// 当前时刻的可读时间戳 `YYYY-MM-DD HH:MM:SS`（UTC）。
fn now_timestamp() -> String {
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let (y, mo, d, h, mi, s) = civil_from_unix(dur.as_secs());
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        y, mo, d, h, mi, s
    )
}

/// 归档文件名：`hmx-YYYYMMDD-HHMMSS-<nanos>.log`（定宽，字典序即时间序）。
fn rotated_name() -> String {
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let (y, mo, d, h, mi, s) = civil_from_unix(dur.as_secs());
    format!(
        "hmx-{:04}{:02}{:02}-{:02}{:02}{:02}-{:09}.log",
        y,
        mo,
        d,
        h,
        mi,
        s,
        dur.subsec_nanos()
    )
}

/// 将 Unix 秒转换为 (年, 月, 日, 时, 分, 秒)（UTC），基于 Howard Hinnant 的
/// `civil_from_days` 算法，无第三方依赖。
fn civil_from_unix(secs: u64) -> (i64, u32, u32, u32, u32, u32) {
    let days = (secs / 86_400) as i64;
    let rem = (secs % 86_400) as u32;
    let hour = rem / 3_600;
    let min = (rem % 3_600) / 60;
    let sec = rem % 60;

    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day, hour, min, sec)
}

// ============================= 属性测试 =============================

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    // Feature: network-capability-expansion, Property 19
    //
    // Property 19: 日志行格式包含时间戳与级别（format_log_line）
    //
    // 对任意时间戳字符串、任意消息字符串与任意日志级别，
    // `format_log_line` 的输出必须同时包含：
    //   - 传入的时间戳子串
    //   - 该级别的标签（INFO / WARN / ERROR）子串
    //   - 传入的消息子串
    // format_log_line 仅做字符串插值，故上述子串包含关系恒成立
    // （即便输入含特殊字符）。
    // Validates: Requirements 8.1
    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, ..ProptestConfig::default() })]

        #[test]
        fn prop_format_log_line_contains_ts_level_and_msg(
            ts in ".*",
            msg in ".*",
            level_idx in 0usize..3,
        ) {
            let level = match level_idx {
                0 => LogLevel::Info,
                1 => LogLevel::Warn,
                _ => LogLevel::Error,
            };

            let line = format_log_line(&ts, level, &msg);

            prop_assert!(
                line.contains(&ts),
                "输出未包含时间戳子串: line={:?}, ts={:?}",
                line,
                ts
            );
            prop_assert!(
                line.contains(level.label()),
                "输出未包含级别标签子串: line={:?}, label={:?}",
                line,
                level.label()
            );
            prop_assert!(
                line.contains(&msg),
                "输出未包含消息子串: line={:?}, msg={:?}",
                line,
                msg
            );
        }
    }

    // Feature: network-capability-expansion, Property 20
    //
    // Property 20: 日志滚动裁剪保留上限（files_to_prune）
    //
    // 对任意现有日志文件名集合与保留上限 `max_files`：
    //   1. 裁剪后保留的文件数 = min(existing.len(), max_files)，恒不超过 max_files。
    //   2. 被删除的均为较旧文件——按字典序升序排序后，pruned 恰为最前的
    //      (len - max_files) 个（最旧者），保留下来的即字典序最大的 max_files 个。
    //   3. 当 existing.len() <= max_files 时不删除任何文件（返回空）。
    // Validates: Requirements 8.2
    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, ..ProptestConfig::default() })]

        #[test]
        fn prop_files_to_prune_retains_upper_bound_and_prunes_oldest(
            existing in prop::collection::vec("[a-z0-9._-]{1,12}", 0..30),
            max_files in 0usize..20,
        ) {
            let prune = files_to_prune(&existing, max_files);

            // 期望：升序排序后，最旧的 (len - max_files) 个被删除
            let mut sorted = existing.clone();
            sorted.sort();
            let expected_prune: Vec<String> = if existing.len() > max_files {
                sorted[..existing.len() - max_files].to_vec()
            } else {
                Vec::new()
            };

            // 属性 2：删除集合精确等于最旧的若干项
            prop_assert_eq!(
                &prune,
                &expected_prune,
                "裁剪结果与「删除最旧项」不一致: existing={:?}, max_files={}",
                existing,
                max_files
            );

            // 属性 1：保留数量 = min(len, max_files)，不超过上限
            let kept = existing.len() - prune.len();
            prop_assert_eq!(
                kept,
                existing.len().min(max_files),
                "保留数量应为 min(len, max_files): existing={:?}, max_files={}",
                existing,
                max_files
            );
            prop_assert!(
                kept <= max_files,
                "保留数量超过上限: kept={}, max_files={}",
                kept,
                max_files
            );

            // 属性 3：文件数不超过上限时不删除
            if existing.len() <= max_files {
                prop_assert!(
                    prune.is_empty(),
                    "文件数未超上限却发生删除: existing={:?}, max_files={}",
                    existing,
                    max_files
                );
            }
        }
    }

    // Feature: network-capability-expansion, Property 21
    //
    // Property 21: 敏感信息脱敏（redact）
    //
    // 对任意嵌入了本机可标识信息的日志消息，`redact` 的输出必须：
    //   1. IPv4：保留前两段、后两段掩码为 `a.b.*.*`，且不再出现完整地址 `a.b.c.d`。
    //   2. IPv6：保留前两段、其余掩码为 `s0:s1::*`，且不再出现完整地址字符串。
    //   3. 用户名路径：`C:\Users\<name>\...` 的用户名段被替换为 `<USER>`，
    //      且不再出现原始 `Users\<name>` 段。
    // 为避免因值巧合导致的伪失败，生成器约束在无歧义的输入空间：
    //   - IPv4 八位组取十进制 0..=255；断言掩码令牌 `a.b.*.*` 必然出现。
    //   - IPv6 各段取非零 1..=0xffff（Display 不触发 `::` 压缩），使完整地址串明确、
    //     且不会与掩码结果 `s0:s1::*` 意外重叠。
    //   - 用户名取字母数字 `[A-Za-z0-9]{1,10}`，脱敏在 IP 掩码之前先行替换，互不干扰。
    // Validates: Requirements 8.3
    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, ..ProptestConfig::default() })]

        #[test]
        fn prop_redact_masks_local_ip_and_username(
            a in any::<u8>(),
            b in any::<u8>(),
            c in any::<u8>(),
            d in any::<u8>(),
            seg in prop::array::uniform8(1u16..=0xffff),
            name in "[A-Za-z0-9]{1,10}",
        ) {
            // ---- 子属性 1：IPv4 后两段掩码 ----
            let ipv4_msg = format!("ip={}.{}.{}.{}", a, b, c, d);
            let ipv4_out = redact(&ipv4_msg);
            let ipv4_masked = format!("{}.{}.*.*", a, b);
            let ipv4_full = format!("{}.{}.{}.{}", a, b, c, d);
            prop_assert!(
                ipv4_out.contains(&ipv4_masked),
                "IPv4 掩码令牌缺失: out={:?}, masked={:?}",
                ipv4_out,
                ipv4_masked
            );
            prop_assert!(
                !ipv4_out.contains(&ipv4_full),
                "IPv4 完整地址未被脱敏: out={:?}, full={:?}",
                ipv4_out,
                ipv4_full
            );

            // ---- 子属性 2：IPv6 前缀外掩码 ----
            let ip6 = std::net::Ipv6Addr::new(
                seg[0], seg[1], seg[2], seg[3], seg[4], seg[5], seg[6], seg[7],
            );
            let ipv6_full = ip6.to_string();
            let ipv6_msg = format!("addr={}", ipv6_full);
            let ipv6_out = redact(&ipv6_msg);
            let ipv6_masked = format!("{:x}:{:x}::*", seg[0], seg[1]);
            prop_assert!(
                ipv6_out.contains(&ipv6_masked),
                "IPv6 掩码令牌缺失: out={:?}, masked={:?}",
                ipv6_out,
                ipv6_masked
            );
            prop_assert!(
                !ipv6_out.contains(&ipv6_full),
                "IPv6 完整地址未被脱敏: out={:?}, full={:?}",
                ipv6_out,
                ipv6_full
            );

            // ---- 子属性 3：用户名路径段替换 ----
            let path_msg = format!("path=C:\\Users\\{}\\AppData", name);
            let path_out = redact(&path_msg);
            prop_assert!(
                path_out.contains("<USER>"),
                "用户名段未替换为 <USER>: out={:?}",
                path_out
            );
            let original_segment = format!("Users\\{}", name);
            prop_assert!(
                !path_out.contains(&original_segment),
                "原始用户名段仍存在: out={:?}, segment={:?}",
                path_out,
                original_segment
            );
        }
    }
}
