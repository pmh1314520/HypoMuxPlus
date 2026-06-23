import { motion } from "framer-motion";
import { Activity, BarChart3, BookOpen, Info, Moon, Settings as SettingsIcon, Stethoscope, Sun } from "lucide-react";
import { useSettings } from "../store";
import { Logo } from "./Logo";
import type { View } from "./shell-types";

interface Props {
  view: View;
  setView: (v: View) => void;
  running: boolean;
}

export function Sidebar({ view, setView, running }: Props) {
  const { t, theme, set } = useSettings();

  const nav: { id: View; label: string; icon: typeof Activity }[] = [
    { id: "dashboard", label: t("navDashboard"), icon: Activity },
    { id: "stats", label: t("navStats"), icon: BarChart3 },
    { id: "diagnostics", label: t("navDiagnostics"), icon: Stethoscope },
    { id: "tutorial", label: t("navTutorial"), icon: BookOpen },
    { id: "settings", label: t("navSettings"), icon: SettingsIcon },
    { id: "about", label: t("navAbout"), icon: Info },
  ];

  return (
    <aside
      className="flex flex-col w-[230px] shrink-0"
      style={{ background: "var(--rail)", borderRight: "1px solid var(--border)" }}
    >
      {/* 品牌区（可拖拽） */}
      <div data-tauri-drag-region className="flex items-center gap-3 px-5 h-[58px] shrink-0">
        <div className="relative pointer-events-none">
          <Logo size={38} />
          {running && (
            <span
              className="absolute -top-0.5 -right-0.5 w-2.5 h-2.5 rounded-full live-dot"
              style={{ background: "var(--ok)", boxShadow: "0 0 8px var(--ok)", border: "2px solid var(--bg-0)" }}
            />
          )}
        </div>
        <div className="leading-none pointer-events-none">
          <div className="font-bold text-[15px] tracking-tight">
            HypoMux<span style={{ color: "var(--cyan)" }}>Plus</span>
          </div>
          <div className="text-[9.5px] mt-1 tracking-[0.12em] uppercase" style={{ color: "var(--text-2)" }}>
            Network Console
          </div>
        </div>
      </div>

      <div className="h-px mx-4 my-1" style={{ background: "var(--border)" }} />

      {/* 导航 */}
      <nav className="flex flex-col gap-1 px-3 py-3">
        <span className="px-3 pb-1.5 text-[10px] tracking-[0.14em] uppercase" style={{ color: "var(--text-2)" }}>
          {t("navSection")}
        </span>
        {nav.map((item) => {
          const active = view === item.id;
          const Icon = item.icon;
          return (
            <button
              key={item.id}
              onClick={() => setView(item.id)}
              className="relative flex items-center gap-3 px-3 py-2.5 rounded-xl text-[13.5px] font-medium transition-colors"
              style={{ color: active ? "var(--text-0)" : "var(--text-1)" }}
            >
              {active && (
                <motion.span
                  layoutId="rail-active"
                  className="absolute inset-0 rounded-xl"
                  style={{ background: "var(--surface-2)", border: "1px solid var(--border)" }}
                  transition={{ type: "spring", stiffness: 400, damping: 32 }}
                />
              )}
              {active && (
                <motion.span
                  layoutId="rail-bar"
                  className="absolute left-0 top-2 bottom-2 w-[3px] rounded-full"
                  style={{ background: "linear-gradient(var(--accent), var(--cyan))" }}
                  transition={{ type: "spring", stiffness: 400, damping: 32 }}
                />
              )}
              <Icon size={17} className="relative z-10" style={{ color: active ? "var(--accent-soft)" : undefined }} />
              <span className="relative z-10">{item.label}</span>
            </button>
          );
        })}
      </nav>

      <div className="flex-1" />

      {/* 底部：主题切换 + 版本 */}
      <div className="px-4 py-4 flex flex-col gap-3">
        <button
          onClick={() => set("theme", theme === "dark" ? "light" : "dark")}
          className="flex items-center justify-between px-3 py-2 rounded-xl text-[12.5px] font-medium transition-colors hover:[background:var(--surface-hover)]"
          style={{ color: "var(--text-1)", border: "1px solid var(--border)" }}
        >
          <span className="flex items-center gap-2">
            {theme === "dark" ? <Moon size={15} /> : <Sun size={15} />}
            {theme === "dark" ? t("themeDark") : t("themeLight")}
          </span>
          <span
            className="relative w-8 h-[18px] rounded-full transition-colors"
            style={{ background: theme === "dark" ? "var(--surface-2)" : "var(--accent)" }}
          >
            <motion.span
              layout
              className="absolute top-[2px] w-3.5 h-3.5 rounded-full bg-white"
              style={{ left: theme === "dark" ? 3 : 16 }}
              transition={{ type: "spring", stiffness: 500, damping: 32 }}
            />
          </span>
        </button>
        <div className="text-[10px] text-center tracking-wide" style={{ color: "var(--text-2)" }}>
          v1.0.0 · AGPL-3.0
        </div>
      </div>
    </aside>
  );
}
