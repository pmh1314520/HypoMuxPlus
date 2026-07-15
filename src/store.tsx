// 全局设置 / 国际化上下文（localStorage 持久化）
import { createContext, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { flushSync } from "react-dom";
import { Lang, translate } from "./i18n";
import type { UpstreamProxy, UpstreamBinding } from "./lib/upstream";
import type { HealthCfg, PerNicDnsCfg } from "./lib/api";

export type Theme = "dark" | "light";

/** 支持 View Transitions API 的 Document（用于主题切换水波纹扩散动效）。 */
type DocumentWithVT = Document & {
  startViewTransition?: (cb: () => void | Promise<void>) => { ready: Promise<void> };
};

/**
 * 记录最近一次指针按下坐标，作为主题切换涟漪的发散原点。
 * 用捕获阶段监听，确保在任何 onClick 之前就已更新为最新坐标。
 */
const lastPointer = { x: 0, y: 0 };
if (typeof window !== "undefined") {
  lastPointer.x = window.innerWidth / 2;
  lastPointer.y = window.innerHeight / 2;
  window.addEventListener(
    "pointerdown",
    (e) => {
      lastPointer.x = e.clientX;
      lastPointer.y = e.clientY;
    },
    true,
  );
}
export type SchedStrategy = "rr" | "least" | "weighted";
export type AccentKey = "blue" | "violet" | "emerald" | "amber" | "rose" | "cyan";
export type HudUnit = "mbps" | "mbit";

export const ACCENTS: Record<AccentKey, { accent: string; deep: string; soft: string; glow: string }> = {
  blue: { accent: "#3b82f6", deep: "#2563eb", soft: "#6ea8ff", glow: "rgba(59,130,246,0.25)" },
  violet: { accent: "#8b5cf6", deep: "#7c3aed", soft: "#a78bfa", glow: "rgba(139,92,246,0.25)" },
  emerald: { accent: "#10b981", deep: "#059669", soft: "#34d399", glow: "rgba(16,185,129,0.25)" },
  amber: { accent: "#f59e0b", deep: "#d97706", soft: "#fbbf24", glow: "rgba(245,158,11,0.25)" },
  rose: { accent: "#f43f5e", deep: "#e11d48", soft: "#fb7185", glow: "rgba(244,63,94,0.25)" },
  cyan: { accent: "#06b6d4", deep: "#0891b2", soft: "#22d3ee", glow: "rgba(6,182,212,0.25)" },
};

interface Settings {
  lang: Lang;
  theme: Theme;
  autoTheme: boolean;
  highContrast: boolean;
  accent: AccentKey;
  socksPort: number;
  httpPort: number;
  closeToTray: boolean;
  autostart: boolean;
  launchMinimized: boolean;
  autoBoost: boolean;
  autoBoostOnApp: boolean;
  strategy: SchedStrategy;
  globalHotkey: boolean;
  notifications: boolean;
  hotkeyCombo: string;
  hotkeyStop: string;
  downLimit: number;
  bypassList: string;
  tunMode: boolean;
  ipVersion: "auto" | "v4first" | "v6first" | "v4only";
  udpAssociate: boolean;
  /** 上游代理链总开关（默认关闭，未启用时行为与既有直连聚合完全一致） */
  upstreamChain: boolean;
  /** 上游全部不可用时的回退策略：回退直连 / 失败 */
  upstreamFallback: "direct" | "fail";
  /** 上游健康探测与加权优选配置（默认 enabled=false，零回归） */
  healthCfg: HealthCfg;
  /** 活跃中继连接数上限（Connection_Cap，默认 4096） */
  connCap: number;
  /** 后台任务并发数上限（Task_Cap，默认 64） */
  taskCap: number;
  /** 系统代理防泄漏看门狗开关（默认开启，正常路径行为与既有等价） */
  proxyGuardian: boolean;
  /** 是否接管系统代理（默认开启：一键加速自动写入 Windows 系统代理；关闭则仅开本地
   *  SOCKS/HTTP 监听端口、不改系统代理，需手动在工具中配置代理或改用 TUN 模式） */
  systemProxy: boolean;
  alwaysOnTop: boolean;
  hudEnabled: boolean;
  hudOpacity: number;
  hudLocked: boolean;
  hudUnit: HudUnit;
  hudShowDown: boolean;
  hudShowUp: boolean;
  hudShowConns: boolean;
  hudShowNics: boolean;
  hudClickThrough: boolean;
  sessionReport: boolean;
  /** 网卡矩阵显示过滤：全部 / 仅物理 / 仅虚拟（仅影响展示，不影响后端调度） */
  nicFilter: "all" | "physical" | "virtual";
}

const DEFAULTS: Settings = {
  lang: "zh",
  theme: "dark",
  autoTheme: false,
  highContrast: false,
  accent: "blue",
  socksPort: 10800,
  httpPort: 10801,
  closeToTray: true,
  autostart: false,
  launchMinimized: false,
  autoBoost: false,
  autoBoostOnApp: false,
  strategy: "weighted",
  globalHotkey: false,
  notifications: false,
  hotkeyCombo: "Control+Alt+H",
  hotkeyStop: "Control+Alt+J",
  downLimit: 0,
  bypassList: "",
  tunMode: false,
  ipVersion: "auto",
  udpAssociate: false,
  upstreamChain: false,
  upstreamFallback: "direct",
  healthCfg: {
    enabled: false,
    intervalMs: 30000,
    timeoutMs: 5000,
    failThreshold: 3,
    cooldownMs: 60000,
  },
  connCap: 4096,
  taskCap: 64,
  proxyGuardian: true,
  systemProxy: true,
  alwaysOnTop: false,
  hudEnabled: false,
  hudOpacity: 0.92,
  hudLocked: false,
  hudUnit: "mbps",
  hudShowDown: true,
  hudShowUp: true,
  hudShowConns: true,
  hudShowNics: false,
  hudClickThrough: false,
  sessionReport: true,
  nicFilter: "all",
};

const STORAGE_KEY = "hmx-plus-settings";
// 上游代理链的节点列表与网卡↔上游映射作为独立持久化状态（各自单独的 localStorage key）
const UPSTREAMS_KEY = "hmx-upstreams";
const UPSTREAM_BINDINGS_KEY = "hmx-upstream-bindings";
// 每网卡 DNS / DoH 映射作为独立持久化状态（单独的 localStorage key）
const PER_NIC_DNS_KEY = "hmx-per-nic-dns";

function load(): Settings {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) return { ...DEFAULTS, ...JSON.parse(raw) };
  } catch {
    /* ignore */
  }
  return DEFAULTS;
}

