import { useEffect, useRef, useState } from "react";
import { getCurrentWindow, PhysicalPosition, LogicalSize } from "@tauri-apps/api/window";
import { Pause, Play } from "lucide-react";
import {
  api,
  emitTrayToggle,
  onBoostState,
  onHudConfig,
  onHudNotice,
  onTelemetry,
  type HudConfig,
  type TelemetryPayload,
} from "../lib/api";
import { ACCENTS, type AccentKey } from "../store";

const HUD_POS_KEY = "hmx-hud-pos";
const SPARK_LEN = 28;
const HUD_WIDTH = 232;

function initialConfig(): HudConfig {
  let s: Record<string, unknown> = {};
  try {
    s = JSON.parse(localStorage.getItem("hmx-plus-settings") || "{}");
  } catch {
    /* ignore */
  }
  const accentKey = (s.accent as AccentKey) || "blue";
  const a = ACCENTS[accentKey] ?? ACCENTS.blue;
  return {
    opacity: typeof s.hudOpacity === "number" ? s.hudOpacity : 0.92,
    locked: !!s.hudLocked,
    unit: (s.hudUnit as string) || "mbps",
    showDown: s.hudShowDown !== false,
    showUp: s.hudShowUp !== false,
    showConns: s.hudShowConns !== false,
    showNics: !!s.hudShowNics,
    accent: a.accent,
    accentSoft: a.soft,
    theme: (s.theme as string) || "dark",
    clickThrough: !!s.hudClickThrough,
  };
}

function fmtSpeed(mbps: number, unit: string): { value: string; label: string } {
  if (unit === "mbit") {
    const v = mbps * 8;
    return { value: v.toFixed(v >= 100 ? 0 : 1), label: "Mbps" };
  }
  return { value: mbps.toFixed(mbps >= 100 ? 0 : 1), label: "MB/s" };
}

