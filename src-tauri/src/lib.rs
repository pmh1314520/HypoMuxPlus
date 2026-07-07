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
mod tunmode;
pub mod service;

use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::Mutex;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State};

/// 全局应用状态
pub struct AppState {
    engine: Mutex<Option<engine::EngineHandle>>,
    /// 全局接管（TUN）运行句柄；None 表示未启用 TUN 模式（仅"直连模式"下使用，服务模式由服务持有句柄）
    tun: Mutex<Option<tunmode::TunHandle>>,
    /// 本次 TUN 是否经服务模式启动（true=服务持有句柄，停止时走 IPC；false=进程内直连）
    tun_via_service: AtomicBool,
    boosting: AtomicBool,
    close_to_tray: AtomicBool,
    /// 悬浮窗是否启用（前端配置同步而来）
    hud_enabled: AtomicBool,
    /// 托盘菜单各项句柄，用于随状态 / 语言动态更新文字
    tray_show: Mutex<Option<tauri::menu::MenuItem<tauri::Wry>>>,
    tray_toggle: Mutex<Option<tauri::menu::MenuItem<tauri::Wry>>>,
    tray_quit: Mutex<Option<tauri::menu::MenuItem<tauri::Wry>>>,
    /// 托盘菜单语言：true=英文，false=中文（跟随客户端选择）
    tray_en: AtomicBool,
    /// 进程感知自动加速：是否启用（检测到下载类应用自动加速）
    app_watch: AtomicBool,
    /// 托盘图标句柄，用于动态渲染实时速度数字图标
    tray: Mutex<Option<tauri::tray::TrayIcon<tauri::Wry>>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            engine: Mutex::new(None),
            tun: Mutex::new(None),
            tun_via_service: AtomicBool::new(false),
            boosting: AtomicBool::new(false),
            close_to_tray: AtomicBool::new(true),
            hud_enabled: AtomicBool::new(false),
            tray_show: Mutex::new(None),
            tray_toggle: Mutex::new(None),
            tray_quit: Mutex::new(None),
            tray_en: AtomicBool::new(false),
            app_watch: AtomicBool::new(false),
            tray: Mutex::new(None),
        }
    }
}

/// 显示悬浮窗（HUD）。
fn show_hud(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("hud") {
        let _ = w.show();
        let _ = w.set_always_on_top(true);
    }
}

/// 隐藏悬浮窗（HUD）。
fn hide_hud(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("hud") {
        let _ = w.hide();
    }
}

/// 进入托盘模式：隐藏主窗口，按需显示 HUD。
fn enter_tray(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.hide();
    }
    if app.state::<AppState>().hud_enabled.load(Ordering::Relaxed) {
        show_hud(app);
    }
}

