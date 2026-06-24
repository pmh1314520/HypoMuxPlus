import type { ReactNode } from "react";

interface Props {
  icon: ReactNode;
  text: string;
  hint?: string;
  compact?: boolean;
}

/** 统一空状态：图标 + 文案，贯穿各面板保持一致观感 */
export function EmptyState({ icon, text, hint, compact }: Props) {
  return (
    <div className="grid place-items-center h-full px-4">
      <div className="flex flex-col items-center text-center" style={{ maxWidth: 280 }}>
        <span
          className={`grid place-items-center rounded-2xl ${compact ? "w-9 h-9 mb-2" : "w-11 h-11 mb-2.5"}`}
          style={{ background: "var(--surface-2)", border: "1px solid var(--border)", color: "var(--text-2)" }}
        >
          {icon}
        </span>
        <div className={`${compact ? "text-[12px]" : "text-[12.5px]"} leading-relaxed`} style={{ color: "var(--text-2)" }}>
          {text}
        </div>
        {hint && (
          <div className="text-[11px] mt-1" style={{ color: "var(--text-2)", opacity: 0.75 }}>
            {hint}
          </div>
        )}
      </div>
    </div>
  );
}
