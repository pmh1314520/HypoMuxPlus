//! 应用兼容性快捷配置模块
//!
//! 对应原 Python 项目 `utils/app_configurator.py`：为常见客户端一键写入 / 还原
//! SOCKS5 代理配置，覆盖系统全局代理无法触达的应用。
//!
//! - Steam：写入 `HKCU\Software\Valve\Steam` 的 Socks5Proxy / Socks5ProxyPort
//! - IDM  ：写入 `HKCU\Software\DownloadManager` 的 ProxyEnable/Type/Host/Port

use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
use winreg::RegKey;

/// 配置或还原 Steam 的 SOCKS5 代理。
pub fn configure_steam(enable: bool, host: &str, port: u16) -> Result<(), String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key = hkcu
        .open_subkey_with_flags(r"Software\Valve\Steam", KEY_WRITE | KEY_READ)
        .map_err(|_| "未找到 Steam 注册表项（可能未安装 Steam）".to_string())?;

    if enable {
        key.set_value("Socks5Proxy", &format!("{host}:{port}"))
            .map_err(|e| format!("写入 Steam 代理失败: {e}"))?;
        key.set_value("Socks5ProxyPort", &(port as u32))
            .map_err(|e| format!("写入 Steam 代理端口失败: {e}"))?;
    } else {
        let _ = key.delete_value("Socks5Proxy");
        let _ = key.delete_value("Socks5ProxyPort");
    }
    Ok(())
}

/// 配置或还原 IDM 的 SOCKS5 代理。
pub fn configure_idm(enable: bool, host: &str, port: u16) -> Result<(), String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key = hkcu
        .open_subkey_with_flags(r"Software\DownloadManager", KEY_WRITE | KEY_READ)
        .map_err(|_| "未找到 IDM 注册表项（可能未安装 IDM）".to_string())?;

    if enable {
        key.set_value("ProxyEnable", &1u32)
            .map_err(|e| format!("写入 IDM 代理开关失败: {e}"))?;
        key.set_value("ProxyType", &5u32)
            .map_err(|e| format!("写入 IDM 代理类型失败: {e}"))?;
        key.set_value("ProxyHost", &host.to_string())
            .map_err(|e| format!("写入 IDM 代理主机失败: {e}"))?;
        key.set_value("ProxyPort", &(port as u32))
            .map_err(|e| format!("写入 IDM 代理端口失败: {e}"))?;
    } else {
        let _ = key.set_value("ProxyEnable", &0u32);
    }
    Ok(())
}
