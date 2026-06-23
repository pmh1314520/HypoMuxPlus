//! Windows 系统代理控制模块
//!
//! 对应原 Python 项目的 `system_proxy.py` 与 `main_window.set_system_proxy`：
//! 通过修改 WinINet 注册表（HKCU\...\Internet Settings）接管/还原全局代理，
//! 写入 `http=...;https=...;socks=...` 全覆盖链条，并调用 InternetSetOptionW
//! 动态刷新，无需重启浏览器即可生效。
//!
//! 同时提供死网关检测（Dead Gateway Detection）开关：多网卡并发下载时，慢速
//! 链路被瞬间塞爆会触发系统死网关检测而将其判定为失效；加速期间关闭该机制
//! 可维持多网卡并发的稳定性，退出时还原系统默认。

use std::os::windows::process::CommandExt;
use std::process::Command;
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
use winreg::RegKey;

#[cfg(windows)]
use windows_sys::Win32::Networking::WinInet::{
    InternetSetOptionW, INTERNET_OPTION_REFRESH, INTERNET_OPTION_SETTINGS_CHANGED,
};

const INTERNET_SETTINGS: &str = r"Software\Microsoft\Windows\CurrentVersion\Internet Settings";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 启用系统代理：写入 HTTP/HTTPS/SOCKS 全覆盖代理链条。
pub fn enable_system_proxy(socks_addr: &str, http_addr: &str) -> Result<(), String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu
        .create_subkey_with_flags(INTERNET_SETTINGS, KEY_WRITE | KEY_READ)
        .map_err(|e| format!("打开注册表失败: {e}"))?;

    key.set_value("ProxyEnable", &1u32)
        .map_err(|e| format!("写入 ProxyEnable 失败: {e}"))?;

    let proxy_value = format!("http={http_addr};https={http_addr};socks={socks_addr}");
    key.set_value("ProxyServer", &proxy_value)
        .map_err(|e| format!("写入 ProxyServer 失败: {e}"))?;

    refresh_proxy();
    Ok(())
}

/// 禁用系统代理（强制还原，防止代理残留导致断网）。
pub fn disable_system_proxy() -> Result<(), String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu
        .create_subkey_with_flags(INTERNET_SETTINGS, KEY_WRITE | KEY_READ)
        .map_err(|e| format!("打开注册表失败: {e}"))?;

    key.set_value("ProxyEnable", &0u32)
        .map_err(|e| format!("写入 ProxyEnable 失败: {e}"))?;
    let _ = key.set_value("ProxyServer", &"");

    refresh_proxy();
    Ok(())
}

/// 读取当前系统代理状态：(是否启用, 代理串)。
pub fn get_system_proxy() -> (bool, String) {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok(key) = hkcu.open_subkey_with_flags(INTERNET_SETTINGS, KEY_READ) {
        let enabled: u32 = key.get_value("ProxyEnable").unwrap_or(0);
        let server: String = key.get_value("ProxyServer").unwrap_or_default();
        return (enabled == 1, server);
    }
    (false, String::new())
}

/// 通知 Windows 动态刷新代理设置，立即生效。
fn refresh_proxy() {
    #[cfg(windows)]
    unsafe {
        InternetSetOptionW(
            std::ptr::null_mut(),
            INTERNET_OPTION_SETTINGS_CHANGED,
            std::ptr::null_mut(),
            0,
        );
        InternetSetOptionW(
            std::ptr::null_mut(),
            INTERNET_OPTION_REFRESH,
            std::ptr::null_mut(),
            0,
        );
    }
}

/// 开关死网关检测。`enabled=false` 关闭（加速期间），`true` 恢复系统默认。
pub fn set_dead_gateway_detection(enabled: bool) -> Result<(), String> {
    let state = if enabled { "enabled" } else { "disabled" };
    let output = Command::new("netsh")
        .args([
            "interface",
            "ipv4",
            "set",
            "global",
            &format!("deadgatewaydetection={state}"),
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("执行 netsh 失败: {e}"))?;

    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).to_string())
    }
}
