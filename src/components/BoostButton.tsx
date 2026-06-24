import { motion } from "framer-motion";
import { Loader2, Power, Zap } from "lucide-react";
import { useSettings } from "../store";

interface Props {
  running: boolean;
  busy: boolean;
  disabled?: boolean;
  onClick: () => void;
}

export function BoostButton({ running, busy, disabled, onClick }: Props) {
  const { t } = useSettings();

  const label = busy
    ? running
      ? t("boostStopping")
      : t("boostStarting")
    : running
    ? t("boostStop")
    : t("boostStart");

  return (
    <motion.button
      whileHover={{ scale: disabled || busy ? 1 : 1.03 }}
      whileTap={{ scale: disabled || busy ? 1 : 0.97 }}
      disabled={disabled || busy}
      onClick={onClick}
      className="relative flex items-center justify-center gap-2.5 h-[48px] px-7 rounded-xl font-semibold text-[14px] text-white overflow-hidden"
      style={{
        background: running
          ? "linear-gradient(135deg, #e0535e, #c43a44)"
          : "linear-gradient(135deg, var(--accent), var(--accent-deep))",
        boxShadow: running ? "0 6px 18px -8px rgba(196,58,68,0.6)" : "0 6px 18px -8px var(--accent-glow)",
        opacity: disabled ? 0.45 : 1,
        cursor: disabled || busy ? "not-allowed" : "pointer",
      }}
    >
      {/* 运行态柔和脉冲光晕 */}
      {running && !busy && (
        <motion.span
          className="absolute inset-0 rounded-xl pointer-events-none"
          style={{ background: "radial-gradient(circle at 50% 50%, rgba(255,255,255,0.2), transparent 62%)" }}
          animate={{ opacity: [0.25, 0.6, 0.25] }}
          transition={{ duration: 2, repeat: Infinity, ease: "easeInOut" }}
        />
      )}
      {/* 待机态轻扫光泽，提示可点击 */}
      {!running && !busy && !disabled && (
        <motion.span
          className="absolute top-0 bottom-0 w-1/3 pointer-events-none"
          style={{ background: "linear-gradient(90deg, transparent, rgba(255,255,255,0.22), transparent)" }}
          animate={{ x: ["-160%", "360%"] }}
          transition={{ duration: 2.4, repeat: Infinity, ease: "easeInOut", repeatDelay: 1.4 }}
        />
      )}
      <span className="relative z-10 flex items-center justify-center gap-2.5">
        {busy ? (
          <Loader2 size={19} className="animate-spin" />
        ) : running ? (
          <Power size={19} />
        ) : (
          <Zap size={19} fill="currentColor" />
        )}
        {label}
      </span>
    </motion.button>
  );
}
