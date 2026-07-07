import { useEffect, useMemo } from "react";
import { motion } from "framer-motion";
import { Gauge, Loader2, RotateCw, X, Zap } from "lucide-react";
import { useSettings } from "../store";
import { AnimatedNumber } from "./AnimatedNumber";
import type { AdapterInfo } from "../lib/api";

interface Props {
  adapters: AdapterInfo[];
  selected: Set<number>;
  speedResults: Record<number, { mbps: number; ok: boolean }>;
  running: boolean;
  onClose: () => void;
  onRun: () => void;
}

const NIC_HUES = ["var(--accent-soft)", "#38bdf8", "#34d399", "#a78bfa", "#f59e0b", "#fb7185"];

/** 一键聚合测速：并发跑分所有已选网卡，展示「单卡速度 → 合并总速度」与提升幅度。 */
export function AggregateSpeedTest({ adapters, selected, speedResults, running, onClose, onRun }: Props) {
  const { t } = useSettings();

  // Esc 关闭（测速中禁用）+ 锁滚动
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !running) onClose();
    };
    window.addEventListener("keydown", onKey);
    const prev = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      window.removeEventListener("keydown", onKey);
      document.body.style.overflow = prev;
    };
  }, [running, onClose]);

  const nics = useMemo(
    () => adapters.filter((a) => selected.has(a.index) && a.ipv4 && a.ipv4 !== "0.0.0.0"),
    [adapters, selected],
  );

  const speeds = nics.map((n) => {
    const r = speedResults[n.index];
    return r && r.ok ? r.mbps : 0;
  });
  const combined = speeds.reduce((s, v) => s + v, 0);
  const best = speeds.reduce((m, v) => Math.max(m, v), 0);
  const maxScale = Math.max(best, 1);
  const improvePct = best > 0 ? Math.round(((combined - best) / best) * 100) : 0;
  const doneCount = nics.filter((n) => speedResults[n.index] !== undefined).length;
  const allDone = !running && doneCount > 0 && doneCount >= nics.length;

  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      className="fixed inset-0 z-[300] grid place-items-center p-6"
      style={{ background: "rgba(0,0,0,0.6)", backdropFilter: "blur(7px)" }}
      onClick={() => !running && onClose()}
    >
      <motion.div
        initial={{ opacity: 0, y: 22, scale: 0.97 }}
        animate={{ opacity: 1, y: 0, scale: 1 }}
        transition={{ type: "spring", stiffness: 250, damping: 26 }}
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
        aria-label={t("aggTitle")}
        className="panel w-[560px] max-w-[94vw] p-6"
        style={{ boxShadow: "var(--shadow)" }}
      >
        <div className="flex items-center gap-3 mb-1">
          <span
            className="grid place-items-center w-10 h-10 rounded-xl shrink-0"
            style={{ background: "var(--surface-2)", border: "1px solid var(--border)", color: "var(--accent-soft)" }}
          >
            <Gauge size={19} />
          </span>
          <div className="flex-1 min-w-0">
            <h2 className="text-[16px] font-bold">{t("aggTitle")}</h2>
            <p className="text-[11.5px] mt-0.5" style={{ color: "var(--text-2)" }}>
              {t("aggTip")}
            </p>
          </div>
          {!running && (
            <button
              onClick={onClose}
              aria-label={t("tipClose")}
              className="grid place-items-center w-8 h-8 rounded-lg transition-colors hover:[background:var(--surface-hover)]"
              style={{ color: "var(--text-2)" }}
            >
              <X size={16} />
            </button>
          )}
        </div>

        {/* 合并速度大读数 */}
        <div
          className="rounded-2xl mt-4 p-5 text-center relative overflow-hidden"
          style={{ background: "linear-gradient(160deg, color-mix(in srgb, var(--accent) 12%, transparent), transparent 70%)", border: "1px solid var(--border)" }}
        >
          <div className="eyebrow justify-center flex items-center gap-1.5">
            <Zap size={12} style={{ color: "var(--accent-soft)" }} /> {t("aggCombined")}
          </div>
          <div className="mono text-[46px] font-bold leading-none mt-2">
            <AnimatedNumber value={combined} decimals={2} />
            <span className="text-[15px] ml-1.5 font-normal" style={{ color: "var(--text-1)" }}>
              {t("unitMbps")}
            </span>
          </div>
          {allDone && best > 0 && improvePct > 0 && (
            <motion.div
              initial={{ opacity: 0, y: 6 }}
              animate={{ opacity: 1, y: 0 }}
              className="inline-flex items-center gap-1.5 mt-3 px-3 py-1 rounded-full text-[12.5px] font-bold"
              style={{ background: "color-mix(in srgb, var(--ok) 16%, transparent)", color: "var(--ok)" }}
            >
              <Zap size={13} /> {t("aggImprove")} +{improvePct}%
            </motion.div>
          )}
          <div className="text-[11px] mt-2" style={{ color: "var(--text-2)" }}>
            {t("aggBestSingle")}: <span className="mono">{best.toFixed(2)}</span> {t("unitMbps")}
          </div>
        </div>

        {/* 各网卡速度条 */}
        <div className="flex flex-col gap-2.5 mt-4">
          {nics.map((n, i) => {
            const r = speedResults[n.index];
            const v = r && r.ok ? r.mbps : 0;
            const pending = r === undefined;
            const hue = NIC_HUES[i % NIC_HUES.length];
            return (
              <div key={n.index}>
                <div className="flex items-center justify-between mb-1 text-[12px]">
                  <span className="flex items-center gap-2 truncate">
                    <span className="w-2.5 h-2.5 rounded-sm shrink-0" style={{ background: hue }} />
                    <span className="truncate font-medium">{n.alias}</span>
                  </span>
                  <span className="mono shrink-0 ml-2" style={{ color: "var(--text-1)" }}>
                    {pending && running ? (
                      <Loader2 size={12} className="animate-spin inline" />
                    ) : r && !r.ok ? (
                      <span style={{ color: "var(--danger)" }}>{t("latencyTimeout")}</span>
                    ) : (
                      <>
                        {v.toFixed(2)} <span style={{ color: "var(--text-2)" }}>{t("unitMbps")}</span>
                      </>
                    )}
                  </span>
                </div>
                <div className="h-2.5 rounded-full overflow-hidden" style={{ background: "var(--surface-2)" }}>
                  <motion.div
                    className="h-full rounded-full"
                    style={{ background: `linear-gradient(90deg, color-mix(in srgb, ${hue} 65%, transparent), ${hue})` }}
                    initial={{ width: 0 }}
                    animate={{ width: `${Math.min((v / maxScale) * 100, 100)}%` }}
                    transition={{ type: "spring", stiffness: 120, damping: 22 }}
                  />
                </div>
              </div>
            );
          })}
        </div>

        {/* 操作 */}
        <button
          onClick={onRun}
          disabled={running}
          className="w-full mt-5 h-[46px] rounded-xl font-semibold text-[14px] text-white flex items-center justify-center gap-2 transition-transform hover:scale-[1.02]"
          style={{
            background: "linear-gradient(135deg, var(--accent), var(--accent-deep))",
            boxShadow: "0 8px 22px -10px var(--accent-glow)",
            opacity: running ? 0.7 : 1,
            cursor: running ? "not-allowed" : "pointer",
          }}
        >
          {running ? <Loader2 size={17} className="animate-spin" /> : <RotateCw size={16} />}
          {running ? t("aggRunning") : doneCount > 0 ? t("aggRetest") : t("aggRun")}
        </button>
      </motion.div>
    </motion.div>
  );
}
