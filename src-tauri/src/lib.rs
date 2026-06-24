//! HypoMuxPlus —— 多网卡带宽聚合工具（Tauri 后端）
//!
//! 衍生自 Hypostasis-Cat 的开源项目 HypoMux（AGPL-3.0）。
//! 衍生开发者：青云制作_彭明航。
//!
//! 后端职责：网卡发现、双协议分流引擎调度、系统代理全生命周期接管/还原、
//! 实时遥测事件推送、系统托盘与应用兼容性配置。

mod appcompat;
mod engine;
mod netadapter;
mod sysproxy;
mod telemetry;

use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::Mutex;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State};

/// 全局应用状态
pub struct AppState {
    engine: Mutex<Option<engine::EngineHandle>>,
    boosting: AtomicBool,
    close_to_tray: AtomicBool,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            engine: Mutex::new(None),
            boosting: AtomicBool::new(false),
            close_to_tray: AtomicBool::new(true),
        }
    }
}

/// 检测是否以管理员身份运行（部分稳定性增强功能需要）。
#[tauri::command]
fn check_admin() -> bool {
    #[cfg(windows)]
    unsafe {
        windows_sys::Win32::UI::Shell::IsUserAnAdmin() != 0
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// 检测 Steam 是否正在运行（开启加速前提醒用户重启 Steam，与原项目一致）。
#[tauri::command]
fn check_steam_running() -> bool {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
        use windows_sys::Win32::System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        };

        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return false;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut found = false;
        if Process32FirstW(snap, &mut entry) != 0 {
            loop {
                let len = entry
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(entry.szExeFile.len());
                let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
                if name.eq_ignore_ascii_case("steam.exe") {
                    found = true;
                    break;
                }
                if Process32NextW(snap, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snap);
        found
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// 扫描所有 Up 且拥有 IPv4 的网卡。
#[tauri::command]
fn scan_adapters() -> Result<Vec<netadapter::AdapterInfo>, String> {
    netadapter::scan_adapters()
}

/// 当前是否处于加速状态。
#[tauri::command]
fn get_boost_state(state: State<'_, AppState>) -> bool {
    state.boosting.load(Ordering::Relaxed)
}

/// 读取系统代理状态。
#[tauri::command]
fn get_system_proxy() -> (bool, String) {
    sysproxy::get_system_proxy()
}

/// 设置关闭行为：true=最小化到托盘，false=直接退出。
#[tauri::command]
fn set_close_to_tray(state: State<'_, AppState>, enabled: bool) {
    state.close_to_tray.store(enabled, Ordering::Relaxed);
}

/// 一键加速：启动分流引擎并接管系统代理。
#[tauri::command]
async fn start_boost(
    app: AppHandle,
    state: State<'_, AppState>,
    nics: Vec<engine::SelectedNic>,
    socks_port: u16,
    http_port: u16,
    strategy: String,
    lang: String,
    down_limit_mbps: f64,
    bypass: Vec<String>,
) -> Result<String, String> {
    if state.boosting.load(Ordering::Relaxed) {
        return Err("引擎已在运行中".into());
    }

    let handle = engine::start(
        app.clone(),
        nics,
        socks_port,
        http_port,
        strategy,
        lang,
        down_limit_mbps,
        bypass,
    )
    .await?;

    let socks_addr = format!("127.0.0.1:{socks_port}");
    let http_addr = format!("127.0.0.1:{http_port}");

    if let Err(e) = sysproxy::enable_system_proxy(&socks_addr, &http_addr) {
        // 接管失败：强制回滚，避免代理残留导致断网
        handle.stop();
        let _ = sysproxy::disable_system_proxy();
        return Err(format!("双协议引擎已监听，但无法写入系统代理: {e}"));
    }

    // 加速期间关闭死网关检测，维持慢速链路不被系统踢掉（需管理员，失败可忽略）
    let _ = sysproxy::set_dead_gateway_detection(false);

    *state.engine.lock() = Some(handle);
    state.boosting.store(true, Ordering::Relaxed);
    let _ = app.emit("hmx-boost-state", true);

    Ok(format!("http={http_addr};https={http_addr};socks={socks_addr}"))
}

/// 停止加速：销毁引擎并强制还原系统代理。
#[tauri::command]
fn stop_boost(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    if let Some(handle) = state.engine.lock().take() {
        handle.stop();
    }
    let _ = sysproxy::disable_system_proxy();
    let _ = sysproxy::set_dead_gateway_detection(true);
    state.boosting.store(false, Ordering::Relaxed);
    let _ = app.emit("hmx-boost-state", false);
    Ok(())
}

/// 配置 / 还原 Steam 代理。
#[tauri::command]
fn configure_steam(enable: bool, port: u16) -> Result<(), String> {
    appcompat::configure_steam(enable, "127.0.0.1", port)
}

/// 配置 / 还原 IDM 代理。
#[tauri::command]
fn configure_idm(enable: bool, port: u16) -> Result<(), String> {
    appcompat::configure_idm(enable, "127.0.0.1", port)
}

/// 读取文本文件（用于配置导入，路径由原生文件对话框提供）。
#[tauri::command]
fn read_text_file(path: String) -> Result<String, String> {
    std::fs::read_to_string(&path).map_err(|e| e.to_string())
}

/// 写入文本文件（用于配置导出，路径由原生文件对话框提供）。
#[tauri::command]
fn write_text_file(path: String, content: String) -> Result<(), String> {
    std::fs::write(&path, content).map_err(|e| e.to_string())
}

/// 检测本地端口是否可用（127.0.0.1 能否成功监听）。
#[tauri::command]
fn is_port_free(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
}

/// 从 start 起向上寻找一个可用端口（最多探测 2000 个）。
#[tauri::command]
fn suggest_free_port(start: u16) -> u16 {
    let mut p = start.max(1024);
    for _ in 0..2000 {
        if std::net::TcpListener::bind(("127.0.0.1", p)).is_ok() {
            return p;
        }
        match p.checked_add(1) {
            Some(n) => p = n,
            None => break,
        }
    }
    start
}

/// 逐张网卡探测出口连通性与延迟（Plus 专属链路体检）。
#[tauri::command]
async fn test_latency(nics: Vec<engine::SelectedNic>) -> Result<Vec<engine::LatencyResult>, String> {
    Ok(engine::test_latency(nics).await)
}

/// 逐张网卡下载测速跑分（Plus 专属）。
#[tauri::command]
async fn speed_test(
    app: AppHandle,
    nics: Vec<engine::SelectedNic>,
    duration: u64,
) -> Result<Vec<engine::SpeedResult>, String> {
    Ok(engine::speed_test(app, nics, duration).await)
}

/// 退出前的统一清理：停止引擎、还原系统代理与死网关检测。
fn cleanup(app: &AppHandle) {
    let state = app.state::<AppState>();
    if let Some(handle) = state.engine.lock().take() {
        handle.stop();
    }
    let _ = sysproxy::disable_system_proxy();
    let _ = sysproxy::set_dead_gateway_detection(true);
    state.boosting.store(false, Ordering::Relaxed);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_window_state::Builder::default()
                // 仅持久化尺寸与位置：避免保存"可见/最小化"状态导致
                // 上次关闭到托盘后，下次启动窗口仍保持隐藏的问题
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::SIZE
                        | tauri_plugin_window_state::StateFlags::POSITION,
                )
                .build(),
        )
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            check_admin,
            check_steam_running,
            scan_adapters,
            get_boost_state,
            get_system_proxy,
            set_close_to_tray,
            start_boost,
            stop_boost,
            configure_steam,
            configure_idm,
            read_text_file,
            write_text_file,
            is_port_free,
            suggest_free_port,
            test_latency,
            speed_test,
        ])
        .setup(|app| {
            // 启动即清除任何残留的系统代理，保证干净起点
            let _ = sysproxy::disable_system_proxy();

            // 构建系统托盘
            let show = MenuItem::with_id(app, "show", "显示主界面 / Show", true, None::<&str>)?;
            let toggle = MenuItem::with_id(app, "toggle", "加速 / 停止 切换 · Toggle Boost", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出程序 / Exit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &toggle, &quit])?;

            let _tray = TrayIconBuilder::with_id("main")
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("HypoMuxPlus · 多网卡带宽聚合工具")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.unminimize();
                            let _ = w.set_focus();
                        }
                    }
                    "toggle" => {
                        // 通知前端执行一键加速 / 停止（沿用主界面的完整加速流程）
                        let _ = app.emit("hmx-tray-toggle", ());
                    }
                    "quit" => {
                        cleanup(app);
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.unminimize();
                            let _ = w.set_focus();
                        }
                    }
                })
                .build(app)?;

            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let state = window.state::<AppState>();
                if state.close_to_tray.load(Ordering::Relaxed) {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("启动 HypoMuxPlus 失败")
        .run(|app_handle, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                cleanup(app_handle);
            }
        });
}
