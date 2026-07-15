//! 系统代理防泄漏看门狗（Proxy_Guardian，Req 5）。
//!
//! 既有 `sysproxy.rs` 的接管前快照仅存于进程内存（`SAVED_PROXY`），主进程被强杀 /
//! 崩溃即丢失，无法在下次启动时补偿还原，可能导致用户「断网」。本模块新增
//! **快照落盘**：接管前把系统代理原始值（`ProxyEnable` / `ProxyServer`）写入
//! `app_config_dir` 下的守护文件；正常还原后删除该文件；下次启动若检测到残留守护
//! 文件即据此补偿还原（Req 5.1/5.2/5.3）。
//!
//! 设计约束：
//! - 注册表读写沿用 `sysproxy.rs` 既有的 WinINet 注册表访问方式（复用而非重造）。
//! - 还原失败按最大次数重试并记可读日志（Req 5.5）。
//! - 守护快照损坏时安全跳过、不阻断启动（Req 5.3）。
//! - 正常设置 / 清除路径行为与既有等价（Req 5.6）；仅操作 `127.0.0.1` 端点（Req 5.7）。

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

/// 守护快照文件名（位于 `app_config_dir`）。
const SNAPSHOT_FILE: &str = "proxy-guardian.json";
/// 还原失败的最大重试次数（Req 5.5）。
const MAX_RESTORE_RETRIES: u32 = 3;

/// HKCU 登录自启注册表路径。
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
/// 登录自愈自启项的值名（唯一标识，卸载时据此删除）。
const RUN_VALUE_NAME: &str = "HypoMuxPlusProxyHeal";
/// Tauri 应用标识符（用于定位 `%APPDATA%` 下的应用配置目录）。
const IDENTIFIER: &str = "com.qingyun.hypomuxplus";

/// 守护目录（在 `setup` 阶段以 `app_config_dir` 初始化一次）。
///
/// `sysproxy.rs` 的 `enable_system_proxy` / `disable_system_proxy` 无法拿到应用目录，
/// 故经此全局在其内部委托到带 `dir` 参数的落盘 / 还原实现。
static GUARDIAN_DIR: OnceLock<PathBuf> = OnceLock::new();

/// 落盘的系统代理原始快照（json）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ProxySnapshot {
    /// 接管前的 `ProxyEnable` 原始值。
    pub enable: u32,
    /// 接管前的 `ProxyServer` 原始值。
    pub server: String,
}

/// 初始化守护目录（在 `setup` 阶段调用一次）。
pub(crate) fn init_dir(dir: PathBuf) {
    let _ = GUARDIAN_DIR.set(dir);
}

/// 守护快照文件的完整路径。
fn snapshot_path(dir: &Path) -> PathBuf {
    dir.join(SNAPSHOT_FILE)
}

/// 接管前捕获系统代理原始快照并落盘（Req 5.1）。
///
/// - 若守护快照已存在，则「首次接管为准」不覆盖（避免二次接管把本程序自身代理写入快照）。
/// - 若既有代理疑似本程序写入（`looks_like_ours`），不作为原始快照落盘（沿用兜底语义）。
pub(crate) fn capture_and_persist(dir: &Path) -> std::io::Result<()> {
    let path = snapshot_path(dir);
    if path.exists() {
        return Ok(());
    }
    let (enable, server) = crate::sysproxy::read_proxy_raw();
    if crate::sysproxy::looks_like_ours(&server) {
        return Ok(());
    }
    let snap = ProxySnapshot { enable, server };
    let json = serde_json::to_string(&snap)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    std::fs::create_dir_all(dir)?;
    std::fs::write(&path, json)?;
    // 落盘快照即代表已接管系统代理：注册登录自愈自启项，供关机 / 强杀后下次登录静默还原。
    register_logon_heal();
    Ok(())
}

/// 正常停止 / 退出时据落盘快照还原系统代理并删除守护文件（Req 5.2）。
///
/// 无守护文件或文件损坏时不还原，仅确保守护文件被清理（幂等）。
pub(crate) fn restore_and_clear(dir: &Path) {
    let path = snapshot_path(dir);
    if let Some(snap) = read_snapshot(&path) {
        restore_with_retry(&snap);
    }
    let _ = std::fs::remove_file(&path);
    // 已正常还原并清理快照：移除登录自愈自启项，避免残留。
    unregister_logon_heal();
}

/// 启动补偿：检测到上一次未被还原的残留守护快照则据其还原（Req 5.3）。
///
/// 快照文件损坏时安全跳过（清理坏文件、不阻断启动）。
pub(crate) fn recover_on_startup(dir: &Path) {
    let path = snapshot_path(dir);
    if !path.exists() {
        return;
    }
    match read_snapshot(&path) {
        Some(snap) => {
            restore_with_retry(&snap);
            let _ = std::fs::remove_file(&path);
        }
        // 快照损坏：安全跳过，清理坏文件以免每次启动重复尝试
        None => {
            let _ = std::fs::remove_file(&path);
        }
    }
    // 残留快照已被补偿处理（还原或清理）：移除登录自愈自启项。
    unregister_logon_heal();
}

/// 死网关判定（纯函数，Req 5.4）：当且仅当系统代理已启用且其指向的本地端口不再监听。
pub(crate) fn is_dead_gateway(proxy_enabled: bool, port_listening: bool) -> bool {
    proxy_enabled && !port_listening
}

/// 读取并反序列化守护快照；不存在或损坏返回 `None`（安全跳过）。
fn read_snapshot(path: &Path) -> Option<ProxySnapshot> {
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str::<ProxySnapshot>(&raw).ok()
}

