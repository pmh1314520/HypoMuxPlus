import { motion } from "framer-motion";

interface Props {
  checked: boolean;
  onChange: (v: boolean) => void;
  disabled?: boolean;
  /** 无障碍标签：当开关无可见文字标签时提供（可选） */
  ariaLabel?: string;
}

/** 自研开关，统一设计语言，替代原生 checkbox。 */
export function Switch({ checked, onChange, disabled, ariaLabel }: Props) {
  return (
    <motion.button
      role="switch"
      aria-checked={checked}
      aria-label={ariaLabel}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      whileTap={disabled ? undefined : { scale: 0.92 }}
      transition={{ type: "spring", stiffness: 500, damping: 30 }}
      className="relative w-[42px] h-[24px] rounded-full transition-colors shrink-0"
      style={{
        background: checked ? "linear-gradient(90deg, var(--accent-deep), var(--accent))" : "var(--surface-2)",
        border: `1px solid ${checked ? "transparent" : "var(--border-strong)"}`,
        boxShadow: checked ? "0 0 12px var(--accent-glow)" : "none",
        opacity: disabled ? 0.5 : 1,
        cursor: disabled ? "not-allowed" : "pointer",
      }}
    >
      <motion.span
        layout
        transition={{ type: "spring", stiffness: 520, damping: 32 }}
        className="absolute top-[2px] h-[18px] rounded-full bg-white"
        style={{ left: checked ? 21 : 2, width: 18, boxShadow: "0 2px 5px rgba(0,0,0,0.35)" }}
      />
    </motion.button>
  );
}
