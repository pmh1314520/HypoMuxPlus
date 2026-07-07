import { useEffect, useRef } from "react";

/**
 * 弹窗通用行为 Hook（统一各模态框的键盘与焦点管理，避免重复实现）：
 * - Esc 关闭（可通过 canClose 动态禁用，如下载/测速进行中不允许关闭）
 * - 打开时锁定背景滚动，关闭时还原
 * - 打开时把焦点移入弹窗面板，关闭时归还给触发元素（键盘 / 读屏可达）
 *
 * 返回值：挂到弹窗面板元素上的 ref（该元素需带 tabIndex={-1} 以便可编程聚焦）。
 */
export function useModal<T extends HTMLElement = HTMLDivElement>(
  onClose: () => void,
  canClose = true,
) {
  const ref = useRef<T>(null);
  // 用 ref 读取最新的可关闭状态，避免因其变化而重挂副作用、抢焦点
  const canCloseRef = useRef(canClose);
  canCloseRef.current = canClose;

  // Esc 关闭 + 背景滚动锁（onClose 变化时安全重挂，无副作用）
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && canCloseRef.current) onClose();
    };
    window.addEventListener("keydown", onKey);
    const prevOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      window.removeEventListener("keydown", onKey);
      document.body.style.overflow = prevOverflow;
    };
  }, [onClose]);

  // 焦点管理：仅在挂载/卸载时各执行一次，避免渲染中反复抢焦点
  useEffect(() => {
    const prevFocus = document.activeElement as HTMLElement | null;
    ref.current?.focus();
    return () => {
      prevFocus?.focus?.();
    };
  }, []);

  return ref;
}