/// 退出托盘模式：显示主窗口并隐藏 HUD。
fn leave_tray(app: &AppHandle) {
    hide_hud(app);
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// 根据运行状态与语言更新托盘「加速/停止」菜单项文字。
fn update_tray_toggle(app: &AppHandle, running: bool) {
    let st = app.state::<AppState>();
    let en = st.tray_en.load(Ordering::Relaxed);
    let item = st.tray_toggle.lock().clone();
    if let Some(item) = item {
        let _ = item.set_text(match (running, en) {
            (true, false) => "停止加速",
            (false, false) => "开始加速",
            (true, true) => "Stop Boost",
            (false, true) => "Start Boost",
        });
    }
}

/// 已知下载 / 游戏平台进程名（小写），用于进程感知自动加速。
const WATCH_PROCESSES: &[&str] = &[
    "steam.exe", "idman.exe", "thunder.exe", "xldl.exe", "baidunetdisk.exe",
    "qbittorrent.exe", "utorrent.exe", "bittorrent.exe", "aria2c.exe", "motrix.exe",
    "fdm.exe", "epicgameslauncher.exe", "battle.net.exe", "eadesktop.exe",
    "origin.exe", "galaxyclient.exe", "wegame.exe", "transmission.exe",
];

/// 启用 / 关闭进程感知自动加速（检测到下载类应用自动开始 / 停止加速）。
#[tauri::command]
fn set_app_watch(state: State<'_, AppState>, enabled: bool) {
    state.app_watch.store(enabled, Ordering::Relaxed);
}

/// 经 tasklist 枚举当前进程，判断是否有目标下载 / 游戏进程在运行。
fn any_watch_process_running() -> bool {
    use std::os::windows::process::CommandExt;
    let out = match std::process::Command::new("tasklist")
        .args(["/fo", "csv", "/nh"])
        .creation_flags(0x0800_0000)
        .output()
    {
        Ok(o) => o,
        Err(_) => return false,
    };
    let text = String::from_utf8_lossy(&out.stdout).to_lowercase();
    WATCH_PROCESSES.iter().any(|p| text.contains(p))
}

/// 3×5 像素字形（位 0b100=左,0b010=中,0b001=右），用于在托盘图标上渲染速度数字。
fn glyph_3x5(c: char) -> [u8; 5] {
    match c {
        '0' => [0b111, 0b101, 0b101, 0b101, 0b111],
        '1' => [0b010, 0b110, 0b010, 0b010, 0b111],
        '2' => [0b111, 0b001, 0b111, 0b100, 0b111],
        '3' => [0b111, 0b001, 0b111, 0b001, 0b111],
        '4' => [0b101, 0b101, 0b111, 0b001, 0b001],
        '5' => [0b111, 0b100, 0b111, 0b001, 0b111],
        '6' => [0b111, 0b100, 0b111, 0b101, 0b111],
        '7' => [0b111, 0b001, 0b010, 0b010, 0b010],
        '8' => [0b111, 0b101, 0b111, 0b101, 0b111],
        '9' => [0b111, 0b101, 0b111, 0b001, 0b111],
        _ => [0, 0, 0, 0, 0],
    }
}

/// 将整数速度（MB/s）渲染为 32×32 RGBA 托盘图标（白字 + 深色描边，适配明暗任务栏）。
fn render_speed_icon(mbps: f64) -> tauri::image::Image<'static> {
    const SIZE: usize = 32;
    let mut buf = vec![0u8; SIZE * SIZE * 4];
    let put = |buf: &mut [u8], x: i32, y: i32, rgba: (u8, u8, u8, u8)| {
        if x < 0 || y < 0 || x >= SIZE as i32 || y >= SIZE as i32 {
            return;
        }
        let i = (y as usize * SIZE + x as usize) * 4;
        buf[i] = rgba.0;
        buf[i + 1] = rgba.1;
        buf[i + 2] = rgba.2;
        buf[i + 3] = rgba.3;
    };
    let n = mbps.round().clamp(0.0, 999.0) as u32;
    let s = n.to_string();
    let count = s.len();
    let scale: i32 = if count <= 1 { 6 } else if count == 2 { 5 } else { 3 };
    let gw = 3 * scale;
    let gap = scale.max(2);
    let total_w = count as i32 * gw + (count as i32 - 1) * gap;
    let mut x0 = (SIZE as i32 - total_w) / 2;
    let y0 = (SIZE as i32 - 5 * scale) / 2;
    for ch in s.chars() {
        let g = glyph_3x5(ch);
        for (ry, row) in g.iter().enumerate() {
            for cx in 0..3i32 {
                if row & (0b100 >> cx) != 0 {
                    for dy in 0..scale {
                        for dx in 0..scale {
                            let px = x0 + cx * scale + dx;
                            let py = y0 + ry as i32 * scale + dy;
                            // 先描边（深色，四周 1px），再覆白字，保证任意任务栏底色下都清晰
                            put(&mut buf, px + 1, py + 1, (10, 14, 22, 220));
                            put(&mut buf, px, py, (255, 255, 255, 255));
                        }
                    }
                }
            }
        }
        x0 += gw + gap;
    }
    tauri::image::Image::new_owned(buf, SIZE as u32, SIZE as u32)
}

/// 更新托盘图标为当前合并速度数字 + 悬停提示。
#[tauri::command]
fn update_tray_speed(app: AppHandle, mbps: f64) {
    let tray = app.state::<AppState>().tray.lock().clone();
    if let Some(tray) = tray {
        let _ = tray.set_icon(Some(render_speed_icon(mbps)));
        let _ = tray.set_tooltip(Some(&format!("HypoMuxPlus · ↓ {mbps:.1} MB/s")));
    }
}

