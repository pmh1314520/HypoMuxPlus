import { motion } from "framer-motion";
import {
  AlertTriangle,
  CheckSquare,
  Cable,
  Download,
  Lightbulb,
  Network,
  Zap,
} from "lucide-react";
import { useSettings } from "../store";

export function TutorialPage() {
  const { t } = useSettings();

  const steps = [
    { icon: Cable, title: t("tutStep1Title"), desc: t("tutStep1Desc") },
    { icon: CheckSquare, title: t("tutStep2Title"), desc: t("tutStep2Desc") },
    { icon: Zap, title: t("tutStep3Title"), desc: t("tutStep3Desc") },
    { icon: Download, title: t("tutStep4Title"), desc: t("tutStep4Desc") },
  ];

  const tips = [t("tutTip1"), t("tutTip2"), t("tutTip3"), t("tutTip4")];

  return (
    <div className="h-full overflow-y-auto px-1 pb-8">
      <div className="max-w-[820px] mx-auto flex flex-col gap-5">
        <p className="text-[13.5px] leading-relaxed" style={{ color: "var(--text-1)" }}>
          {t("tutIntro")}
        </p>

        {/* 步骤时间线 */}
        <div className="panel p-6">
          <div className="relative flex flex-col gap-1">
            {steps.map((s, i) => {
              const Icon = s.icon;
              const last = i === steps.length - 1;
              return (
                <motion.div
                  key={i}
                  initial={{ opacity: 0, x: -10 }}
                  animate={{ opacity: 1, x: 0 }}
                  transition={{ delay: i * 0.07 }}
                  className="relative flex gap-4 pb-6"
                >
                  {/* 连接线 */}
                  {!last && (
                    <span
                      className="absolute left-[19px] top-[40px] bottom-0 w-px"
                      style={{ background: "var(--border-strong)" }}
                    />
                  )}
                  {/* 序号 + 图标 */}
                  <div className="relative shrink-0">
                    <div
                      className="grid place-items-center w-10 h-10 rounded-xl"
                      style={{
                        background: "var(--surface-2)",
                        border: "1px solid var(--border-strong)",
                        color: "var(--accent-soft)",
                      }}
                    >
                      <Icon size={18} />
                    </div>
                    <span
                      className="absolute -top-1.5 -right-1.5 grid place-items-center w-5 h-5 rounded-full text-[10px] font-bold text-white"
                      style={{ background: "var(--accent)" }}
                    >
                      {i + 1}
                    </span>
                  </div>
                  <div className="pt-0.5">
                    <div className="text-[10px] tracking-[0.14em] uppercase mb-0.5" style={{ color: "var(--text-2)" }}>
                      {t("tutStepLabel")} {i + 1}
                    </div>
                    <h3 className="text-[15px] font-semibold mb-1">{s.title}</h3>
                    <p className="text-[12.5px] leading-relaxed" style={{ color: "var(--text-1)" }}>
                      {s.desc}
                    </p>
                  </div>
                </motion.div>
              );
            })}
          </div>
        </div>

        {/* 工作原理 */}
        <div className="panel p-6">
          <div className="flex items-center gap-2 mb-3">
            <Network size={16} style={{ color: "var(--accent-soft)" }} />
            <h3 className="font-semibold text-[14px]">{t("howTitle")}</h3>
          </div>
          <p className="text-[12.5px] leading-relaxed mb-4" style={{ color: "var(--text-1)" }}>
            {t("howDesc")}
          </p>
          <pre
            className="mono text-[11px] leading-relaxed p-4 rounded-lg overflow-x-auto"
            style={{ background: "var(--surface-2)", border: "1px solid var(--border)", color: "var(--text-2)" }}
          >{`多线程下载流量
      │  WinINet 系统代理接管
      ▼  http/https → :10801 · socks → :10800
分流引擎 (Rust · tokio)
      │  Round-Robin 轮询
      ▼  IP_UNICAST_IF 接口强绑定 + bind
   ├─ 网卡 1 ─┐
   ├─ 网卡 2 ─┼─►  物理带宽叠加
   └─ 网卡 N ─┘`}</pre>
        </div>

        {/* 重要提示 */}
        <div className="panel p-6">
          <div className="flex items-center gap-2 mb-3">
            <Lightbulb size={16} style={{ color: "var(--warn)" }} />
            <h3 className="font-semibold text-[14px]">{t("tutTipsTitle")}</h3>
          </div>
          <ul className="flex flex-col gap-2.5">
            {tips.map((tip, i) => (
              <li key={i} className="flex gap-2.5 text-[12.5px] leading-relaxed" style={{ color: "var(--text-1)" }}>
                <AlertTriangle size={14} className="shrink-0 mt-0.5" style={{ color: "var(--text-2)" }} />
                {tip}
              </li>
            ))}
          </ul>
        </div>
      </div>
    </div>
  );
}
