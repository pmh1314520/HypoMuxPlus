import { useState } from "react";
import { motion } from "framer-motion";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { ArrowDownToLine, BarChart3, Clock, Database, FileDown, Gauge, Network, RotateCcw, Shuffle, TrendingUp, Zap } from "lucide-react";
import { useSettings } from "../store";
import { api } from "../lib/api";
import { AnimatedNumber } from "./AnimatedNumber";
import { Tooltip } from "./Tooltip";
import { useToast } from "./Toast";

interface Props {
  lifetimeMB: number;
  lifetimePeak: number;
  lifetimeSeconds: number;
  sessionMB: number;
  sessionPeak: number;
  uptime: number;
  totalConn: number;
  running: boolean;
  dailyMB: Record<string, number>;
  onReset: () => void;
}

function fmtData(mb: number): string {
  const g = (n: number, d: number) => n.toLocaleString("en-US", { minimumFractionDigits: d, maximumFractionDigits: d });
  if (mb >= 1048576) return g(mb / 1048576, 2) + " TB";
  if (mb >= 1024) return g(mb / 1024, 2) + " GB";
  return g(mb, 0) + " MB";
}
function fmtTime(sec: number): string {
  const h = Math.floor(sec / 3600);
  const m = Math.floor((sec % 3600) / 60);
  const s = Math.floor(sec % 60);
  if (h > 0) return `${h}h ${m}m`;
  if (m > 0) return `${m}m ${s}s`;
  return `${s}s`;
}

const container = { hidden: {}, show: { transition: { staggerChildren: 0.06 } } };
const item = { hidden: { opacity: 0, y: 16 }, show: { opacity: 1, y: 0 } };

export function StatsPage(props: Props) {
  const { t, strategy } = useSettings();
  const [confirmReset, setConfirmReset] = useState(false);

  const stratLabel =
    strategy === "rr" ? t("schedRR") : strategy === "least" ? t("schedLeast") : t("schedWeighted");

  const lifetime = [
    { icon: Database, label: t("lifetimeTotal"), value: fmtData(props.lifetimeMB), accent: true },
    { icon: TrendingUp, label: t("statLifetimePeak"), value: `${props.lifetimePeak.toFixed(2)}`, unit: "MB/s" },
    { icon: Clock, label: t("statLifetimeTime"), value: fmtTime(props.lifetimeSeconds) },
  ];

  const session = [
    { icon: ArrowDownToLine, label: t("statSessionData"), value: fmtData(props.sessionMB) },
    { icon: TrendingUp, label: t("statSessionPeak"), value: props.sessionPeak.toFixed(2), unit: "MB/s" },
    { icon: Network, label: t("statSessionConn"), value: String(props.totalConn) },
    { icon: Clock, label: t("statSessionUptime"), value: props.running ? fmtTime(props.uptime) : t("statIdle") },
  ];

  return (
    <div className="h-full overflow-y-auto px-1 pb-8">
      <motion.div variants={container} initial="hidden" animate="show" className="max-w-[920px] mx-auto flex flex-col gap-6">
        {/* 历史累计 */}
        <div>
          <div className="eyebrow mb-3 flex items-center gap-2">
            <Gauge size={13} /> {t("statLifetime")}
            <div className="flex-1" />
            {confirmReset ? (
              <span className="flex items-center gap-2 normal-case">
                <button
                  onClick={() => {
                    props.onReset();
                    setConfirmReset(false);
                  }}
                  className="text-[11px] font-semibold px-2 py-0.5 rounded-md"
                  style={{ background: "var(--danger)", color: "#fff" }}
                >
                  {t("statResetConfirm")}
                </button>
                <button
                  onClick={() => setConfirmReset(false)}
                  className="text-[11px] px-2 py-0.5 rounded-md"
                  style={{ background: "var(--surface-2)", border: "1px solid var(--border)", color: "var(--text-1)" }}
                >
                  {t("btnCancel")}
                </button>
              </span>
            ) : (
              <Tooltip label={t("statReset")} placement="left">
                <button
                  onClick={() => setConfirmReset(true)}
                  className="flex items-center gap-1 text-[11px] px-2 py-0.5 rounded-md normal-case transition-colors"
                  style={{ background: "var(--surface-2)", border: "1px solid var(--border)", color: "var(--text-2)" }}
                >
                  <RotateCcw size={11} /> {t("statReset")}
                </button>
              </Tooltip>
            )}
          </div>
          <div className="grid grid-cols-3 gap-4">
            {lifetime.map((s) => (
              <StatCard key={s.label} {...s} />
            ))}
          </div>
        </div>

        {/* 本次会话 */}
        <div>
          <div className="eyebrow mb-3 flex items-center gap-2">
            <Zap size={13} /> {t("statSession")}
          </div>
          <div className="grid grid-cols-4 gap-4">
            {session.map((s) => (
              <StatCard key={s.label} {...s} />
            ))}
          </div>
        </div>

        {/* 近 14 天加速流量趋势 */}
        <DailyChart dailyMB={props.dailyMB} />

        {/* 当前调度策略 */}
        <motion.div
          variants={item}
          className="panel p-5 flex items-center gap-4"
          style={{ background: "linear-gradient(160deg, rgba(59,130,246,0.06), transparent 60%)" }}
        >
          <div
            className="grid place-items-center w-11 h-11 rounded-xl"
            style={{ background: "var(--surface-2)", border: "1px solid var(--border)", color: "var(--accent-soft)" }}
          >
            <Shuffle size={19} />
          </div>
          <div>
            <div className="eyebrow">{t("statStrategy")}</div>
            <div className="text-[18px] font-bold mt-1">{stratLabel}</div>
          </div>
        </motion.div>
      </motion.div>
    </div>
  );
}