/// 还原托盘默认图标与提示（停止加速时调用）。
#[tauri::command]
fn reset_tray_icon(app: AppHandle) {
    let tray = app.state::<AppState>().tray.lock().clone();
    if let Some(tray) = tray {
        if let Some(def) = app.default_window_icon() {
            let _ = tray.set_icon(Some(def.clone()));
        }
        let _ = tray.set_tooltip(Some("HypoMuxPlus · 多网卡带宽聚合工具"));
    }
}

/// 处理命令行参数（CLI 控制）：boost/start、stop、toggle、show、quit。
/// 第二个实例的参数经 single-instance 转发到首个实例执行。
fn handle_cli(app: &AppHandle, argv: &[String]) {
    let args: Vec<String> = argv.iter().map(|s| s.to_lowercase()).collect();
    let has = |k: &str| args.iter().any(|a| a == k);
    if has("--quit") || has("--exit") {
        cleanup(app);
        app.exit(0);
        return;
    }
    if has("--show") {
        leave_tray(app);
    }
    if has("--boost") || has("--start") {
        let _ = app.emit("hmx-cli", "start");
    } else if has("--stop") {
        let _ = app.emit("hmx-cli", "stop");
    } else if has("--toggle") {
        let _ = app.emit("hmx-cli", "toggle");
    }
}