export function Hud() {
  const [cfg, setCfg] = useState<HudConfig>(initialConfig);
  const [running, setRunning] = useState(false);
  const [tele, setTele] = useState<TelemetryPayload | null>(null);
  const [hist, setHist] = useState<number[]>(new Array(SPARK_LEN).fill(0));
  const [nicHist, setNicHist] = useState<Record<string, number[]>>({});
  const [notice, setNotice] = useState<{ kind: string; msg: string } | null>(null);
  const cardRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    document.documentElement.style.background = "transparent";
    document.body.style.background = "transparent";
    const r = document.getElementById("root");
    if (r) r.style.background = "transparent";
  }, []);

  // 恢复上次保存的位置
  useEffect(() => {
    (async () => {
      try {
        const raw = localStorage.getItem(HUD_POS_KEY);
        if (raw) {
          const { x, y } = JSON.parse(raw);
          if (Number.isFinite(x) && Number.isFinite(y)) {
            await getCurrentWindow().setPosition(new PhysicalPosition(Math.round(x), Math.round(y)));
          }
        }
      } catch {
        /* ignore */
      }
    })();
  }, []);

  // 订阅配置 / 数据 / 吸附事件
  useEffect(() => {
    const uns: Array<() => void> = [];
    onHudConfig((c) => setCfg(c)).then((u) => uns.push(u));
    onBoostState((r) => setRunning(r)).then((u) => uns.push(u));
    api.getBoostState().then(setRunning).catch(() => {});
    onTelemetry((p) => {
      setTele(p);
      setHist((prev) => [...prev.slice(1), p.total.downMbps]);
      setNicHist((prev) => {
        const next: Record<string, number[]> = {};
        for (const n of p.perNic) {
          const base = prev[n.name] ?? new Array(SPARK_LEN).fill(0);
          next[n.name] = [...base.slice(-(SPARK_LEN - 1)), n.downMbps];
        }
        return next;
      });
    }).then((u) => uns.push(u));
    let noticeTimer: ReturnType<typeof setTimeout> | undefined;
    onHudNotice((n) => {
      setNotice(n);
      if (noticeTimer) clearTimeout(noticeTimer);
      noticeTimer = setTimeout(() => setNotice(null), 3600);
    }).then((u) => uns.push(u));
    return () => {
      if (noticeTimer) clearTimeout(noticeTimer);
      uns.forEach((u) => u());
    };
  }, []);

  // 保存拖动后的位置
  useEffect(() => {
    let un: (() => void) | undefined;
    getCurrentWindow()
      .onMoved(({ payload }) => {
        localStorage.setItem(HUD_POS_KEY, JSON.stringify({ x: payload.x, y: payload.y }));
      })
      .then((u) => (un = u));
    return () => un?.();
  }, []);

  // 内容高度自适应：随显示项 / 分网卡数量动态调整窗口高度
  useEffect(() => {
    const el = cardRef.current;
    if (!el) return;
    const resize = () => {
      const h = Math.ceil(el.offsetHeight) + 12; // 12 = 外层 p-1.5 上下内边距
      getCurrentWindow().setSize(new LogicalSize(HUD_WIDTH, h)).catch(() => {});
    };
    const ro = new ResizeObserver(resize);
    ro.observe(el);
    resize();
    return () => ro.disconnect();
  }, []);

  // 点击穿透：开启后悬浮窗忽略鼠标事件，点击穿到下层窗口（在主程序设置中关闭可恢复交互）
  useEffect(() => {
    getCurrentWindow().setIgnoreCursorEvents(cfg.clickThrough).catch(() => {});
  }, [cfg.clickThrough]);

  const down = tele?.total.downMbps ?? 0;
  const up = tele?.total.upMbps ?? 0;
  const conns = tele?.total.connections ?? 0;
  const d = fmtSpeed(down, cfg.unit);
  const u = fmtSpeed(up, cfg.unit);

  const max = Math.max(...hist, 0.001);
  const w = 200;
  const h = 26;
  const pts = hist
    .map((v, i) => `${((i / (hist.length - 1)) * w).toFixed(1)},${(h - 1 - (v / max) * (h - 2)).toFixed(1)}`)
    .join(" ");

  const light = cfg.theme === "light";
  const txt0 = light ? "#111722" : "#e7eaee";
  const txt2 = light ? "#8995a4" : "#5b636d";
  const cardBg = (light ? "rgba(255,255,255," : "rgba(16,19,26,") + cfg.opacity + ")";

  const nics = cfg.showNics ? (tele?.perNic ?? []).slice(0, 4) : [];
  const drag = !cfg.locked ? "" : undefined;

  return (
    <div
      data-tauri-drag-region={drag}
      onDoubleClick={() => api.restoreMain().catch(() => {})}
      className="w-screen p-1.5 select-none"
      style={{ cursor: cfg.locked ? "default" : "grab" }}
    >
      <div
        ref={cardRef}
        data-tauri-drag-region={drag}
        className="w-full rounded-2xl px-3.5 py-3 flex flex-col gap-1.5"
        style={{
          background: cardBg,
          border: `1px solid ${light ? "rgba(15,30,60,0.12)" : "rgba(255,255,255,0.1)"}`,
          boxShadow: "0 12px 34px -14px rgba(0,0,0,0.6)",
          backdropFilter: "blur(14px)",
        }}
      >
        {/* 顶部：状态 + 品牌 + 启停按钮 */}
        <div data-tauri-drag-region={drag} className="flex items-center gap-2">
          <span
            className="w-2 h-2 rounded-full"
            style={{ background: running ? "#3ecf8e" : txt2, boxShadow: running ? "0 0 7px #3ecf8e" : "none" }}
          />
          <span className="text-[11px] font-bold tracking-tight" style={{ color: txt0 }}>
            HypoMux<span style={{ color: cfg.accentSoft }}>Plus</span>
          </span>
          <div className="flex-1" />
          <button
            onClick={() => emitTrayToggle()}
            className="grid place-items-center w-[22px] h-[22px] rounded-md transition-transform hover:scale-110"
            style={{
              background: running ? "rgba(240,97,109,0.16)" : cfg.accent,
              color: running ? "#f0616d" : "#fff",
            }}
          >
            {running ? <Pause size={12} /> : <Play size={12} />}
          </button>
        </div>

        {/* 迷你曲线 */}
        <svg data-tauri-drag-region={drag} width="100%" height={h} viewBox={`0 0 ${w} ${h}`} preserveAspectRatio="none" className="block">
          <polyline
            points={pts}
            fill="none"
            stroke={cfg.accentSoft}
            strokeWidth="1.6"
            strokeLinejoin="round"
            strokeLinecap="round"
            opacity={running ? 1 : 0.4}
          />
        </svg>

        {/* 总览指标 */}
        {(cfg.showDown || cfg.showUp || cfg.showConns) && (
          <div data-tauri-drag-region={drag} className="flex items-end justify-between gap-2">
            {cfg.showDown && (
              <Metric label="↓" value={d.value} unit={d.label} color={cfg.accentSoft} txt0={txt0} txt2={txt2} />
            )}
            {cfg.showUp && <Metric label="↑" value={u.value} unit={u.label} color={txt0} txt0={txt0} txt2={txt2} />}
            {cfg.showConns && (
              <Metric label="⇄" value={String(conns)} unit="conns" color={txt0} txt0={txt0} txt2={txt2} />
            )}
          </div>
        )}

        {/* 分网卡明细 */}
        {nics.length > 0 && (
          <div data-tauri-drag-region={drag} className="flex flex-col gap-1 mt-0.5 pt-1.5" style={{ borderTop: `1px solid ${light ? "rgba(15,30,60,0.08)" : "rgba(255,255,255,0.06)"}` }}>
            {nics.map((n) => {
              const nh = nicHist[n.name] ?? [];
              const nmax = Math.max(...nh, 0.001);
              const sw = 64;
              const sh = 14;
              const npts =
                nh.length > 1
                  ? nh
                      .map((v, i) => `${((i / (nh.length - 1)) * sw).toFixed(1)},${(sh - 1 - (v / nmax) * (sh - 2)).toFixed(1)}`)
                      .join(" ")
                  : `0,${sh - 1} ${sw},${sh - 1}`;
              return (
                <div key={n.index} className="flex items-center gap-2">
                  <span className="text-[9px] truncate flex-1" style={{ color: txt2 }}>
                    {n.name}
                  </span>
                  <svg width={sw} height={sh} viewBox={`0 0 ${sw} ${sh}`} preserveAspectRatio="none" className="shrink-0">
                    <polyline
                      points={npts}
                      fill="none"
                      stroke={cfg.accentSoft}
                      strokeWidth="1.3"
                      strokeLinejoin="round"
                      strokeLinecap="round"
                      opacity={n.downMbps > 0 ? 1 : 0.4}
                    />
                  </svg>
                  <span className="text-[9px] mono w-[34px] text-right" style={{ color: txt0 }}>
                    {fmtSpeed(n.downMbps, cfg.unit).value}
                  </span>
                </div>
              );
            })}
          </div>
        )}
        {/* HUD 内提示（来自主窗口的同步通知，托盘模式下也能看到反馈） */}
        {notice && (
          <div
            className="flex items-center gap-1.5 mt-0.5 px-2 py-1.5 rounded-lg text-[10.5px] leading-snug"
            style={{
              background:
                notice.kind === "error"
                  ? "rgba(240,97,109,0.16)"
                  : notice.kind === "warning"
                  ? "rgba(227,179,65,0.16)"
                  : notice.kind === "success"
                  ? "rgba(62,207,142,0.16)"
                  : "var(--surface-2)",
              color:
                notice.kind === "error"
                  ? "#f0616d"
                  : notice.kind === "warning"
                  ? "#e3b341"
                  : notice.kind === "success"
                  ? "#3ecf8e"
                  : txt0,
              border: `1px solid ${light ? "rgba(15,30,60,0.1)" : "rgba(255,255,255,0.08)"}`,
            }}
          >
            {notice.msg}
          </div>
        )}
      </div>
    </div>
  );
}

function Metric({
  label,
  value,
  unit,
  color,
  txt0,
  txt2,
}: {
  label: string;
  value: string;
  unit: string;
  color: string;
  txt0: string;
  txt2: string;
}) {
  return (
    <div className="flex flex-col leading-none min-w-0">
      <span className="text-[9px] mono" style={{ color: txt2 }}>
        {label} {unit}
      </span>
      <span className="text-[16px] font-bold mono mt-0.5 truncate" style={{ color: color || txt0 }}>
        {value}
      </span>
    </div>
  );
}
