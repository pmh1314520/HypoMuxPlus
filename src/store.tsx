// 全局设置 / 国际化上下文（localStorage 持久化）
import { createContext, useContext, useEffect, useMemo, useState, type ReactNode } from "react";
import { Lang, translate } from "./i18n";

export type Theme = "dark" | "light";

interface Settings {
  lang: Lang;
  theme: Theme;
  socksPort: number;
  httpPort: number;
  closeToTray: boolean;
  autostart: boolean;
  launchMinimized: boolean;
  autoBoost: boolean;
}

const DEFAULTS: Settings = {
  lang: "zh",
  theme: "dark",
  socksPort: 10800,
  httpPort: 10801,
  closeToTray: true,
  autostart: false,
  launchMinimized: false,
  autoBoost: false,
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

  useEffect(() => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(settings));
    document.documentElement.setAttribute("data-theme", settings.theme);
    document.documentElement.lang = settings.lang === "zh" ? "zh-CN" : "en";
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