function DailyChart({ dailyMB }: { dailyMB: Record<string, number> }) {
  const { t } = useSettings();
  const toast = useToast();
  const days: { key: string; label: string; mb: number }[] = [];
  const now = new Date();
  for (let i = 13; i >= 0; i--) {
    const d = new Date(now);
    d.setDate(now.getDate() - i);
    const m = String(d.getMonth() + 1).padStart(2, "0");
    const day = String(d.getDate()).padStart(2, "0");
    const key = `${d.getFullYear()}-${m}-${day}`;
    days.push({ key, label: `${m}/${day}`, mb: dailyMB[key] ?? 0 });
  }
  const max = Math.max(...days.map((d) => d.mb), 1);
  const total = days.reduce((s, d) => s + d.mb, 0);

  // 导出全部每日加速流量为 CSV
  const exportCsv = async () => {
    const keys = Object.keys(dailyMB).sort();
    if (keys.length === 0) return;
    const rows = ["Date,Accelerated_MB", ...keys.map((k) => `${k},${dailyMB[k].toFixed(2)}`)];
    try {
      const stamp = new Date().toISOString().slice(0, 10);
      const path = await saveDialog({
        defaultPath: `hypomuxplus-traffic-${stamp}.csv`,
        filters: [{ name: "CSV", extensions: ["csv"] }],
      });
      if (!path) return;
      await api.writeTextFile(path, rows.join("\r\n"));
      toast("success", t("msgCsvExported"));
    } catch (e) {
      toast("error", String(e));
    }
  };

  return (
    <motion.div variants={item} className="panel p-5">
      <div className="flex items-center justify-between mb-4">
        <div className="eyebrow flex items-center gap-2">
          <BarChart3 size={13} style={{ color: "var(--accent-soft)" }} /> {t("statDailyTitle")}
        </div>
        <div className="flex items-center gap-3">
          <div className="text-[12px] mono" style={{ color: "var(--text-2)" }}>
            {fmtData(total)}
          </div>
          {total > 0 && (
            <Tooltip label={t("statExportCsv")} placement="left">
              <button
                onClick={exportCsv}
                className="flex items-center gap-1 text-[11px] px-2 py-0.5 rounded-md normal-case transition-colors"
                style={{ background: "var(--surface-2)", border: "1px solid var(--border)", color: "var(--text-2)" }}
              >
                <FileDown size={11} /> {t("statExportCsv")}
              </button>
            </Tooltip>
          )}
        </div>
      </div>
      {total <= 0 ? (
        <div className="grid place-items-center py-10 text-[12.5px]" style={{ color: "var(--text-2)" }}>
          {t("statDailyEmpty")}
        </div>
      ) : (
        <div className="flex items-end gap-1.5 h-[120px]">
          {days.map((d, i) => {
            const h = Math.max(2, Math.round((d.mb / max) * 100));
            const isToday = i === days.length - 1;
            return (
              <div key={d.key} className="flex-1 flex flex-col items-center gap-1.5 min-w-0">
                <Tooltip label={`${d.label} · ${fmtData(d.mb)}`} placement="top">
                  <div className="w-full flex items-end justify-center" style={{ height: 96 }}>
                    <motion.div
                      initial={{ height: 0 }}
                      animate={{ height: `${h}%` }}
                      transition={{ type: "spring", stiffness: 200, damping: 26 }}
                      className="w-full rounded-t-md"
                      style={{
                        background: d.mb > 0 ? "linear-gradient(var(--accent-soft), var(--accent))" : "var(--surface-2)",
                        minHeight: 2,
                        outline: isToday ? "1px solid var(--accent-soft)" : "none",
                        outlineOffset: 1,
                      }}
                    />
                  </div>
                </Tooltip>
                <span
                  className="text-[9px] tabular-nums"
                  style={{ color: isToday ? "var(--accent-soft)" : "var(--text-2)", fontWeight: isToday ? 700 : 400 }}
                >
                  {d.label}
                </span>
              </div>
            );
          })}
        </div>
      )}
    </motion.div>
  );
}

function StatCard({
  icon: Icon,
  label,
  value,
  unit,
  accent,
}: {
  icon: typeof Database;
  label: string;
  value: string;
  unit?: string;
  accent?: boolean;
}) {
  const numeric = !isNaN(parseFloat(value)) && /^[\d.]+$/.test(value);
  return (
    <motion.div
      variants={item}
      className="panel p-5"
      style={accent ? { background: "linear-gradient(160deg, rgba(59,130,246,0.08), transparent 60%)" } : undefined}
    >
      <div className="flex items-center gap-1.5 eyebrow mb-3">
        <Icon size={13} style={{ color: "var(--accent-soft)" }} />
        {label}
      </div>
      <div className="text-[26px] font-bold mono leading-none">
        {numeric ? <AnimatedNumber value={parseFloat(value)} decimals={value.includes(".") ? 2 : 0} /> : value}
        {unit && (
          <span className="text-[12px] ml-1 font-normal" style={{ color: "var(--text-2)" }}>
            {unit}
          </span>
        )}
      </div>
    </motion.div>
  );
}
