// 统一的剪贴板复制工具。
// 优先使用异步 Clipboard API；在非安全上下文 / 无焦点等导致失败时，
// 回退到隐藏 textarea + execCommand("copy")，最大化复制成功率。
export async function copyText(text: string): Promise<boolean> {
  try {
    if (navigator.clipboard && window.isSecureContext !== false) {
      await navigator.clipboard.writeText(text);
      return true;
    }
  } catch {
    /* 继续走回退方案 */
  }
  return legacyCopy(text);
}

/** 回退方案：借助临时 textarea 与 execCommand 完成复制。 */
function legacyCopy(text: string): boolean {
  try {
    const ta = document.createElement("textarea");
    ta.value = text;
    // 移出视口且不可聚焦滚动，避免视觉抖动
    ta.style.position = "fixed";
    ta.style.top = "-9999px";
    ta.style.left = "-9999px";
    ta.setAttribute("readonly", "");
    document.body.appendChild(ta);
    ta.select();
    const ok = document.execCommand("copy");
    document.body.removeChild(ta);
    return ok;
  } catch {
    return false;
  }
}
