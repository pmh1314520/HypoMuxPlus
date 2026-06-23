import { ArrowDownToLine, ArrowUpFromLine, Clock, Network, TrendingUp } from "lucide-react";
import { useSettings } from "../store";
import { AnimatedNumber } from "./AnimatedNumber";
import { Sparkline } from "./Sparkline";
import { BoostButton } from "./BoostButton";
import type { TelemetryPayload } from "../lib/api";

interface Props {
  telemetry: TelemetryPayload | null;
  history: number[];
  peak: number;
  uptime: number;
  running: boolean;
  busy: boolean;
  canBoost: boolean;
  onBoost: () => void;
}

function fmtUptime(sec: number): string {
  const h = Math.floor(sec / 3600);
  const m = Math.floor((sec % 3600) / 60);
  const s = Math.floor(sec % 60);
  return `${h.toString().padStart(2, "0")}:${m.toString().padStart(2, "0")}:${s
    .toString()
    .padStart(2, "0")}`;
}

export function SpeedHero({
  telemetry,
  history,
  peak,
  uptime,
  running,
  busy,
  canBoost,
  onBoost,
}: Props) {
  const { t } = useSettings();
  const total = telemetry?.total ?? { downMbps: 0, upMbps: 0, connections: 0 };

  const stats = [
    {
      icon: ArrowUpFromLine,
      label: t("uplink"),
      value: total.upMbps.toFixed(2),
      unit: t("unitMbps"),
    },
    { icon: Network, label: t("totalConn"), value: String(total.connections), unit: "" },
    { icon: TrendingUp, label: t("peakSpeed"), value: peak.toFixed(2), unit: t("unitMbps") },
    { icon: Clock, label: t("elapsed"), value: fmtUptime(uptime), unit: "" },
  ];

  return (
    <div className="glass relative overflow-hidden p-6" style={{ boxShadow: "var(--shadow)" }}>
      {/* 顶部流动渐变条（运行态） */}
      {running && (
        <div className="absolute top-0 left-0 right-0 h-[3px] flow-border" />
      )}

      <div className="flex items-stretch gap-6">
        {/* 左：合并下行总速度 */}
        <div className="flex flex-col justify-between min-w-[280px]">
          <div className="flex items-center gap-2 text-[13px]" style={{ color: "var(--text-2)" }}>
            <ArrowDownToLine size={15} style={{ color: "var(--accent-soft)" }} />
            {t("combinedDown")}
          </div>
          <div className="flex items-end gap-2 my-1">
            <AnimatedNumber
              value={total.downMbps}
              decimals={2}
              className="text-[64px] font-bold leading-none glow-text tabular-nums"
            />
            <span className="text-[18px] mb-2 font-medium" style={{ color: "var(--text-1)" }}>
              {t("unitMbps")}
            </span>
          </div>
          <div className="mt-2">
            <BoostButton running={running} busy={busy} disabled={!canBoost} onClick={onBoost} />
          </div>
        </div>

        {/* 右：波形 + 统计 */}
        <div className="flex-1 flex flex-col">
          <div className="flex-1 min-h-[90px] grid place-items-stretch">
            <Sparkline data={history} height={96} running={running} />
          </div>
          <div className="grid grid-cols-4 gap-3 mt-3">
            {stats.map((s) => {
              const Icon = s.icon;
              return (
                <div
                  key={s.label}
                  className="rounded-xl px-3 py-2.5"
                  style={{ background: "var(--surface-strong)", border: "1px solid var(--border)" }}
                >
                  <div
                    className="flex items-center gap-1.5 text-[11px] mb-1"
                    style={{ color: "var(--text-2)" }}
                  >
                    <Icon size={12} />
                    {s.label}
                  </div>
                  <div className="text-[18px] font-semibold tabular-nums leading-none">
                    {s.value}
                    {s.unit && (
                      <span className="text-[11px] ml-1 font-normal" style={{ color: "var(--text-2)" }}>
                        {s.unit}
                      </span>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
        </div>
      </div>
    </div>
  );
}
