import { openUrl } from "@tauri-apps/plugin-opener";
import { ExternalLink, GitBranch, Heart, ScrollText, User } from "lucide-react";
import { useSettings } from "../store";
import { Logo } from "./Logo";

const REPO = "https://github.com/Hypostasis-Cat/HypoMux";
const TECH = ["Tauri 2", "Rust", "tokio", "React 19", "TypeScript", "TailwindCSS"];

export function AboutPage() {
  const { t } = useSettings();

  return (
    <div className="h-full overflow-y-auto px-1 pb-8">
      <div className="max-w-[820px] mx-auto flex flex-col gap-5">
        {/* 品牌头 */}
        <div className="panel p-7 flex items-center gap-5">
          <Logo size={64} />
          <div>
            <div className="flex items-center gap-2.5">
              <h2 className="text-[22px] font-bold tracking-tight">
                HypoMux <span style={{ color: "var(--accent-soft)" }}>Plus</span>
              </h2>
              <span
                className="mono text-[11px] px-2 py-0.5 rounded-md"
                style={{ background: "var(--surface-2)", border: "1px solid var(--border)", color: "var(--text-1)" }}
              >
                v1.0.0
              </span>
            </div>
            <p className="text-[13px] mt-1.5" style={{ color: "var(--text-1)" }}>
              {t("aboutTagline")}
            </p>
          </div>
        </div>

        {/* 描述 + 技术栈 */}
        <div className="panel p-6">
          <p className="text-[13px] leading-relaxed mb-4" style={{ color: "var(--text-1)" }}>
            {t("aboutDesc")}
          </p>
          <div className="eyebrow mb-2.5">{t("aboutTech")}</div>
          <div className="flex flex-wrap gap-2">
            {TECH.map((tech) => (
              <span
                key={tech}
                className="mono text-[11.5px] px-2.5 py-1 rounded-md"
                style={{ background: "var(--surface-2)", border: "1px solid var(--border)", color: "var(--text-1)" }}
              >
                {tech}
              </span>
            ))}
          </div>
        </div>

        {/* 信息卡 */}
        <div className="grid grid-cols-2 gap-4">
          <InfoCard icon={<User size={15} />} label={t("aboutAuthor")} value="青云制作_彭明航" />
          <InfoCard icon={<ScrollText size={15} />} label={t("aboutLicense")} value="AGPL-3.0" />
        </div>

        {/* 原项目 */}
        <div className="panel p-6">
          <div className="flex items-center gap-2 mb-3">
            <GitBranch size={16} style={{ color: "var(--accent-soft)" }} />
            <h3 className="font-semibold text-[14px]">{t("aboutOriginal")}</h3>
          </div>
          <button
            onClick={() => openUrl(REPO)}
            className="flex items-center gap-2 text-[13px] mono hover:underline"
            style={{ color: "var(--accent-soft)" }}
          >
            {REPO}
            <ExternalLink size={13} />
          </button>
        </div>

        {/* 致谢 */}
        <div className="panel p-6">
          <div className="flex items-center gap-2 mb-3">
            <Heart size={16} style={{ color: "var(--danger)" }} />
            <h3 className="font-semibold text-[14px]">{t("aboutThanks")}</h3>
          </div>
          <p className="text-[12.5px] leading-relaxed" style={{ color: "var(--text-1)" }}>
            {t("aboutThanksDesc")}
          </p>
        </div>
      </div>
    </div>
  );
}

function InfoCard({ icon, label, value }: { icon: React.ReactNode; label: string; value: string }) {
  return (
    <div className="panel p-5">
      <div className="flex items-center gap-2 eyebrow mb-2">
        <span style={{ color: "var(--text-2)" }}>{icon}</span>
        {label}
      </div>
      <div className="text-[15px] font-semibold">{value}</div>
    </div>
  );
}
