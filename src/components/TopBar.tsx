import { Minus, Pin, PinOff, RefreshCw, Square, X } from "lucide-react";
import { useSettings } from "../store";
import { win } from "../lib/api";
import { Tooltip } from "./Tooltip";
import type { View } from "./shell-types";

interface Props {
  view: View;
  running: boolean;
  loading: boolean;
  onRefresh: () => void;
}

export function TopBar({ view, running, loading, onRefresh }: Props) {
  const { t, alwaysOnTop, set } = useSettings();

  const titleMap: Record<View, string> = {
    dashboard: t("navDashboard"),
    stats: t("navStats"),
    diagnostics: t("navDiagnostics"),
    tutorial: t("navTutorial"),
    settings: t("settingsTitle"),
    about: t("navAbout"),
  };
  const descMap: Record<View, string> = {
    dashboard: t("topDashDesc"),
    stats: t("topStatsDesc"),
    diagnostics: t("topDiagDesc"),
    tutorial: t("topTutorialDesc"),
    settings: t("topSettingsDesc"),
    about: t("topAboutDesc"),
  };
  const title = titleMap[view];
  const desc = descMap[view];

  return (
    <div data-tauri-drag-region className="flex items-center h-[58px] px-5 shrink-0 gap-4">
      <div className="leading-none pointer-events-none">
        <div className="flex items-center gap-2.5">
          <h1 className="text-[16px] font-bold tracking-tight">{title}</h1>
          {view === "dashboard" && (
            <span
              className="flex items-center gap-1.5 px-2 py-0.5 rounded-full text-[10.5px] font-semibold"
              style={{
                background: running ? "rgba(54,211,153,0.12)" : "var(--surface-2)",
                color: running ? "var(--ok)" : "var(--text-2)",
                border: `1px solid ${running ? "rgba(54,211,153,0.25)" : "var(--border)"}`,
              }}
            >
              <span
                className={`w-1.5 h-1.5 rounded-full ${running ? "live-dot" : ""}`}
                style={{ background: running ? "var(--ok)" : "var(--text-2)" }}
              />
              {running ? t("stateActive") : t("stateIdle")}
            </span>
          )}
        </div>
        <div className="text-[11px] mt-1.5" style={{ color: "var(--text-2)" }}>
          {desc}
        </div>
      </div>

      <div className="flex-1" />

      <Tooltip label={alwaysOnTop ? t("tipUnpin") : t("tipPin")} placement="bottom">
        <button
          onClick={() => set("alwaysOnTop", !alwaysOnTop)}
          className="grid place-items-center w-8 h-8 rounded-lg transition-colors hover:[background:var(--surface-hover)]"
          style={{ color: alwaysOnTop ? "var(--accent-soft)" : "var(--text-1)" }}
        >
          {alwaysOnTop ? <Pin size={15} /> : <PinOff size={15} />}
        </button>
      </Tooltip>

      {view === "dashboard" && (
        <Tooltip label={t("tipRefresh")} placement="bottom">
          <button
            onClick={onRefresh}
            disabled={running}
            className="grid place-items-center w-8 h-8 rounded-lg transition-colors hover:[background:var(--surface-hover)]"
            style={{ color: "var(--text-1)", opacity: running ? 0.4 : 1, cursor: running ? "not-allowed" : "pointer" }}
          >
            <RefreshCw size={15} className={loading ? "animate-spin" : ""} />
          </button>
        </Tooltip>
      )}

      {/* 窗口控制 */}
      <div className="flex items-center gap-0.5">
        <WinBtn onClick={() => win.minimize()} label={t("tipMinimize")}>
          <Minus size={15} />
        </WinBtn>
        <WinBtn onClick={() => win.toggleMaximize()} label={t("tipMaximize")}>
          <Square size={12} />
        </WinBtn>
        <WinBtn danger onClick={() => win.close()} label={t("tipClose")}>
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
  label,
}: {
  children: React.ReactNode;
  onClick: () => void;
  danger?: boolean;
  label: string;
}) {
  return (
    <Tooltip label={label} placement="bottom">
      <button
        onClick={onClick}
        className="grid place-items-center w-8 h-8 rounded-lg transition-colors"
        style={{ color: "var(--text-1)" }}
        onMouseEnter={(e) => (e.currentTarget.style.background = danger ? "var(--danger)" : "var(--surface-hover)")}
        onMouseLeave={(e) => (e.currentTarget.style.background = "transparent")}
      >
        {children}
      </button>
    </Tooltip>
  );
}
