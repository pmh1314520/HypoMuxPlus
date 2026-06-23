import { openUrl } from "@tauri-apps/plugin-opener";
import { motion } from "framer-motion";
import { Coffee, Database, ExternalLink, GitBranch, Heart, ScrollText, User } from "lucide-react";
import { useSettings } from "../store";
import { Logo } from "./Logo";
import wechatQr from "../assets/sponsor-wechat.png";
import alipayQr from "../assets/sponsor-alipay.jpg";

const REPO = "https://github.com/Hypostasis-Cat/HypoMux";
const TECH = ["Tauri 2", "Rust", "tokio", "React 19", "TypeScript", "TailwindCSS"];

const container = { hidden: {}, show: { transition: { staggerChildren: 0.07 } } };
const item = { hidden: { opacity: 0, y: 16 }, show: { opacity: 1, y: 0 } };

function fmtData(mb: number): string {
  if (mb >= 1048576) return (mb / 1048576).toFixed(2) + " TB";
  if (mb >= 1024) return (mb / 1024).toFixed(2) + " GB";
  return mb.toFixed(0) + " MB";
}

export function AboutPage({ lifetimeMB }: { lifetimeMB: number }) {
  const { t } = useSettings();

  return (
    <div className="h-full overflow-y-auto px-1 pb-8">
      <motion.div
        variants={container}
        initial="hidden"
        animate="show"
        className="max-w-[840px] mx-auto flex flex-col gap-5"
      >
        {/* 品牌头 */}
        <motion.div variants={item} className="panel relative overflow-hidden p-7 flex items-center gap-5">
          <div
            className="absolute -top-16 -right-16 w-56 h-56 rounded-full pointer-events-none"
            style={{ background: "radial-gradient(circle, var(--accent-glow), transparent 70%)" }}
          />
          <Logo size={66} />
          <div className="relative">
            <div className="flex items-center gap-2.5">
              <h2 className="text-[23px] font-bold tracking-tight">
                HypoMux<span style={{ color: "var(--accent-soft)" }}>Plus</span>
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
        </motion.div>

        {/* 累计加速流量 */}
        <motion.div
          variants={item}
          className="panel p-5 flex items-center gap-4"
          style={{ background: "linear-gradient(160deg, rgba(59,130,246,0.07), transparent 60%)" }}
        >
          <div
            className="grid place-items-center w-11 h-11 rounded-xl"
            style={{ background: "var(--surface-2)", border: "1px solid var(--border)", color: "var(--accent-soft)" }}
          >
            <Database size={19} />
          </div>
          <div>
            <div className="eyebrow">{t("lifetimeTotal")}</div>
            <div className="text-[24px] font-bold mono leading-none mt-1.5">{fmtData(lifetimeMB)}</div>
          </div>
        </motion.div>

        {/* 描述 + 技术栈 */}
        <motion.div variants={item} className="panel p-6">
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
        </motion.div>

        {/* 赞助支持 */}
        <motion.div
          variants={item}
          className="panel relative overflow-hidden p-6"
          style={{ background: "linear-gradient(160deg, rgba(59,130,246,0.06), transparent 60%)" }}
        >
          <div className="flex items-center gap-2 mb-2">
            <Coffee size={16} style={{ color: "var(--warn)" }} />
            <h3 className="font-semibold text-[14px]">{t("aboutSponsor")}</h3>
          </div>
          <p className="text-[12.5px] leading-relaxed mb-5 max-w-[640px]" style={{ color: "var(--text-1)" }}>
            {t("aboutSponsorDesc")}
          </p>
          <div className="grid grid-cols-2 gap-4 max-w-[440px]">
            <QrCard src={wechatQr} label={t("sponsorWechat")} color="#07c160" />
            <QrCard src={alipayQr} label={t("sponsorAlipay")} color="#1677ff" />
          </div>
        </motion.div>

        {/* 信息卡 */}
        <motion.div variants={item} className="grid grid-cols-2 gap-4">
          <InfoCard icon={<User size={15} />} label={t("aboutAuthor")} value="青云制作_彭明航" />
          <InfoCard icon={<ScrollText size={15} />} label={t("aboutLicense")} value="AGPL-3.0" />
        </motion.div>

        {/* 项目仓库 */}
        <motion.div variants={item} className="panel p-6">
          <div className="flex items-center gap-2 mb-3">
            <GitBranch size={16} style={{ color: "var(--accent-soft)" }} />
            <h3 className="font-semibold text-[14px]">{t("aboutRepo")}</h3>
          </div>
          <div className="flex flex-wrap gap-2.5">
            <RepoLink url="https://github.com/pmh1314520/HypoMuxPlus" label="GitHub" />
            <RepoLink url="https://gitee.com/peng-minghang/hypo-mux-plus" label="Gitee" />
          </div>
        </motion.div>

        {/* 原项目 */}
        <motion.div variants={item} className="panel p-6">
          <div className="flex items-center gap-2 mb-3">
            <GitBranch size={16} style={{ color: "var(--text-2)" }} />
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
        </motion.div>

        {/* 致谢 */}
        <motion.div variants={item} className="panel p-6">
          <div className="flex items-center gap-2 mb-3">
            <Heart size={16} style={{ color: "var(--danger)" }} />
            <h3 className="font-semibold text-[14px]">{t("aboutThanks")}</h3>
          </div>
          <p className="text-[12.5px] leading-relaxed" style={{ color: "var(--text-1)" }}>
            {t("aboutThanksDesc")}
          </p>
        </motion.div>
      </motion.div>
    </div>
  );
}

function QrCard({ src, label, color }: { src: string; label: string; color: string }) {
  return (
    <div
      className="flex flex-col items-center gap-2.5 p-3 rounded-xl"
      style={{ background: "var(--surface-2)", border: "1px solid var(--border)" }}
    >
      <div className="rounded-lg overflow-hidden bg-white p-1.5" style={{ width: "100%" }}>
        <img src={src} alt={label} className="w-full block rounded-md" style={{ aspectRatio: "1 / 1", objectFit: "cover" }} />
      </div>
      <span className="flex items-center gap-1.5 text-[12.5px] font-semibold">
        <span className="w-2 h-2 rounded-full" style={{ background: color }} />
        {label}
      </span>
    </div>
  );
}

function RepoLink({ url, label }: { url: string; label: string }) {
  return (
    <button
      onClick={() => openUrl(url)}
      className="flex items-center gap-2 px-3.5 py-2 rounded-lg text-[12.5px] font-medium transition-colors"
      style={{ background: "var(--surface-2)", border: "1px solid var(--border)", color: "var(--text-0)" }}
    >
      <GitBranch size={14} style={{ color: "var(--accent-soft)" }} />
      {label}
      <ExternalLink size={12} style={{ color: "var(--text-2)" }} />
    </button>
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