/// 按客户端选择的语言刷新整个托盘菜单（显示主界面 / 加速切换 / 退出）。
#[tauri::command]
fn set_tray_language(app: AppHandle, en: bool) {
    let st = app.state::<AppState>();
    st.tray_en.store(en, Ordering::Relaxed);
    if let Some(show) = st.tray_show.lock().clone() {
        let _ = show.set_text(if en { "Show Window" } else { "显示主界面" });
    }
    if let Some(quit) = st.tray_quit.lock().clone() {
        let _ = quit.set_text(if en { "Exit" } else { "退出程序" });
    }
    let running = st.boosting.load(Ordering::Relaxed);
    drop(st);
    update_tray_toggle(&app, running);
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

/// 同步悬浮窗启用状态：禁用时立即隐藏；启用且主窗口当前不可见时立即显示。
#[tauri::command]
fn set_hud_enabled(app: AppHandle, enabled: bool) {
    let state = app.state::<AppState>();
    state.hud_enabled.store(enabled, Ordering::Relaxed);
    if !enabled {
        hide_hud(&app);
        return;
    }
    // 启用：仅当主窗口已隐藏（托盘模式）时才显示 HUD
    let main_visible = app
        .get_webview_window("main")
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(true);
    if !main_visible {
        show_hud(&app);
    }
}

/// 最小化到托盘（隐藏主窗口，按需显示 HUD）。
#[tauri::command]
fn hide_to_tray(app: AppHandle) {
    enter_tray(&app);
}

/// 从托盘 / 悬浮窗恢复主窗口。
#[tauri::command]
fn restore_main(app: AppHandle) {
    leave_tray(&app);
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
    rules: Vec<engine::RouteRuleDef>,
    tun_mode: bool,
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
        lang.clone(),
        down_limit_mbps,
        bypass,
        rules,
    )
    .await?;

    let socks_addr = format!("127.0.0.1:{socks_port}");
    let http_addr = format!("127.0.0.1:{http_port}");

    if tun_mode {
        // 全局接管：优先走服务模式（普通权限即可），服务不可用时回退到进程内直连模式（需 GUI 自身管理员）。
        if service::is_available().await {
            match service::client_command(&format!("START {socks_port}")).await {
                Ok(r) if r == "OK" => {
                    state.tun_via_service.store(true, Ordering::Relaxed);
                }
                Ok(r) => {
                    handle.stop();
                    return Err(format!("TUN 服务启动失败: {}", r.trim_start_matches("ERR ").trim()));
                }
                Err(e) => {
                    handle.stop();
                    return Err(format!("与 TUN 服务通信失败: {e}"));
                }
            }
        } else {
            match tunmode::start(socks_port).await {
                Ok(t) => {
                    state.tun_via_service.store(false, Ordering::Relaxed);
                    *state.tun.lock() = Some(t);
                }
                Err(e) => {
                    handle.stop();
                    return Err(format!(
                        "TUN 全局接管启动失败: {e}（提示：可在设置里安装“TUN 服务模式”，或以管理员身份运行本程序）"
                    ));
                }
            }
        }
    } else if let Err(e) = sysproxy::enable_system_proxy(&socks_addr, &http_addr) {
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
    update_tray_toggle(&app, true);

    Ok(format!("http={http_addr};https={http_addr};socks={socks_addr}"))
}

/// 停止 TUN 全局接管：服务模式经 IPC 通知服务拆除，直连模式停止本进程句柄。
fn stop_tun(state: &AppState) {
    if state.tun_via_service.swap(false, Ordering::Relaxed) {
        let _ = tauri::async_runtime::block_on(service::client_command("STOP"));
    } else if let Some(t) = state.tun.lock().take() {
        t.stop();
    }
}

/// 停止加速：销毁引擎并强制还原系统代理。
#[tauri::command]
fn stop_boost(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    if let Some(handle) = state.engine.lock().take() {
        handle.stop();
    }
    stop_tun(state.inner());
    let _ = sysproxy::disable_system_proxy();
    let _ = sysproxy::set_dead_gateway_detection(true);
    state.boosting.store(false, Ordering::Relaxed);
    let _ = app.emit("hmx-boost-state", false);
    update_tray_toggle(&app, false);
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

/// 安装 TUN 服务模式（自我提权，弹一次 UAC）。装好后普通权限即可开启 TUN，无需每次管理员。
#[tauri::command]
async fn install_tun_service() -> Result<(), String> {
    let code = tauri::async_runtime::spawn_blocking(|| service::run_self_elevated("--install-service"))
        .await
        .map_err(|e| e.to_string())??;
    if code != 0 {
        return Err("服务安装未完成（可能取消了 UAC 授权）".into());
    }
    // 轮询等待服务管道就绪
    for _ in 0..20 {
        if service::is_available().await {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }
    if service::is_installed() {
        Ok(())
    } else {
        Err("服务安装失败".into())
    }
}

/// 卸载 TUN 服务模式（自我提权）。
#[tauri::command]
async fn uninstall_tun_service() -> Result<(), String> {
    let code = tauri::async_runtime::spawn_blocking(|| service::run_self_elevated("--uninstall-service"))
        .await
        .map_err(|e| e.to_string())??;
    if code != 0 {
        return Err("服务卸载未完成".into());
    }
    Ok(())
}

/// 查询 TUN 服务状态：(是否已安装, 是否可用)。
#[tauri::command]
async fn tun_service_status() -> (bool, bool) {
    let installed = service::is_installed();
    let available = if installed { service::is_available().await } else { false };
    (installed, available)
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

/// 写入二进制文件（用于诊断报告导出 PNG，字节数组由前端 Canvas 提供）。
#[tauri::command]
fn write_binary_file(path: String, data: Vec<u8>) -> Result<(), String> {
    std::fs::write(&path, data).map_err(|e| e.to_string())
}

/// 拉取远程文本（用于分流规则订阅：从 URL 导入规则列表）。
#[tauri::command]
async fn fetch_text(url: String) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client.get(&url).send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status().as_u16()));
    }
    resp.text().await.map_err(|e| e.to_string())
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

// ============================== 应用内更新 ==============================

const GITEE_RELEASES_API: &str = "https://gitee.com/api/v5/repos/peng-minghang/hypo-mux-plus/releases";

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateInfo {
    current: String,
    latest: String,
    has_update: bool,
    url: String,
    notes: String,
}

/// 比较版本号 a 是否大于 b（点分数字，缺位按 0 处理）。
fn version_gt(a: &str, b: &str) -> bool {
    let pa: Vec<u32> = a.split('.').map(|x| x.trim().parse().unwrap_or(0)).collect();
    let pb: Vec<u32> = b.split('.').map(|x| x.trim().parse().unwrap_or(0)).collect();
    for i in 0..pa.len().max(pb.len()) {
        let x = pa.get(i).copied().unwrap_or(0);
        let y = pb.get(i).copied().unwrap_or(0);
        if x != y {
            return x > y;
        }
    }
    false
}

/// 检查更新：拉取 Gitee 仓库 Releases 列表，取最新正式版与当前版本比对。
#[tauri::command]
async fn check_update(app: AppHandle) -> Result<UpdateInfo, String> {
    let current = app.package_info().version.to_string();
    let client = reqwest::Client::builder()
        .user_agent("HypoMuxPlus")
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;
    let body = client
        .get(GITEE_RELEASES_API)
        .send()
        .await
        .map_err(|e| format!("网络请求失败: {e}"))?
        .text()
        .await
        .map_err(|e| format!("读取响应失败: {e}"))?;
    let arr: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("解析响应失败: {e}"))?;
    let releases = arr.as_array().ok_or("Releases 响应格式异常")?;
    // 列表按时间升序，最后一个非预发布版即为最新版
    let latest = releases
        .iter()
        .filter(|r| !r["prerelease"].as_bool().unwrap_or(false))
        .last()
        .ok_or("仓库暂无发布版本")?;
    let tag = latest["tag_name"].as_str().unwrap_or("").trim().to_string();
    let notes = latest["body"].as_str().unwrap_or("").to_string();
    let latest_ver = tag.trim_start_matches('v').trim_start_matches('V').to_string();
    let url = format!(
        "https://gitee.com/peng-minghang/hypo-mux-plus/releases/download/{tag}/HypoMuxPlus.exe"
    );
    let has_update = !latest_ver.is_empty() && version_gt(&latest_ver, &current);
    Ok(UpdateInfo { current, latest: latest_ver, has_update, url, notes })
}

