import { useState } from "react";
import { motion } from "framer-motion";
import {
  AlertTriangle,
  CheckSquare,
  Cable,
  ChevronDown,
  Download,
  HelpCircle,
  Lightbulb,
  Network,
  Zap,
} from "lucide-react";
import { useSettings } from "../store";

export function TutorialPage() {
  const { t } = useSettings();
  const [openFaq, setOpenFaq] = useState<number | null>(0);

  const steps = [
    { icon: Cable, title: t("tutStep1Title"), desc: t("tutStep1Desc") },
    { icon: CheckSquare, title: t("tutStep2Title"), desc: t("tutStep2Desc") },
    { icon: Zap, title: t("tutStep3Title"), desc: t("tutStep3Desc") },
    { icon: Download, title: t("tutStep4Title"), desc: t("tutStep4Desc") },
  ];

  const tips = [t("tutTip1"), t("tutTip2"), t("tutTip3"), t("tutTip4")];

  const faqs = [
    { q: t("faqQ1"), a: t("faqA1") },
    { q: t("faqQ2"), a: t("faqA2") },
    { q: t("faqQ3"), a: t("faqA3") },
    { q: t("faqQ4"), a: t("faqA4") },
    { q: t("faqQ5"), a: t("faqA5") },
  ];

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
          <svg className="hmx-diagram" viewBox="0 0 440 404" role="img" aria-label="data flow">
            <g>
              <path className="pipe" d="M220,62 V100" />
              <path className="pipe" d="M220,152 V188" />
              <path className="pipe" d="M220,240 V265 H74 V290" />
              <path className="pipe" d="M220,240 V290" />
              <path className="pipe" d="M220,240 V265 H366 V290" />
              <path className="pipe" d="M74,334 V343 H220 V352" />
              <path className="pipe" d="M220,334 V352" />
              <path className="pipe" d="M366,334 V343 H220 V352" />
            </g>
            <g>
              <path className="flow" d="M220,62 V100" />
              <path className="flow" style={{ animationDelay: "-0.2s" }} d="M220,152 V188" />
              <path className="flow" style={{ animationDelay: "-0.1s" }} d="M220,240 V265 H74 V290" />
              <path className="flow" style={{ animationDelay: "-0.35s" }} d="M220,240 V290" />
              <path className="flow" style={{ animationDelay: "-0.5s" }} d="M220,240 V265 H366 V290" />
              <path className="flow" style={{ animationDelay: "-0.15s" }} d="M74,334 V343 H220 V352" />
              <path className="flow" style={{ animationDelay: "-0.4s" }} d="M220,334 V352" />
              <path className="flow" style={{ animationDelay: "-0.6s" }} d="M366,334 V343 H220 V352" />
            </g>
            <rect className="junction" x="216.5" y="261.5" width="7" height="7" />
            <rect className="junction" x="216.5" y="339.5" width="7" height="7" />
            <g>
              <rect className="node" x="128" y="18" width="184" height="44" rx="8" />
              <text className="nlabel" x="220" y="45" textAnchor="middle">{t("diagTraffic")}</text>

              <rect className="node" x="116" y="100" width="208" height="52" rx="8" />
              <text className="nlabel" x="220" y="122" textAnchor="middle">{t("diagProxy")}</text>
              <text className="nsub" x="220" y="140" textAnchor="middle">:10801 · :10800</text>

              <rect className="node node-accent" x="116" y="188" width="208" height="52" rx="8" />
              <text className="nlabel" x="220" y="210" textAnchor="middle">{t("diagEngine")}</text>
              <text className="nsub" x="220" y="228" textAnchor="middle">Round-Robin · tokio</text>

              <rect className="node" x="14" y="290" width="120" height="44" rx="8" />
              <text className="nlabel" x="74" y="317" textAnchor="middle">{t("diagNic1")}</text>
              <rect className="node" x="160" y="290" width="120" height="44" rx="8" />
              <text className="nlabel" x="220" y="317" textAnchor="middle">{t("diagNic2")}</text>
              <rect className="node" x="306" y="290" width="120" height="44" rx="8" />
              <text className="nlabel" x="366" y="317" textAnchor="middle">{t("diagNicN")}</text>

              <rect className="node node-accent" x="116" y="352" width="208" height="44" rx="8" />
              <text className="nlabel" x="220" y="379" textAnchor="middle">{t("diagStacked")}</text>
            </g>
          </svg>
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

        {/* 常见问题 FAQ */}
        <div className="panel p-6">
          <div className="flex items-center gap-2 mb-4">
            <HelpCircle size={16} style={{ color: "var(--accent-soft)" }} />
            <h3 className="font-semibold text-[14px]">{t("faqTitle")}</h3>
          </div>
          <div className="flex flex-col gap-2">
            {faqs.map((f, i) => {
              const open = openFaq === i;
              return (
                <div
                  key={i}
                  className="rounded-xl overflow-hidden"
                  style={{ background: "var(--surface-2)", border: "1px solid var(--border)" }}
                >
                  <button
                    onClick={() => setOpenFaq(open ? null : i)}
                    className="w-full flex items-center gap-3 px-4 py-3 text-left"
                  >
                    <span className="text-[13px] font-medium flex-1" style={{ color: "var(--text-0)" }}>
                      {f.q}
                    </span>
                    <ChevronDown
                      size={16}
                      className="shrink-0 transition-transform"
                      style={{ color: "var(--text-2)", transform: open ? "rotate(180deg)" : "none" }}
                    />
                  </button>
                  <motion.div
                    initial={false}
                    animate={{ height: open ? "auto" : 0, opacity: open ? 1 : 0 }}
                    transition={{ duration: 0.2, ease: "easeOut" }}
                    style={{ overflow: "hidden" }}
                  >
                    <p
                      className="px-4 pb-3.5 text-[12.5px] leading-relaxed"
                      style={{ color: "var(--text-1)", borderTop: "1px solid var(--border)", paddingTop: "12px" }}
                    >
                      {f.a}
                    </p>
                  </motion.div>
                </div>
              );
            })}
          </div>
        </div>
      </div>
    </div>
  );
}
