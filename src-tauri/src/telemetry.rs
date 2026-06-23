//! 实时流量遥测模块
//!
//! 通过 Win32 IPHLPAPI 的 `GetIfEntry2` 按接口索引（IfIndex）直接读取网卡内核
//! 计数器的累计收发字节（InOctets / OutOctets），逐秒求差得到真实下行/上行
//! 速率。对应原 Python 项目用 `psutil.net_io_counters(pernic=True)` 的采样，
//! 但精度更高、零第三方依赖、可直接按接口索引匹配。

#[cfg(windows)]
use windows_sys::Win32::NetworkManagement::IpHelper::{GetIfEntry2, MIB_IF_ROW2};

/// 读取指定接口索引的累计 (收字节, 发字节)。失败返回 (0, 0)。
#[cfg(windows)]
pub fn read_octets(if_index: u32) -> (u64, u64) {
    unsafe {
        // MIB_IF_ROW2 较大，需先清零并设置 InterfaceIndex
        let mut row: MIB_IF_ROW2 = std::mem::zeroed();
        row.InterfaceIndex = if_index;
        if GetIfEntry2(&mut row) == 0 {
            (row.InOctets, row.OutOctets)
        } else {
            (0, 0)
        }
    }
}

#[cfg(not(windows))]
pub fn read_octets(_if_index: u32) -> (u64, u64) {
    (0, 0)
}
