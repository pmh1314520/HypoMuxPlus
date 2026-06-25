import { useEffect } from "react";
import { motion } from "framer-motion";
import { CheckCircle2, Cpu, Download, Zap } from "lucide-react";
import { useSettings } from "../store";
import { Logo } from "./Logo";

interface Props {
  onClose: () => void;
}

/** 首屏使用引导：仅首次启动时展示一次（localStorage 标记），三步快速上手。 */
export function Onboarding({ onClose }: Props) {
  const { t } = useSettings();

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    const prev = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      window.removeEventListener("keydown", onKey);
      document.body.style.overflow = prev;
    };
  }, [onClose]);

  const steps = [
    { icon: <Cpu size={16} />, text: t("obStep1") },
    { icon: <Zap size={16} />, text: t("obStep2") },
    { icon: <Download size={16} />, text: t("obStep3") },
  ];

  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      className="fixed inset-0 z-[200] grid place-items-center"
      style={{ background: "rgba(0,0,0,0.55)", backdropFilter: "blur(6px)" }}
    >
      <motion.div
        initial={{ opacity: 0, y: 20, scale: 0.97 }}
        animate={{ opacity: 1, y: 0, scale: 1 }}
        transition={{ type: "spring", stiffness: 260, damping: 26 }}
        className="panel w-[440px] max-w-[90vw] p-7"
        style={{ boxShadow: "var(--shadow)" }}
      >
        <div className="flex flex-col items-center text-center">
          <Logo size={54} />
          <h2 className="text-[20px] font-bold mt-4">{t("obTitle")}</h2>
          <p className="text-[12.5px] mt-1.5" style={{ color: "var(--text-2)" }}>
            {t("obSubtitle")}
          </p>
        </div>

        <div className="flex flex-col gap-3 mt-6">
          {steps.map((s, i) => (
            <motion.div
              key={i}
              initial={{ opacity: 0, x: -12 }}
              animate={{ opacity: 1, x: 0 }}
              transition={{ delay: 0.1 + i * 0.08 }}
              className="flex items-center gap-3 px-4 py-3 rounded-xl"
              style={{ background: "var(--surface-2)", border: "1px solid var(--border)" }}
            >
              <span
                className="grid place-items-center w-8 h-8 rounded-lg shrink-0"
                style={{ background: "var(--surface)", color: "var(--accent-soft)", border: "1px solid var(--border)" }}
              >
                {s.icon}
              </span>
              <span className="text-[12.5px] leading-snug" style={{ color: "var(--text-1)" }}>
                <span className="mono mr-1.5" style={{ color: "var(--accent-soft)" }}>
                  {i + 1}
                </span>
                {s.text}
              </span>
            </motion.div>
          ))}
        </div>

        <p className="text-[11px] text-center mt-4 flex items-center justify-center gap-1.5" style={{ color: "var(--text-2)" }}>
          <CheckCircle2 size={12} style={{ color: "var(--ok)" }} />
          {t("obTip")}
        </p>

        <button
          onClick={onClose}
          className="w-full mt-6 h-[44px] rounded-xl font-semibold text-[14px] text-white transition-transform hover:scale-[1.02]"
          style={{ background: "linear-gradient(135deg, var(--accent), var(--accent-deep))", boxShadow: "0 8px 22px -10px var(--accent-glow)" }}
        >
          {t("obStart")}
        </button>
      </motion.div>
    </motion.div>
  );
}
