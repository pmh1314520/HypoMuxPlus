import { motion } from "framer-motion";
import { Activity, Check, CheckSquare, Layers, Loader2, RefreshCw, Square } from "lucide-react";
import { useSettings } from "../store";
import type { AdapterInfo, LatencyResult, NicTelemetry } from "../lib/api";

interface Props {
  adapters: AdapterInfo[];
  selected: Set<number>;
  toggle: (index: number) => void;
  selectAll: () => void;
  deselectAll: () => void;
  refresh: () => void;
  perNic: Record<string, NicTelemetry>;
  running: boolean;
  loading: boolean;
  latencies: Record<number, LatencyResult>;
  testing: boolean;
  onTest: () => void;
}

export function AdapterTable({
  adapters,
  selected,
  toggle,
  selectAll,
  deselectAll,
  refresh,
  perNic,
  running,
  loading,
  latencies,
  testing,
  onTest,
}: Props) {
  const { t } = useSettings();

  return (
    <div className="glass flex flex-col overflow-hidden" style={{ boxShadow: "var(--shadow)" }}>
      {/* 卡片头 */}
      <div
        className="flex items-center gap-3 px-5 py-3.5 shrink-0"
        style={{ borderBottom: "1px solid var(--border)" }}
      >
        <Layers size={17} style={{ color: "var(--accent-soft)" }} />
        <div className="flex flex-col leading-tight">
          <span className="font-semibold text-[14px]">{t("adaptersTitle")}</span>
          <span className="text-[11px]" style={{ color: "var(--text-2)" }}>
            {t("adaptersHint")}
          </span>
        </div>
        <div className="flex-1" />
        <span className="text-[11px] px-2 py-1 rounded-md" style={{ background: "var(--surface-2)", color: "var(--text-1)" }}>
          {t("selectedCount", { n: selected.size })}
        </span>
        <HeaderBtn onClick={onTest} disabled={testing}>
          {testing ? <Loader2 size={14} className="animate-spin" /> : <Activity size={14} />}{" "}
          {testing ? t("latencyTesting") : t("latencyTest")}
        </HeaderBtn>
        <HeaderBtn onClick={selectAll} disabled={running}>
          <CheckSquare size={14} /> {t("selectAll")}
        </HeaderBtn>
        <HeaderBtn onClick={deselectAll} disabled={running}>
          <Square size={14} /> {t("deselectAll")}
        </HeaderBtn>
        <HeaderBtn onClick={refresh} disabled={running}>
          <RefreshCw size={14} className={loading ? "animate-spin" : ""} />
        </HeaderBtn>
      </div>

      {/* 列头 */}
      <div
        className="grid items-center px-5 py-2.5 text-[11px] font-medium shrink-0"
        style={{ gridTemplateColumns: "44px 1fr 140px 110px 70px", color: "var(--text-2)" }}
      >
        <span>{t("colSelect")}</span>
        <span>{t("colAlias")}</span>
        <span>{t("colIpv4")}</span>
        <span className="text-right">{t("colSpeed")}</span>
        <span className="text-right">{t("colConn")}</span>
      </div>

      {/* 行 */}
      <div className="flex-1 overflow-y-auto px-2 pb-2">
        {adapters.length === 0 ? (
          <div className="grid place-items-center h-full text-[13px]" style={{ color: "var(--text-2)" }}>
            {loading ? t("statusLoading") : t("statusNoAdapters")}
          </div>
        ) : (
          adapters.map((a) => {
            const checked = selected.has(a.index);
            const tele = perNic[a.alias];
            const hasIp = a.ipv4 && a.ipv4 !== "0.0.0.0";
            return (
              <motion.div
                key={a.index}
                layout
                onClick={() => hasIp && !running && toggle(a.index)}
                className="grid items-center px-3 py-2.5 my-0.5 rounded-xl cursor-pointer transition-colors"
                style={{
                  gridTemplateColumns: "44px 1fr 140px 110px 70px",
                  background: checked ? "var(--surface-hover)" : "transparent",
                  border: checked ? "1px solid var(--border-strong)" : "1px solid transparent",
                  boxShadow: checked ? "inset 3px 0 0 var(--accent)" : "none",
                  cursor: hasIp && !running ? "pointer" : "not-allowed",
                  opacity: hasIp ? 1 : 0.5,
                }}
                onMouseEnter={(e) => {
                  if (!checked) e.currentTarget.style.background = "var(--surface)";
                }}
                onMouseLeave={(e) => {
                  if (!checked) e.currentTarget.style.background = "transparent";
                }}
              >
                {/* 复选框 */}
                <div
                  className="grid place-items-center w-[22px] h-[22px] rounded-md transition-colors"
                  style={{
                    background: checked ? "var(--accent)" : "transparent",
                    border: `1.5px solid ${checked ? "var(--accent)" : "var(--border-strong)"}`,
                  }}
                >
                  {checked && <Check size={14} color="#fff" strokeWidth={3} />}
                </div>

                {/* 别名 + 描述 */}
                <div className="flex flex-col leading-tight min-w-0 pr-2">
                  <span className="text-[13px] font-medium truncate">{a.alias}</span>
                  <span className="text-[10px] truncate" style={{ color: "var(--text-2)" }}>
                    {a.description}
                  </span>
                </div>

                {/* IPv4 + 延迟体检结果 */}
                <div className="flex flex-col gap-1 min-w-0">
                  <span className="text-[12px] tabular-nums truncate" style={{ color: hasIp ? "var(--text-1)" : "var(--text-2)" }}>
                    {hasIp ? a.ipv4 : t("noValidIp")}
                  </span>
                  {latencies[a.index] && (
                    <span
                      className="text-[10px] mono px-1.5 py-0.5 rounded w-fit"
                      style={{
                        background: latencies[a.index].ok ? "rgba(62,207,142,0.13)" : "rgba(240,97,109,0.13)",
                        color: latencies[a.index].ok ? "var(--ok)" : "var(--danger)",
                      }}
                    >
                      {latencies[a.index].ok ? `${latencies[a.index].latencyMs} ms` : t("latencyTimeout")}
                    </span>
                  )}
                </div>

                {/* 速度 */}
                <span
                  className="text-[13px] font-semibold tabular-nums text-right"
                  style={{ color: tele && tele.downMbps > 0 ? "var(--accent-soft)" : "var(--text-2)" }}
                >
                  {tele ? tele.downMbps.toFixed(2) : "0.00"}
                </span>

                {/* 连接数 */}
                <span className="text-[13px] tabular-nums text-right" style={{ color: "var(--text-1)" }}>
                  {tele ? tele.connections : "—"}
                </span>
              </motion.div>
            );
          })
        )}
      </div>
    </div>
  );
}

function HeaderBtn({
  children,
  onClick,
  disabled,
}: {
  children: React.ReactNode;
  onClick: () => void;
  disabled?: boolean;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg text-[12px] font-medium transition-colors"
      style={{
        background: "var(--surface-strong)",
        border: "1px solid var(--border)",
        color: "var(--text-1)",
        opacity: disabled ? 0.4 : 1,
        cursor: disabled ? "not-allowed" : "pointer",
      }}
    >
      {children}
    </button>
  );
}
