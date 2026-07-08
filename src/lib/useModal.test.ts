// Feature: network-capability-expansion, Task 11.7
// useModal 行为示例测试：ESC 关闭、canClose 门控、背景滚动锁、焦点进入/归还。
// 无 @testing-library/react 依赖，直接以 react-dom/client + act 挂载最小组件。
// Validates: Requirements 7.3
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { createElement } from "react";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { useModal } from "./useModal";

// React 19 的 act 需要该标志才能在测试环境中静默运行
(globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT =
  true;

let container: HTMLDivElement;
let root: Root;

/** 最小弹窗组件：把 useModal 返回的 ref 挂到可编程聚焦的面板上 */
function TestModal(props: { onClose: () => void; canClose?: boolean }) {
  const ref = useModal<HTMLDivElement>(props.onClose, props.canClose);
  return createElement("div", {
    ref,
    tabIndex: -1,
    "data-testid": "modal-panel",
  });
}

/** 在 act 中挂载 TestModal，返回面板元素 */
function mount(props: { onClose: () => void; canClose?: boolean }) {
  act(() => {
    root.render(createElement(TestModal, props));
  });
  return container.querySelector<HTMLDivElement>('[data-testid="modal-panel"]');
}

/** 派发一次 keydown 事件 */
function pressKey(key: string) {
  act(() => {
    window.dispatchEvent(new KeyboardEvent("keydown", { key }));
  });
}

beforeEach(() => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => {
    root.unmount();
  });
  container.remove();
  document.body.style.overflow = "";
  vi.restoreAllMocks();
});

describe("useModal 行为 (Task 11.7 / Req 7.3)", () => {
  it("按下 Escape 时调用 onClose", () => {
    const onClose = vi.fn();
    mount({ onClose });

    pressKey("Escape");

    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("非 Escape 键不触发 onClose", () => {
    const onClose = vi.fn();
    mount({ onClose });

    pressKey("Enter");
    pressKey("a");

    expect(onClose).not.toHaveBeenCalled();
  });

  it("canClose 为 false 时 Escape 不触发 onClose", () => {
    const onClose = vi.fn();
    mount({ onClose, canClose: false });

    pressKey("Escape");

    expect(onClose).not.toHaveBeenCalled();
  });

  it("挂载时锁定背景滚动，卸载后还原", () => {
    document.body.style.overflow = "auto";
    const onClose = vi.fn();
    mount({ onClose });

    // 打开时锁定为 hidden
    expect(document.body.style.overflow).toBe("hidden");

    act(() => {
      root.unmount();
    });

    // 关闭后还原为打开前的值
    expect(document.body.style.overflow).toBe("auto");
  });

  it("打开时把焦点移入面板，关闭时归还触发元素", () => {
    // 预先聚焦一个“触发按钮”
    const trigger = document.createElement("button");
    document.body.appendChild(trigger);
    trigger.focus();
    expect(document.activeElement).toBe(trigger);

    const onClose = vi.fn();
    const panel = mount({ onClose });

    // 打开后焦点进入面板
    expect(document.activeElement).toBe(panel);

    act(() => {
      root.unmount();
    });

    // 关闭后焦点归还触发元素
    expect(document.activeElement).toBe(trigger);
    trigger.remove();
  });

  it("卸载后 Escape 不再触发 onClose（事件监听已解绑）", () => {
    const onClose = vi.fn();
    mount({ onClose });

    act(() => {
      root.unmount();
    });

    pressKey("Escape");

    expect(onClose).not.toHaveBeenCalled();
  });
});
