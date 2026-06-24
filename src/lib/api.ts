// 与 Rust 后端交互的类型化封装层
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
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
  startBoost: (nics: SelectedNic[], socksPort: number, httpPort: number, strategy: string, lang: string) =>
    invoke<string>("start_boost", { nics, socksPort, httpPort, strategy, lang }),
  stopBoost: () => invoke<void>("stop_boost"),
  testLatency: (nics: SelectedNic[]) => invoke<LatencyResult[]>("test_latency", { nics }),
  speedTest: (nics: SelectedNic[], duration: number) =>
    invoke<SpeedResult[]>("speed_test", { nics, duration }),
  configureSteam: (enable: boolean, port: number) =>
    invoke<void>("configure_steam", { enable, port }),
  configureIdm: (enable: boolean, port: number) => invoke<void>("configure_idm", { enable, port }),
};

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

// ---- 窗口控制 ----
export const win = {
  minimize: () => getCurrentWindow().minimize(),
  toggleMaximize: () => getCurrentWindow().toggleMaximize(),
  close: () => getCurrentWindow().close(),
  hide: () => getCurrentWindow().hide(),
  startDragging: () => getCurrentWindow().startDragging(),
};
