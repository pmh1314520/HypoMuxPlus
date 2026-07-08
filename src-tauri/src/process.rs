//! Process_Resolver —— 将连接关联到发起进程（PID → 可执行文件名）。
//!
//! 用途：按进程名分流（Req 5）。给定一条连接的本地端点（本地地址 + 本地端口），
//! 经 Win32 `GetExtendedTcpTable(TCP_TABLE_OWNER_PID_ALL)` 反查 owning PID，
//! 再经 `QueryFullProcessImageNameW` 解析可执行文件完整路径并提取小写文件名。
//!
//! 性能约束：`GetExtendedTcpTable` / `OpenProcess` 是相对昂贵的系统调用，
//! 因此仅在**新连接建立时调用一次**，并用短 TTL 缓存把开销降到最低：
//! - `(localAddr, localPort) -> PID`：TTL≈1s（端点短命，端口会复用）
//! - `PID -> name`：TTL≈10s（进程名相对稳定）
//!
//! 健壮性：所有系统调用封装在 `Option` 中，任何失败（查不到、权限不足、
//! 句柄无效等）均返回 `None`，绝不 panic；上层据此回退到域名规则与调度策略。
//!
//! 纯函数 `find_pid_by_endpoint` / `exe_name_from_path` 不触碰系统调用，
//! 便于单元测试与属性测试（见任务 4.7）。

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[cfg(windows)]
use std::net::{Ipv4Addr, Ipv6Addr};

/// `(localAddr, localPort) -> PID` 缓存 TTL（端点短命，约 1 秒）。
const ENDPOINT_TTL: Duration = Duration::from_secs(1);
/// `PID -> name` 缓存 TTL（进程名相对稳定，约 10 秒）。
const NAME_TTL: Duration = Duration::from_secs(10);

/// 系统 TCP 连接表中的一行（纯数据结构）。
///
/// 从 `MIB_TCPTABLE_OWNER_PID` / `MIB_TCP6TABLE_OWNER_PID` 解析而来，
/// 剥离系统调用后便于单元测试端点反查逻辑。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TcpRow {
    /// 本地地址（IPv4 或 IPv6）
    pub local_addr: IpAddr,
    /// 本地端口（主机字节序）
    pub local_port: u16,
    /// 拥有该连接的进程 PID
    pub pid: u32,
}

/// 纯函数：在连接表行集合中按 `(本地地址, 本地端口)` 精确匹配并返回其 PID。
///
/// 存在匹配行时返回该行 PID；不存在匹配时返回 `None`。不触碰任何系统调用。
pub(crate) fn find_pid_by_endpoint(rows: &[TcpRow], local: SocketAddr) -> Option<u32> {
    let ip = local.ip();
    let port = local.port();
    rows.iter()
        .find(|r| r.local_port == port && r.local_addr == ip)
        .map(|r| r.pid)
}

/// 纯函数：从可执行文件完整路径提取小写文件名。
///
/// 取最后一个路径分隔符（`\\` 或 `/`）之后的部分，并转为小写。
/// 例：`"C:\\Program Files\\Steam\\steam.exe"` -> `"steam.exe"`。
/// 无分隔符时返回整串（小写）；空串或以分隔符结尾时返回空串。
pub(crate) fn exe_name_from_path(path: &str) -> String {
    path.rsplit(|c| c == '\\' || c == '/')
        .next()
        .unwrap_or(path)
        .to_ascii_lowercase()
}

/// 进程反查器：以短 TTL 缓存包裹昂贵的系统连接表 / 进程名查询。
///
/// 线程安全（内部 `Mutex`）；`resolve` 仅在新连接建立时调用一次。
pub(crate) struct ProcessResolver {
    /// `(本地地址, 本地端口) -> (PID, 写入时刻)`，TTL≈1s
    endpoint_cache: Mutex<HashMap<(IpAddr, u16), (u32, Instant)>>,
    /// `PID -> (可执行文件名, 写入时刻)`，TTL≈10s
    name_cache: Mutex<HashMap<u32, (String, Instant)>>,
}

impl ProcessResolver {
    /// 构造一个空缓存的进程反查器。
    pub(crate) fn new() -> Self {
        Self {
            endpoint_cache: Mutex::new(HashMap::new()),
            name_cache: Mutex::new(HashMap::new()),
        }
    }

    /// 反查发起连接的进程可执行文件名（小写）。
    ///
    /// 流程：本地端点 -> PID（连接表反查，带缓存） -> 可执行文件名（进程名解析，带缓存）。
    /// 任一环节失败（查不到 / 权限不足 / 句柄无效）均返回 `None`，绝不 panic。
    #[cfg(windows)]
    pub(crate) fn resolve(&self, local: SocketAddr) -> Option<String> {
        let pid = self.resolve_pid(local)?;
        self.resolve_name(pid)
    }

    /// 非 Windows 平台的桩实现：始终返回 `None`，使模块可在任意目标编译/测试。
    #[cfg(not(windows))]
    pub(crate) fn resolve(&self, _local: SocketAddr) -> Option<String> {
        None
    }

