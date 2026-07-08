import { useId } from "react";

interface Props {
  data: number[];
  running?: boolean;
  /** 网格基准刻度数（横线数量） */
  gridLines?: number;
}

/**
 * 实时下行吞吐面积图（纯 SVG）。
 * 含动态 Y 轴刻度网格、渐变填充、辉光折线与当前值锚点，
 * 营造网络监控仪表盘的专业观感。自适应容器尺寸。
 */
export function AreaChart({ data, running = false, gridLines = 4 }: Props) {
  const gid = useId();
  const W = 1000;
  const H = 260;
  const padTop = 18;
  const padBottom = 10;
  const n = data.length;

  const rawMax = Math.max(...data, 0.1);
  // 取“友好”的刻度上限
  const niceMax = niceCeil(rawMax * 1.2);

  const stepX = n > 1 ? W / (n - 1) : 0;
  const yOf = (v: number) => padTop + (1 - Math.min(v, niceMax) / niceMax) * (H - padTop - padBottom);

  const pts = data.map((v, i) => [i * stepX, yOf(v)] as const);
  const line =
    pts.length > 1 ? pts.map((p, i) => `${i ? "L" : "M"}${p[0].toFixed(1)},${p[1].toFixed(1)}`).join(" ") : "";
  const area = pts.length > 1 ? `${line} L${W},${H - padBottom} L0,${H - padBottom} Z` : "";

  const last = pts[pts.length - 1];

  const gridYs = Array.from({ length: gridLines + 1 }, (_, i) => padTop + (i * (H - padTop - padBottom)) / gridLines);

  return (
    <svg viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none" width="100%" height="100%" style={{ display: "block" }}>
      <defs>
        <linearGradient id={`f-${gid}`} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor="var(--accent)" stopOpacity={running ? 0.4 : 0.16} />
          <stop offset="55%" stopColor="var(--cyan)" stopOpacity={running ? 0.12 : 0.05} />
          <stop offset="100%" stopColor="var(--accent)" stopOpacity="0" />
        </linearGradient>
        <linearGradient id={`l-${gid}`} x1="0" y1="0" x2="1" y2="0">
          <stop offset="0%" stopColor="var(--accent-deep)" />
          <stop offset="100%" stopColor="var(--cyan)" />
        </linearGradient>
      </defs>

      {/* 网格横线 */}
      {gridYs.map((y, i) => (
        <line
          key={i}
          x1="0"
          x2={W}
          y1={y.toFixed(1)}
          y2={y.toFixed(1)}
          stroke="var(--border)"
          strokeWidth="1"
          strokeDasharray={i === gridLines ? "0" : "3 6"}
          vectorEffect="non-scaling-stroke"
        />
      ))}

      {area && <path d={area} fill={`url(#f-${gid})`} />}
      {line && (
        <path
          d={line}
          fill="none"
          stroke={`url(#l-${gid})`}
          strokeWidth={2.4}
          strokeLinejoin="round"
          strokeLinecap="round"
          vectorEffect="non-scaling-stroke"
          style={{ filter: running ? "drop-shadow(0 0 5px var(--accent-glow))" : "none" }}
        />
      )}

      {/* 当前值锚点 */}
      {running && last && (
        <g>
          <circle cx={last[0]} cy={last[1]} r="9" fill="var(--cyan)" opacity="0.18" />
          <circle cx={last[0]} cy={last[1]} r="3.5" fill="var(--cyan)" vectorEffect="non-scaling-stroke" />
        </g>
      )}
    </svg>
  );
}

export function niceCeil(v: number): number {
  if (v <= 1) return 1;
  const mag = Math.pow(10, Math.floor(Math.log10(v)));
  const norm = v / mag;
  const step = norm <= 1 ? 1 : norm <= 2 ? 2 : norm <= 5 ? 5 : 10;
  return step * mag;
}
