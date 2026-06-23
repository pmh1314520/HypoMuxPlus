import { motion } from "framer-motion";
import { ArrowDownToLine, Clock, Database, Gauge, Network, Shuffle, TrendingUp, Zap } from "lucide-react";
import { useSettings } from "../store";
import { AnimatedNumber } from "./AnimatedNumber";

interface Props {
  lifetimeMB: number;
  lifetimePeak: number;
  lifetimeSeconds: number;
  sessionMB: number;
  sessionPeak: number;
  uptime: number;
  totalConn: number;
  running: boolean;
}

function fmtData(mb: number): string {
  if (mb >= 1048576) return (mb / 1048576).toFixed(2) + " TB";
  if (mb >= 1024) return (mb / 1024).toFixed(2) + " GB";
  return mb.toFixed(0) + " MB";
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
