import { useEffect, useRef } from "react";
import { Terminal, Trash2 } from "lucide-react";
import { useSettings } from "../store";
import { Tooltip } from "./Tooltip";

interface Props {
  logs: string[];
  clear: () => void;
}

function lineColor(line: string): string {
  if (line.includes("失败") || line.includes("异常") || line.includes("failed") || line.includes("Error"))
    return "var(--danger)";
  if (line.includes("调度") || line.includes("dispatch") || line.includes("调度分配"))
    return "var(--accent-soft)";
  if (line.includes("启动") || line.includes("started") || line.includes("HypoMux"))
    return "var(--ok)";
  return "var(--text-1)";
}

export function Console({ logs, clear }: Props) {
  const { t } = useSettings();
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (ref.current) ref.current.scrollTop = ref.current.scrollHeight;
  }, [logs]);

  return (
    <div className="glass flex flex-col overflow-hidden" style={{ boxShadow: "var(--shadow)" }}>
      <div
        className="flex items-center gap-3 px-5 py-3.5 shrink-0"
        style={{ borderBottom: "1px solid var(--border)" }}
      >
        <Terminal size={17} style={{ color: "var(--accent-soft)" }} />
        <span className="font-semibold text-[14px]">{t("consoleTitle")}</span>
        <div className="flex-1" />
        <Tooltip label={t("consoleClear")} placement="left">
          <button
            onClick={clear}
            className="grid place-items-center w-7 h-7 rounded-lg transition-colors hover:[background:var(--surface-hover)]"
            style={{ color: "var(--text-2)" }}
          >
            <Trash2 size={14} />
          </button>
        </Tooltip>
      </div>

      <div
        ref={ref}
        className="flex-1 overflow-y-auto px-4 py-3 font-mono text-[11.5px] leading-[1.7] space-y-0.5"
      >
        {logs.length === 0 ? (
          <div className="grid place-items-center h-full" style={{ color: "var(--text-2)" }}>
            {t("consoleEmpty")}
          </div>
        ) : (
          logs.map((line, i) => (
            <div key={i} style={{ color: lineColor(line) }} className="break-all">
              {line}
            </div>
          ))
        )}
      </div>
    </div>
  );
}
