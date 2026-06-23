import { motion } from "framer-motion";
import { Activity, Minus, Moon, Settings as SettingsIcon, Square, Sun, Waypoints, X } from "lucide-react";
import { useSettings } from "../store";
import { win } from "../lib/api";

export type View = "dashboard" | "settings";

interface Props {
  view: View;
  setView: (v: View) => void;
  running: boolean;
}

export function TitleBar({ view, setView, running }: Props) {
  const { t, theme, set } = useSettings();

  const tabs: { id: View; label: string; icon: typeof Activity }[] = [
    { id: "dashboard", label: t("navDashboard"), icon: Activity },
    { id: "settings", label: t("navSettings"), icon: SettingsIcon },
  ];

  return (
    <div
      data-tauri-drag-region
      className="flex items-center h-14 px-4 gap-4 shrink-0 select-none"
      style={{ borderBottom: "1px solid var(--border)" }}
    >
      {/* Logo + 标题 */}
      <div className="flex items-center gap-2.5 pointer-events-none">
        <div
          className="grid place-items-center w-9 h-9 rounded-xl"
          style={{
            background: "linear-gradient(135deg, var(--accent), var(--accent-soft))",
            boxShadow: "0 6px 18px var(--accent-glow)",
          }}
        >
          <Waypoints size={20} color="#fff" />
        </div>
        <div className="leading-none">
          <div className="font-bold text-[15px] tracking-wide">
            HypoMux <span style={{ color: "var(--accent-soft)" }}>Plus</span>
          </div>
          <div className="text-[10px] mt-1" style={{ color: "var(--text-2)" }}>
            {t("appSubtitle")}
          </div>
        </div>
      </div>

      {/* 运行态指示灯 */}
      <div className="flex items-center gap-1.5 pointer-events-none ml-1">
        <span
          className="w-2 h-2 rounded-full"
          style={{
            background: running ? "var(--ok)" : "var(--text-2)",
            boxShadow: running ? "0 0 8px var(--ok)" : "none",
          }}
        />
      </div>

      {/* 导航 Tabs */}
      <div className="flex items-center gap-1 ml-3">
        {tabs.map((tab) => {
          const active = view === tab.id;
          const Icon = tab.icon;
          return (
            <button
              key={tab.id}
              onClick={() => setView(tab.id)}
              className="relative flex items-center gap-1.5 px-3.5 py-1.5 rounded-lg text-[13px] font-medium transition-colors"
              style={{ color: active ? "var(--text-0)" : "var(--text-2)" }}
            >
              {active && (
                <motion.span
                  layoutId="navpill"
                  className="absolute inset-0 rounded-lg"
                  style={{ background: "var(--surface-strong)", border: "1px solid var(--border)" }}
                  transition={{ type: "spring", stiffness: 380, damping: 30 }}
                />
              )}
              <Icon size={15} className="relative z-10" />
              <span className="relative z-10">{tab.label}</span>
            </button>
          );
        })}
      </div>

      <div className="flex-1" />

      {/* 主题切换 */}
      <button
        onClick={() => set("theme", theme === "dark" ? "light" : "dark")}
        className="grid place-items-center w-8 h-8 rounded-lg transition-colors hover:[background:var(--surface-hover)]"
        style={{ color: "var(--text-1)" }}
        title="Theme"
      >
        {theme === "dark" ? <Sun size={16} /> : <Moon size={16} />}
      </button>

      {/* 窗口控制 */}
      <div className="flex items-center gap-0.5 ml-1">
        <WinBtn onClick={() => win.minimize()}>
          <Minus size={15} />
        </WinBtn>
        <WinBtn onClick={() => win.toggleMaximize()}>
          <Square size={12} />
        </WinBtn>
        <WinBtn danger onClick={() => win.close()}>
          <X size={15} />
        </WinBtn>
      </div>
    </div>
  );
}

function WinBtn({
  children,
  onClick,
  danger,
}: {
  children: React.ReactNode;
  onClick: () => void;
  danger?: boolean;
}) {
  return (
    <button
      onClick={onClick}
      className="grid place-items-center w-8 h-8 rounded-lg transition-colors"
      style={{ color: "var(--text-1)" }}
      onMouseEnter={(e) =>
        (e.currentTarget.style.background = danger ? "var(--danger)" : "var(--surface-hover)")
      }
      onMouseLeave={(e) => (e.currentTarget.style.background = "transparent")}
    >
      {children}
    </button>
  );
}
