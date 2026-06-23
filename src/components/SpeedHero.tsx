import { ArrowDownToLine, ArrowUpFromLine, Clock, Network, TrendingUp } from "lucide-react";
import { useSettings } from "../store";
import { AnimatedNumber } from "./AnimatedNumber";
import { AreaChart } from "./AreaChart";
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
  return `${h.toString().padStart(2, "0")}:${m.toString().padStart(2, "0")}:${s.toString().padStart(2, "0")}`;
}

export function SpeedHero({ telemetry, history, peak, uptime, running, busy, canBoost, onBoost }: Props) {
  const { t } = useSettings();
  const total = telemetry?.total ?? { downMbps: 0, upMbps: 0, connections: 0 };

  const chips = [
    { icon: ArrowUpFromLine, label: t("uplink"), value: total.upMbps.toFixed(2), unit: t("unitMbps") },
    { icon: Network, label: t("totalConn"), value: String(total.connections), unit: "" },
    { icon: TrendingUp, label: t("peakSpeed"), value: peak.toFixed(2), unit: t("unitMbps") },
    { icon: Clock, label: t("elapsed"), value: fmtUptime(uptime), unit: "" },
  ];

  return (
    <div className="panel relative overflow-hidden h-[252px] shrink-0">
      {/* 运行态顶部流光条 */}
      {running && <div className="absolute top-0 left-0 right-0 h-[2px] flow-border z-20" />}

      {/* 背景实况图表 */}
      <div className="absolute inset-0">
        <AreaChart data={history} running={running} />
      </div>
      {/* 左侧渐变遮罩，保证读数清晰 */}
      <div
        className="absolute inset-0"
        style={{
          background:
            "linear-gradient(100deg, var(--bg-1) 0%, color-mix(in srgb, var(--bg-1) 55%, transparent) 36%, transparent 70%)",
        }}
      />

      {/* 内容层 */}
      <div className="relative z-10 h-full flex flex-col justify-between p-6">
        <div className="flex items-start justify-between">
          {/* 主读数 */}
          <div>
            <div className="flex items-center gap-2 text-[12px] tracking-wide uppercase" style={{ color: "var(--text-2)" }}>
              <ArrowDownToLine size={14} style={{ color: "var(--cyan)" }} />
              {t("combinedDown")}
            </div>
            <div className="flex items-end gap-2 mt-1">
              <AnimatedNumber
                value={total.downMbps}
                decimals={2}
                className="mono text-[68px] font-bold leading-[0.95] glow-text"
              />
              <span className="text-[18px] mb-2.5 font-medium" style={{ color: "var(--text-1)" }}>
                {t("unitMbps")}
              </span>
            </div>
          </div>

          {/* 右上：端点 + 加速按钮 */}
          <div className="flex flex-col items-end gap-3">
            <BoostButton running={running} busy={busy} disabled={!canBoost} onClick={onBoost} />
          </div>
        </div>

        {/* 底部指标条 */}
        <div className="grid grid-cols-4 gap-2.5">
          {chips.map((c) => {
            const Icon = c.icon;
            return (
              <div
                key={c.label}
                className="rounded-xl px-3.5 py-2.5"
                style={{ background: "var(--surface-2)", border: "1px solid var(--border)", backdropFilter: "blur(6px)" }}
              >
                <div className="flex items-center gap-1.5 text-[10.5px] tracking-wide uppercase mb-1" style={{ color: "var(--text-2)" }}>
                  <Icon size={12} />
                  {c.label}
                </div>
                <div className="mono text-[19px] font-semibold leading-none">
                  {c.value}
                  {c.unit && (
                    <span className="text-[11px] ml-1 font-normal" style={{ color: "var(--text-2)" }}>
                      {c.unit}
                    </span>
                  )}
                </div>
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}
