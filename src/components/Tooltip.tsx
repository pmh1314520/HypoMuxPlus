import { useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { AnimatePresence, motion } from "framer-motion";

type Placement = "top" | "bottom" | "left" | "right";

interface Props {
  label: ReactNode;
  placement?: Placement;
  children: ReactNode;
}

/**
 * 自研悬浮提示，彻底替代浏览器原生 title。
 * 使用 Portal 渲染到 body 顶层，fixed 定位 + 弹性动效，与设计系统统一观感。
 */
export function Tooltip({ label, placement = "top", children }: Props) {
  const ref = useRef<HTMLSpanElement>(null);
  const [show, setShow] = useState(false);
  const [pos, setPos] = useState({ x: 0, y: 0 });

  const open = () => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    const gap = 14;
    switch (placement) {
      case "bottom":
        setPos({ x: r.left + r.width / 2, y: r.bottom + gap });
        break;
      case "left":
        setPos({ x: r.left - gap, y: r.top + r.height / 2 });
        break;
      case "right":
        setPos({ x: r.right + gap, y: r.top + r.height / 2 });
        break;
      default:
        setPos({ x: r.left + r.width / 2, y: r.top - gap });
    }
    setShow(true);
  };

  const transform =
    placement === "top"
      ? "translate(-50%, -100%)"
      : placement === "bottom"
      ? "translate(-50%, 0)"
      : placement === "left"
      ? "translate(-100%, -50%)"
      : "translate(0, -50%)";

  const initial =
    placement === "top"
      ? { opacity: 0, y: 4, scale: 0.96 }
      : placement === "bottom"
      ? { opacity: 0, y: -4, scale: 0.96 }
      : { opacity: 0, scale: 0.96 };

  return (
    <span
      ref={ref}
      onMouseEnter={open}
      onMouseLeave={() => setShow(false)}
      onMouseDown={() => setShow(false)}
      className="inline-flex"
    >
      {children}
      {createPortal(
        <AnimatePresence>
          {show && (
            <motion.div
              initial={initial}
              animate={{ opacity: 1, y: 0, scale: 1 }}
              exit={{ opacity: 0, scale: 0.96 }}
              transition={{ duration: 0.13, ease: "easeOut" }}
              className="fixed z-[1000] pointer-events-none px-2.5 py-1.5 rounded-lg text-[11.5px] font-medium whitespace-nowrap"
              style={{
                left: pos.x,
                top: pos.y,
                transform,
                background: "color-mix(in srgb, var(--bg-1) 94%, transparent)",
                color: "var(--text-0)",
                border: "1px solid var(--border-strong)",
                boxShadow: "inset 0 1px 0 var(--hl), 0 10px 28px -8px rgba(0,0,0,0.6)",
                backdropFilter: "blur(14px) saturate(150%)",
                WebkitBackdropFilter: "blur(14px) saturate(150%)",
              }}
            >
              {label}
            </motion.div>
          )}
        </AnimatePresence>,
        document.body,
      )}
    </span>
  );
}
