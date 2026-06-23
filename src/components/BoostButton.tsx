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
      className={`relative flex items-center justify-center gap-2.5 h-[52px] px-8 rounded-2xl font-semibold text-[15px] text-white overflow-hidden ${
        running && !busy ? "pulse-ring" : ""
      }`}
      style={{
        background: running
          ? "linear-gradient(135deg, #ff5d5d, #d83a3a)"
          : "linear-gradient(135deg, var(--accent), var(--accent-soft))",
        boxShadow: running ? "0 10px 30px rgba(216,58,58,0.4)" : "0 10px 30px var(--accent-glow)",
        opacity: disabled ? 0.5 : 1,
        cursor: disabled || busy ? "not-allowed" : "pointer",
      }}
    >
      {busy ? (
        <Loader2 size={19} className="animate-spin" />
      ) : running ? (
        <Power size={19} />
      ) : (
        <Zap size={19} fill="currentColor" />
      )}
      {label}
    </motion.button>
  );
}
