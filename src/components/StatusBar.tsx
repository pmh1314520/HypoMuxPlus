import { Cpu, Network, ShieldAlert, ShieldCheck, Wifi } from "lucide-react";
import { useSettings } from "../store";
import { useAppVersion } from "../lib/version";
import { Tooltip } from "./Tooltip";
import { useToast } from "./Toast";

interface Props {
  running: boolean;
  admin: boolean;
  selectedCount: number;
  socksPort: number;
  httpPort: number;
  totalConn: number;
}

export function StatusBar({ running, admin, selectedCount, socksPort, httpPort, totalConn }: Props) {
  const { t } = useSettings();
  const toast = useToast();
  const version = useAppVersion();

  const copyAddr = async (addr: string) => {
    try {
      await navigator.clipboard.writeText(addr);
      toast("success", t("msgCopied"));
    } catch {
      /* ignore */
    }
  };

  return (
    <div
      className="flex items-center h-[28px] px-4 gap-5 shrink-0 text-[11px] whitespace-nowrap overflow-hidden"
      style={{ background: "var(--rail)", borderTop: "1px solid var(--border)", color: "var(--text-2)" }}
    >
      {/* 引擎状态 */}
      <span className="flex items-center gap-1.5">
        <span
          className={`w-2 h-2 rounded-full ${running ? "live-dot" : ""}`}
          style={{ background: running ? "var(--ok)" : "var(--text-2)", boxShadow: running ? "0 0 6px var(--ok)" : "none" }}
        />
        <span style={{ color: running ? "var(--ok)" : "var(--text-2)", fontWeight: 600 }}>
          {running ? t("statusRunning") : t("statusStopped")}
        </span>
      </span>

      <Sep />

      {/* 监听端点（点击复制） */}
      <Tooltip label={t("tipCopyAddr")} placement="top">
        <button
          onClick={() => copyAddr(`127.0.0.1:${socksPort}`)}
          className="flex items-center gap-1.5 transition-colors hover:[color:var(--accent-soft)]"
        >
          <Network size={12} />
          <span className="mono">SOCKS5 127.0.0.1:{socksPort}</span>
        </button>
      </Tooltip>
      <Tooltip label={t("tipCopyAddr")} placement="top">
        <button
          onClick={() => copyAddr(`127.0.0.1:${httpPort}`)}
          className="flex items-center gap-1.5 transition-colors hover:[color:var(--accent-soft)]"
        >
          <Wifi size={12} />
          <span className="mono">HTTP 127.0.0.1:{httpPort}</span>
        </button>
      </Tooltip>

      <Sep />

      {/* 选中网卡 / 连接 */}
      <span className="flex items-center gap-1.5">
        <Cpu size={12} />
        {t("selectedCount", { n: selectedCount })}
      </span>
      {running && (
        <span className="mono" style={{ color: "var(--accent-soft)" }}>
          ⇄ {totalConn} conns
        </span>
      )}

      <div className="flex-1" />

      {/* 权限徽章 */}
      <Tooltip label={admin ? t("adminOk") : t("adminWarn")} placement="top">
        <span className="flex items-center gap-1.5" style={{ color: admin ? "var(--ok)" : "var(--warn)" }}>
          {admin ? <ShieldCheck size={12} /> : <ShieldAlert size={12} />}
          {admin ? t("adminBadgeOk") : t("adminBadgeNo")}
        </span>
      </Tooltip>

      <Sep />
      <span className="mono" style={{ color: "var(--text-2)" }}>
        HypoMuxPlus v{version}
      </span>
    </div>
  );
}

function Sep() {
  return <span className="w-px h-3.5 self-center" style={{ background: "var(--border)" }} />;
}
