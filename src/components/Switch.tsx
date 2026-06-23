import { motion } from "framer-motion";

interface Props {
  checked: boolean;
  onChange: (v: boolean) => void;
  disabled?: boolean;
}

/** 自研开关，统一设计语言，替代原生 checkbox。 */
export function Switch({ checked, onChange, disabled }: Props) {
  return (
    <button
      role="switch"
      aria-checked={checked}
      disabled={disabled}
      onClick={() => onChange(!checked)}
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
        className="absolute top-[2px] w-[18px] h-[18px] rounded-full bg-white"
        style={{ left: checked ? 21 : 2, boxShadow: "0 2px 5px rgba(0,0,0,0.35)" }}
      />
    </button>
  );
}
