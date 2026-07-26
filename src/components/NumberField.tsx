import { useEffect, useRef, useState } from "react";
import { Minus, Plus } from "lucide-react";
import { useSettings } from "../store";

interface Props {
  value: number;
  onChange: (v: number) => void;
  min?: number;
  max?: number;
  disabled?: boolean;
  /** 无障碍：描述该数值字段的用途（供屏幕阅读器朗读），如「HTTP 端口」 */
  ariaLabel?: string;
}

/**
 * 自研数字步进输入，规避浏览器原生 number 控件的样式与微调按钮。
 *
 * 关键：编辑期用本地字符串草稿承接输入，仅在失焦 / 回车 / 步进按钮时才 clamp 提交。
 * 否则每次按键即 clamp 会打断输入——当目标值的前缀数字小于 `min` 时（如 min=5 想输入
 * 「12」，键入首字符「1」被 clamp 成「5」，继续输入得到「52」而非「12」），无法正常录入。
 */
export function NumberField({ value, onChange, min = 1, max = 65535, disabled, ariaLabel }: Props) {
  const { t } = useSettings();
  const clamp = (v: number) => Math.min(max, Math.max(min, Number.isFinite(v) ? v : min));

  const [focused, setFocused] = useState(false);
  const [draft, setDraft] = useState("");
  // 记录上一次外部 value，避免受控 value 在编辑期被本地草稿覆盖导致光标 / 数值抖动。
  const lastValueRef = useRef(value);
  lastValueRef.current = value;

  // 非编辑态：展示受控 value；编辑态：展示本地草稿（允许暂时为空或不足 min 的中间态）。
  const display = focused ? draft : String(value);

  // 提交草稿：空串按 min 处理，否则解析并 clamp，再上抛给父级。
  const commit = () => {
    const digits = draft.replace(/\D/g, "");
    const next = digits === "" ? min : clamp(parseInt(digits, 10));
    onChange(next);
    setDraft(String(next));
  };

  // 组件卸载 / disabled 变化时，若仍在编辑态则不残留草稿（下次聚焦重新取值）。
  useEffect(() => {
    if (disabled && focused) {
      setFocused(false);
    }
  }, [disabled, focused]);

  const step = (delta: number) => {
    // 步进以“当前有效值”为基准（编辑态优先用草稿解析值，回退受控 value）。
    const base = focused
      ? draft.replace(/\D/g, "") === ""
        ? value
        : parseInt(draft.replace(/\D/g, ""), 10)
      : value;
    const next = clamp((Number.isFinite(base) ? base : value) + delta);
    onChange(next);
    if (focused) setDraft(String(next));
  };

  return (
    <div
      className="flex items-center rounded-lg overflow-hidden"
      style={{ border: "1px solid var(--border)", background: "var(--surface-2)", opacity: disabled ? 0.5 : 1 }}
    >
      <StepBtn onClick={() => step(-1)} disabled={disabled} label={t("stepDecrease")}>
        <Minus size={13} />
      </StepBtn>
      <input
        type="text"
        inputMode="numeric"
        value={display}
        disabled={disabled}
        aria-label={ariaLabel}
        onFocus={() => {
          setDraft(String(lastValueRef.current));
          setFocused(true);
        }}
        onChange={(e) => setDraft(e.target.value.replace(/\D/g, ""))}
        onBlur={() => {
          commit();
          setFocused(false);
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            commit();
            (e.target as HTMLInputElement).blur();
          } else if (e.key === "ArrowUp") {
            e.preventDefault();
            step(1);
          } else if (e.key === "ArrowDown") {
            e.preventDefault();
            step(-1);
          }
        }}
        className="w-[58px] text-center mono text-[13px] bg-transparent outline-none border-none"
        style={{ color: "var(--text-0)" }}
      />
      <StepBtn onClick={() => step(1)} disabled={disabled} label={t("stepIncrease")}>
        <Plus size={13} />
      </StepBtn>
    </div>
  );
}

function StepBtn({
  children,
  onClick,
  disabled,
  label,
}: {
  children: React.ReactNode;
  onClick: () => void;
  disabled?: boolean;
  label: string;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      aria-label={label}
      className="grid place-items-center w-7 h-8 transition-colors hover:[background:var(--surface-hover)]"
      style={{ color: "var(--text-1)", cursor: disabled ? "not-allowed" : "pointer" }}
    >
      {children}
    </button>
  );
}
