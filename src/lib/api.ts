// 与 Rust 后端交互的类型化封装层
import { invoke } from "@tauri-apps/api/core";
import { emit, listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

export interface AdapterInfo {
  index: number;
  alias: string;
  ipv4: string;
  description: string;
  isUp: boolean;
}

export interface SelectedNic {
  index: number;
  name: string;
  ip: string;
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
  latencyMs: number;
  ok: boolean;
}

export interface SpeedResult {
  index: number;
  name: string;
  mbps: number;
  ok: boolean;
}

export interface ConnInfo {
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
  ) =>
    invoke<string>("start_boost", { nics, socksPort, httpPort, strategy, lang, downLimitMbps, bypass }),
  stopBoost: () => invoke<void>("stop_boost"),
  testLatency: (nics: SelectedNic[]) => invoke<LatencyResult[]>("test_latency", { nics }),
  speedTest: (nics: SelectedNic[], duration: number) =>
    invoke<SpeedResult[]>("speed_test", { nics, duration }),
  configureSteam: (enable: boolean, port: number) =>
    invoke<void>("configure_steam", { enable, port }),
  configureIdm: (enable: boolean, port: number) => invoke<void>("configure_idm", { enable, port }),
  readTextFile: (path: string) => invoke<string>("read_text_file", { path }),
  writeTextFile: (path: string, content: string) => invoke<void>("write_text_file", { path, content }),
  writeBinaryFile: (path: string, data: number[]) => invoke<void>("write_binary_file", { path, data }),
  setTrayLanguage: (en: boolean) => invoke<void>("set_tray_language", { en }),
  isPortFree: (port: number) => invoke<boolean>("is_port_free", { port }),
  suggestFreePort: (start: number) => invoke<number>("suggest_free_port", { start }),
  checkUpdate: () => invoke<UpdateInfo>("check_update"),
  downloadAndInstall: (url: string) => invoke<void>("download_and_install", { url }),
  setHudEnabled: (enabled: boolean) => invoke<void>("set_hud_enabled", { enabled }),
  hideToTray: () => invoke<void>("hide_to_tray"),
  restoreMain: () => invoke<void>("restore_main"),
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

export const onConnClosed = (cb: (c: ConnInfo) => void): Promise<UnlistenFn> =>
  listen<ConnInfo>("hmx-conn-closed", (e) => cb(e.payload));

// ---- 窗口控制 ----
export const win = {
  minimize: () => getCurrentWindow().minimize(),
  toggleMaximize: () => getCurrentWindow().toggleMaximize(),
  close: () => getCurrentWindow().close(),
  hide: () => getCurrentWindow().hide(),
  startDragging: () => getCurrentWindow().startDragging(),
  setAlwaysOnTop: (v: boolean) => getCurrentWindow().setAlwaysOnTop(v),
};
