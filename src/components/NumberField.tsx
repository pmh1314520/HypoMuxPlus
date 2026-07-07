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

/** 自研数字步进输入，规避浏览器原生 number 控件的样式与微调按钮。 */
export function NumberField({ value, onChange, min = 1, max = 65534, disabled, ariaLabel }: Props) {
  const { t } = useSettings();
  const clamp = (v: number) => Math.min(max, Math.max(min, v || min));

  return (
    <div
      className="flex items-center rounded-lg overflow-hidden"
      style={{ border: "1px solid var(--border)", background: "var(--surface-2)", opacity: disabled ? 0.5 : 1 }}
    >
      <StepBtn onClick={() => onChange(clamp(value - 1))} disabled={disabled} label={t("stepDecrease")}>
        <Minus size={13} />
      </StepBtn>
      <input
        type="text"
        inputMode="numeric"
        value={value}
        disabled={disabled}
        aria-label={ariaLabel}
        onChange={(e) => onChange(clamp(parseInt(e.target.value.replace(/\D/g, "") || "0", 10)))}
        className="w-[58px] text-center mono text-[13px] bg-transparent outline-none border-none"
        style={{ color: "var(--text-0)" }}
      />
      <StepBtn onClick={() => onChange(clamp(value + 1))} disabled={disabled} label={t("stepIncrease")}>
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
