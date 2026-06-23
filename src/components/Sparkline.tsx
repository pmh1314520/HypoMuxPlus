import { useId } from "react";

interface Props {
  data: number[];
  height?: number;
  running?: boolean;
}

/**
 * 实时下行速度波形图（纯 SVG，零依赖）。
 * 自适应宽度，根据数据动态归一化，运行态带渐变填充与发光描边。
 */
export function Sparkline({ data, height = 90, running = false }: Props) {
  const gid = useId();
  const W = 600;
  const H = height;
  const pad = 6;
  const n = data.length;

  const max = Math.max(1, ...data) * 1.15;
  const stepX = n > 1 ? (W - pad * 2) / (n - 1) : 0;

  const points = data.map((v, i) => {
    const x = pad + i * stepX;
    const y = H - pad - (Math.min(v, max) / max) * (H - pad * 2);
    return [x, y] as const;
  });

  const line =
    points.length > 1
      ? points.map((p, i) => `${i === 0 ? "M" : "L"}${p[0].toFixed(1)},${p[1].toFixed(1)}`).join(" ")
      : "";

  const area =
    points.length > 1
      ? `${line} L${points[points.length - 1][0].toFixed(1)},${H - pad} L${points[0][0].toFixed(1)},${
          H - pad
        } Z`
      : "";

  return (
    <svg
      viewBox={`0 0 ${W} ${H}`}
      preserveAspectRatio="none"
      width="100%"
      height={H}
      style={{ display: "block" }}
    >
      <defs>
        <linearGradient id={`fill-${gid}`} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor="var(--accent)" stopOpacity={running ? 0.42 : 0.18} />
          <stop offset="100%" stopColor="var(--accent)" stopOpacity="0" />
        </linearGradient>
      </defs>
      {area && <path d={area} fill={`url(#fill-${gid})`} />}
      {line && (
        <path
          d={line}
          fill="none"
          stroke="var(--accent)"
          strokeWidth={2.2}
          strokeLinejoin="round"
          strokeLinecap="round"
          style={{ filter: running ? "drop-shadow(0 0 6px var(--accent-glow))" : "none" }}
        />
      )}
    </svg>
  );
}
