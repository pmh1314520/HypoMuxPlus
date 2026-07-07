import { useEffect, useRef, useState } from "react";
import { motion } from "framer-motion";
import { ClipboardCopy, History, Inbox, ListTree, Search, Terminal, Trash2 } from "lucide-react";
import { useSettings } from "../store";
import { Tooltip } from "./Tooltip";
import { copyText } from "../lib/clipboard";
import { useToast } from "./Toast";
import { EmptyState } from "./EmptyState";
import type { ConnInfo } from "../lib/api";
import type { ClosedConn } from "../App";

interface Props {
  logs: string[];
  clearLogs: () => void;
  connections: ConnInfo[];
  connHistory: ClosedConn[];
  clearHistory: () => void;
  running: boolean;
}

function lineColor(line: string): string {
  const l = line.toLowerCase();
  if (l.includes("失败") || l.includes("异常") || l.includes("failed") || l.includes("error"))
    return "var(--danger)";
  if (l.includes("调度") || l.includes("dispatch")) return "var(--accent-soft)";
  if (l.includes("启动") || l.includes("started") || l.includes("hypomux")) return "var(--ok)";
  return "var(--text-1)";
}

export function MonitorPanel({ logs, clearLogs, connections, connHistory, clearHistory, running }: Props) {
  const { t } = useSettings();
  const toast = useToast();
  const [tab, setTab] = useState<"log" | "conns" | "history">("log");
  const [nicFilter, setNicFilter] = useState<string | null>(null);
  const [protoFilter, setProtoFilter] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const logRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (tab === "log" && logRef.current) logRef.current.scrollTop = logRef.current.scrollHeight;
  }, [logs, tab]);

  const q = query.trim().toLowerCase();
  const match = (c: { proto: string; target: string; nic: string }) =>
    (!nicFilter || c.nic === nicFilter) &&
    (!protoFilter || c.proto === protoFilter) &&
    (!q || c.target.toLowerCase().includes(q));
  const filteredConns = connections.filter(match);
  const filteredHistory = connHistory.filter(match);
  const connNicNames = Array.from(new Set(connections.map((c) => c.nic)));
  const histNicNames = Array.from(new Set(connHistory.map((c) => c.nic)));

  const exportConns = async () => {
    if (filteredConns.length === 0) {
      toast("warning", t("connExportEmpty"));
      return;
    }
    const text = filteredConns.map((c) => `[${c.proto}] ${c.target} -> ${c.nic}`).join("\n");
    const ok = await copyText(text);
    toast(ok ? "success" : "error", t(ok ? "msgConnCopied" : "msgCopyFailed"));
  };

  const copyOne = async (target: string) => {
    const ok = await copyText(target);
    toast(ok ? "success" : "error", t(ok ? "msgCopied" : "msgCopyFailed"));
  };

  const tabs: { id: "log" | "conns" | "history"; label: string; icon: typeof Terminal; badge?: number }[] = [
    { id: "log", label: t("monitorLog"), icon: Terminal },
    { id: "conns", label: t("monitorConns"), icon: ListTree, badge: connections.length },
    { id: "history", label: t("monitorHistory"), icon: History, badge: connHistory.length },
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
        {tab === "conns" && (
          <Tooltip label={t("connExport")} placement="left">
            <button
              onClick={exportConns}
              className="grid place-items-center w-7 h-7 rounded-lg transition-colors hover:[background:var(--surface-hover)]"
              style={{ color: "var(--text-2)" }}
            >
              <ClipboardCopy size={14} />
            </button>
          </Tooltip>
        )}
        {tab === "history" && connHistory.length > 0 && (
          <Tooltip label={t("historyClear")} placement="left">
            <button
              onClick={clearHistory}
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
            <EmptyState icon={<Terminal size={20} />} text={t("consoleEmpty")} compact />
          ) : (
            logs.map((line, i) => (
              <div key={i} style={{ color: lineColor(line) }} className="break-all">
                {line}
              </div>
            ))
          )}
        </div>
      ) : tab === "history" ? (
        <div className="flex-1 min-h-0 flex flex-col">
          {connHistory.length > 0 && (
            <FilterBar
              nicNames={histNicNames}
              nicFilter={nicFilter}
              setNicFilter={setNicFilter}
              protoFilter={protoFilter}
              setProtoFilter={setProtoFilter}
              query={query}
              setQuery={setQuery}
              t={t}
            />
          )}
          <div className="flex-1 overflow-y-auto px-3 py-2">
            {connHistory.length === 0 ? (
              <EmptyState icon={<History size={20} />} text={t("historyEmpty")} compact />
            ) : filteredHistory.length === 0 ? (
              <EmptyState icon={<Search size={20} />} text={t("connNoMatch")} compact />
            ) : (
              filteredHistory.map((c) => (
                <div
                  key={`${c.at}-${c.id}`}
                  onClick={() => copyOne(c.target)}

                  className="flex items-center gap-2 px-2.5 py-1.5 rounded-lg cursor-pointer transition-colors hover:[background:var(--surface-hover)]"
                  style={{ borderBottom: "1px solid var(--border)" }}
                >
                  <span
                    className="mono text-[9px] px-1.5 py-0.5 rounded shrink-0"
                    style={{
                      background: c.proto === "SOCKS" ? "color-mix(in srgb, var(--accent) 14%, transparent)" : "rgba(34,197,94,0.14)",
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
                  <span className="mono text-[10px] shrink-0" style={{ color: "var(--text-2)" }}>
                    {new Date(c.at).toLocaleTimeString()}
                  </span>
                </div>
              ))
            )}
          </div>
        </div>
      ) : (
        <div className="flex-1 min-h-0 flex flex-col">
          {/* 过滤栏：按协议 / 出口网卡 + 目标搜索 */}
          {running && connections.length > 0 && (
            <FilterBar
              nicNames={connNicNames}
              nicFilter={nicFilter}
              setNicFilter={setNicFilter}
              protoFilter={protoFilter}
              setProtoFilter={setProtoFilter}
              query={query}
              setQuery={setQuery}
              t={t}
            />
          )}
          <div className="flex-1 overflow-y-auto px-3 py-2">
            {!running || connections.length === 0 ? (
              <EmptyState icon={<Inbox size={20} />} text={t("connEmpty")} compact />
            ) : filteredConns.length === 0 ? (
              <EmptyState icon={<Search size={20} />} text={t("connNoMatch")} compact />
            ) : (
              filteredConns.map((c) => (
                <div
                  key={c.id}
                  onClick={() => copyOne(c.target)}

                  className="flex items-center gap-2 px-2.5 py-1.5 rounded-lg cursor-pointer transition-colors hover:[background:var(--surface-hover)]"
                  style={{ borderBottom: "1px solid var(--border)" }}
                >
                  <span
                    className="mono text-[9px] px-1.5 py-0.5 rounded shrink-0"
                    style={{
                      background: c.proto === "SOCKS" ? "color-mix(in srgb, var(--accent) 14%, transparent)" : "rgba(34,197,94,0.14)",
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
        </div>
      )}
    </div>
  );
}

function FilterBar({
  nicNames,
  nicFilter,
  setNicFilter,
  protoFilter,
  setProtoFilter,
  query,
  setQuery,
  t,
}: {
  nicNames: string[];
  nicFilter: string | null;
  setNicFilter: (v: string | null) => void;
  protoFilter: string | null;
  setProtoFilter: (v: string | null) => void;
  query: string;
  setQuery: (v: string) => void;
  t: (k: string, vars?: Record<string, string | number>) => string;
}) {
  return (
    <div className="flex items-center gap-2 px-3 py-2 shrink-0 flex-wrap" style={{ borderBottom: "1px solid var(--border)" }}>
      <FilterChip
        active={!nicFilter && !protoFilter}
        onClick={() => {
          setNicFilter(null);
          setProtoFilter(null);
        }}
      >
        {t("connFilterAll")}
      </FilterChip>
      {["SOCKS", "HTTP"].map((p) => (
        <FilterChip key={p} active={protoFilter === p} onClick={() => setProtoFilter(protoFilter === p ? null : p)}>
          {p}
        </FilterChip>
      ))}
      {nicNames.map((n) => (
        <FilterChip key={n} active={nicFilter === n} onClick={() => setNicFilter(nicFilter === n ? null : n)}>
          {n}
        </FilterChip>
      ))}
      <div className="flex-1 min-w-[80px]" />
      <input
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        placeholder={t("connSearchPlaceholder")}
        spellCheck={false}
        className="px-2.5 py-1 rounded-lg text-[11.5px] outline-none"
        style={{ background: "var(--surface-2)", border: "1px solid var(--border)", color: "var(--text-0)", width: 150 }}
      />
    </div>
  );
}

function FilterChip({ active, onClick, children }: { active: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      onClick={onClick}
      className="px-2.5 py-1 rounded-lg text-[11px] font-medium transition-colors"
      style={{
        background: active ? "var(--accent)" : "var(--surface-2)",
        color: active ? "#fff" : "var(--text-1)",
        border: `1px solid ${active ? "var(--accent)" : "var(--border)"}`,
      }}
    >
      {children}
    </button>
  );
}
