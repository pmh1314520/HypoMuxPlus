import { useEffect, useState } from "react";
import { motion } from "framer-motion";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { Activity, ArrowDownToLine, ClipboardList, ImageDown, Loader2, RotateCw, ShieldCheck, Stethoscope, TrendingUp, Waves, PackageX } from "lucide-react";
import { useSettings } from "../store";
import { useToast } from "./Toast";
import { Tooltip } from "./Tooltip";
import { AreaChart } from "./AreaChart";
import { api } from "../lib/api";
import { copyText } from "../lib/clipboard";
import { buildReportLines, appendTrendPoint, capTrend, type DiagReportRow, type DiagTrend, type DiagTrendPoint } from "../lib/diag";
import type { AdapterInfo, LatencyResult } from "../lib/api";

interface Props {
  adapters: AdapterInfo[];
  latencies: Record<number, LatencyResult>;
  speedResults: Record<number, { mbps: number; ok: boolean }>;
  diagnosing: boolean;
  onDiagnose: () => void;
  onTestOne: (a: AdapterInfo) => Promise<void>;
  onApplyHealthy: (indices: number[]) => void;
}

const DIAG_HISTORY_KEY = "hmx-diag-history";
type DiagHistory = Record<number, { grade: string; ts: number }>;
function loadDiagHistory(): DiagHistory {
  try {
    const raw = localStorage.getItem(DIAG_HISTORY_KEY);
    if (raw) {
      const obj = JSON.parse(raw);
      if (obj && typeof obj === "object") return obj;
    }
  } catch {
    /* ignore */
  }
  return {};
}

