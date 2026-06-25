import { useEffect, useState, type ReactNode } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { AnimatePresence, motion } from "framer-motion";
import { Coffee, Compass, Copy, Database, ExternalLink, FolderGit2, GitFork, Globe, Heart, Maximize2, MessageCircle, MonitorSmartphone, RefreshCw, ScrollText, ShieldAlert, ShieldCheck, User, X } from "lucide-react";
import { useSettings } from "../store";
import { useAppVersion } from "../lib/version";
import { useToast } from "./Toast";
import { Logo } from "./Logo";
import { GitHubIcon, GiteeIcon } from "./BrandIcons";
import wechatQr from "../assets/sponsor-wechat.png";
import alipayQr from "../assets/sponsor-alipay.jpg";

const ORIGINAL = "https://github.com/Hypostasis-Cat/HypoMux";
const GITHUB = "https://github.com/pmh1314520/HypoMuxPlus";
const GITEE = "https://gitee.com/peng-minghang/hypo-mux-plus";
const WEBSITE = "https://hmp.pmhs.top";
const WECHAT = "QyPmh20061026";
const QQ = "2124691573";
const TECH = ["Tauri 2", "Rust", "tokio", "React 19", "TypeScript", "TailwindCSS"];

const container = { hidden: {}, show: { transition: { staggerChildren: 0.07 } } };
const item = { hidden: { opacity: 0, y: 16 }, show: { opacity: 1, y: 0 } };

function fmtData(mb: number): string {
  if (mb >= 1048576) return (mb / 1048576).toFixed(2) + " TB";
  if (mb >= 1024) return (mb / 1024).toFixed(2) + " GB";
  return mb.toFixed(0) + " MB";
}

