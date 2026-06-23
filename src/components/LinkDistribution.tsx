import { motion } from "framer-motion";
import { GitFork } from "lucide-react";
import { useSettings } from "../store";
import type { NicTelemetry } from "../lib/api";

interface Props {
  perNic: Record<string, NicTelemetry>;
  running: boolean;
}

// 链路区分色：蓝 / 青 / 蓝绿 / 靛 / 紫
const HUES = ["#2f87ff", "#22d3ee", "#34d399", "#818cf8", "#c084fc"];

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
          <div className="grid place-items-center h-full text-center text-[12.5px] px-4" style={{ color: "var(--text-2)" }}>
            {t("linkDistEmpty")}
          </div>
        ) : (
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
                      style={{ background: `linear-gradient(90deg, ${hue}aa, ${hue})`, boxShadow: `0 0 10px ${hue}66` }}
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
        )}
      </div>
    </div>
  );
}
