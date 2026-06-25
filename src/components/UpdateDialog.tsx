import { useEffect, useState } from "react";
import { motion } from "framer-motion";
import { Download, Loader2, Rocket, X } from "lucide-react";
import { api, type UpdateInfo } from "../lib/api";
import { useSettings } from "../store";
import { useToast } from "./Toast";

interface Props {
  info: UpdateInfo;
  onClose: () => void;
}

export function UpdateDialog({ info, onClose }: Props) {
  const { t } = useSettings();
  const toast = useToast();
  const [installing, setInstalling] = useState(false);

  // Esc 关闭（安装中禁用）+ 锁定背景滚动
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !installing) onClose();
    };
    window.addEventListener("keydown", onKey);
    const prev = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      window.removeEventListener("keydown", onKey);
      document.body.style.overflow = prev;
    };
  }, [installing, onClose]);

  const install = async () => {
    setInstalling(true);
    try {
      // 成功后后端会退出当前实例并由更新脚本替换重启
      await api.downloadAndInstall(info.url);
    } catch (e) {
      setInstalling(false);
      toast("error", t("updFailed", { err: String(e) }));
    }
  };

  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      className="fixed inset-0 z-[400] grid place-items-center p-6"
      style={{ background: "rgba(0,0,0,0.55)", backdropFilter: "blur(6px)" }}
      onClick={() => !installing && onClose()}
    >
      <motion.div
        initial={{ opacity: 0, y: 20, scale: 0.97 }}
        animate={{ opacity: 1, y: 0, scale: 1 }}
        transition={{ type: "spring", stiffness: 260, damping: 26 }}
        onClick={(e) => e.stopPropagation()}
        className="panel w-[460px] max-w-[92vw] p-6"
        style={{ boxShadow: "var(--shadow)" }}
      >
        <div className="flex items-center gap-3 mb-4">
          <span
            className="grid place-items-center w-10 h-10 rounded-xl shrink-0"
            style={{ background: "var(--surface-2)", border: "1px solid var(--border)", color: "var(--accent-soft)" }}
          >
            <Rocket size={19} />
          </span>
          <div className="min-w-0 flex-1">
            <h2 className="text-[16px] font-bold">{t("updTitle")}</h2>
            <div className="text-[12px] mt-0.5 mono" style={{ color: "var(--text-2)" }}>
              v{info.current} → <span style={{ color: "var(--accent-soft)" }}>v{info.latest}</span>
            </div>
          </div>
          {!installing && (
            <button
              onClick={onClose}
              className="grid place-items-center w-8 h-8 rounded-lg transition-colors hover:[background:var(--surface-hover)]"
              style={{ color: "var(--text-2)" }}
            >
              <X size={16} />
            </button>
          )}
        </div>

        {info.notes && (
          <div
            className="rounded-xl px-4 py-3 mb-4 max-h-[260px] overflow-y-auto text-[12.5px] leading-relaxed whitespace-pre-wrap"
            style={{ background: "var(--surface-2)", border: "1px solid var(--border)", color: "var(--text-1)" }}
          >
            {info.notes}
          </div>
        )}

        <div className="flex items-center gap-2.5">
          <button
            onClick={install}
            disabled={installing}
            className="flex-1 flex items-center justify-center gap-2 h-[44px] rounded-xl font-semibold text-[14px] text-white transition-transform hover:scale-[1.02]"
            style={{
              background: "linear-gradient(135deg, var(--accent), var(--accent-deep))",
              boxShadow: "0 8px 22px -10px var(--accent-glow)",
              opacity: installing ? 0.7 : 1,
              cursor: installing ? "not-allowed" : "pointer",
            }}
          >
            {installing ? <Loader2 size={17} className="animate-spin" /> : <Download size={17} />}
            {installing ? t("updInstalling") : t("updInstall")}
          </button>
          {!installing && (
            <button
              onClick={onClose}
              className="px-4 h-[44px] rounded-xl font-medium text-[13.5px] transition-colors"
              style={{ background: "var(--surface-2)", border: "1px solid var(--border)", color: "var(--text-1)" }}
            >
              {t("updLater")}
            </button>
          )}
        </div>
        {installing && (
          <p className="text-[11.5px] mt-3 text-center" style={{ color: "var(--text-2)" }}>
            {t("updInstallingHint")}
          </p>
        )}
      </motion.div>
    </motion.div>
  );
}