function loadUpstreams(): UpstreamProxy[] {
  try {
    const raw = localStorage.getItem(UPSTREAMS_KEY);
    if (raw) {
      const arr = JSON.parse(raw);
      if (Array.isArray(arr))
        return arr.filter(
          (u) =>
            u &&
            typeof u.id === "string" &&
            (u.kind === "socks5" || u.kind === "http") &&
            typeof u.host === "string" &&
            typeof u.port === "number",
        );
    }
  } catch {
    /* ignore */
  }
  return [];
}

function loadUpstreamBindings(): UpstreamBinding[] {
  try {
    const raw = localStorage.getItem(UPSTREAM_BINDINGS_KEY);
    if (raw) {
      const arr = JSON.parse(raw);
      if (Array.isArray(arr))
        return arr.filter(
          (b) => b && typeof b.ifIndex === "number" && Array.isArray(b.upstreamIds),
        );
    }
  } catch {
    /* ignore */
  }
  return [];
}

function loadPerNicDns(): PerNicDnsCfg[] {
  try {
    const raw = localStorage.getItem(PER_NIC_DNS_KEY);
    if (raw) {
      const arr = JSON.parse(raw);
      if (Array.isArray(arr))
        return arr.filter(
          (d) =>
            d &&
            typeof d.ifIndex === "number" &&
            (d.kind === "plain" || d.kind === "doh") &&
            typeof d.endpoint === "string",
        );
    }
  } catch {
    /* ignore */
  }
  return [];
}

interface Ctx extends Settings {
  set: <K extends keyof Settings>(key: K, value: Settings[K]) => void;
  /**
   * 切换主题并附带「水波纹扩散」动效：以最近点击处为圆心，用 View Transitions API
   * 从半径 0 向外扩散揭示新主题（返回旧主题时对称收回）；同时关闭跟随系统主题。
   * 浏览器不支持 View Transitions 或用户偏好减少动效时，回退为既有平滑过渡切换。
   */
  setThemeAnimated: (next: Theme) => void;
  t: (key: string, vars?: Record<string, string | number>) => string;
  /** 上游代理链节点列表（持久化于 localStorage key `hmx-upstreams`）。 */
  upstreams: UpstreamProxy[];
  setUpstreams: (upstreams: UpstreamProxy[]) => void;
  /** 网卡↔上游映射（持久化于 localStorage key `hmx-upstream-bindings`）。 */
  upstreamBindings: UpstreamBinding[];
  setUpstreamBindings: (bindings: UpstreamBinding[]) => void;
  /** 每网卡 DNS / DoH 映射（持久化于 localStorage key `hmx-per-nic-dns`）。 */
  perNicDns: PerNicDnsCfg[];
  setPerNicDns: (dns: PerNicDnsCfg[]) => void;
}

const SettingsCtx = createContext<Ctx | null>(null);

