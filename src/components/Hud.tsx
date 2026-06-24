import { useEffect, useState } from "react";
import { getCurrentWindow, currentMonitor, PhysicalPosition } from "@tauri-apps/api/window";
import {
  api,
  onBoostState,
  onHudConfig,
  onHudSnap,
  onTelemetry,
  type HudConfig,
  type TelemetryPayload,
} from "../lib/api";
import { ACCENTS, type AccentKey } from "../store";

const HUD_POS_KEY = "hmx-hud-pos";
const SPARK_LEN = 28;

// 从主设置 localStorage 读取初始 HUD 配置（与主窗口同源共享）
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
    accent: a.accent,
    accentSoft: a.soft,
    theme: (s.theme as string) || "dark",
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

  // 透明背景：HUD 文档根透明
  useEffect(() => {
    document.documentElement.style.background = "transparent";
    document.body.style.background = "transparent";
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
    }).then((u) => uns.push(u));
    onHudSnap((corner) => void snapTo(corner)).then((u) => uns.push(u));
    return () => uns.forEach((u) => u());
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

  const snapTo = async (corner: string) => {
    try {
      const mon = await currentMonitor();
      if (!mon) return;
      const win = getCurrentWindow();
      const size = await win.outerSize();
      const m = 16 * (mon.scaleFactor || 1);
      const sx = mon.position.x;
      const sy = mon.position.y;
      const sw = mon.size.width;
      const sh = mon.size.height;
      let x = sx + m;
      let y = sy + m;
      if (corner.includes("r")) x = sx + sw - size.width - m;
      if (corner.includes("b")) y = sy + sh - size.height - m;
      await win.setPosition(new PhysicalPosition(Math.round(x), Math.round(y)));
      localStorage.setItem(HUD_POS_KEY, JSON.stringify({ x: Math.round(x), y: Math.round(y) }));
    } catch {
      /* ignore */
    }
  };

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
  const cardBg = light ? "rgba(255,255,255,VAR)" : "rgba(16,19,26,VAR)";
  const txt0 = light ? "#111722" : "#e7eaee";
  const txt2 = light ? "#8995a4" : "#5b636d";

  return (
    <div
      data-tauri-drag-region={!cfg.locked ? "" : undefined}
      onDoubleClick={() => api.restoreMain().catch(() => {})}
      className="w-screen h-screen p-1.5 select-none"
      style={{ cursor: cfg.locked ? "default" : "grab" }}
    >
      <div
        data-tauri-drag-region={!cfg.locked ? "" : undefined}
        className="w-full h-full rounded-2xl px-3.5 py-3 flex flex-col gap-1.5 pointer-events-auto"
        style={{
          background: cardBg.replace("VAR", String(cfg.opacity)),
          border: `1px solid ${light ? "rgba(15,30,60,0.12)" : "rgba(255,255,255,0.1)"}`,
          boxShadow: "0 12px 34px -14px rgba(0,0,0,0.6)",
          backdropFilter: "blur(14px)",
        }}
      >
        {/* 顶部：品牌 + 状态点 */}
        <div data-tauri-drag-region={!cfg.locked ? "" : undefined} className="flex items-center gap-2">
          <span
            className="w-2 h-2 rounded-full"
            style={{
              background: running ? "#3ecf8e" : txt2,
              boxShadow: running ? "0 0 7px #3ecf8e" : "none",
            }}
          />
          <span className="text-[11px] font-bold tracking-tight" style={{ color: txt0 }}>
            HypoMux<span style={{ color: cfg.accentSoft }}>Plus</span>
          </span>
          <div className="flex-1" />
          <span className="text-[9px] mono" style={{ color: txt2 }}>
            {running ? "LIVE" : "IDLE"}
          </span>
        </div>

        {/* 迷你曲线 */}
        <svg width="100%" height={h} viewBox={`0 0 ${w} ${h}`} preserveAspectRatio="none" className="block">
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

        {/* 指标 */}
        <div className="flex items-end justify-between gap-2">
          {cfg.showDown && (
            <Metric label="↓" value={d.value} unit={d.label} color={cfg.accentSoft} txt0={txt0} txt2={txt2} />
          )}
          {cfg.showUp && <Metric label="↑" value={u.value} unit={u.label} color={txt0} txt0={txt0} txt2={txt2} />}
          {cfg.showConns && (
            <Metric label="⇄" value={String(conns)} unit="conns" color={txt0} txt0={txt0} txt2={txt2} />
          )}
        </div>
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
