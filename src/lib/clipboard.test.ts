// Feature: network-capability-expansion, Task 11.7
// clipboard 回退行为示例测试：jsdom 下 mock navigator.clipboard 不可用，
// 断言回退到 document.execCommand 文本兜底方案。
// Validates: Requirements 7.3
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { copyText } from "./clipboard";

// 保存被改写的全局，测试后完整还原，避免污染其它用例
let originalClipboardDescriptor: PropertyDescriptor | undefined;
let originalSecureDescriptor: PropertyDescriptor | undefined;
let originalExecCommand: unknown;

/** 覆写 navigator.clipboard（jsdom 默认不存在，需以可配置属性注入） */
function setClipboard(value: unknown) {
  Object.defineProperty(navigator, "clipboard", {
    value,
    configurable: true,
    writable: true,
  });
}

/** 覆写 window.isSecureContext（jsdom 默认为 false，会跳过异步 API 路径） */
function setSecureContext(value: boolean) {
  Object.defineProperty(window, "isSecureContext", {
    value,
    configurable: true,
    writable: true,
  });
}

beforeEach(() => {
  originalClipboardDescriptor = Object.getOwnPropertyDescriptor(
    navigator,
    "clipboard",
  );
  originalSecureDescriptor = Object.getOwnPropertyDescriptor(
    window,
    "isSecureContext",
  );
  originalExecCommand = (document as unknown as { execCommand?: unknown })
    .execCommand;
});

afterEach(() => {
  // 还原 navigator.clipboard
  if (originalClipboardDescriptor) {
    Object.defineProperty(navigator, "clipboard", originalClipboardDescriptor);
  } else {
    // 原本不存在则删除注入的属性
    delete (navigator as unknown as { clipboard?: unknown }).clipboard;
  }
  // 还原 window.isSecureContext
  if (originalSecureDescriptor) {
    Object.defineProperty(window, "isSecureContext", originalSecureDescriptor);
  } else {
    delete (window as unknown as { isSecureContext?: unknown }).isSecureContext;
  }
  // 还原 document.execCommand
  (document as unknown as { execCommand?: unknown }).execCommand =
    originalExecCommand;
  vi.restoreAllMocks();
});

describe("copyText 剪贴板复制 (Task 11.7 / Req 7.3)", () => {
  it("异步 Clipboard API 可用时优先使用 writeText 并返回 true", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    setClipboard({ writeText });
    setSecureContext(true);
    // 若走到回退方案会调用 execCommand，这里断言其未被调用以证明走了首选路径
    const execCommand = vi.fn().mockReturnValue(true);
    (document as unknown as { execCommand: unknown }).execCommand = execCommand;

    const ok = await copyText("hello");

    expect(ok).toBe(true);
    expect(writeText).toHaveBeenCalledTimes(1);
    expect(writeText).toHaveBeenCalledWith("hello");
    expect(execCommand).not.toHaveBeenCalled();
  });

  it("Clipboard API 不可用时回退到 execCommand('copy') 并返回 true", async () => {
    // 模拟 navigator.clipboard 不存在（非安全上下文 / 老环境）
    setClipboard(undefined);
    setSecureContext(false);
    const execCommand = vi.fn().mockReturnValue(true);
    (document as unknown as { execCommand: unknown }).execCommand = execCommand;

    const ok = await copyText("fallback-text");

    expect(ok).toBe(true);
    expect(execCommand).toHaveBeenCalledTimes(1);
    expect(execCommand).toHaveBeenCalledWith("copy");
  });

  it("writeText 抛错时同样回退到 execCommand 并返回 true", async () => {
    const writeText = vi.fn().mockRejectedValue(new Error("denied"));
    setClipboard({ writeText });
    setSecureContext(true);
    const execCommand = vi.fn().mockReturnValue(true);
    (document as unknown as { execCommand: unknown }).execCommand = execCommand;

    const ok = await copyText("retry-text");

    expect(ok).toBe(true);
    expect(writeText).toHaveBeenCalledTimes(1);
    expect(execCommand).toHaveBeenCalledWith("copy");
  });

  it("Clipboard API 与 execCommand 均失败时返回 false", async () => {
    setClipboard(undefined);
    setSecureContext(false);
    const execCommand = vi.fn().mockReturnValue(false);
    (document as unknown as { execCommand: unknown }).execCommand = execCommand;

    const ok = await copyText("nope");

    expect(ok).toBe(false);
    expect(execCommand).toHaveBeenCalledWith("copy");
  });

  it("execCommand 抛出异常时被捕获并返回 false", async () => {
    setClipboard(undefined);
    setSecureContext(false);
    const execCommand = vi.fn(() => {
      throw new Error("execCommand not supported");
    });
    (document as unknown as { execCommand: unknown }).execCommand = execCommand;

    const ok = await copyText("boom");

    expect(ok).toBe(false);
  });
});