export function SettingsProvider({ children }: { children: ReactNode }) {
  const [settings, setSettings] = useState<Settings>(load);
  const [upstreams, setUpstreams] = useState<UpstreamProxy[]>(loadUpstreams);
  const [upstreamBindings, setUpstreamBindings] = useState<UpstreamBinding[]>(loadUpstreamBindings);
  const [perNicDns, setPerNicDns] = useState<PerNicDnsCfg[]>(loadPerNicDns);
  const firstRun = useRef(true);
  // 本次主题变更是否由 View Transitions 涟漪驱动：若是则跳过 theme-anim 的 CSS 过渡，
  // 避免过渡使 VT 快照捕获到「过渡起点的旧配色」，导致涟漪揭示不出新主题。
  const skipAnimRef = useRef(false);

  // 主题 / 强调色 / 对比度切换时短暂启用过渡动画，使配色变化平滑
  useEffect(() => {
    if (firstRun.current) {
      firstRun.current = false;
      return;
    }
    // VT 涟漪切换：本次跳过 CSS 过渡（涟漪本身已提供动效），并复位标志
    if (skipAnimRef.current) {
      skipAnimRef.current = false;
      return;
    }
    const root = document.documentElement;
    root.classList.add("theme-anim");
    const id = window.setTimeout(() => root.classList.remove("theme-anim"), 340);
    return () => window.clearTimeout(id);
  }, [settings.theme, settings.accent, settings.highContrast]);

  const set = <K extends keyof Settings>(key: K, value: Settings[K]) =>
    setSettings((s) => ({ ...s, [key]: value }));

  // 主题切换水波纹扩散：以最近点击处为圆心，用 View Transitions + clip-path 揭示新主题
  const setThemeAnimated = (next: Theme) => {
    const applyNow = () => setSettings((s) => ({ ...s, theme: next, autoTheme: false }));
    const doc = document as DocumentWithVT;
    const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    // 不支持 View Transitions 或用户偏好减少动效：回退为既有平滑过渡切换
    if (typeof doc.startViewTransition !== "function" || reduceMotion) {
      applyNow();
      return;
    }
    const { x, y } = lastPointer;
    // 涟漪终止半径 = 圆心到四角的最大距离，确保扩散能覆盖整个视口
    const endRadius = Math.hypot(
      Math.max(x, window.innerWidth - x),
      Math.max(y, window.innerHeight - y),
    );
    // 跳过本次 theme-anim 的 CSS 过渡，让 VT「新」快照即刻捕获最终配色
    skipAnimRef.current = true;
    const vt = doc.startViewTransition(() => flushSync(applyNow));
    vt.ready
      .then(() => {
        document.documentElement.animate(
          {
            clipPath: [
              `circle(0px at ${x}px ${y}px)`,
              `circle(${endRadius}px at ${x}px ${y}px)`,
            ],
          },
          {
            duration: 480,
            easing: "cubic-bezier(0.33, 1, 0.68, 1)",
            pseudoElement: "::view-transition-new(root)",
          },
        );
      })
      .catch(() => {
        // 动画启动失败不影响主题已切换的最终结果，复位标志避免影响后续切换
        skipAnimRef.current = false;
      });
  };

  // 跟随系统主题：开启后实时同步 Windows 深 / 浅色，并监听系统切换
  useEffect(() => {
    if (!settings.autoTheme) return;
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const apply = () =>
      setSettings((s) => (s.autoTheme ? { ...s, theme: mq.matches ? "dark" : "light" } : s));
    apply();
    mq.addEventListener("change", apply);
    return () => mq.removeEventListener("change", apply);
  }, [settings.autoTheme]);

  useEffect(() => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(settings));
    const root = document.documentElement;
    root.setAttribute("data-theme", settings.theme);
    root.setAttribute("data-contrast", settings.highContrast ? "on" : "off");
    root.lang = settings.lang === "zh" ? "zh-CN" : "en";
    const a = ACCENTS[settings.accent] ?? ACCENTS.blue;
    root.style.setProperty("--accent", a.accent);
    root.style.setProperty("--accent-deep", a.deep);
    root.style.setProperty("--accent-soft", a.soft);
    root.style.setProperty("--accent-glow", a.glow);
  }, [settings]);

  // 持久化上游代理链节点列表
  useEffect(() => {
    localStorage.setItem(UPSTREAMS_KEY, JSON.stringify(upstreams));
  }, [upstreams]);

  // 持久化网卡↔上游映射
  useEffect(() => {
    localStorage.setItem(UPSTREAM_BINDINGS_KEY, JSON.stringify(upstreamBindings));
  }, [upstreamBindings]);

  // 持久化每网卡 DNS / DoH 映射
  useEffect(() => {
    localStorage.setItem(PER_NIC_DNS_KEY, JSON.stringify(perNicDns));
  }, [perNicDns]);

  const value = useMemo<Ctx>(
    () => ({
      ...settings,
      set,
      setThemeAnimated,
      t: (key, vars) => translate(settings.lang, key, vars),
      upstreams,
      setUpstreams,
      upstreamBindings,
      setUpstreamBindings,
      perNicDns,
      setPerNicDns,
    }),
    [settings, upstreams, upstreamBindings, perNicDns],
  );

  return <SettingsCtx.Provider value={value}>{children}</SettingsCtx.Provider>;
}

export function useSettings(): Ctx {
  const ctx = useContext(SettingsCtx);
  if (!ctx) throw new Error("useSettings must be used within SettingsProvider");
  return ctx;
}
