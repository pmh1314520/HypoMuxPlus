import { useId } from "react";

interface Props {
  size?: number;
  /** 圆角半径（相对 48 坐标系） */
  radius?: number;
  glow?: boolean;
  /** 加速运行中：中心汇聚节点发出脉冲波纹 */
  running?: boolean;
}

/**
 * HypoMuxPlus 品牌标识。
 * 视觉概念：三路独立链路（多网卡）经中心节点汇聚为一条高带宽主干 ——
 * 直观隐喻"多网卡带宽聚合 / 链路多路复用"。深蓝→遥测青渐变，工业精密感。
 */
export function Logo({ size = 40, radius = 13, glow = true, running = false }: Props) {
  const id = useId();
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 48 48"
      fill="none"
      style={{ filter: glow ? "drop-shadow(0 6px 16px rgba(34,211,238,0.35))" : "none" }}
    >
      <defs>
        <linearGradient id={`bg-${id}`} x1="4" y1="2" x2="44" y2="46" gradientUnits="userSpaceOnUse">
          <stop offset="0%" stopColor="#0a64e0" />
          <stop offset="55%" stopColor="#1f8fff" />
          <stop offset="100%" stopColor="#22d3ee" />
        </linearGradient>
        <linearGradient id={`trunk-${id}`} x1="24" y1="24" x2="40" y2="24" gradientUnits="userSpaceOnUse">
          <stop offset="0%" stopColor="#ffffff" />
          <stop offset="100%" stopColor="#e6fbff" />
        </linearGradient>
      </defs>

      {/* 渐变底座 */}
      <rect x="0" y="0" width="48" height="48" rx={radius} fill={`url(#bg-${id})`} />
      {/* 顶部内高光 + 描边 */}
      <rect
        x="1"
        y="1"
        width="46"
        height="46"
        rx={radius - 1}
        fill="none"
        stroke="rgba(255,255,255,0.28)"
        strokeWidth="1"
      />
      <path d={`M${radius} 1 H${48 - radius}`} stroke="rgba(255,255,255,0.55)" strokeWidth="1.4" strokeLinecap="round" />

      {/* 汇聚链路 */}
      <g fill="none" strokeLinecap="round">
        <path d="M11 14 C 19 14, 18 24, 24 24" stroke="#ffffff" strokeOpacity="0.62" strokeWidth="2.2" />
        <path d="M11 24 H 24" stroke="#ffffff" strokeOpacity="0.95" strokeWidth="2.2" />
        <path d="M11 34 C 19 34, 18 24, 24 24" stroke="#ffffff" strokeOpacity="0.62" strokeWidth="2.2" />
        <path d="M24 24 H 39" stroke={`url(#trunk-${id})`} strokeWidth="3.4" />
      </g>

      {/* 节点 */}
      <circle cx="11" cy="14" r="2.5" fill="#ffffff" fillOpacity="0.82" />
      <circle cx="11" cy="24" r="2.5" fill="#ffffff" />
      <circle cx="11" cy="34" r="2.5" fill="#ffffff" fillOpacity="0.82" />
      {/* 中心汇聚节点（带光晕） */}
      <circle cx="24" cy="24" r="6" fill="#ffffff" fillOpacity="0.18" />
      <circle cx="24" cy="24" r="3.6" fill="#ffffff" />
      {/* 运行态：中心节点向外扩散的脉冲波纹 */}
      {running && (
        <circle cx="24" cy="24" r="6" fill="none" stroke="#ffffff" strokeWidth="1.2">
          <animate attributeName="r" values="5;11;5" dur="2.2s" repeatCount="indefinite" />
          <animate attributeName="stroke-opacity" values="0.55;0;0.55" dur="2.2s" repeatCount="indefinite" />
        </circle>
      )}
      {/* 主干输出节点 */}
      <circle cx="39" cy="24" r="3.6" fill="#ffffff" />
      <circle cx="39" cy="24" r="6" fill="none" stroke="#ffffff" strokeOpacity="0.5" strokeWidth="1.4" />
    </svg>
  );
}
