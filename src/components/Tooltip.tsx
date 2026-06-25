import { useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { AnimatePresence, motion } from "framer-motion";

type Placement = "top" | "bottom" | "left" | "right";

interface Props {
  label: ReactNode;
  placement?: Placement;
  children: ReactNode;
}

const GAP = 10; // 与触发元素的间距，避免遮挡
const MARGIN = 8; // 与视口边缘的安全距离

/**
 * 自研悬浮提示，彻底替代浏览器原生 title。
 * Portal 渲染到 body 顶层，测量自身尺寸后做视口边界钳制与越界翻转，
 * 既不遮挡触发元素，也不会显示到屏幕外。
 */
export function Tooltip({ label, placement = "top", children }: Props) {
  const ref = useRef<HTMLSpanElement>(null);
  const tipRef = useRef<HTMLDivElement>(null);
  const [show, setShow] = useState(false);
  const [coords, setCoords] = useState<{ left: number; top: number } | null>(null);

  const place = () => {
    const trigger = ref.current;
    const tip = tipRef.current;
    if (!trigger || !tip) return;
    const r = trigger.getBoundingClientRect();
    const tw = tip.offsetWidth;
    const th = tip.offsetHeight;
    const vw = window.innerWidth;
    const vh = window.innerHeight;

    let p = placement;
    // 越界自动翻转到相对侧
    if (p === "top" && r.top - GAP - th < MARGIN) p = "bottom";
    else if (p === "bottom" && r.bottom + GAP + th > vh - MARGIN) p = "top";
    else if (p === "left" && r.left - GAP - tw < MARGIN) p = "right";
    else if (p === "right" && r.right + GAP + tw > vw - MARGIN) p = "left";

    let left: number;
    let top: number;
    if (p === "top" || p === "bottom") {
      left = r.left + r.width / 2 - tw / 2;
      top = p === "top" ? r.top - GAP - th : r.bottom + GAP;
    } else {
      left = p === "left" ? r.left - GAP - tw : r.right + GAP;
      top = r.top + r.height / 2 - th / 2;
    }
    // 钳制在视口内
    left = Math.max(MARGIN, Math.min(left, vw - tw - MARGIN));
    top = Math.max(MARGIN, Math.min(top, vh - th - MARGIN));
    setCoords({ left, top });
  };

  // 显示后立即测量定位（绘制前完成，无闪烁）
  useLayoutEffect(() => {
    if (show) place();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [show]);

  return (
    <span
      ref={ref}
      onMouseEnter={() => setShow(true)}
      onMouseLeave={() => {
        setShow(false);
        setCoords(null);
      }}
      onMouseDown={() => {
        setShow(false);
        setCoords(null);
      }}
      className="inline-flex"
    >
      {children}
      {createPortal(
        <AnimatePresence>
          {show && (
            <motion.div
              ref={tipRef}
              initial={{ opacity: 0 }}
              animate={{ opacity: coords ? 1 : 0 }}
              exit={{ opacity: 0 }}
              transition={{ duration: 0.12, ease: "easeOut" }}
              className="fixed z-[1000] pointer-events-none px-2.5 py-1.5 rounded-lg text-[11.5px] font-medium whitespace-nowrap"
              style={{
                left: coords ? coords.left : -9999,
                top: coords ? coords.top : -9999,
                maxWidth: "min(320px, calc(100vw - 16px))",
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
