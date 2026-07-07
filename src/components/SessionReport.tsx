import { useEffect } from "react";
import { motion } from "framer-motion";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { Clock, Database, Gauge, ImageDown, Network, TrendingUp, X, Zap } from "lucide-react";
import { useSettings } from "../store";
import { api } from "../lib/api";
import { useToast } from "./Toast";

export interface SessionStats {
  mb: number;
  peak: number;
  secs: number;
  nics: number;
}

interface Props {
  stats: SessionStats;
  onClose: () => void;
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

/** 会话战报卡片：加速结束后展示本次成果，可一键导出 PNG 分享。 */
export function SessionReport({ stats, onClose }: Props) {
  const { t } = useSettings();
  const toast = useToast();

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    const prev = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      window.removeEventListener("keydown", onKey);
      document.body.style.overflow = prev;
    };
  }, [onClose]);

  const avg = stats.secs > 0 ? stats.mb / stats.secs : 0;
  // 估算节省时长：多网卡并进相对单卡的粗略收益（仅供参考）
  const savedMin = stats.nics > 1 ? (stats.secs * (stats.nics - 1)) / stats.nics / 60 : 0;

  const metrics = [
    { icon: Database, label: t("reportTotal"), value: fmtData(stats.mb), accent: true },
    { icon: TrendingUp, label: t("reportPeak"), value: `${stats.peak.toFixed(2)} ${t("unitMbps")}` },
    { icon: Gauge, label: t("reportAvg"), value: `${avg.toFixed(2)} ${t("unitMbps")}` },
    { icon: Network, label: t("reportNics"), value: String(stats.nics) },
    { icon: Clock, label: t("reportDuration"), value: fmtTime(stats.secs) },
    { icon: Zap, label: t("reportSaved"), value: `${savedMin.toFixed(1)} ${t("reportMin")}` },
  ];

  const exportImg = async () => {
    const cs = getComputedStyle(document.documentElement);
    const cv = (n: string, fb: string) => cs.getPropertyValue(n).trim() || fb;
    const accent = cv("--accent", "#3b82f6");
    const accentDeep = cv("--accent-deep", "#2563eb");
    const W = 600;
    const H = 380;
    const dpr = 2;
    const canvas = document.createElement("canvas");
    canvas.width = W * dpr;
    canvas.height = H * dpr;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.scale(dpr, dpr);
    const FONT = "'Inter','Microsoft YaHei UI',system-ui,sans-serif";
    const MONO = "'JetBrains Mono',Consolas,monospace";

    // 渐变背景
    const grad = ctx.createLinearGradient(0, 0, W, H);
    grad.addColorStop(0, "#0a0e18");
    grad.addColorStop(1, "#0e1424");
    ctx.fillStyle = grad;
    ctx.fillRect(0, 0, W, H);
    // 顶部强调条
    const bar = ctx.createLinearGradient(0, 0, W, 0);
    bar.addColorStop(0, accent);
    bar.addColorStop(1, accentDeep);
    ctx.fillStyle = bar;
    ctx.fillRect(0, 0, W, 5);

    ctx.textBaseline = "alphabetic";
    ctx.fillStyle = "#eef1f6";
    ctx.font = `700 26px ${FONT}`;
    ctx.fillText("HypoMux", 40, 58);
    const w1 = ctx.measureText("HypoMux").width;
    ctx.fillStyle = cv("--accent-soft", "#6ea8ff");
    ctx.fillText("Plus", 40 + w1, 58);
    ctx.fillStyle = "#9aa5b4";
    ctx.font = `600 15px ${FONT}`;
    ctx.fillText(t("reportTitle"), 40, 84);

    // 主数据：本次下行
    ctx.fillStyle = "#5a6573";
    ctx.font = `600 12px ${FONT}`;
    ctx.fillText(t("reportTotal").toUpperCase(), 40, 130);
    ctx.fillStyle = "#eef1f6";
    ctx.font = `700 52px ${MONO}`;
    ctx.fillText(fmtData(stats.mb), 40, 180);

    // 指标网格（2×2 副指标）
    const cells = [
      [t("reportPeak"), `${stats.peak.toFixed(2)} MB/s`],
      [t("reportAvg"), `${avg.toFixed(2)} MB/s`],
      [t("reportNics"), String(stats.nics)],
      [t("reportDuration"), fmtTime(stats.secs)],
    ];
    cells.forEach(([label, value], i) => {
      const cx = 40 + (i % 2) * 280;
      const cy = 230 + Math.floor(i / 2) * 62;
      ctx.fillStyle = "#5a6573";
      ctx.font = `600 11px ${FONT}`;
      ctx.fillText(String(label).toUpperCase(), cx, cy);
      ctx.fillStyle = "#cfd6e0";
      ctx.font = `600 22px ${MONO}`;
      ctx.fillText(String(value), cx, cy + 26);
    });

    // 页脚
    ctx.fillStyle = "#5a6573";
    ctx.font = `400 12px ${FONT}`;
    ctx.fillText(`${t("reportShareNote")} · hmp.pmhs.top`, 40, H - 22);

    try {
      const blob: Blob | null = await new Promise((res) => canvas.toBlob(res, "image/png"));
      if (!blob) return;
      const buf = await blob.arrayBuffer();
      const stamp = new Date().toISOString().slice(0, 19).replace(/[:T]/g, "-");
      const path = await saveDialog({
        defaultPath: `hypomuxplus-report-${stamp}.png`,
        filters: [{ name: "PNG", extensions: ["png"] }],
      });
      if (!path) return;
      await api.writeBinaryFile(path, Array.from(new Uint8Array(buf)));
      toast("success", t("msgImgExported"));
    } catch (e) {
      toast("error", String(e));
    }
  };

  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      className="fixed inset-0 z-[320] grid place-items-center p-6"
      style={{ background: "rgba(0,0,0,0.6)", backdropFilter: "blur(7px)" }}
      onClick={onClose}
    >
      <motion.div
        initial={{ opacity: 0, y: 22, scale: 0.96 }}
        animate={{ opacity: 1, y: 0, scale: 1 }}
        transition={{ type: "spring", stiffness: 250, damping: 26 }}
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
        aria-label={t("reportTitle")}
        className="panel w-[520px] max-w-[94vw] p-6"
        style={{ boxShadow: "var(--shadow)" }}
      >
        <div className="flex items-center gap-3 mb-4">
          <span
            className="grid place-items-center w-10 h-10 rounded-xl shrink-0"
            style={{ background: "linear-gradient(135deg, var(--accent), var(--accent-deep))", color: "#fff" }}
          >
            <Zap size={19} />
          </span>
          <div className="flex-1">
            <h2 className="text-[16px] font-bold">{t("reportTitle")}</h2>
          </div>
          <button
            onClick={onClose}
            aria-label={t("reportClose")}
            className="grid place-items-center w-8 h-8 rounded-lg transition-colors hover:[background:var(--surface-hover)]"
            style={{ color: "var(--text-2)" }}
          >
            <X size={16} />
          </button>
        </div>

        <div className="grid grid-cols-3 gap-3">
          {metrics.map((m) => {
            const Icon = m.icon;
            return (
              <div
                key={m.label}
                className="rounded-xl p-3.5"
                style={{
                  background: m.accent
                    ? "linear-gradient(160deg, color-mix(in srgb, var(--accent) 14%, transparent), transparent 70%)"
                    : "var(--surface-2)",
                  border: "1px solid var(--border)",
                }}
              >
                <div className="flex items-center gap-1.5 eyebrow mb-2">
                  <Icon size={12} style={{ color: "var(--accent-soft)" }} />
                  {m.label}
                </div>
                <div className="mono text-[17px] font-bold leading-none">{m.value}</div>
              </div>
            );
          })}
        </div>

        <div className="flex items-center gap-2.5 mt-5">
          <button
            onClick={exportImg}
            className="flex-1 flex items-center justify-center gap-2 h-[44px] rounded-xl font-semibold text-[14px] text-white transition-transform hover:scale-[1.02]"
            style={{ background: "linear-gradient(135deg, var(--accent), var(--accent-deep))", boxShadow: "0 8px 22px -10px var(--accent-glow)" }}
          >
            <ImageDown size={17} /> {t("reportExport")}
          </button>
          <button
            onClick={onClose}
            className="px-5 h-[44px] rounded-xl font-medium text-[13.5px] transition-colors"
            style={{ background: "var(--surface-2)", border: "1px solid var(--border)", color: "var(--text-1)" }}
          >
            {t("reportClose")}
          </button>
        </div>
      </motion.div>
    </motion.div>
  );
}
