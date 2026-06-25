import { Component, type ReactNode } from "react";
import { useSettings } from "../store";

interface Labels {
  title: string;
  desc: string;
  reload: string;
  detail: string;
}

interface InnerProps {
  children: ReactNode;
  labels: Labels;
}

interface State {
  hasError: boolean;
  message: string;
}

/**
 * 全局错误边界：捕获渲染期异常，避免单个组件出错导致整屏白屏。
 * 加速进程运行在 Rust 后端，界面重载不会中断加速。
 */
class ErrorBoundaryInner extends Component<InnerProps, State> {
  state: State = { hasError: false, message: "" };

  static getDerivedStateFromError(err: unknown): State {
    return { hasError: true, message: err instanceof Error ? err.message : String(err) };
  }

  componentDidCatch(err: unknown) {
    // 记录到控制台，便于排查；不上报任何外部服务
    console.error("[HypoMuxPlus] UI error captured by ErrorBoundary:", err);
  }

  render() {
    if (!this.state.hasError) return this.props.children;
    const { labels } = this.props;
    return (
      <div
        className="h-screen w-screen grid place-items-center p-8"
        style={{ background: "var(--bg-0, #0a0e18)", color: "var(--text-0, #eef1f6)" }}
      >
        <div
          className="w-[460px] max-w-[92vw] rounded-2xl p-7 text-center"
          style={{
            background: "var(--surface-2, rgba(255,255,255,0.045))",
            border: "1px solid var(--border, rgba(255,255,255,0.1))",
            boxShadow: "0 30px 80px -40px rgba(0,0,0,0.7)",
          }}
        >
          <div
            className="grid place-items-center w-14 h-14 rounded-2xl mx-auto mb-4"
            style={{
              background: "color-mix(in srgb, var(--danger, #ef4444) 16%, transparent)",
              color: "var(--danger, #ef4444)",
            }}
          >
            <svg viewBox="0 0 24 24" width="26" height="26" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <path d="M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
              <line x1="12" y1="9" x2="12" y2="13" />
              <line x1="12" y1="17" x2="12.01" y2="17" />
            </svg>
          </div>
          <h1 className="text-[18px] font-bold mb-2">{labels.title}</h1>
          <p className="text-[13px] leading-relaxed mb-4" style={{ color: "var(--text-1, #9aa5b4)" }}>
            {labels.desc}
          </p>
          {this.state.message && (
            <details className="text-left mb-5">
              <summary
                className="text-[11.5px] cursor-pointer select-none mb-1.5"
                style={{ color: "var(--text-2, #5a6573)" }}
              >
                {labels.detail}
              </summary>
              <pre
                className="text-[11px] leading-relaxed whitespace-pre-wrap break-words rounded-lg p-3 max-h-[140px] overflow-y-auto"
                style={{
                  background: "var(--surface, rgba(255,255,255,0.022))",
                  border: "1px solid var(--border, rgba(255,255,255,0.1))",
                  color: "var(--text-1, #9aa5b4)",
                  fontFamily: "'JetBrains Mono', Consolas, monospace",
                }}
              >
                {this.state.message}
              </pre>
            </details>
          )}
          <button
            onClick={() => window.location.reload()}
            className="w-full h-[44px] rounded-xl font-semibold text-[14px] text-white transition-transform hover:scale-[1.02]"
            style={{
              background: "linear-gradient(135deg, var(--accent, #3b82f6), var(--accent-deep, #2563eb))",
              boxShadow: "0 8px 22px -10px var(--accent-glow, rgba(59,130,246,0.25))",
              cursor: "pointer",
            }}
          >
            {labels.reload}
          </button>
        </div>
      </div>
    );
  }
}

export function ErrorBoundary({ children }: { children: ReactNode }) {
  const { t } = useSettings();
  return (
    <ErrorBoundaryInner
      labels={{ title: t("errTitle"), desc: t("errDesc"), reload: t("errReload"), detail: t("errDetail") }}
    >
      {children}
    </ErrorBoundaryInner>
  );
}
