import { useEffect, useRef, useState } from "react";
import { motion } from "framer-motion";
import { ListTree, Terminal, Trash2 } from "lucide-react";
import { useSettings } from "../store";
import { Tooltip } from "./Tooltip";
import type { ConnInfo } from "../lib/api";

interface Props {
  logs: string[];
  clearLogs: () => void;
  connections: ConnInfo[];
  running: boolean;
}

function lineColor(line: string): string {
  if (line.includes("失败") || line.includes("异常") || line.includes("failed") || line.includes("Error"))
    return "var(--danger)";
  if (line.includes("调度") || line.includes("dispatch")) return "var(--accent-soft)";
  if (line.includes("启动") || line.includes("started") || line.includes("HypoMux")) return "var(--ok)";
  return "var(--text-1)";
}

export function MonitorPanel({ logs, clearLogs, connections, running }: Props) {
  const { t } = useSettings();
  const [tab, setTab] = useState<"log" | "conns">("log");
  const logRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (tab === "log" && logRef.current) logRef.current.scrollTop = logRef.current.scrollHeight;
  }, [logs, tab]);

  const tabs: { id: "log" | "conns"; label: string; icon: typeof Terminal; badge?: number }[] = [
    { id: "log", label: t("monitorLog"), icon: Terminal },
    { id: "conns", label: t("monitorConns"), icon: ListTree, badge: connections.length },
  ];

  return (
    <div className="glass flex flex-col overflow-hidden" style={{ boxShadow: "var(--shadow)" }}>
      <div className="panel-head flex items-center gap-1 px-3 py-2 shrink-0">
        {tabs.map((tb) => {
          const active = tab === tb.id;
          const Icon = tb.icon;
          return (
            <button
              key={tb.id}
              onClick={() => setTab(tb.id)}
              className="relative flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[12.5px] font-medium transition-colors"
              style={{ color: active ? "var(--text-0)" : "var(--text-2)" }}
            >
              {active && (
                <motion.span
                  layoutId="monitor-tab"
                  className="absolute inset-0 rounded-lg"
                  style={{ background: "var(--surface-2)", border: "1px solid var(--border)" }}
                  transition={{ type: "spring", stiffness: 400, damping: 32 }}
                />
              )}
              <Icon size={14} className="relative z-10" />
              <span className="relative z-10">{tb.label}</span>
              {tb.badge ? (
                <span
                  className="relative z-10 mono text-[10px] px-1.5 rounded-full"
                  style={{ background: "var(--accent)", color: "#fff" }}
                >
                  {tb.badge}
                </span>
              ) : null}
            </button>
          );
        })}
        <div className="flex-1" />
        {tab === "log" && (
          <Tooltip label={t("consoleClear")} placement="left">
            <button
              onClick={clearLogs}
              className="grid place-items-center w-7 h-7 rounded-lg transition-colors hover:[background:var(--surface-hover)]"
              style={{ color: "var(--text-2)" }}
            >
              <Trash2 size={14} />
            </button>
          </Tooltip>
        )}
      </div>

      {tab === "log" ? (
        <div
          ref={logRef}
          className="flex-1 overflow-y-auto px-4 py-3 font-mono text-[11.5px] leading-[1.7] space-y-0.5"
        >
          {logs.length === 0 ? (
            <div className="grid place-items-center h-full" style={{ color: "var(--text-2)" }}>
              {t("consoleEmpty")}
            </div>
          ) : (
            logs.map((line, i) => (
              <div key={i} style={{ color: lineColor(line) }} className="break-all">
                {line}
              </div>
            ))
          )}
        </div>
      ) : (
        <div className="flex-1 overflow-y-auto px-3 py-2">
          {!running || connections.length === 0 ? (
            <div className="grid place-items-center h-full text-[12.5px]" style={{ color: "var(--text-2)" }}>
              {t("connEmpty")}
            </div>
          ) : (
            connections.map((c, i) => (
              <div
                key={i}
                className="flex items-center gap-2 px-2.5 py-1.5 rounded-lg"
                style={{ borderBottom: "1px solid var(--border)" }}
              >
                <span
                  className="mono text-[9px] px-1.5 py-0.5 rounded shrink-0"
                  style={{
                    background: c.proto === "SOCKS" ? "rgba(59,130,246,0.14)" : "rgba(34,197,94,0.14)",
                    color: c.proto === "SOCKS" ? "var(--accent-soft)" : "var(--ok)",
                  }}
                >
                  {c.proto}
                </span>
                <span className="mono text-[11.5px] truncate flex-1" style={{ color: "var(--text-1)" }}>
                  {c.target}
                </span>
                <span className="text-[11px] font-medium shrink-0" style={{ color: "var(--accent-soft)" }}>
                  {c.nic}
                </span>
              </div>
            ))
          )}
        </div>
      )}
    </div>
  );
}
