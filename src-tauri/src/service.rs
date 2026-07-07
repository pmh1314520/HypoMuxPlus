//! 服务模式：以 SYSTEM 权限常驻的 Windows 服务承载 TUN 全局接管
//!
//! 目的：像 Clash 的服务模式那样,把“建虚拟网卡 + 改路由”这类特权操作交给一个
//! 一次性安装(装时弹一次 UAC)、以 SYSTEM 常驻的后台服务执行;主程序(普通权限)
//! 通过命名管道 IPC 通知服务开启/关闭 TUN,从而“每次启动都无需管理员”。
//!
//! 单二进制多入口:同一个 exe 通过命令行参数区分角色——
//!   - `--service`          由 SCM 拉起,作为服务主体运行(SYSTEM)
//!   - `--install-service`  提权实例执行:注册并启动服务
//!   - `--uninstall-service`提权实例执行:停止并删除服务
//!   - 无参数               正常 GUI 主程序
//!
//! IPC 协议(命名管道 `\\.\pipe\HypoMuxPlusTun`,文本行):
//!   请求 `PING` / `START <socks_port>` / `STOP`  →  响应 `OK` 或 `ERR <msg>`

use std::ffi::{c_void, OsString};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeServer, ServerOptions};
use tokio_util::sync::CancellationToken;

use windows_service::service::{
    ServiceAccess, ServiceControl, ServiceControlAccept, ServiceErrorControl, ServiceExitCode,
    ServiceInfo, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use windows_service::{define_windows_service, service_dispatcher};

use crate::tunmode::{self, TunHandle};

/// 服务内部标识名
pub const SERVICE_NAME: &str = "HypoMuxPlusTun";
/// 服务显示名
const SERVICE_DISPLAY: &str = "HypoMuxPlus TUN 全局接管服务";
/// 控制管道名
const PIPE_NAME: &str = r"\\.\pipe\HypoMuxPlusTun";
/// 管道安全描述符(SDDL):SYSTEM/内置管理员全权,认证用户读写(允许普通权限的 GUI 连接控制)
const PIPE_SDDL: &str = "D:(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;AU)";

// ============================ 安装 / 卸载(需管理员) ============================

/// 注册并启动服务(设为开机自启,SYSTEM 账户运行)。必须在管理员进程中调用。
pub fn install() -> Result<(), String> {
    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )
    .map_err(|e| format!("打开服务管理器失败: {e}"))?;

    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let info = ServiceInfo {
        name: OsString::from(SERVICE_NAME),
        display_name: OsString::from(SERVICE_DISPLAY),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: exe,
        launch_arguments: vec![OsString::from("--service")],
        dependencies: vec![],
        account_name: None, // LocalSystem
        account_password: None,
    };

    let access = ServiceAccess::CHANGE_CONFIG | ServiceAccess::START | ServiceAccess::QUERY_STATUS;
    let service = match manager.create_service(&info, access) {
        Ok(s) => s,
        // 已存在则复用
        Err(_) => manager
            .open_service(SERVICE_NAME, access)
            .map_err(|e| format!("创建/打开服务失败: {e}"))?,
    };

    let need_start = match service.query_status() {
        Ok(s) => s.current_state != ServiceState::Running,
        Err(_) => true,
    };
    if need_start {
        service
            .start(&Vec::<OsString>::new())
            .map_err(|e| format!("启动服务失败: {e}"))?;
    }
    Ok(())
}

/// 停止并删除服务。必须在管理员进程中调用。
pub fn uninstall() -> Result<(), String> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .map_err(|e| format!("打开服务管理器失败: {e}"))?;
    let service = manager
        .open_service(
            SERVICE_NAME,
            ServiceAccess::STOP | ServiceAccess::DELETE | ServiceAccess::QUERY_STATUS,
        )
        .map_err(|e| format!("打开服务失败(可能未安装): {e}"))?;

    if let Ok(s) = service.query_status() {
        if s.current_state != ServiceState::Stopped {
            let _ = service.stop();
            for _ in 0..25 {
                std::thread::sleep(Duration::from_millis(200));
                if let Ok(s2) = service.query_status() {
                    if s2.current_state == ServiceState::Stopped {
                        break;
                    }
                }
            }
        }
    }
    service.delete().map_err(|e| format!("删除服务失败: {e}"))?;
    Ok(())
}

