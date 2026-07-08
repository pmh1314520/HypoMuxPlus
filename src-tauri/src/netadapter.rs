//! 网卡发现模块
//!
//! 通过 Win32 IPHLPAPI 的 `GetAdaptersAddresses`（`AF_UNSPEC` 双栈枚举）直接枚举
//! 所有处于 Up 状态、拥有 IPv4 地址的物理/虚拟网卡，拿到权威的接口索引（IfIndex）、
//! 友好名称（FriendlyName）、单播 IPv4 地址与代表性全局单播 IPv6 地址。
//!
//! 这是修复 WinError 10049（同网段多网卡 bind 本地 IP 随机命中错误网卡）的
//! 物理地基：拿到 IfIndex 后，代理引擎用 `IP_UNICAST_IF` 把出站 socket 死锁
//! 在指定网卡上，彻底绕过 Windows 默认路由查找。
//!
//! 对应原 Python 项目 `utils/network_utils.py` 中的 `get_adapter_if_indices`
//! 与 `scan_network_adapters` 的功能合并。

use serde::Serialize;
use std::net::{Ipv4Addr, Ipv6Addr};

#[cfg(windows)]
use windows_sys::Win32::Foundation::{ERROR_BUFFER_OVERFLOW, ERROR_SUCCESS};
#[cfg(windows)]
use windows_sys::Win32::NetworkManagement::IpHelper::{
    GetAdaptersAddresses, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER,
    GAA_FLAG_SKIP_MULTICAST, IP_ADAPTER_ADDRESSES_LH,
};
#[cfg(windows)]
use windows_sys::Win32::Networking::WinSock::{AF_INET, AF_INET6, AF_UNSPEC, SOCKADDR_IN, SOCKADDR_IN6};

/// 接口运行状态：Up
#[cfg(windows)]
const IF_OPER_STATUS_UP: i32 = 1;

/// 虚拟 / 隧道 / VPN 网卡关键字（仅用于标记，不再作为扫描期硬过滤条件）。
/// 命中后将 `is_virtual` 置为 true，交由前端过滤器决定是否展示，默认展示全部网卡。
/// 覆盖 Clash/Mihomo TUN、WSL/Hyper-V vEthernet、各类 TAP/VPN、虚拟机网卡等。
#[cfg(windows)]
const VIRTUAL_NIC_KEYWORDS: &[&str] = &[
    "wintun", "tun", "tap-windows", "tap adapter", "tap-win", "openvpn",
    "wireguard", "zerotier", "tailscale", "hyper-v", "vethernet", "virtual",
    "vmware", "virtualbox", "vbox", "docker", "wsl", "loopback", "bluetooth",
    "clash", "mihomo", "meta", "singbox", "sing-box", "radmin", "hamachi",
    "npcap", "pcap", "miniport", "kernel debug", "teredo", "isatap",
];

/// 判断网卡名称 / 描述是否疑似虚拟网卡（大小写不敏感子串匹配）。
#[cfg(windows)]
fn is_virtual_nic(alias: &str, description: &str) -> bool {
    let a = alias.to_ascii_lowercase();
    let d = description.to_ascii_lowercase();
    VIRTUAL_NIC_KEYWORDS
        .iter()
        .any(|kw| a.contains(kw) || d.contains(kw))
}

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
    /// 代表性全局单播 IPv6 地址（优先于 fe80::/10 链路本地）；无 IPv6 时为空字符串
    pub ipv6: String,
    /// 网卡描述（厂商/适配器型号）
    pub description: String,
    /// 是否处于活动（Up）状态
    pub is_up: bool,
    /// 是否疑似虚拟 / 隧道 / VPN / 环回 / 链路本地 / fake-ip 网卡。
    /// 仅作标记，供前端过滤器使用；扫描期不再据此剔除网卡。
    pub is_virtual: bool,
}

/// 判定一个 IPv6 地址是否属于链路本地段 `fe80::/10`。
///
/// `fe80::/10` 的前 10 位为 `1111111010`，即首个 16 位段与掩码 `0xffc0`
/// 相与后等于 `0xfe80`。
pub(crate) fn is_link_local_v6(ip: &Ipv6Addr) -> bool {
    (ip.segments()[0] & 0xffc0) == 0xfe80
}

