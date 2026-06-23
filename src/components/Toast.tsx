import { createContext, useCallback, useContext, useState, type ReactNode } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { CheckCircle2, AlertTriangle, Info, XCircle } from "lucide-react";

type ToastKind = "success" | "warning" | "error" | "info";
interface ToastItem {
  id: number;
  kind: ToastKind;
  msg: string;
}

const ToastCtx = createContext<(kind: ToastKind, msg: string) => void>(() => {});

const ICONS: Record<ToastKind, ReactNode> = {
  success: <CheckCircle2 size={18} />,
  warning: <AlertTriangle size={18} />,
  error: <XCircle size={18} />,
  info: <Info size={18} />,
};

const COLORS: Record<ToastKind, string> = {
  success: "var(--ok)",
  warning: "var(--warn)",
  error: "var(--danger)",
  info: "var(--accent)",
};

export function ToastProvider({ children }: { children: ReactNode }) {
  const [items, setItems] = useState<ToastItem[]>([]);

  const push = useCallback((kind: ToastKind, msg: string) => {
    const id = Date.now() + Math.random();
    setItems((prev) => [...prev, { id, kind, msg }]);
    setTimeout(() => setItems((prev) => prev.filter((t) => t.id !== id)), 4200);
  }, []);

  return (
    <ToastCtx.Provider value={push}>
      {children}
      <div className="fixed top-16 right-5 z-[999] flex flex-col gap-2.5 pointer-events-none">
        <AnimatePresence>
          {items.map((t) => (
            <motion.div
              key={t.id}
              initial={{ opacity: 0, x: 60, scale: 0.9 }}
              animate={{ opacity: 1, x: 0, scale: 1 }}
              exit={{ opacity: 0, x: 60, scale: 0.9 }}
              transition={{ type: "spring", stiffness: 380, damping: 30 }}
              className="glass flex items-center gap-3 px-4 py-3 max-w-[360px] shadow-2xl"
              style={{ borderLeft: `3px solid ${COLORS[t.kind]}` }}
            >
              <span style={{ color: COLORS[t.kind] }}>{ICONS[t.kind]}</span>
              <span className="text-[13px] leading-snug" style={{ color: "var(--text-0)" }}>
                {t.msg}
              </span>
            </motion.div>
          ))}
        </AnimatePresence>
      </div>
    </ToastCtx.Provider>
  );
}

export const useToast = () => useContext(ToastCtx);