/// 查询服务是否已注册(仅需查询权限,普通用户可调用)。
pub fn is_installed() -> bool {
    match ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT) {
        Ok(m) => m
            .open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS)
            .is_ok(),
        Err(_) => false,
    }
}

/// 以管理员身份重新拉起自身执行某个参数(弹一次 UAC),等待其结束并返回退出码。
/// 用 PowerShell 的 `Start-Process -Verb RunAs -Wait` 触发 UAC,避免直接调用
/// ShellExecuteEx 的繁琐 FFI;用户取消 UAC 时 PowerShell 以非零码退出。
pub fn run_self_elevated(arg: &str) -> Result<i32, String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe_str = exe.to_string_lossy().replace('\'', "''");
    let ps = format!(
        "try {{ $p = Start-Process -FilePath '{exe_str}' -ArgumentList '{arg}' -Verb RunAs -WindowStyle Hidden -PassThru -Wait; exit $p.ExitCode }} catch {{ exit 1 }}"
    );
    let status = std::process::Command::new("powershell")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-WindowStyle", "Hidden", "-Command", &ps])
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|e| format!("提权启动失败: {e}"))?;
    Ok(status.code().unwrap_or(1))
}

// ============================ IPC 客户端(主程序调用) ============================

/// 向服务发送一条命令并返回响应。主程序(普通权限)用它控制 TUN。
pub async fn client_command(cmd: &str) -> Result<String, String> {
    use windows_sys::Win32::Foundation::ERROR_PIPE_BUSY;

    let mut client = {
        let mut attempt = 0;
        loop {
            match ClientOptions::new().open(PIPE_NAME) {
                Ok(c) => break c,
                Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) && attempt < 40 => {
                    attempt += 1;
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Err(e) => return Err(format!("无法连接 TUN 服务: {e}")),
            }
        }
    };

    client
        .write_all(cmd.as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    let _ = client.flush().await;

    let mut buf = [0u8; 512];
    let n = tokio::time::timeout(Duration::from_secs(12), client.read(&mut buf))
        .await
        .map_err(|_| "TUN 服务响应超时".to_string())?
        .map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&buf[..n]).trim().to_string())
}

/// 服务是否可用(已安装、已运行且管道可通)。
pub async fn is_available() -> bool {
    matches!(client_command("PING").await, Ok(r) if r == "OK")
}

// ============================ 服务主体(SYSTEM 运行) ============================

define_windows_service!(ffi_service_main, service_main);

/// 服务入口:由 SCM 通过 `--service` 拉起后调用(经 main.rs 分发)。
pub fn run() -> Result<(), String> {
    service_dispatcher::start(SERVICE_NAME, ffi_service_main).map_err(|e| e.to_string())
}

fn service_main(_arguments: Vec<OsString>) {
    // 忽略错误:服务已尽力而为,异常时进程退出由 SCM 记录
    let _ = run_service();
}

