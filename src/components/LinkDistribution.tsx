import { motion } from "framer-motion";
import { GitFork } from "lucide-react";
import { useSettings } from "../store";
import { EmptyState } from "./EmptyState";
import type { NicTelemetry } from "../lib/api";

interface Props {
  perNic: Record<string, NicTelemetry>;
  running: boolean;
}

// 链路区分色（数据可视化多序列）：首位跟随强调色，其余为沉稳辅助色板
const HUES = ["var(--accent-soft)", "#38bdf8", "#34d399", "#a78bfa", "#f59e0b"];

export function LinkDistribution({ perNic, running }: Props) {
  const { t } = useSettings();
  const list = Object.values(perNic).sort((a, b) => b.downMbps - a.downMbps);
  const total = list.reduce((s, n) => s + n.downMbps, 0);

  return (
    <div className="panel flex flex-col overflow-hidden">
      <div className="panel-head flex items-center gap-3 px-5 py-3.5 shrink-0">
        <GitFork size={17} style={{ color: "var(--cyan)" }} />
        <span className="font-semibold text-[14px]">{t("linkDistTitle")}</span>
      </div>

      <div className="flex-1 overflow-y-auto px-5 py-4">
        {!running || list.length === 0 ? (
          <EmptyState icon={<GitFork size={20} />} text={t("linkDistEmpty")} />
        ) : (
          <>
            <MiniFlow list={list} total={total} />
            <div className="flex flex-col gap-3.5">
            {list.map((n, i) => {
              const share = total > 0 ? n.downMbps / total : 0;
              const hue = HUES[i % HUES.length];
              return (
                <div key={n.name}>
                  <div className="flex items-center justify-between mb-1.5">
                    <span className="flex items-center gap-2 text-[12.5px] font-medium truncate">
                      <span className="w-2.5 h-2.5 rounded-sm shrink-0" style={{ background: hue, boxShadow: `0 0 6px ${hue}` }} />
                      <span className="truncate">{n.name}</span>
                    </span>
                    <span className="mono text-[12px] shrink-0 ml-2" style={{ color: "var(--text-1)" }}>
                      {n.downMbps.toFixed(2)} <span style={{ color: "var(--text-2)" }}>MB/s · {n.connections}c</span>
                    </span>
                  </div>
                  <div className="h-2.5 rounded-full overflow-hidden" style={{ background: "var(--surface-2)" }}>
                    <motion.div
                      className="h-full rounded-full"
                      style={{ background: `linear-gradient(90deg, color-mix(in srgb, ${hue} 67%, transparent), ${hue})`, boxShadow: `0 0 10px color-mix(in srgb, ${hue} 40%, transparent)` }}
                      initial={{ width: 0 }}
                      animate={{ width: `${Math.max(share * 100, n.downMbps > 0 ? 3 : 0)}%` }}
                      transition={{ type: "spring", stiffness: 120, damping: 22 }}
                    />
                  </div>
                  <div className="mono text-[10px] mt-1 text-right" style={{ color: "var(--text-2)" }}>
                    {(share * 100).toFixed(1)}%
                  </div>
                </div>
              );
            })}
          </div>
          </>
        )}
      </div>
    </div>
  );
}

/** 实时分流流向图：流量从中心源沿曲线流向各网卡，线宽随实时占比、流速动画体现活跃度。 */
function MiniFlow({ list, total }: { list: NicTelemetry[]; total: number }) {
  const items = list.slice(0, 5);
  const n = items.length;
  const H = Math.max(64, n * 26 + 16);
  const sx = 26;
  const sy = H / 2;
  const ex = 250;
  return (
    <svg className="hmx-diagram" viewBox={`0 0 280 ${H}`} style={{ height: H, marginBottom: 14 }} aria-hidden="true">
      {items.map((nic, i) => {
        const y = n <= 1 ? sy : 14 + (i * (H - 28)) / (n - 1);
        const share = total > 0 ? nic.downMbps / total : 0;
        const hue = HUES[i % HUES.length];
        const d = `M${sx},${sy} C140,${sy} 140,${y} ${ex},${y}`;
        return (
          <g key={nic.name}>
            <path className="pipe" d={d} />
            {nic.downMbps > 0 && (
              <path className="flow" d={d} style={{ stroke: hue, strokeWidth: 1.6 + share * 7, filter: "none" }} />
            )}
            <circle cx={ex} cy={y} r="4.5" fill={hue} />
          </g>
        );
      })}
      <circle cx={sx} cy={sy} r="6" fill="var(--accent-soft)" />
      <circle cx={sx} cy={sy} r="9.5" fill="none" stroke="var(--accent-soft)" strokeOpacity="0.4" strokeWidth="1.4" />
    </svg>
  );
}
