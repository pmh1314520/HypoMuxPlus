import { createContext, useCallback, useContext, useRef, useState, type ReactNode } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { CheckCircle2, AlertTriangle, Info, X, XCircle } from "lucide-react";
import { useSettings } from "../store";

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
  const { t } = useSettings();
  const [items, setItems] = useState<ToastItem[]>([]);
  const timers = useRef<Map<number, ReturnType<typeof setTimeout>>>(new Map());

  const remove = useCallback((id: number) => {
    const tm = timers.current.get(id);
    if (tm) {
      clearTimeout(tm);
      timers.current.delete(id);
    }
    setItems((prev) => prev.filter((t) => t.id !== id));
  }, []);

  const schedule = useCallback(
    (id: number) => {
      const existing = timers.current.get(id);
      if (existing) clearTimeout(existing);
      timers.current.set(
        id,
        setTimeout(() => remove(id), 4200),
      );
    },
    [remove],
  );

  const pause = useCallback((id: number) => {
    const tm = timers.current.get(id);
    if (tm) {
      clearTimeout(tm);
      timers.current.delete(id);
    }
  }, []);

  const push = useCallback(
    (kind: ToastKind, msg: string) => {
      const id = Date.now() + Math.random();
      setItems((prev) => [...prev, { id, kind, msg }]);
      schedule(id);
    },
    [schedule],
  );

  return (
    <ToastCtx.Provider value={push}>
      {children}
      {/* aria-live 区域：让屏幕阅读器自动播报新出现的通知 */}
      <div
        className="fixed top-16 right-5 z-[999] flex flex-col gap-2.5 pointer-events-none"
        aria-live="polite"
        aria-atomic="false"
      >
        <AnimatePresence>
          {items.map((item) => (
            <motion.div
              key={item.id}
              role={item.kind === "error" || item.kind === "warning" ? "alert" : "status"}
              initial={{ opacity: 0, x: 60, scale: 0.9 }}
              animate={{ opacity: 1, x: 0, scale: 1 }}
              exit={{ opacity: 0, x: 60, scale: 0.9 }}
              transition={{ type: "spring", stiffness: 380, damping: 30 }}
              onMouseEnter={() => pause(item.id)}
              onMouseLeave={() => schedule(item.id)}
              className="glass flex items-center gap-3 pl-4 pr-2.5 py-3 max-w-[360px] shadow-2xl pointer-events-auto"
              style={{ borderLeft: `3px solid ${COLORS[item.kind]}` }}
            >
              <span style={{ color: COLORS[item.kind] }}>{ICONS[item.kind]}</span>
              <span className="text-[13px] leading-snug flex-1" style={{ color: "var(--text-0)" }}>
                {item.msg}
              </span>
              <button
                onClick={() => remove(item.id)}
                aria-label={t("toastDismiss")}
                className="grid place-items-center w-6 h-6 rounded-md shrink-0 transition-colors hover:[background:var(--surface-hover)]"
                style={{ color: "var(--text-2)" }}
              >
                <X size={13} />
              </button>
            </motion.div>
          ))}
        </AnimatePresence>
      </div>
    </ToastCtx.Provider>
  );
}

export const useToast = () => useContext(ToastCtx);