fn run_service() -> Result<(), String> {
    let cancel = Arc::new(CancellationToken::new());
    let cancel_ctrl = cancel.clone();

    // 注册服务控制处理器:响应停止/关机
    let status_handle = service_control_handler::register(SERVICE_NAME, move |control| {
        match control {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                cancel_ctrl.cancel();
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    })
    .map_err(|e| e.to_string())?;

    let set_state = |state: ServiceState, accept: ServiceControlAccept| ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted: accept,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    };

    status_handle
        .set_service_status(set_state(
            ServiceState::Running,
            ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
        ))
        .map_err(|e| e.to_string())?;

    // 多线程 tokio 运行时承载管道服务与 TUN 用户态栈
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    rt.block_on(pipe_server(cancel));

    status_handle
        .set_service_status(set_state(ServiceState::Stopped, ServiceControlAccept::empty()))
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// 创建一个带宽松安全描述符的命名管道实例(允许普通用户连接控制)。
fn make_pipe_server() -> std::io::Result<NamedPipeServer> {
    use windows_sys::Win32::Foundation::{LocalFree, HLOCAL};
    use windows_sys::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;
    use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;

    let sddl: Vec<u16> = PIPE_SDDL.encode_utf16().chain(std::iter::once(0)).collect();
    let mut psd: *mut c_void = std::ptr::null_mut();
    let ok = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1, // SDDL_REVISION_1
            &mut psd,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error());
    }

    let mut sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: psd,
        bInheritHandle: 0,
    };
    let res = unsafe {
        ServerOptions::new()
            .create_with_security_attributes_raw(PIPE_NAME, &mut sa as *mut _ as *mut c_void)
    };
    unsafe {
        LocalFree(psd as HLOCAL);
    }
    res
}

/// 管道服务循环:逐个接受客户端连接并处理单次请求-响应,直到服务停止。
async fn pipe_server(cancel: Arc<CancellationToken>) {
    let tun: Arc<Mutex<Option<TunHandle>>> = Arc::new(Mutex::new(None));

    loop {
        let server = match make_pipe_server() {
            Ok(s) => s,
            Err(_) => {
                tokio::select! {
                    _ = cancel.cancelled() => break,
                    _ = tokio::time::sleep(Duration::from_millis(500)) => continue,
                }
            }
        };

        tokio::select! {
            _ = cancel.cancelled() => break,
            r = server.connect() => {
                if r.is_err() {
                    continue;
                }
                handle_conn(server, tun.clone()).await;
            }
        }
    }

    // 服务停止:确保拆除 TUN,还原路由
    let taken = tun.lock().take();
    if let Some(h) = taken {
        h.stop();
    }
}

/// 处理单个客户端连接:读一条命令,执行,回一条响应。
async fn handle_conn(mut server: NamedPipeServer, tun: Arc<Mutex<Option<TunHandle>>>) {
    let mut buf = [0u8; 256];
    let n = match tokio::time::timeout(Duration::from_secs(5), server.read(&mut buf)).await {
        Ok(Ok(n)) if n > 0 => n,
        _ => return,
    };
    let cmd = String::from_utf8_lossy(&buf[..n]).trim().to_string();
    let resp = process_command(&cmd, &tun).await;
    let _ = server.write_all(resp.as_bytes()).await;
    let _ = server.flush().await;
    // 稍等让客户端读走响应后再断开
    tokio::time::sleep(Duration::from_millis(50)).await;
    let _ = server.disconnect();
}

/// 解析并执行一条控制命令。注意:绝不跨 `.await` 持有锁。
async fn process_command(cmd: &str, tun: &Arc<Mutex<Option<TunHandle>>>) -> String {
    let mut parts = cmd.split_whitespace();
    match parts.next() {
        Some("PING") => "OK".to_string(),
        Some("STOP") => {
            let h = tun.lock().take();
            if let Some(h) = h {
                h.stop();
            }
            "OK".to_string()
        }
        Some("START") => {
            let port: u16 = match parts.next().and_then(|p| p.parse().ok()) {
                Some(p) => p,
                None => return "ERR 缺少 SOCKS 端口".to_string(),
            };
            // 若已在运行则先停,避免重复接管
            let old = tun.lock().take();
            if let Some(h) = old {
                h.stop();
            }
            match tunmode::start(port).await {
                Ok(h) => {
                    *tun.lock() = Some(h);
                    "OK".to_string()
                }
                Err(e) => format!("ERR {e}"),
            }
        }
        _ => "ERR 未知命令".to_string(),
    }
}