// 诊断趋势历史（独立于"上次评级"历史）：每卡按时间追加采样点，上限 50，持久化。
const DIAG_TREND_KEY = "hmx-diag-trend";
const DIAG_TREND_MAX = 50;
type TrendMetric = "latencyMs" | "jitterMs" | "lossPct" | "mbps";
function loadDiagTrend(): DiagTrend {
  try {
    const raw = localStorage.getItem(DIAG_TREND_KEY);
    if (raw) {
      const obj = JSON.parse(raw);
      if (obj && typeof obj === "object") return obj as DiagTrend;
    }
  } catch {
    /* ignore */
  }
  return {};
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

/** 抖动的展示文本：成功且有效时 "N ms"，否则占位 "—" */
function jitterText(lat: LatencyResult | undefined): string {
  return lat && lat.ok && typeof lat.jitterMs === "number" && lat.jitterMs >= 0 ? `${lat.jitterMs} ms` : "—";
}
/** 丢包率的展示文本：有值时百分比，否则占位 "—" */
function lossText(lat: LatencyResult | undefined): string {
  return lat && typeof lat.lossPct === "number" ? `${Math.round(lat.lossPct * 100)}%` : "—";
}

const container = { hidden: {}, show: { transition: { staggerChildren: 0.06 } } };
const item = { hidden: { opacity: 0, y: 16 }, show: { opacity: 1, y: 0 } };

export function DiagnosticsPage({ adapters, latencies, speedResults, diagnosing, onDiagnose, onTestOne, onApplyHealthy }: Props) {
  const { t } = useSettings();
  const toast = useToast();
  const [testingIdx, setTestingIdx] = useState<number | null>(null);
  const [history, setHistory] = useState<DiagHistory>(loadDiagHistory);
  const [trend, setTrend] = useState<DiagTrend>(loadDiagTrend);
  const [trendMetric, setTrendMetric] = useState<TrendMetric>("latencyMs");
  const valid = adapters.filter((a) => a.ipv4 && a.ipv4 !== "0.0.0.0");
  const hasResults = valid.some((a) => latencies[a.index] || speedResults[a.index]);

  // 诊断结果落地为历史（下次进入诊断页可见"上次评级"），并据此自适应
  useEffect(() => {
    if (!hasResults) return;
    setHistory((prev) => {
      const next = { ...prev };
      for (const a of valid) {
        const g = gradeOf(latencies[a.index], speedResults[a.index]);
        if (g.key !== "gradePending") next[a.index] = { grade: g.key, ts: Date.now() };
      }
      localStorage.setItem(DIAG_HISTORY_KEY, JSON.stringify(next));
      return next;
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [latencies, speedResults]);

  // 诊断结果追加到趋势历史（每卡一采样点，复用纯函数 appendTrendPoint/capTrend，上限裁剪）
  useEffect(() => {
    if (!hasResults) return;
    setTrend((prev) => {
      const next: DiagTrend = { ...prev };
      const ts = Date.now();
      for (const a of valid) {
        const lat = latencies[a.index];
        const sp = speedResults[a.index];
        if (!lat && !sp) continue;
        const point: DiagTrendPoint = {
          ts,
          latencyMs: lat && lat.ok ? lat.latencyMs : -1,
          jitterMs: lat && typeof lat.jitterMs === "number" ? lat.jitterMs : -1,
          lossPct: lat && typeof lat.lossPct === "number" ? lat.lossPct : lat && !lat.ok ? 1 : 0,
          mbps: sp && sp.ok ? sp.mbps : 0,
          ok: !!(lat?.ok || sp?.ok),
        };
        next[a.index] = capTrend(appendTrendPoint(next[a.index] ?? [], point), DIAG_TREND_MAX);
      }
      localStorage.setItem(DIAG_TREND_KEY, JSON.stringify(next));
      return next;
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [latencies, speedResults]);

  // 把趋势点按所选指标映射为绘图数值序列（丢包率转百分比，其余取原值，负值/失败归零便于展示）
  const metricSeries = (points: DiagTrendPoint[] | undefined): number[] =>
    (points ?? []).map((p) => {
      const v =
        trendMetric === "lossPct"
          ? p.lossPct * 100
          : trendMetric === "mbps"
            ? p.mbps
            : (p[trendMetric] as number);
      return v > 0 ? v : 0;
    });
  const hasTrend = valid.some((a) => (trend[a.index]?.length ?? 0) > 1);

  // 健康网卡：综合评级为优秀/良好/一般者
  const applyHealthy = () => {
    const healthy = valid.filter((a) => {
      const g = gradeOf(latencies[a.index], speedResults[a.index]).key;
      return g === "gradeExcellent" || g === "gradeGood" || g === "gradeFair";
    });
    if (healthy.length === 0) {
      toast("warning", t("diagNoHealthy"));
      return;
    }
    onApplyHealthy(healthy.map((a) => a.index));
    toast("success", t("msgHealthyApplied", { n: healthy.length }));
  };

  const retestOne = async (a: AdapterInfo) => {
    if (diagnosing || testingIdx !== null) return;
    setTestingIdx(a.index);
    try {
      await onTestOne(a);
    } finally {
      setTestingIdx(null);
    }
  };

  // 生成纯文本体检报告并复制到剪贴板
  const copyReport = async () => {
    if (!hasResults) {
      toast("warning", t("diagReportNoData"));
      return;
    }
    const rows: DiagReportRow[] = valid.map((a) => {
      const lat = latencies[a.index];
      const sp = speedResults[a.index];
      const g = gradeOf(lat, sp);
      return {
        alias: a.alias,
        ipv4: a.ipv4,
        latency: lat ? (lat.ok ? `${lat.latencyMs} ms` : t("latencyTimeout")) : "—",
        jitter: jitterText(lat),
        loss: lossText(lat),
        speed: sp ? (sp.ok ? `${sp.mbps.toFixed(1)} MB/s` : t("latencyTimeout")) : "—",
        grade: t(g.key),
      };
    });
    const lines = buildReportLines(
      rows,
      {
        title: t("diagReportTitle"),
        latency: t("diagLatency"),
        jitter: t("diagJitter"),
        loss: t("diagLoss"),
        speed: t("diagSpeed"),
        grade: t("diagGrade"),
      },
      new Date().toLocaleString(),
    );
    const ok = await copyText(lines.join("\n"));
    toast(ok ? "success" : "error", t(ok ? "msgReportCopied" : "msgCopyFailed"));
  };

  // 将体检结果绘制为 PNG 图片并保存（便于分享）
  const exportImg = async () => {
    if (!hasResults) {
      toast("warning", t("diagReportNoData"));
      return;
    }
    const cs = getComputedStyle(document.documentElement);
    const cv = (name: string, fb: string) => cs.getPropertyValue(name).trim() || fb;
    const resolve = (s: string) => {
      const m = /var\((--[\w-]+)\)/.exec(s);
      return m ? cs.getPropertyValue(m[1]).trim() || "#888" : s;
    };
    const bg = cv("--bg-1", "#0a0e18");
    const surface = cv("--surface-2", "rgba(255,255,255,0.05)");
    const border = cv("--border", "rgba(255,255,255,0.1)");
    const text0 = cv("--text-0", "#eef1f6");
    const text1 = cv("--text-1", "#9aa5b4");
    const text2 = cv("--text-2", "#5a6573");
    const accent = cv("--accent-soft", "#6ea8ff");

    const rows = valid.length;
    const W = 940;
    const padX = 32;
    const headerH = 130;
    const rowH = 58;
    const footerH = 46;
    const H = headerH + rows * rowH + footerH;
    const dpr = 2;
    const canvas = document.createElement("canvas");
    canvas.width = W * dpr;
    canvas.height = H * dpr;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.scale(dpr, dpr);
    const FONT = "'Inter','Microsoft YaHei UI',system-ui,sans-serif";
    const MONO = "'JetBrains Mono',Consolas,monospace";

    // 背景
    ctx.fillStyle = bg;
    ctx.fillRect(0, 0, W, H);

    // 标题
    ctx.textBaseline = "alphabetic";
    ctx.fillStyle = text0;
    ctx.font = `700 24px ${FONT}`;
    ctx.fillText("HypoMux", padX, 52);
    const w1 = ctx.measureText("HypoMux").width;
    ctx.fillStyle = accent;
    ctx.fillText("Plus", padX + w1, 52);
    ctx.fillStyle = text1;
    ctx.font = `500 14px ${FONT}`;
    ctx.fillText(t("diagReportTitle"), padX, 78);
    ctx.fillStyle = text2;
    ctx.font = `400 12px ${MONO}`;
    ctx.fillText(new Date().toLocaleString(), padX, 98);

    // 列标题
    const colName = padX;
    const colLat = 360;
    const colJitter = 470;
    const colLoss = 570;
    const colSpeed = 660;
    const colGrade = 790;
    const headY = headerH - 10;
    ctx.font = `600 11px ${FONT}`;
    ctx.fillStyle = text2;
    ctx.fillText(t("colAlias").toUpperCase(), colName, headY);
    ctx.fillText(t("diagLatency").toUpperCase(), colLat, headY);
    ctx.fillText(t("diagJitter").toUpperCase(), colJitter, headY);
    ctx.fillText(t("diagLoss").toUpperCase(), colLoss, headY);
    ctx.fillText(t("diagSpeed").toUpperCase(), colSpeed, headY);
    ctx.fillText(t("diagGrade").toUpperCase(), colGrade, headY);
    ctx.strokeStyle = border;
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(padX, headerH);
    ctx.lineTo(W - padX, headerH);
    ctx.stroke();

    // 每张网卡一行
    valid.forEach((a, i) => {
      const lat = latencies[a.index];
      const sp = speedResults[a.index];
      const g = gradeOf(lat, sp);
      const gc = resolve(g.color);
      const y = headerH + i * rowH;
      const cy = y + rowH / 2;
      // 行底分隔线
      ctx.strokeStyle = border;
      ctx.beginPath();
      ctx.moveTo(padX, y + rowH);
      ctx.lineTo(W - padX, y + rowH);
      ctx.stroke();
      // 别名 + IP
      ctx.textBaseline = "middle";
      ctx.fillStyle = text0;
      ctx.font = `600 15px ${FONT}`;
      const alias = a.alias.length > 22 ? a.alias.slice(0, 21) + "…" : a.alias;
      ctx.fillText(alias, colName, cy - 9);
      ctx.fillStyle = text2;
      ctx.font = `400 12px ${MONO}`;
      ctx.fillText(a.ipv4, colName, cy + 10);
      // 延迟
      ctx.font = `600 15px ${MONO}`;
      ctx.fillStyle = !lat || lat.ok ? text0 : resolve("var(--danger)");
      ctx.fillText(lat ? (lat.ok ? `${lat.latencyMs} ms` : t("latencyTimeout")) : "—", colLat, cy);
      // 抖动
      ctx.fillStyle = !lat || lat.ok ? text0 : resolve("var(--danger)");
      ctx.fillText(jitterText(lat), colJitter, cy);
      // 丢包
      ctx.fillStyle = !lat || lat.lossPct < 1 ? text0 : resolve("var(--danger)");
      ctx.fillText(lossText(lat), colLoss, cy);
      // 吞吐
      ctx.fillStyle = !sp || sp.ok ? text0 : resolve("var(--danger)");
      ctx.fillText(sp ? (sp.ok ? `${sp.mbps.toFixed(1)} MB/s` : t("latencyTimeout")) : "—", colSpeed, cy);
      // 评级徽章
      ctx.font = `700 13px ${FONT}`;
      const label = t(g.key);
      const tw = ctx.measureText(label).width;
      const bx = colGrade;
      const bw = tw + 20;
      const bh = 24;
      const byy = cy - bh / 2;
      ctx.fillStyle = surface;
      const r = 8;
      ctx.beginPath();
      ctx.roundRect(bx, byy, bw, bh, r);
      ctx.fill();
      ctx.fillStyle = gc;
      ctx.textBaseline = "middle";
      ctx.fillText(label, bx + 10, cy + 1);
    });

    // 页脚
    ctx.textBaseline = "alphabetic";
    ctx.fillStyle = text2;
    ctx.font = `400 11px ${FONT}`;
    ctx.fillText("hmp.pmhs.top · HypoMuxPlus", padX, H - 18);

    try {
      const blob: Blob | null = await new Promise((res) => canvas.toBlob(res, "image/png"));
      if (!blob) return;
      const buf = await blob.arrayBuffer();
      const stamp = new Date().toISOString().slice(0, 19).replace(/[:T]/g, "-");
      const path = await saveDialog({
        defaultPath: `hypomuxplus-diagnostics-${stamp}.png`,
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
          <motion.button
            whileTap={{ scale: 0.97 }}
            disabled={!hasResults}
            onClick={copyReport}
            className="flex items-center gap-2 h-[42px] px-4 rounded-xl font-semibold text-[13.5px] shrink-0 transition-colors"
            style={{
              background: "var(--surface-2)",
              border: "1px solid var(--border)",
              color: "var(--text-1)",
              opacity: hasResults ? 1 : 0.5,
              cursor: hasResults ? "pointer" : "not-allowed",
            }}
          >
            <ClipboardList size={16} />
            {t("diagCopyReport")}
          </motion.button>
          <motion.button
            whileTap={{ scale: 0.97 }}
            disabled={!hasResults}
            onClick={exportImg}
            className="flex items-center gap-2 h-[42px] px-4 rounded-xl font-semibold text-[13.5px] shrink-0 transition-colors"
            style={{
              background: "var(--surface-2)",
              border: "1px solid var(--border)",
              color: "var(--text-1)",
              opacity: hasResults ? 1 : 0.5,
              cursor: hasResults ? "pointer" : "not-allowed",
            }}
          >
            <ImageDown size={16} />
            {t("diagExportImg")}
          </motion.button>
          <motion.button
            whileTap={{ scale: 0.97 }}
            disabled={!hasResults}
            onClick={applyHealthy}
            className="flex items-center gap-2 h-[42px] px-4 rounded-xl font-semibold text-[13.5px] shrink-0 transition-colors"
            style={{
              background: "color-mix(in srgb, var(--ok) 16%, transparent)",
              border: "1px solid color-mix(in srgb, var(--ok) 35%, transparent)",
              color: "var(--ok)",
              opacity: hasResults ? 1 : 0.5,
              cursor: hasResults ? "pointer" : "not-allowed",
            }}
          >
            <ShieldCheck size={16} />
            {t("diagApplyHealthy")}
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
                    <div className="flex items-center gap-2 shrink-0">
                      {history[a.index] && (
                        <span
                          className="text-[10px] px-1.5 py-0.5 rounded-md whitespace-nowrap"
                          style={{ background: "var(--surface-2)", color: "var(--text-2)" }}
                        >
                          {t("diagLast")}: {t(history[a.index].grade)}
                        </span>
                      )}
                      <span
                        className="text-[12px] font-bold px-2.5 py-1 rounded-lg whitespace-nowrap"
                        style={{ background: `color-mix(in srgb, ${g.color} 16%, transparent)`, color: g.color }}
                      >
                        {t(g.key)}
                      </span>
                      <Tooltip label={t("diagRetest")} placement="top">
                        <button
                          onClick={() => retestOne(a)}
                          disabled={diagnosing || testingIdx !== null}
                          className="grid place-items-center w-7 h-7 rounded-lg transition-colors hover:[background:var(--surface-hover)]"
                          style={{
                            color: "var(--text-2)",
                            opacity: diagnosing || (testingIdx !== null && testingIdx !== a.index) ? 0.4 : 1,
                            cursor: diagnosing || testingIdx !== null ? "not-allowed" : "pointer",
                          }}
                        >
                          <RotateCw size={14} className={testingIdx === a.index ? "animate-spin" : ""} />
                        </button>
                      </Tooltip>
                    </div>
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
                      icon={<Waves size={13} />}
                      label={t("diagJitter")}
                      value={lat ? (lat.ok && lat.jitterMs >= 0 ? `${lat.jitterMs}` : "—") : "—"}
                      unit={lat && lat.ok && lat.jitterMs >= 0 ? "ms" : ""}
                      ok={!lat || lat.ok}
                    />
                    <Metric
                      icon={<PackageX size={13} />}
                      label={t("diagLoss")}
                      value={lat ? lossText(lat).replace("%", "") : "—"}
                      unit={lat && typeof lat.lossPct === "number" ? "%" : ""}
                      ok={!lat || lat.lossPct < 1}
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

        {/* 诊断历史趋势曲线（Plus 专属）：按所选指标展示各网卡随时间变化 */}
        {hasTrend && (
          <div className="panel p-5 flex flex-col gap-4">
            <div className="flex items-center gap-2 flex-wrap">
              <span className="flex items-center gap-1.5 text-[13px] font-semibold">
                <TrendingUp size={15} style={{ color: "var(--accent-soft)" }} />
                {t("diagTrend")}
              </span>
              <div className="flex-1" />
              <div className="flex items-center gap-1 rounded-lg p-0.5" style={{ background: "var(--surface-2)", border: "1px solid var(--border)" }}>
                {(
                  [
                    ["latencyMs", t("diagLatency")],
                    ["jitterMs", t("diagJitter")],
                    ["lossPct", t("diagLoss")],
                    ["mbps", t("diagSpeed")],
                  ] as [TrendMetric, string][]
                ).map(([m, label]) => (
                  <button
                    key={m}
                    onClick={() => setTrendMetric(m)}
                    aria-label={label}
                    className="px-2.5 py-1 rounded-md text-[12px] font-medium transition-colors"
                    style={{
                      background: trendMetric === m ? "color-mix(in srgb, var(--accent) 20%, transparent)" : "transparent",
                      color: trendMetric === m ? "var(--accent-soft)" : "var(--text-2)",
                    }}
                  >
                    {label}
                  </button>
                ))}
              </div>
            </div>
            <div className="grid gap-4" style={{ gridTemplateColumns: "repeat(auto-fill, minmax(280px, 1fr))" }}>
              {valid
                .filter((a) => (trend[a.index]?.length ?? 0) > 1)
                .map((a) => (
                  <div key={a.index} className="rounded-xl p-3" style={{ background: "var(--surface-2)", border: "1px solid var(--border)" }}>
                    <div className="flex items-center justify-between mb-2">
                      <span className="text-[12px] font-medium truncate">{a.alias}</span>
                      <span className="mono text-[10px]" style={{ color: "var(--text-2)" }}>
                        {trend[a.index]?.length ?? 0} pts
                      </span>
                    </div>
                    <div style={{ height: 120 }}>
                      <AreaChart data={metricSeries(trend[a.index])} />
                    </div>
                  </div>
                ))}
            </div>
          </div>
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
