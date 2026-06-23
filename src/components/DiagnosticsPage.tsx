import { motion } from "framer-motion";
import { Activity, ArrowDownToLine, Loader2, Stethoscope } from "lucide-react";
import { useSettings } from "../store";
import type { AdapterInfo, LatencyResult } from "../lib/api";

interface Props {
  adapters: AdapterInfo[];
  latencies: Record<number, LatencyResult>;
  speedResults: Record<number, { mbps: number; ok: boolean }>;
  diagnosing: boolean;
  onDiagnose: () => void;
}

type Grade = { key: string; color: string };

function gradeOf(lat: LatencyResult | undefined, sp: { mbps: number; ok: boolean } | undefined): Grade {
  if (!lat && !sp) return { key: "gradePending", color: "var(--text-2)" };
  if (lat && !lat.ok && sp && !sp.ok) return { key: "gradeFail", color: "var(--danger)" };
  if (sp && sp.ok) {
    const m = sp.mbps;
    if (m >= 30) return { key: "gradeExcellent", color: "var(--ok)" };
    if (m >= 15) return { key: "gradeGood", color: "var(--accent-soft)" };
    if (m >= 5) return { key: "gradeFair", color: "var(--warn)" };
    return { key: "gradeSlow", color: "var(--text-1)" };
  }
  if (lat && lat.ok) {
    const ms = lat.latencyMs;
    if (ms <= 40) return { key: "gradeGood", color: "var(--accent-soft)" };
    if (ms <= 100) return { key: "gradeFair", color: "var(--warn)" };
    return { key: "gradeSlow", color: "var(--text-1)" };
  }
  return { key: "gradeFail", color: "var(--danger)" };
}

const container = { hidden: {}, show: { transition: { staggerChildren: 0.06 } } };
const item = { hidden: { opacity: 0, y: 16 }, show: { opacity: 1, y: 0 } };

export function DiagnosticsPage({ adapters, latencies, speedResults, diagnosing, onDiagnose }: Props) {
  const { t } = useSettings();
  const valid = adapters.filter((a) => a.ipv4 && a.ipv4 !== "0.0.0.0");

  return (
    <div className="h-full overflow-y-auto px-1 pb-8">
      <div className="max-w-[920px] mx-auto flex flex-col gap-5">
        <div className="flex items-start gap-4 flex-wrap">
          <p className="flex-1 min-w-[260px] text-[13px] leading-relaxed" style={{ color: "var(--text-1)" }}>
            {t("diagHint")}
          </p>
          <motion.button
            whileTap={{ scale: 0.97 }}
            disabled={diagnosing || valid.length === 0}
            onClick={onDiagnose}
            className="flex items-center gap-2 h-[42px] px-5 rounded-xl font-semibold text-[13.5px] text-white shrink-0"
            style={{
              background: "linear-gradient(135deg, var(--accent), var(--accent-deep))",
              boxShadow: "0 6px 18px -8px var(--accent-glow)",
              opacity: diagnosing || valid.length === 0 ? 0.5 : 1,
              cursor: diagnosing || valid.length === 0 ? "not-allowed" : "pointer",
            }}
          >
            {diagnosing ? <Loader2 size={17} className="animate-spin" /> : <Stethoscope size={17} />}
            {diagnosing ? t("diagRunning") : t("diagRun")}
          </motion.button>
        </div>

        {valid.length === 0 ? (
          <div className="panel grid place-items-center py-16 text-[13px]" style={{ color: "var(--text-2)" }}>
            {t("diagNoNics")}
          </div>
        ) : (
          <motion.div
            variants={container}
            initial="hidden"
            animate="show"
            className="grid gap-4"
            style={{ gridTemplateColumns: "repeat(auto-fill, minmax(280px, 1fr))" }}
          >
            {valid.map((a) => {
              const lat = latencies[a.index];
              const sp = speedResults[a.index];
              const g = gradeOf(lat, sp);
              return (
                <motion.div key={a.index} variants={item} className="panel p-5">
                  <div className="flex items-start justify-between gap-3 mb-4">
                    <div className="min-w-0">
                      <div className="text-[14px] font-semibold truncate">{a.alias}</div>
                      <div className="mono text-[11px] mt-0.5 truncate" style={{ color: "var(--text-2)" }}>
                        {a.ipv4}
                      </div>
                    </div>
                    <span
                      className="shrink-0 text-[12px] font-bold px-2.5 py-1 rounded-lg"
                      style={{ background: `color-mix(in srgb, ${g.color} 16%, transparent)`, color: g.color }}
                    >
                      {t(g.key)}
                    </span>
                  </div>
                  <div className="grid grid-cols-2 gap-3">
                    <Metric
                      icon={<Activity size={13} />}
                      label={t("diagLatency")}
                      value={lat ? (lat.ok ? `${lat.latencyMs}` : t("latencyTimeout")) : "—"}
                      unit={lat && lat.ok ? "ms" : ""}
                      ok={!lat || lat.ok}
                    />
                    <Metric
                      icon={<ArrowDownToLine size={13} />}
                      label={t("diagSpeed")}
                      value={sp ? (sp.ok ? sp.mbps.toFixed(1) : t("latencyTimeout")) : "—"}
                      unit={sp && sp.ok ? "MB/s" : ""}
                      ok={!sp || sp.ok}
                    />
                  </div>
                </motion.div>
              );
            })}
          </motion.div>
        )}
      </div>
    </div>
  );
}

function Metric({
  icon,
  label,
  value,
  unit,
  ok,
}: {
  icon: React.ReactNode;
  label: string;
  value: string;
  unit: string;
  ok: boolean;
}) {
  return (
    <div className="rounded-xl px-3 py-2.5" style={{ background: "var(--surface-2)", border: "1px solid var(--border)" }}>
      <div className="flex items-center gap-1.5 eyebrow mb-1.5">
        {icon}
        {label}
      </div>
      <div className="mono text-[18px] font-semibold leading-none" style={{ color: ok ? "var(--text-0)" : "var(--danger)" }}>
        {value}
        {unit && (
          <span className="text-[10px] ml-1 font-normal" style={{ color: "var(--text-2)" }}>
            {unit}
          </span>
        )}
      </div>
    </div>
  );
}