/// 从一组 IPv6 单播地址中挑选代表性地址：全局单播优先于 `fe80::/10` 链路本地。
///
/// 若集合中存在至少一个非链路本地地址，返回其中第一个（保持枚举顺序）；
/// 若集合为空或全部为链路本地地址，返回 `None`（上层映射为空字符串）。
pub(crate) fn select_global_ipv6(addrs: &[Ipv6Addr]) -> Option<Ipv6Addr> {
    addrs.iter().find(|ip| !is_link_local_v6(ip)).copied()
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
                AF_UNSPEC as u32,
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
                // 遍历单播地址链表：IPv4 取首个（行为不变），IPv6 收集后择代表
                let mut ipv4: Option<Ipv4Addr> = None;
                let mut ipv6_addrs: Vec<Ipv6Addr> = Vec::new();
                let mut uni = adapter.FirstUnicastAddress;
                while !uni.is_null() {
                    let u = &*uni;
                    let sa = u.Address.lpSockaddr;
                    if !sa.is_null() {
                        match (*sa).sa_family {
                            AF_INET => {
                                // 仅记录首个 IPv4（与既有逻辑一致）
                                if ipv4.is_none() {
                                    let sin = sa as *const SOCKADDR_IN;
                                    let raw = (*sin).sin_addr.S_un.S_addr; // 网络字节序
                                    let b = raw.to_ne_bytes();
                                    ipv4 = Some(Ipv4Addr::new(b[0], b[1], b[2], b[3]));
                                }
                            }
                            AF_INET6 => {
                                let sin6 = sa as *const SOCKADDR_IN6;
                                // IN6_ADDR 以网络字节序存放 16 字节，直接构造 Ipv6Addr
                                let bytes = (*sin6).sin6_addr.u.Byte;
                                ipv6_addrs.push(Ipv6Addr::from(bytes));
                            }
                            _ => {}
                        }
                    }
                    uni = u.Next;
                }

                // 保持既有行为：仅当网卡具备有效 IPv4 时才纳入列表
                if let Some(ip) = ipv4 {
                    // 代表性全局 IPv6：全局单播优先于链路本地，无则空字符串
                    let ipv6 = select_global_ipv6(&ipv6_addrs)
                        .map(|a| a.to_string())
                        .unwrap_or_default();

                    // 不再硬过滤任何网卡：环回 127/8、链路本地 169.254/16(APIPA)、
                    // 198.18/15（Clash/Mihomo fake-ip 段）以及虚拟/隧道/VPN 网卡均一并返回，
                    // 仅打上 is_virtual 标记，由前端过滤器决定是否展示（默认展示全部）。
                    let o = ip.octets();
                    let is_loopback = o[0] == 127;
                    let is_link_local = o[0] == 169 && o[1] == 254;
                    let is_fake_ip = o[0] == 198 && (o[1] == 18 || o[1] == 19);
                    let alias = pwstr_to_string(adapter.FriendlyName);
                    let description = pwstr_to_string(adapter.Description);
                    let is_virtual =
                        is_loopback || is_link_local || is_fake_ip || is_virtual_nic(&alias, &description);
                    adapters.push(AdapterInfo {
                        index: if_index,
                        alias,
                        ipv4: ip.to_string(),
                        ipv6,
                        description,
                        is_up,
                        is_virtual,
                    });
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

#[cfg(test)]
mod tests {
    use super::{is_link_local_v6, select_global_ipv6};
    use proptest::prelude::*;
    use std::net::Ipv6Addr;

    proptest! {
        // Feature: network-capability-expansion, Property 5
        // 代表性全局 IPv6 选择（select_global_ipv6）：
        // 存在至少一个全局单播（非链路本地）时返回非链路本地地址；
        // 集合为空或全为链路本地时返回 None。
        #![proptest_config(ProptestConfig { cases: 100, ..ProptestConfig::default() })]

        #[test]
        fn prop_select_global_ipv6(raw in proptest::collection::vec(any::<u128>(), 0..16)) {
            let addrs: Vec<Ipv6Addr> = raw.into_iter().map(Ipv6Addr::from).collect();
            let has_global = addrs.iter().any(|ip| !is_link_local_v6(ip));

            match select_global_ipv6(&addrs) {
                Some(picked) => {
                    // 存在全局单播时才应返回 Some，且返回值必非链路本地
                    prop_assert!(has_global);
                    prop_assert!(!is_link_local_v6(&picked));
                    // 返回值必来自输入集合
                    prop_assert!(addrs.contains(&picked));
                }
                None => {
                    // 返回 None 当且仅当集合为空或全部为链路本地
                    prop_assert!(!has_global);
                }
            }
        }
    }
}
