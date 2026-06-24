//! 网卡发现模块
//!
//! 通过 Win32 IPHLPAPI 的 `GetAdaptersAddresses` 直接枚举所有处于 Up 状态、
//! 拥有 IPv4 地址的物理/虚拟网卡，拿到权威的接口索引（IfIndex）、友好名称
//! （FriendlyName）与单播 IPv4 地址。
//!
//! 这是修复 WinError 10049（同网段多网卡 bind 本地 IP 随机命中错误网卡）的
//! 物理地基：拿到 IfIndex 后，代理引擎用 `IP_UNICAST_IF` 把出站 socket 死锁
//! 在指定网卡上，彻底绕过 Windows 默认路由查找。
//!
//! 对应原 Python 项目 `utils/network_utils.py` 中的 `get_adapter_if_indices`
//! 与 `scan_network_adapters` 的功能合并。

use serde::Serialize;
use std::net::Ipv4Addr;

#[cfg(windows)]
use windows_sys::Win32::Foundation::{ERROR_BUFFER_OVERFLOW, ERROR_SUCCESS};
#[cfg(windows)]
use windows_sys::Win32::NetworkManagement::IpHelper::{
    GetAdaptersAddresses, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER,
    GAA_FLAG_SKIP_MULTICAST, IP_ADAPTER_ADDRESSES_LH,
};
#[cfg(windows)]
use windows_sys::Win32::Networking::WinSock::{AF_INET, SOCKADDR_IN};

/// 接口运行状态：Up
#[cfg(windows)]
const IF_OPER_STATUS_UP: i32 = 1;

/// 暴露给前端的网卡信息。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdapterInfo {
    /// 接口索引（IfIndex），物理层绑定的权威标识
    pub index: u32,
    /// 网卡友好名称（与流量计数器键一致），如「以太网」「WLAN」
    pub alias: String,
    /// 第一个有效的 IPv4 地址
    pub ipv4: String,
    /// 网卡描述（厂商/适配器型号）
    pub description: String,
    /// 是否处于活动（Up）状态
    pub is_up: bool,
}

#[cfg(windows)]
fn pwstr_to_string(ptr: *const u16) -> String {
    if ptr.is_null() {
        return String::new();
    }
    let mut len = 0usize;
    // 安全：以 NUL 结尾的宽字符串
    unsafe {
        while *ptr.add(len) != 0 {
            len += 1;
        }
        let slice = std::slice::from_raw_parts(ptr, len);
        String::from_utf16_lossy(slice)
    }
}

/// 枚举所有 Up 且拥有 IPv4 的网卡。
#[cfg(windows)]
pub fn scan_adapters() -> Result<Vec<AdapterInfo>, String> {
    let flags = (GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER) as u32;

    // 初始 16KB，按官方建议预分配；不足时按返回的 size 重试
    let mut size: u32 = 16 * 1024;
    let mut buffer: Vec<u8> = Vec::new();
    let mut ret: u32 = ERROR_BUFFER_OVERFLOW;

    for _ in 0..4 {
        buffer.resize(size as usize, 0);
        ret = unsafe {
            GetAdaptersAddresses(
                AF_INET as u32,
                flags,
                std::ptr::null_mut(),
                buffer.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH,
                &mut size,
            )
        };
        if ret == ERROR_BUFFER_OVERFLOW {
            continue;
        }
        break;
    }

    if ret != ERROR_SUCCESS {
        return Err(format!("GetAdaptersAddresses 返回错误码 {ret}"));
    }

    let mut adapters: Vec<AdapterInfo> = Vec::new();
    let mut cursor = buffer.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;

    unsafe {
        while !cursor.is_null() {
            let adapter = &*cursor;
            let if_index = adapter.Anonymous1.Anonymous.IfIndex;
            let is_up = adapter.OperStatus == IF_OPER_STATUS_UP;

            if is_up && if_index != 0 {
                // 取第一个 IPv4 单播地址
                let mut ipv4: Option<Ipv4Addr> = None;
                let mut uni = adapter.FirstUnicastAddress;
                while !uni.is_null() {
                    let u = &*uni;
                    let sa = u.Address.lpSockaddr;
                    if !sa.is_null() && (*sa).sa_family == AF_INET {
                        let sin = sa as *const SOCKADDR_IN;
                        let raw = (*sin).sin_addr.S_un.S_addr; // 网络字节序
                        let b = raw.to_ne_bytes();
                        ipv4 = Some(Ipv4Addr::new(b[0], b[1], b[2], b[3]));
                        break;
                    }
                    uni = u.Next;
                }

                if let Some(ip) = ipv4 {
                    // 过滤无出口能力的地址：环回 127.0.0.0/8、链路本地 169.254.0.0/16（APIPA）
                    let o = ip.octets();
                    let is_loopback = o[0] == 127;
                    let is_link_local = o[0] == 169 && o[1] == 254;
                    if !is_loopback && !is_link_local {
                        adapters.push(AdapterInfo {
                            index: if_index,
                            alias: pwstr_to_string(adapter.FriendlyName),
                            ipv4: ip.to_string(),
                            description: pwstr_to_string(adapter.Description),
                            is_up,
                        });
                    }
                }
            }

            cursor = adapter.Next;
        }
    }

    Ok(adapters)
}

#[cfg(not(windows))]
pub fn scan_adapters() -> Result<Vec<AdapterInfo>, String> {
    Err("HypoMuxPlus 仅支持 Windows 平台".into())
}
