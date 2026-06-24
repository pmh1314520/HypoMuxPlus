// 全局设置 / 国际化上下文（localStorage 持久化）
import { createContext, useContext, useEffect, useMemo, useState, type ReactNode } from "react";
import { Lang, translate } from "./i18n";

export type Theme = "dark" | "light";
export type SchedStrategy = "rr" | "least" | "weighted";
export type AccentKey = "blue" | "violet" | "emerald" | "amber" | "rose" | "cyan";

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
  accent: AccentKey;
  socksPort: number;
  httpPort: number;
  closeToTray: boolean;
  autostart: boolean;
  launchMinimized: boolean;
  autoBoost: boolean;
  strategy: SchedStrategy;
  globalHotkey: boolean;
  notifications: boolean;
  hotkeyCombo: string;
  hotkeyStop: string;
  downLimit: number;
  bypassList: string;
}

const DEFAULTS: Settings = {
  lang: "zh",
  theme: "dark",
  autoTheme: false,
  accent: "blue",
  socksPort: 10800,
  httpPort: 10801,
  closeToTray: true,
  autostart: false,
  launchMinimized: false,
  autoBoost: false,
  strategy: "weighted",
  globalHotkey: false,
  notifications: false,
  hotkeyCombo: "Control+Alt+H",
  hotkeyStop: "Control+Alt+J",
  downLimit: 0,
  bypassList: "",
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
