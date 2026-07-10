// 与 Rust 后端交互的类型化封装层
import { invoke } from "@tauri-apps/api/core";
import { emit, listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { UpstreamProxy, UpstreamBinding } from "./upstream";

// 上游代理链类型的单一真源为 lib/upstream.ts，此处 re-export 供依赖 api.ts 的调用方使用，避免重复定义
export type { UpstreamProxy, UpstreamBinding } from "./upstream";

/** 上游代理链回退策略：回退直连 / 失败。 */
export type UpstreamFallback = "direct" | "fail";

/**
 * 上游健康探测配置（Health_Prober）。默认 enabled=false，未启用时全部上游视为 Healthy，
 * 走既有 pick_upstream_for_nic 调度序，行为与升级前一致（零回归）。
 */
export interface HealthCfg {
  /** 是否启用后台健康探测与加权优选（默认 false） */
  enabled: boolean;
  /** 探测间隔（毫秒，缺省 30000） */
  intervalMs: number;
  /** 探测超时（毫秒，缺省 5000） */
  timeoutMs: number;
  /** 连续失败达该阈值进入熔断（缺省 3） */
  failThreshold: number;
  /** 熔断冷却期（毫秒，缺省 60000） */
  cooldownMs: number;
}

/**
 * 每网卡独立 DNS / DoH 配置（Per_NIC_DNS），按网卡 IfIndex 映射。
 * 未配置的网卡走既有全局 DNS / DoH 解析路径（零回归）。
 */
export interface PerNicDnsCfg {
  /** 网卡接口索引（IfIndex） */
  ifIndex: number;
  /** 解析方式：明文 DNS 服务器 / DoH 端点 */
  kind: "plain" | "doh";
  /** 明文 DNS 地址（如 "1.1.1.1"）或 DoH URL（如 "https://dns.google/dns-query"） */
  endpoint: string;
}

export interface AdapterInfo {
  index: number;
  alias: string;
  ipv4: string;
  /** 网卡的 IPv6 地址；无 IPv6 时为空字符串 */
  ipv6: string;
  description: string;
  isUp: boolean;
  /** 是否疑似虚拟/隧道/VPN/环回等网卡（仅作标记，供前端过滤器使用） */
  isVirtual: boolean;
}

export interface SelectedNic {
  index: number;
  name: string;
  ip: string;
  /** 网卡的 IPv6 地址（可选，无则省略） */
  ipv6?: string;
  /** 调度权重（默认 100） */
  weight?: number;
  /** 单卡下行限速 MB/s（0=不限速） */
  limit_mbps?: number;
}

export interface NicTelemetry {
  index: number;
  name: string;
  downMbps: number;
  upMbps: number;
  connections: number;
}

export interface LatencyResult {
  index: number;
  name: string;
  /** 代表性 RTT（成功时=avg），-1 表示全失败 */
  latencyMs: number;
  ok: boolean;
  /** 最小 RTT（ms），-1 表示全失败 */
  minMs: number;
  /** 平均 RTT（ms），-1 表示全失败 */
  avgMs: number;
  /** 抖动（ms，成功样本标准差），-1 表示不可用 */
  jitterMs: number;
  /** 丢包率 0~1 */
  lossPct: number;
}

export interface SpeedResult {
  index: number;
  name: string;
  mbps: number;
  ok: boolean;
}

export interface ConnInfo {
  /** 连接唯一自增 ID（＝发生顺序），供前端稳定排序与列表 key */
  id: number;
  target: string;
  nic: string;
  proto: string;
}

export interface UpdateInfo {
  current: string;
  latest: string;
  hasUpdate: boolean;
  url: string;
  notes: string;
}

export interface TelemetryPayload {
  perNic: NicTelemetry[];
  total: { downMbps: number; upMbps: number; connections: number };
}

export const api = {
  checkAdmin: () => invoke<boolean>("check_admin"),
  checkSteamRunning: () => invoke<boolean>("check_steam_running"),
  scanAdapters: () => invoke<AdapterInfo[]>("scan_adapters"),
  getBoostState: () => invoke<boolean>("get_boost_state"),
  getSystemProxy: () => invoke<[boolean, string]>("get_system_proxy"),
  setCloseToTray: (enabled: boolean) => invoke<void>("set_close_to_tray", { enabled }),
  startBoost: (
    nics: SelectedNic[],
    socksPort: number,
    httpPort: number,
    strategy: string,
    lang: string,
    downLimitMbps: number,
    bypass: string[],
    rules: { pattern: string; action: string; kind?: "domain" | "process" }[],
    tunMode: boolean,
    ipVersion: string,
    udpAssociate: boolean,
    upstreams: UpstreamProxy[],
    upstreamBindings: UpstreamBinding[],
    upstreamChain: boolean,
    upstreamFallback: UpstreamFallback,
    healthCfg: HealthCfg,
    perNicDns: PerNicDnsCfg[],
    connCap: number,
    taskCap: number,
    proxyGuardian: boolean,
  ) =>
    invoke<string>("start_boost", { nics, socksPort, httpPort, strategy, lang, downLimitMbps, bypass, rules, tunMode, ipVersion, udpAssociate, upstreams, upstreamBindings, upstreamChain, upstreamFallback, healthCfg, perNicDns, connCap, taskCap, proxyGuardian }),
  stopBoost: () => invoke<void>("stop_boost"),
  testLatency: (nics: SelectedNic[]) => invoke<LatencyResult[]>("test_latency", { nics }),
  speedTest: (nics: SelectedNic[], duration: number) =>
    invoke<SpeedResult[]>("speed_test", { nics, duration }),
  configureSteam: (enable: boolean, port: number) =>
    invoke<void>("configure_steam", { enable, port }),
  configureIdm: (enable: boolean, port: number) => invoke<void>("configure_idm", { enable, port }),
  installTunService: () => invoke<void>("install_tun_service"),
  uninstallTunService: () => invoke<void>("uninstall_tun_service"),
  tunServiceStatus: () => invoke<[boolean, boolean]>("tun_service_status"),
  readTextFile: (path: string) => invoke<string>("read_text_file", { path }),
  writeTextFile: (path: string, content: string) => invoke<void>("write_text_file", { path, content }),
  writeBinaryFile: (path: string, data: number[]) => invoke<void>("write_binary_file", { path, data }),
  setTrayLanguage: (en: boolean) => invoke<void>("set_tray_language", { en }),
  setAppWatch: (enabled: boolean) => invoke<void>("set_app_watch", { enabled }),
  updateTraySpeed: (mbps: number) => invoke<void>("update_tray_speed", { mbps }),
  resetTrayIcon: () => invoke<void>("reset_tray_icon"),
  fetchText: (url: string) => invoke<string>("fetch_text", { url }),
  isPortFree: (port: number) => invoke<boolean>("is_port_free", { port }),
  suggestFreePort: (start: number) => invoke<number>("suggest_free_port", { start }),
  checkUpdate: () => invoke<UpdateInfo>("check_update"),
  downloadAndInstall: (url: string) => invoke<void>("download_and_install", { url }),
  setHudEnabled: (enabled: boolean) => invoke<void>("set_hud_enabled", { enabled }),
  hideToTray: () => invoke<void>("hide_to_tray"),
  restoreMain: () => invoke<void>("restore_main"),
  /** 在系统文件管理器中打开本地日志文件夹 */
  openLogDir: () => invoke<void>("open_log_dir"),
};

export interface HudConfig {
  opacity: number;
  locked: boolean;
  unit: string;
  showDown: boolean;
  showUp: boolean;
  showConns: boolean;
  showNics: boolean;
  accent: string;
  accentSoft: string;
  theme: string;
  clickThrough: boolean;
}

/** 主窗口推送 HUD 配置；HUD 窗口订阅以实时应用 */
export const emitHudConfig = (cfg: HudConfig) => emit("hmx-hud-config", cfg);
export const onHudConfig = (cb: (cfg: HudConfig) => void): Promise<UnlistenFn> =>
  listen<HudConfig>("hmx-hud-config", (e) => cb(e.payload));
/** 主窗口请求 HUD 吸附到指定角落 */
export const emitHudSnap = (corner: string) => emit("hmx-hud-snap", corner);
export const onHudSnap = (cb: (corner: string) => void): Promise<UnlistenFn> =>
  listen<string>("hmx-hud-snap", (e) => cb(e.payload));

// ---- 事件订阅 ----
export const onLog = (cb: (line: string) => void): Promise<UnlistenFn> =>
  listen<string>("hmx-log", (e) => cb(e.payload));

export const onTelemetry = (cb: (p: TelemetryPayload) => void): Promise<UnlistenFn> =>
  listen<TelemetryPayload>("hmx-telemetry", (e) => cb(e.payload));

export const onBoostState = (cb: (running: boolean) => void): Promise<UnlistenFn> =>
  listen<boolean>("hmx-boost-state", (e) => cb(e.payload));

export const onConnections = (cb: (c: ConnInfo[]) => void): Promise<UnlistenFn> =>
  listen<ConnInfo[]>("hmx-connections", (e) => cb(e.payload));

export const onSpeedTest = (cb: (r: SpeedResult) => void): Promise<UnlistenFn> =>
  listen<SpeedResult>("hmx-speedtest", (e) => cb(e.payload));

export const onTrayToggle = (cb: () => void): Promise<UnlistenFn> =>
  listen("hmx-tray-toggle", () => cb());
/** HUD 触发一键加速 / 停止（复用主窗口的切换流程） */
export const emitTrayToggle = () => emit("hmx-tray-toggle");

/** 主窗口把提示同步推送给 HUD（托盘模式下主窗口不可见时仍能反馈） */
export const emitHudNotice = (kind: string, msg: string) => emit("hmx-hud-notice", { kind, msg });
export const onHudNotice = (cb: (n: { kind: string; msg: string }) => void): Promise<UnlistenFn> =>
  listen<{ kind: string; msg: string }>("hmx-hud-notice", (e) => cb(e.payload));

export const onNicAlert = (cb: (a: { name: string; alive: boolean }) => void): Promise<UnlistenFn> =>
  listen<{ name: string; alive: boolean }>("hmx-nic-alert", (e) => cb(e.payload));

/** 进程感知自动加速：后端检测到/退出下载类应用时推送 boost=true/false */
export const onAutoBoost = (cb: (boost: boolean) => void): Promise<UnlistenFn> =>
  listen<boolean>("hmx-autoboost", (e) => cb(e.payload));

/** CLI 控制：第二个实例转发的命令（start/stop/toggle） */
export const onCli = (cb: (action: string) => void): Promise<UnlistenFn> =>
  listen<string>("hmx-cli", (e) => cb(e.payload));

export const onConnClosed = (cb: (c: ConnInfo) => void): Promise<UnlistenFn> =>
  listen<ConnInfo>("hmx-conn-closed", (e) => cb(e.payload));

/** 应用内更新下载进度：percent 为 0~100，total=0 表示服务器未提供长度 */
export interface UpdateProgress {
  downloaded: number;
  total: number;
  percent: number;
}
export const onUpdateProgress = (cb: (p: UpdateProgress) => void): Promise<UnlistenFn> =>
  listen<UpdateProgress>("hmx-update-progress", (e) => cb(e.payload));

// ---- 窗口控制 ----
export const win = {
  minimize: () => getCurrentWindow().minimize(),
  toggleMaximize: () => getCurrentWindow().toggleMaximize(),
  close: () => getCurrentWindow().close(),
  hide: () => getCurrentWindow().hide(),
  startDragging: () => getCurrentWindow().startDragging(),
  setAlwaysOnTop: (v: boolean) => getCurrentWindow().setAlwaysOnTop(v),
};