    /// 由本地端点反查 owning PID（带 TTL 缓存）。
    #[cfg(windows)]
    fn resolve_pid(&self, local: SocketAddr) -> Option<u32> {
        let key = (local.ip(), local.port());
        let now = Instant::now();

        // 命中未过期缓存则零系统调用
        if let Ok(cache) = self.endpoint_cache.lock() {
            if let Some((pid, ts)) = cache.get(&key) {
                if now.duration_since(*ts) < ENDPOINT_TTL {
                    return Some(*pid);
                }
            }
        }

        // 未命中：查询系统连接表（AF_INET + AF_INET6）
        let rows = query_tcp_rows();
        let pid = find_pid_by_endpoint(&rows, local)?;

        if let Ok(mut cache) = self.endpoint_cache.lock() {
            cache.insert(key, (pid, now));
        }
        Some(pid)
    }

    /// 由 PID 解析可执行文件名（带 TTL 缓存）。
    #[cfg(windows)]
    fn resolve_name(&self, pid: u32) -> Option<String> {
        let now = Instant::now();

        if let Ok(cache) = self.name_cache.lock() {
            if let Some((name, ts)) = cache.get(&pid) {
                if now.duration_since(*ts) < NAME_TTL {
                    return Some(name.clone());
                }
            }
        }

        let name = query_process_name(pid)?;

        if let Ok(mut cache) = self.name_cache.lock() {
            cache.insert(pid, (name.clone(), now));
        }
        Some(name)
    }
}

/// 查询系统 TCP 连接表（IPv4 + IPv6），解析为 [`TcpRow`] 向量。
///
/// 任一族查询失败仅跳过该族，不影响另一族与整体（返回已解析到的行）。
#[cfg(windows)]
fn query_tcp_rows() -> Vec<TcpRow> {
    let mut rows = Vec::new();
    query_tcp4_rows(&mut rows);
    query_tcp6_rows(&mut rows);
    rows
}

/// 查询 IPv4 TCP 连接表（`AF_INET`，`TCP_TABLE_OWNER_PID_ALL`）。
#[cfg(windows)]
fn query_tcp4_rows(out: &mut Vec<TcpRow>) {
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GetExtendedTcpTable, MIB_TCPTABLE_OWNER_PID, TCP_TABLE_OWNER_PID_ALL,
    };
    use windows_sys::Win32::Networking::WinSock::AF_INET;

    // 第一次调用：传 null 缓冲以获取所需字节数
    let mut size: u32 = 0;
    unsafe {
        GetExtendedTcpTable(
            std::ptr::null_mut(),
            &mut size,
            0, // bOrder = FALSE
            AF_INET as u32,
            TCP_TABLE_OWNER_PID_ALL,
            0,
        );
    }
    if size == 0 {
        return;
    }

    let mut buf: Vec<u8> = vec![0u8; size as usize];
    let ret = unsafe {
        GetExtendedTcpTable(
            buf.as_mut_ptr() as *mut core::ffi::c_void,
            &mut size,
            0,
            AF_INET as u32,
            TCP_TABLE_OWNER_PID_ALL,
            0,
        )
    };
    // NO_ERROR == 0
    if ret != 0 {
        return;
    }

    unsafe {
        let table = &*(buf.as_ptr() as *const MIB_TCPTABLE_OWNER_PID);
        let n = table.dwNumEntries as usize;
        let rows_ptr = table.table.as_ptr();
        for i in 0..n {
            let row = &*rows_ptr.add(i);
            // dwLocalAddr 为网络字节序 u32，to_ne_bytes 得到 [a,b,c,d]
            let addr = Ipv4Addr::from(row.dwLocalAddr.to_ne_bytes());
            // dwLocalPort 低 16 位为网络字节序端口
            let port = u16::from_be((row.dwLocalPort & 0xFFFF) as u16);
            out.push(TcpRow {
                local_addr: IpAddr::V4(addr),
                local_port: port,
                pid: row.dwOwningPid,
            });
        }
    }
}

/// 查询 IPv6 TCP 连接表（`AF_INET6`，`TCP_TABLE_OWNER_PID_ALL`）。
#[cfg(windows)]
fn query_tcp6_rows(out: &mut Vec<TcpRow>) {
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GetExtendedTcpTable, MIB_TCP6TABLE_OWNER_PID, TCP_TABLE_OWNER_PID_ALL,
    };
    use windows_sys::Win32::Networking::WinSock::AF_INET6;

    let mut size: u32 = 0;
    unsafe {
        GetExtendedTcpTable(
            std::ptr::null_mut(),
            &mut size,
            0,
            AF_INET6 as u32,
            TCP_TABLE_OWNER_PID_ALL,
            0,
        );
    }
    if size == 0 {
        return;
    }

    let mut buf: Vec<u8> = vec![0u8; size as usize];
    let ret = unsafe {
        GetExtendedTcpTable(
            buf.as_mut_ptr() as *mut core::ffi::c_void,
            &mut size,
            0,
            AF_INET6 as u32,
            TCP_TABLE_OWNER_PID_ALL,
            0,
        )
    };
    if ret != 0 {
        return;
    }

    unsafe {
        let table = &*(buf.as_ptr() as *const MIB_TCP6TABLE_OWNER_PID);
        let n = table.dwNumEntries as usize;
        let rows_ptr = table.table.as_ptr();
        for i in 0..n {
            let row = &*rows_ptr.add(i);
            // ucLocalAddr 为 16 字节网络字节序 IPv6 地址
            let addr = Ipv6Addr::from(row.ucLocalAddr);
            let port = u16::from_be((row.dwLocalPort & 0xFFFF) as u16);
            out.push(TcpRow {
                local_addr: IpAddr::V6(addr),
                local_port: port,
                pid: row.dwOwningPid,
            });
        }
    }
}

