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
