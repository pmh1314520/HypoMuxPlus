// HypoMux Plus 国际化字典（简体中文 / English）
// 所有界面文本统一从此处取，确保中英文套件完整对应。

export type Lang = "zh" | "en";

export const DICT: Record<Lang, Record<string, string>> = {
  zh: {
    appName: "HypoMux Plus",
    appSubtitle: "多网卡带宽聚合 · 双协议分流加速引擎",
    navSection: "导航",
    topDashDesc: "多网卡分流调度与实时吞吐监控",
    topSettingsDesc: "引擎参数、外观与应用兼容性配置",
    stateActive: "运行中",
    stateIdle: "空闲",
    adminOk: "已获得管理员权限，全部功能可用",
    adminBadgeOk: "管理员",
    adminBadgeNo: "标准权限",
    warnSteamRunning: "检测到 Steam 正在运行，建议重启 Steam 客户端以使多链路加速完全生效",

    // 标题栏 / 导航
    navDashboard: "加速控制台",
    navSettings: "设置",
    tipMinimize: "最小化",
    tipMaximize: "最大化 / 还原",
    tipClose: "关闭",
    tipRefresh: "重新扫描网卡",

    // 状态
    statusLoading: "正在加载网卡…",
    statusReady: "就绪 · 已发现 {count} 张网卡",
    statusNoAdapters: "未发现可用网卡",
    statusLoadFailed: "网卡加载失败",
    statusStarting: "正在启动分流引擎…",
    statusStopping: "正在停止…",
    statusStopped: "已停止 · 就绪",
    statusRunning: "分流引擎运行中",
    statusStartFailed: "启动失败",

    // 数据大屏
    combinedDown: "合并下行总速度",
    unitMbps: "MB/s",
    uplink: "上行",
    totalConn: "总连接数",
    peakSpeed: "峰值",
    elapsed: "已运行",

    // 网卡表
    adaptersTitle: "网卡分流矩阵",
    adaptersHint: "勾选参与带宽聚合的活动网卡",
    colSelect: "选择",
    colAlias: "网卡别名",
    colIpv4: "IPv4 地址",
    colSpeed: "实时速度 (MB/s)",
    colConn: "连接数",
    selectAll: "全选",
    deselectAll: "清空",
    noValidIp: "无有效 IPv4",
    selectedCount: "已选 {n} 张",

    // 控制台
    consoleTitle: "调度控制台",
    consoleEmpty: "等待连接调度日志…",
    consoleClear: "清空",

    // 加速按钮
    boostStart: "一键加速",
    boostStop: "停止加速",
    boostStarting: "启动中…",
    boostStopping: "停止中…",

    // 提示 / 消息
    warnNoSelection: "请至少勾选一张拥有有效 IPv4 的网卡",
    msgBoostStarted: "已接管系统代理 · HTTP/HTTPS + SOCKS5 双协议分流已生效",
    msgBoostStopped: "已停止加速 · 系统代理已安全还原",
    msgStartFailed: "启动失败：{err}",
    msgScanFailed: "扫描网卡失败：{err}",
    msgPortsSaved: "端口已保存",
    adminWarn: "未以管理员身份运行：核心分流可用，但死网关检测等稳定性增强功能将不可用。建议右键「以管理员身份运行」。",

    // 设置页
    settingsTitle: "设置",
    settingsGeneral: "通用",
    settingLanguage: "界面语言",
    settingTheme: "外观主题",
    themeDark: "深色",
    themeLight: "浅色",
    settingPorts: "本地代理端口",
    portHttp: "HTTP / HTTPS",
    portSocks: "SOCKS5",
    settingCloseBehavior: "关闭按钮行为",
    closeToTray: "最小化到系统托盘",
    closeToExit: "直接退出程序",

    appcompatTitle: "应用兼容性",
    appcompatHint: "为不读取系统代理的客户端一键写入 SOCKS5 配置（重启对应客户端后生效）",
    steamConfig: "Steam 代理配置",
    idmConfig: "IDM 代理配置",
    btnApply: "写入配置",
    btnRestore: "还原",
    msgSteamApplied: "Steam 代理已写入，重启 Steam 后生效",
    msgSteamRestored: "Steam 代理配置已还原",
    msgIdmApplied: "IDM 代理已写入，重启 IDM 后生效",
    msgIdmRestored: "IDM 代理配置已还原",
    msgConfigFailed: "操作失败：{err}",

    aboutTitle: "关于",
    aboutVersion: "版本",
    aboutAuthor: "开发者",
    aboutDerived: "衍生自原项目",
    aboutLicense: "开源协议",
    aboutOriginal: "原项目",
    aboutDesc:
      "HypoMux Plus 是基于 Hypostasis-Cat 的开源项目 HypoMux 二次开发的现代化桌面客户端，采用 Tauri + React + Rust 重构，遵循 AGPL-3.0 协议开源。",

    // 工作原理小卡
    howTitle: "工作原理",
    howDesc:
      "通过 L3 物理层套接字绑定（IP_UNICAST_IF）将每条出站连接钉死在指定网卡，配合双协议本地代理在多张网卡间轮询分发，实现多线程下载的带宽物理叠加。",
  },

  en: {
    appName: "HypoMux Plus",
    appSubtitle: "Multi-NIC Bandwidth Aggregation · Dual-Protocol Splitting Engine",
    navSection: "Navigation",
    topDashDesc: "Multi-NIC dispatch & real-time throughput monitoring",
    topSettingsDesc: "Engine parameters, appearance & app compatibility",
    stateActive: "Active",
    stateIdle: "Idle",
    adminOk: "Running as administrator, all features available",
    adminBadgeOk: "Administrator",
    adminBadgeNo: "Standard",
    warnSteamRunning:
      "Steam is running. Restart the Steam client for multi-link acceleration to take full effect.",

    navDashboard: "Console",
    navSettings: "Settings",
    tipMinimize: "Minimize",
    tipMaximize: "Maximize / Restore",
    tipClose: "Close",
    tipRefresh: "Rescan adapters",

    statusLoading: "Loading adapters…",
    statusReady: "Ready · {count} adapter(s) found",
    statusNoAdapters: "No available adapters",
    statusLoadFailed: "Failed to load adapters",
    statusStarting: "Starting splitting engine…",
    statusStopping: "Stopping…",
    statusStopped: "Stopped · Ready",
    statusRunning: "Splitting engine running",
    statusStartFailed: "Start failed",

    combinedDown: "Combined Download Speed",
    unitMbps: "MB/s",
    uplink: "Upload",
    totalConn: "Connections",
    peakSpeed: "Peak",
    elapsed: "Uptime",

    adaptersTitle: "NIC Splitting Matrix",
    adaptersHint: "Select active adapters to join bandwidth aggregation",
    colSelect: "Select",
    colAlias: "Adapter Alias",
    colIpv4: "IPv4 Address",
    colSpeed: "Speed (MB/s)",
    colConn: "Conns",
    selectAll: "Select All",
    deselectAll: "Clear",
    noValidIp: "No valid IPv4",
    selectedCount: "{n} selected",

    consoleTitle: "Dispatch Console",
    consoleEmpty: "Waiting for connection dispatch logs…",
    consoleClear: "Clear",

    boostStart: "Boost",
    boostStop: "Stop",
    boostStarting: "Starting…",
    boostStopping: "Stopping…",

    warnNoSelection: "Please select at least one adapter with a valid IPv4",
    msgBoostStarted: "System proxy engaged · HTTP/HTTPS + SOCKS5 dual-protocol splitting active",
    msgBoostStopped: "Acceleration stopped · System proxy safely restored",
    msgStartFailed: "Start failed: {err}",
    msgScanFailed: "Adapter scan failed: {err}",
    msgPortsSaved: "Ports saved",
    adminWarn:
      "Not running as administrator: core splitting works, but stability features like dead-gateway detection are unavailable. Right-click and 'Run as administrator' is recommended.",

    settingsTitle: "Settings",
    settingsGeneral: "General",
    settingLanguage: "Language",
    settingTheme: "Appearance",
    themeDark: "Dark",
    themeLight: "Light",
    settingPorts: "Local Proxy Ports",
    portHttp: "HTTP / HTTPS",
    portSocks: "SOCKS5",
    settingCloseBehavior: "Close Button Behavior",
    closeToTray: "Minimize to system tray",
    closeToExit: "Exit the program",

    appcompatTitle: "App Compatibility",
    appcompatHint:
      "One-click SOCKS5 config for clients that ignore the system proxy (restart the client to take effect)",
    steamConfig: "Steam Proxy Config",
    idmConfig: "IDM Proxy Config",
    btnApply: "Apply",
    btnRestore: "Restore",
    msgSteamApplied: "Steam proxy written. Restart Steam to take effect",
    msgSteamRestored: "Steam proxy config restored",
    msgIdmApplied: "IDM proxy written. Restart IDM to take effect",
    msgIdmRestored: "IDM proxy config restored",
    msgConfigFailed: "Operation failed: {err}",

    aboutTitle: "About",
    aboutVersion: "Version",
    aboutAuthor: "Developer",
    aboutDerived: "Derived from",
    aboutLicense: "License",
    aboutOriginal: "Original Project",
    aboutDesc:
      "HypoMux Plus is a modernized desktop client based on the open-source HypoMux by Hypostasis-Cat, rebuilt with Tauri + React + Rust and released under AGPL-3.0.",

    howTitle: "How It Works",
    howDesc:
      "Each outbound connection is pinned to a specific NIC via L3 socket binding (IP_UNICAST_IF). A dual-protocol local proxy round-robins connections across selected adapters to physically stack bandwidth for multi-threaded downloads.",
  },
};

export function translate(lang: Lang, key: string, vars?: Record<string, string | number>): string {
  let text = DICT[lang][key] ?? DICT.zh[key] ?? key;
  if (vars) {
    for (const [k, v] of Object.entries(vars)) {
      text = text.replace(`{${k}}`, String(v));
    }
  }
  return text;
}