/// 据快照还原系统代理，失败按最大次数重试并记可读日志（Req 5.5）。
fn restore_with_retry(snap: &ProxySnapshot) {
    let mut attempt: u32 = 0;
    loop {
        match crate::sysproxy::restore_proxy_raw(snap.enable, &snap.server) {
            Ok(()) => return,
            Err(e) => {
                attempt += 1;
                eprintln!("[Proxy_Guardian] 还原系统代理失败(第 {attempt} 次): {e}");
                if attempt >= MAX_RESTORE_RETRIES {
                    eprintln!(
                        "[Proxy_Guardian] 已达最大重试次数({MAX_RESTORE_RETRIES})，放弃还原"
                    );
                    return;
                }
            }
        }
    }
}

// ── 全局目录委托变体：供无法拿到应用目录的 `sysproxy.rs` 内部调用 ──────────────

/// 使用全局守护目录捕获并落盘快照（`sysproxy::enable_system_proxy` 内部调用）。
pub(crate) fn capture_and_persist_default() {
    if let Some(dir) = GUARDIAN_DIR.get() {
        let _ = capture_and_persist(dir);
    }
}

/// 使用全局守护目录还原并清理快照（`sysproxy::disable_system_proxy` 内部调用）。
pub(crate) fn restore_and_clear_default() {
    if let Some(dir) = GUARDIAN_DIR.get() {
        restore_and_clear(dir);
    }
}

// ── 登录自愈自启项：绑定到守护快照生命周期 ──────────────────────────────────────
//
// 场景（Req 5 延伸）：用户接管系统代理后直接关机 / 强制断电，主进程来不及执行
// 正常还原路径，系统代理指针残留在注册表中。虽然本程序下次「启动」时会经
// `recover_on_startup` 补偿还原，但用户可能不会立刻再打开本程序，导致这段时间内
// 系统浏览器等因指向已失效的本地端口而「断网」。
//
// 为此，在**接管系统代理（落盘快照）时**注册一个 HKCU Run 登录自启项，指向
// `HypoMuxPlus.exe --heal-proxy`；下次用户登录 Windows 时该轻量实例会静默执行
// 补偿还原并自删此自启项后退出，无需用户手动打开主程序即可自愈。正常停止 / 退出
// （删除快照）时同步移除该自启项，避免残留。

/// `%APPDATA%\<identifier>` 应用配置目录（供 `--heal-proxy` 轻量实例定位守护快照）。
///
/// 该轻量实例不拉起 Tauri，无法通过 `app.path().app_config_dir()` 获取目录，
/// 故直接据 `APPDATA` 环境变量与应用标识符拼出，与 Tauri 的默认取值一致。
pub(crate) fn default_config_dir() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    if appdata.is_empty() {
        return None;
    }
    Some(PathBuf::from(appdata).join(IDENTIFIER))
}

/// 注册登录自愈自启项：写 HKCU Run 值 = `"<当前 exe 绝对路径>" --heal-proxy`。
///
/// 失败仅记日志、不阻断主流程（自愈是尽力而为的补偿机制）。
fn register_logon_heal() {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[Proxy_Guardian] 注册登录自愈项失败(无法获取 exe 路径): {e}");
            return;
        }
    };
    let cmd = format!("\"{}\" --heal-proxy", exe.display());
    let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    match hkcu.create_subkey_with_flags(RUN_KEY, winreg::enums::KEY_WRITE | winreg::enums::KEY_READ)
    {
        Ok((key, _)) => {
            if let Err(e) = key.set_value(RUN_VALUE_NAME, &cmd) {
                eprintln!("[Proxy_Guardian] 写入登录自愈项失败: {e}");
            }
        }
        Err(e) => eprintln!("[Proxy_Guardian] 打开 Run 注册表键失败: {e}"),
    }
}

/// 移除登录自愈自启项（幂等：不存在时忽略错误）。
pub(crate) fn unregister_logon_heal() {
    let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    if let Ok(key) = hkcu.open_subkey_with_flags(RUN_KEY, winreg::enums::KEY_WRITE) {
        let _ = key.delete_value(RUN_VALUE_NAME);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, ..ProptestConfig::default() })]

        // Feature: pro-differentiation-and-hardening, Property 7
        // 死网关判定（is_dead_gateway）：对任意布尔组合 (proxy_enabled, port_listening)，
        // 判定为死网关当且仅当系统代理仍启用且本地端口不再监听，即等价于 (e && !l)；
        // 且对任意输入不 panic。
        // Validates: Requirements 5.4
        #[test]
        fn prop_is_dead_gateway_iff_enabled_and_not_listening(
            proxy_enabled in any::<bool>(),
            port_listening in any::<bool>(),
        ) {
            prop_assert_eq!(
                is_dead_gateway(proxy_enabled, port_listening),
                proxy_enabled && !port_listening
            );
        }
    }

    #[test]
    fn is_dead_gateway_true_only_when_enabled_and_not_listening() {
        assert!(is_dead_gateway(true, false));
        assert!(!is_dead_gateway(true, true));
        assert!(!is_dead_gateway(false, false));
        assert!(!is_dead_gateway(false, true));
    }

    #[test]
    fn snapshot_roundtrips_through_json() {
        let snap = ProxySnapshot { enable: 1, server: "socks=127.0.0.1:1080".to_string() };
        let json = serde_json::to_string(&snap).unwrap();
        let back: ProxySnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(snap, back);
    }

    #[test]
    fn read_snapshot_returns_none_on_corrupt_file() {
        let dir = std::env::temp_dir().join(format!("hmx-pg-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = snapshot_path(&dir);
        std::fs::write(&path, b"{ not valid json").unwrap();
        assert!(read_snapshot(&path).is_none());
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }
}