export function AboutPage({ lifetimeMB, admin, onReplayGuide, onCheckUpdate }: { lifetimeMB: number; admin: boolean; onReplayGuide: () => void; onCheckUpdate: () => void }) {
  const { t } = useSettings();
  const toast = useToast();
  const version = useAppVersion();
  const [zoom, setZoom] = useState<{ src: string; label: string } | null>(null);

  const copy = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      toast("success", t("msgCopied"));
    } catch {
      /* ignore */
    }
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setZoom(null);
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  return (
    <div className="h-full overflow-y-auto px-1 pb-8">
      <motion.div variants={container} initial="hidden" animate="show" className="max-w-[840px] mx-auto flex flex-col gap-5">
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
                v{version}
              </span>
            </div>
            <p className="text-[13px] mt-1.5" style={{ color: "var(--text-1)" }}>
              {t("aboutTagline")}
            </p>
            <div className="flex items-center gap-2 mt-3 flex-wrap">
              <button
                onClick={onReplayGuide}
                className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[12px] font-medium transition-colors"
                style={{ background: "var(--surface-2)", border: "1px solid var(--border)", color: "var(--text-1)" }}
              >
                <Compass size={13} style={{ color: "var(--accent-soft)" }} />
                {t("aboutReplayGuide")}
              </button>
              <button
                onClick={() => openUrl(WEBSITE)}
                className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[12px] font-medium transition-colors"
                style={{ background: "var(--surface-2)", border: "1px solid var(--border)", color: "var(--text-1)" }}
              >
                <Globe size={13} style={{ color: "var(--accent-soft)" }} />
                {t("aboutVisitWebsite")}
              </button>
              <button
                onClick={onCheckUpdate}
                className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[12px] font-medium text-white transition-transform hover:scale-105"
                style={{ background: "linear-gradient(135deg, var(--accent-deep), var(--accent))" }}
              >
                <RefreshCw size={13} />
                {t("aboutCheckUpdate")}
              </button>
            </div>
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
            <QrCard src={wechatQr} label={t("sponsorWechat")} color="#07c160" onZoom={setZoom} />
            <QrCard src={alipayQr} label={t("sponsorAlipay")} color="#1677ff" onZoom={setZoom} />
          </div>
        </motion.div>

        {/* 信息卡 */}
        <motion.div variants={item} className="grid grid-cols-2 gap-4">
          <InfoCard icon={<User size={15} />} label={t("aboutAuthor")} value="青云制作_彭明航" />
          <InfoCard icon={<ScrollText size={15} />} label={t("aboutLicense")} value="AGPL-3.0" />
          <InfoCard
            icon={admin ? <ShieldCheck size={15} /> : <ShieldAlert size={15} />}
            label={t("aboutPermission")}
            value={admin ? t("adminBadgeOk") : t("adminBadgeNo")}
            valueColor={admin ? "var(--ok)" : "var(--warn)"}
          />
          <InfoCard icon={<MonitorSmartphone size={15} />} label={t("aboutPlatform")} value="Windows 10 / 11" />
        </motion.div>

        {/* 项目仓库 */}
        <motion.div variants={item} className="panel p-6">
          <div className="flex items-center gap-2 mb-3">
            <FolderGit2 size={16} style={{ color: "var(--accent-soft)" }} />
            <h3 className="font-semibold text-[14px]">{t("aboutRepo")}</h3>
          </div>
          <div className="flex flex-wrap gap-2.5">
            <RepoLink url={GITHUB} label="GitHub" icon={<GitHubIcon size={15} />} />
            <RepoLink url={GITEE} label="Gitee" icon={<GiteeIcon size={15} style={{ color: "#c71d23" }} />} />
          </div>
        </motion.div>

        {/* 联系开发者 */}
        <motion.div variants={item} className="panel p-6">
          <div className="flex items-center gap-2 mb-3">
            <MessageCircle size={16} style={{ color: "var(--accent-soft)" }} />
            <h3 className="font-semibold text-[14px]">{t("aboutContact")}</h3>
          </div>
          <div className="flex flex-wrap gap-2.5">
            <ContactChip label={t("aboutWechat")} value={WECHAT} color="#07c160" onCopy={() => copy(WECHAT)} />
            <ContactChip label={t("aboutQQ")} value={QQ} color="#1296db" onCopy={() => copy(QQ)} />
          </div>
        </motion.div>

        {/* 原项目 */}
        <motion.div variants={item} className="panel p-6">
          <div className="flex items-center gap-2 mb-3">
            <GitFork size={16} style={{ color: "var(--text-2)" }} />
            <h3 className="font-semibold text-[14px]">{t("aboutOriginal")}</h3>
          </div>
          <button
            onClick={() => openUrl(ORIGINAL)}
            className="flex items-center gap-2 text-[13px] mono hover:underline"
            style={{ color: "var(--accent-soft)" }}
          >
            {ORIGINAL}
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

      {/* 二维码全屏放大 */}
      <AnimatePresence>
        {zoom && (
          <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            onClick={() => setZoom(null)}
            className="fixed inset-0 z-[1000] grid place-items-center p-6"
            style={{
              background: "color-mix(in srgb, var(--bg-0) 86%, transparent)",
              backdropFilter: "blur(12px)",
            }}
          >
            <button
              onClick={() => setZoom(null)}
              className="absolute top-5 right-5 grid place-items-center w-11 h-11 rounded-xl"
              style={{ background: "var(--surface-2)", border: "1px solid var(--border-strong)", color: "var(--text-0)" }}
            >
              <X size={18} />
            </button>
            <motion.div
              initial={{ scale: 0.92, opacity: 0 }}
              animate={{ scale: 1, opacity: 1 }}
              exit={{ scale: 0.92, opacity: 0 }}
              transition={{ type: "spring", stiffness: 280, damping: 26 }}
              onClick={(e) => e.stopPropagation()}
              className="flex flex-col items-center gap-3"
            >
              <img
                src={zoom.src}
                alt={zoom.label}
                className="rounded-2xl bg-white p-3"
                style={{ maxWidth: "min(440px, 92vw)", maxHeight: "82vh", width: "auto", height: "auto" }}
              />
              <span className="text-[14px] font-semibold" style={{ color: "var(--text-1)" }}>
                {zoom.label}
              </span>
            </motion.div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

function QrCard({
  src,
  label,
  color,
  onZoom,
}: {
  src: string;
  label: string;
  color: string;
  onZoom: (z: { src: string; label: string }) => void;
}) {
  return (
    <button
      onClick={() => onZoom({ src, label })}
      className="group flex flex-col items-center gap-2.5 p-3 rounded-xl transition-all hover:scale-[1.03]"
      style={{ background: "var(--surface-2)", border: "1px solid var(--border)", cursor: "zoom-in" }}
    >
      <div
        className="relative rounded-lg overflow-hidden bg-white p-2 w-full grid place-items-center"
        style={{ aspectRatio: "3 / 4" }}
      >
        <img
          src={src}
          alt={label}
          className="rounded-md transition-transform duration-300 group-hover:scale-105"
          style={{ maxWidth: "100%", maxHeight: "100%", width: "auto", height: "auto", objectFit: "contain" }}
        />
        <span
          className="absolute inset-0 grid place-items-center opacity-0 group-hover:opacity-100 transition-opacity"
          style={{ background: "rgba(0,0,0,0.18)" }}
        >
          <span className="grid place-items-center w-8 h-8 rounded-full" style={{ background: "rgba(255,255,255,0.9)", color: "#111" }}>
            <Maximize2 size={15} />
          </span>
        </span>
      </div>
      <span className="flex items-center gap-1.5 text-[12.5px] font-semibold">
        <span className="w-2 h-2 rounded-full" style={{ background: color }} />
        {label}
      </span>
    </button>
  );
}

function ContactChip({
  label,
  value,
  color,
  onCopy,
}: {
  label: string;
  value: string;
  color: string;
  onCopy: () => void;
}) {
  return (
    <button
      onClick={onCopy}
      className="group flex items-center gap-2.5 px-3.5 py-2.5 rounded-xl transition-colors hover:[background:var(--surface-hover)]"
      style={{ background: "var(--surface-2)", border: "1px solid var(--border)" }}
    >
      <span className="w-2.5 h-2.5 rounded-full shrink-0" style={{ background: color }} />
      <span className="flex flex-col items-start leading-tight">
        <span className="text-[10px] eyebrow">{label}</span>
        <span className="mono text-[13px] font-semibold" style={{ color: "var(--text-0)" }}>
          {value}
        </span>
      </span>
      <Copy size={13} className="ml-1 opacity-50 group-hover:opacity-100 transition-opacity" style={{ color: "var(--text-2)" }} />
    </button>
  );
}

function RepoLink({ url, label, icon }: { url: string; label: string; icon: ReactNode }) {
  return (
    <button
      onClick={() => openUrl(url)}
      className="flex items-center gap-2 px-3.5 py-2 rounded-lg text-[12.5px] font-medium transition-colors"
      style={{ background: "var(--surface-2)", border: "1px solid var(--border)", color: "var(--text-0)" }}
    >
      {icon}
      {label}
      <ExternalLink size={12} style={{ color: "var(--text-2)" }} />
    </button>
  );
}

function InfoCard({ icon, label, value, valueColor }: { icon: ReactNode; label: string; value: string; valueColor?: string }) {
  return (
    <div className="panel p-5">
      <div className="flex items-center gap-2 eyebrow mb-2">
        <span style={{ color: "var(--text-2)" }}>{icon}</span>
        {label}
      </div>
      <div className="text-[15px] font-semibold" style={{ color: valueColor }}>{value}</div>
    </div>
  );
}