/// 由 PID 经 `OpenProcess` + `QueryFullProcessImageNameW` 解析可执行文件名（小写）。
///
/// 使用 `PROCESS_QUERY_LIMITED_INFORMATION` 访问权限（对多数进程可用且无需提权）。
/// 任何失败均返回 `None`；句柄在返回前必被关闭。
#[cfg(windows)]
fn query_process_name(pid: u32) -> Option<String> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };

    if pid == 0 {
        return None;
    }

    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }

        // MAX_PATH 宽字符缓冲；lpdwSize 输入容量、输出实际写入的字符数（不含 NUL）
        let mut buf = [0u16; 260];
        let mut size = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut size);
        CloseHandle(handle);

        if ok == 0 || size == 0 {
            return None;
        }
        let path = String::from_utf16_lossy(&buf[..size as usize]);
        Some(exe_name_from_path(&path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn exe_name_extracts_lowercase_filename_from_windows_path() {
        assert_eq!(
            exe_name_from_path("C:\\Program Files\\Steam\\steam.exe"),
            "steam.exe"
        );
    }

    #[test]
    fn exe_name_lowercases_mixed_case() {
        assert_eq!(exe_name_from_path("D:\\Games\\Foo\\Bar.EXE"), "bar.exe");
    }

    #[test]
    fn exe_name_handles_forward_slash_and_no_separator() {
        assert_eq!(exe_name_from_path("/usr/bin/CURL"), "curl");
        assert_eq!(exe_name_from_path("plain.exe"), "plain.exe");
    }

    #[test]
    fn find_pid_matches_endpoint() {
        let rows = vec![
            TcpRow {
                local_addr: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
                local_port: 5000,
                pid: 111,
            },
            TcpRow {
                local_addr: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10)),
                local_port: 6000,
                pid: 222,
            },
        ];
        let local = SocketAddr::from((Ipv4Addr::new(192, 168, 1, 10), 6000));
        assert_eq!(find_pid_by_endpoint(&rows, local), Some(222));
    }

    #[test]
    fn find_pid_returns_none_when_no_match() {
        let rows = vec![TcpRow {
            local_addr: IpAddr::V6(Ipv6Addr::LOCALHOST),
            local_port: 443,
            pid: 999,
        }];
        // 端口不匹配
        let miss_port = SocketAddr::from((Ipv6Addr::LOCALHOST, 444));
        assert_eq!(find_pid_by_endpoint(&rows, miss_port), None);
        // 地址不匹配
        let miss_addr = SocketAddr::from((Ipv4Addr::new(10, 0, 0, 1), 443));
        assert_eq!(find_pid_by_endpoint(&rows, miss_addr), None);
    }

    use proptest::prelude::*;

    /// 生成小地址池的 IP：v4/v6 各取少量取值，提升行与查询命中概率，
    /// 以同时覆盖「命中」与「未命中」两条路径。
    fn arb_small_ip() -> impl Strategy<Value = IpAddr> {
        prop_oneof![
            (0u8..4).prop_map(|n| IpAddr::V4(Ipv4Addr::new(10, 0, 0, n))),
            (0u16..4).prop_map(|n| IpAddr::V6(Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, n))),
        ]
    }

    /// 生成一行连接表数据：小地址池 + 小端口池 + 任意 PID。
    fn arb_row() -> impl Strategy<Value = TcpRow> {
        (arb_small_ip(), 0u16..8, any::<u32>()).prop_map(|(local_addr, local_port, pid)| TcpRow {
            local_addr,
            local_port,
            pid,
        })
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, ..ProptestConfig::default() })]

        // Feature: network-capability-expansion, Property 13
        // 连接表端点反查 PID：存在匹配 (本地地址,本地端口) 行则返回其 PID，否则 None。
        // 期望值以与实现相同的「首个匹配」(.find) 语义计算。
        #[test]
        fn prop_find_pid_by_endpoint_matches_first_row(
            rows in prop::collection::vec(arb_row(), 0..12),
            qip in arb_small_ip(),
            qport in 0u16..8,
        ) {
            let query = SocketAddr::new(qip, qport);
            let expected = rows
                .iter()
                .find(|r| r.local_port == qport && r.local_addr == qip)
                .map(|r| r.pid);
            prop_assert_eq!(find_pid_by_endpoint(&rows, query), expected);
        }
    }
}