/// 下载新版本并在退出后自动替换当前可执行文件、重新启动（应用内全量更新）。
#[tauri::command]
async fn download_and_install(app: AppHandle, url: String) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .user_agent("HypoMuxPlus")
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| e.to_string())?;
    let mut resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("下载失败: {e}"))?
        .error_for_status()
        .map_err(|e| format!("下载失败: {e}"))?;
    let total = resp.content_length().unwrap_or(0);

    // 流式下载并向前端推送进度，驱动更新弹窗的进度条
    let mut bytes: Vec<u8> = Vec::with_capacity(total.max(1) as usize);
    let mut downloaded: u64 = 0;
    let mut last_emit = std::time::Instant::now();
    let emit_progress = |app: &AppHandle, downloaded: u64, total: u64, done: bool| {
        let percent = if total > 0 {
            (downloaded as f64 / total as f64 * 100.0).clamp(0.0, 100.0)
        } else if done {
            100.0
        } else {
            0.0
        };
        let _ = app.emit(
            "hmx-update-progress",
            serde_json::json!({ "downloaded": downloaded, "total": total, "percent": percent }),
        );
    };
    emit_progress(&app, 0, total, false);
    while let Some(chunk) = resp.chunk().await.map_err(|e| format!("下载失败: {e}"))? {
        bytes.extend_from_slice(&chunk);
        downloaded += chunk.len() as u64;
        if last_emit.elapsed() >= std::time::Duration::from_millis(80) {
            emit_progress(&app, downloaded, total, false);
            last_emit = std::time::Instant::now();
        }
    }
    emit_progress(&app, downloaded, total.max(downloaded), true);

    if bytes.len() < 1024 * 1024 {
        return Err("下载内容异常（体积过小）".into());
    }

    let cur = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = cur.parent().ok_or("无法定位安装目录")?.to_path_buf();
    let new_path = dir.join("HypoMuxPlus.update.exe");
    std::fs::write(&new_path, &bytes).map_err(|e| format!("写入更新文件失败: {e}"))?;

    #[cfg(windows)]
    {
        // 脚本写入系统临时目录（而非安装目录），更新后不会在程序文件夹留下 .bat 残留
        let bat = std::env::temp_dir().join("hmx_update.bat");
        let cur_str = cur.to_string_lossy().replace('/', "\\");
        let new_str = new_path.to_string_lossy().replace('/', "\\");
        let pid = std::process::id();
        // 按 PID 精确等待主进程退出 -> 覆盖旧 exe -> 重新启动 -> 自删脚本。
        // 全程在隐藏控制台内执行（仅 CREATE_NO_WINDOW），不会弹出任何终端窗口。
        let script = format!(
            "@echo off\r\nchcp 65001>nul\r\n:wait\r\ntasklist /fi \"PID eq {pid}\" /nh 2>nul | find \"{pid}\" >nul && (ping 127.0.0.1 -n 2 >nul & goto wait)\r\nmove /y \"{new_str}\" \"{cur_str}\" >nul\r\nstart \"\" \"{cur_str}\"\r\ndel \"%~f0\"\r\n"
        );
        std::fs::write(&bat, &script).map_err(|e| format!("写入更新脚本失败: {e}"))?;
        use std::os::windows::process::CommandExt;
        // 仅 CREATE_NO_WINDOW：cmd 在隐藏控制台运行，ping/tasklist/find/move/start
        // 等子命令继承该隐藏控制台，全程不闪任何黑色终端窗口。
        // 注意：不可叠加 DETACHED_PROCESS，否则子控制台程序会各自新建可见窗口而疯狂闪烁。
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        std::process::Command::new("cmd")
            .arg("/c")
            .arg(&bat)
            .current_dir(std::env::temp_dir())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| format!("启动更新程序失败: {e}"))?;
    }

    // 退出当前实例，让更新脚本完成替换并重启
    cleanup(&app);
    app.exit(0);
    Ok(())
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
    stop_tun(state.inner());
    let _ = sysproxy::disable_system_proxy();
    let _ = sysproxy::set_dead_gateway_detection(true);
    state.boosting.store(false, Ordering::Relaxed);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // single-instance 必须最先注册：第二个实例的命令行参数会转发到首个实例
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            handle_cli(app, &argv);
        }))
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
            set_hud_enabled,
            hide_to_tray,
            restore_main,
            start_boost,
            stop_boost,
            configure_steam,
            configure_idm,
            install_tun_service,
            uninstall_tun_service,
            tun_service_status,
            read_text_file,
            write_text_file,
            write_binary_file,
            fetch_text,
            set_tray_language,
            update_tray_speed,
            reset_tray_icon,
            set_app_watch,
            is_port_free,
            suggest_free_port,
            check_update,
            download_and_install,
            test_latency,
            speed_test,
        ])
        .setup(|app| {
            // 启动时仅清理疑似本程序上次崩溃残留的系统代理，不触碰 Clash 等第三方代理
            sysproxy::clear_residual_proxy();
            // 清理上次可能残留的 TUN 接管路由（崩溃遗留），幂等无副作用
            tunmode::cleanup_residual_routes();

            // 首个实例自身的命令行参数（CLI 控制）：延迟到前端就绪后执行
            {
                let app_handle = app.handle().clone();
                let args: Vec<String> = std::env::args().skip(1).collect();
                if !args.is_empty() {
                    tauri::async_runtime::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_millis(1800)).await;
                        handle_cli(&app_handle, &args);
                    });
                }
            }

            // 进程感知自动加速：后台轮询目标进程，状态变化时通知前端
            {
                let app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let mut last_present = false;
                    loop {
                        tokio::time::sleep(std::time::Duration::from_secs(4)).await;
                        let enabled = app_handle.state::<AppState>().app_watch.load(Ordering::Relaxed);
                        if !enabled {
                            last_present = false;
                            continue;
                        }
                        let present = any_watch_process_running();
                        if present != last_present {
                            last_present = present;
                            let _ = app_handle.emit("hmx-autoboost", present);
                        }
                    }
                });
            }

            // 构建系统托盘（初始中文，随客户端语言由 set_tray_language 刷新）
            let show = MenuItem::with_id(app, "show", "显示主界面", true, None::<&str>)?;
            let toggle = MenuItem::with_id(app, "toggle", "开始加速", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出程序", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &toggle, &quit])?;
            // 保存句柄以便随加速状态 / 语言动态更新文字
            {
                let st = app.state::<AppState>();
                st.tray_show.lock().replace(show.clone());
                st.tray_toggle.lock().replace(toggle.clone());
                st.tray_quit.lock().replace(quit.clone());
            }

            let _tray = TrayIconBuilder::with_id("main")
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("HypoMuxPlus · 多网卡带宽聚合工具")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        leave_tray(app);
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
                        leave_tray(tray.app_handle());
                    }
                })
                .build(app)?;

            // 保存托盘句柄，供动态速度图标更新
            app.state::<AppState>().tray.lock().replace(_tray);

            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // 仅主窗口的关闭走"最小化到托盘"逻辑；HUD 窗口关闭不拦截
                if window.label() != "main" {
                    return;
                }
                let state = window.state::<AppState>();
                if state.close_to_tray.load(Ordering::Relaxed) {
                    api.prevent_close();
                    enter_tray(&window.app_handle());
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
