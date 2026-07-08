// 全局设置 / 国际化上下文（localStorage 持久化）
import { createContext, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { Lang, translate } from "./i18n";

export type Theme = "dark" | "light";
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
};

const STORAGE_KEY = "hmx-plus-settings";

function load(): Settings {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) return { ...DEFAULTS, ...JSON.parse(raw) };
  } catch {
    /* ignore */
  }
  return DEFAULTS;
}

interface Ctx extends Settings {
  set: <K extends keyof Settings>(key: K, value: Settings[K]) => void;
  t: (key: string, vars?: Record<string, string | number>) => string;
}

const SettingsCtx = createContext<Ctx | null>(null);

export function SettingsProvider({ children }: { children: ReactNode }) {
  const [settings, setSettings] = useState<Settings>(load);
  const firstRun = useRef(true);

  // 主题 / 强调色 / 对比度切换时短暂启用过渡动画，使配色变化平滑
  useEffect(() => {
    if (firstRun.current) {
      firstRun.current = false;
      return;
    }
    const root = document.documentElement;
    root.classList.add("theme-anim");
    const id = window.setTimeout(() => root.classList.remove("theme-anim"), 340);
    return () => window.clearTimeout(id);
  }, [settings.theme, settings.accent, settings.highContrast]);

  const set = <K extends keyof Settings>(key: K, value: Settings[K]) =>
    setSettings((s) => ({ ...s, [key]: value }));

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

  const value = useMemo<Ctx>(
    () => ({
      ...settings,
      set,
      t: (key, vars) => translate(settings.lang, key, vars),
    }),
    [settings],
  );

  return <SettingsCtx.Provider value={value}>{children}</SettingsCtx.Provider>;
}

export function useSettings(): Ctx {
  const ctx = useContext(SettingsCtx);
  if (!ctx) throw new Error("useSettings must be used within SettingsProvider");
  return ctx;
}
