import { useState } from "react";
import { motion } from "framer-motion";
import { Bookmark, Check, CheckSquare, Layers, Plus, RefreshCw, Square, X } from "lucide-react";
import { useSettings } from "../store";
import { useToast } from "./Toast";
import { Tooltip } from "./Tooltip";
import type { AdapterInfo, NicTelemetry } from "../lib/api";

interface NicProfile {
  name: string;
  indices: number[];
}

const PROFILES_KEY = "hmx-nic-profiles";

function loadProfiles(): NicProfile[] {
  try {
    const raw = localStorage.getItem(PROFILES_KEY);
    if (raw) {
      const arr = JSON.parse(raw);
      if (Array.isArray(arr)) return arr.filter((p) => p && typeof p.name === "string" && Array.isArray(p.indices));
    }
  } catch {
    /* ignore */
  }
  return [];
}

interface Props {
  adapters: AdapterInfo[];
  selected: Set<number>;
  toggle: (index: number) => void;
  selectAll: () => void;
  deselectAll: () => void;
  applySelection: (indices: number[]) => void;
  refresh: () => void;
  perNic: Record<string, NicTelemetry>;
  running: boolean;
  loading: boolean;
}

export function AdapterTable({
  adapters,
  selected,
  toggle,
  selectAll,
  deselectAll,
  applySelection,
  refresh,
  perNic,
  running,
  loading,
}: Props) {
  const { t } = useSettings();
  const toast = useToast();
  const [profiles, setProfiles] = useState<NicProfile[]>(loadProfiles);
  const [naming, setNaming] = useState(false);
  const [draftName, setDraftName] = useState("");

  const persist = (next: NicProfile[]) => {
    setProfiles(next);
    localStorage.setItem(PROFILES_KEY, JSON.stringify(next));
  };

  const saveProfile = () => {
    if (selected.size === 0) {
      toast("warning", t("profileNoSel"));
      return;
    }
    const name = draftName.trim();
    if (!name) return;
    const indices = [...selected];
    const next = [...profiles.filter((p) => p.name !== name), { name, indices }];
    persist(next);
    setDraftName("");
    setNaming(false);
    toast("success", t("profileSaved"));
  };

  const applyProfile = (p: NicProfile) => {
    applySelection(p.indices);
    toast("info", t("profileApplied", { name: p.name }));
  };

  const deleteProfile = (name: string) => persist(profiles.filter((p) => p.name !== name));

  return (
    <div className="glass flex flex-col overflow-hidden" style={{ boxShadow: "var(--shadow)" }}>
      {/* 卡片头 */}
      <div
        className="flex items-center gap-3 px-5 py-3.5 shrink-0"
        style={{ borderBottom: "1px solid var(--border)" }}
      >
        <Layers size={17} style={{ color: "var(--accent-soft)" }} />
        <div className="flex flex-col leading-tight">
          <span className="font-semibold text-[14px]">{t("adaptersTitle")}</span>
          <span className="text-[11px]" style={{ color: "var(--text-2)" }}>
            {t("adaptersHint")}
          </span>
        </div>
        <div className="flex-1" />
        <span className="text-[11px] px-2 py-1 rounded-md" style={{ background: "var(--surface-2)", color: "var(--text-1)" }}>
          {t("selectedCount", { n: selected.size })}
        </span>
        <HeaderBtn onClick={selectAll} disabled={running}>
          <CheckSquare size={14} /> {t("selectAll")}
        </HeaderBtn>
        <HeaderBtn onClick={deselectAll} disabled={running}>
          <Square size={14} /> {t("deselectAll")}
        </HeaderBtn>
        <HeaderBtn onClick={refresh} disabled={running}>
          <RefreshCw size={14} className={loading ? "animate-spin" : ""} />
        </HeaderBtn>
      </div>

      {/* 网卡方案预设栏 */}
      <div
        className="flex items-center gap-2 px-5 py-2 shrink-0 flex-wrap"
        style={{ borderBottom: "1px solid var(--border)", background: "var(--surface)" }}
      >
        <span className="flex items-center gap-1.5 text-[11px] font-medium" style={{ color: "var(--text-2)" }}>
          <Bookmark size={13} style={{ color: "var(--accent-soft)" }} />
          {t("profiles")}
        </span>
        {profiles.length === 0 && !naming && (
          <span className="text-[11px]" style={{ color: "var(--text-2)" }}>
            {t("profileEmpty")}
          </span>
        )}
        {profiles.map((p) => (
          <span
            key={p.name}
            className="flex items-center gap-1 pl-2.5 pr-1 py-1 rounded-lg text-[11.5px] transition-colors"
            style={{ background: "var(--surface-strong)", border: "1px solid var(--border)" }}
          >
            <Tooltip label={t("profileApplyTip")} placement="top">
              <button
                onClick={() => !running && applyProfile(p)}
                disabled={running}
                className="font-medium transition-colors hover:[color:var(--accent-soft)]"
                style={{ color: "var(--text-1)", cursor: running ? "not-allowed" : "pointer", opacity: running ? 0.5 : 1 }}
              >
                {p.name}
                <span className="ml-1 mono" style={{ color: "var(--text-2)" }}>
                  ·{p.indices.length}
                </span>
              </button>
            </Tooltip>
            <Tooltip label={t("profileDeleteTip")} placement="top">
              <button
                onClick={() => deleteProfile(p.name)}
                className="grid place-items-center w-4 h-4 rounded transition-colors hover:[background:var(--surface-2)]"
                style={{ color: "var(--text-2)" }}
              >
                <X size={11} />
              </button>
            </Tooltip>
          </span>
        ))}
        <div className="flex-1" />
        {naming ? (
          <div className="flex items-center gap-1.5">
            <input
              autoFocus
              value={draftName}
              onChange={(e) => setDraftName(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") saveProfile();
                if (e.key === "Escape") {
                  setNaming(false);
                  setDraftName("");
                }
              }}
              maxLength={24}
              placeholder={t("profileNamePlaceholder")}
              className="px-2.5 py-1 rounded-lg text-[11.5px] outline-none"
              style={{ background: "var(--surface-2)", border: "1px solid var(--accent)", color: "var(--text-0)", width: 150 }}
            />
            <button
              onClick={saveProfile}
              className="px-2.5 py-1 rounded-lg text-[11.5px] font-medium text-white transition-transform hover:scale-105"
              style={{ background: "var(--accent)" }}
            >
              {t("btnConfirm")}
            </button>
            <button
              onClick={() => {
                setNaming(false);
                setDraftName("");
              }}
              className="px-2.5 py-1 rounded-lg text-[11.5px] font-medium transition-colors"
              style={{ background: "var(--surface-strong)", border: "1px solid var(--border)", color: "var(--text-1)" }}
            >
              {t("btnCancel")}
            </button>
          </div>
        ) : (
          <HeaderBtn onClick={() => setNaming(true)} disabled={running}>
            <Plus size={13} /> {t("profileSave")}
          </HeaderBtn>
        )}
      </div>

      {/* 列头 */}
      <div
        className="grid items-center px-5 py-2.5 text-[11px] font-medium shrink-0"
        style={{ gridTemplateColumns: "44px 1fr 140px 110px 70px", color: "var(--text-2)" }}
      >
        <span>{t("colSelect")}</span>
        <span>{t("colAlias")}</span>
        <span>{t("colIpv4")}</span>
        <span className="text-right">{t("colSpeed")}</span>
        <span className="text-right">{t("colConn")}</span>
      </div>

      {/* 行 */}
      <div className="flex-1 overflow-y-auto px-2 pb-2">
        {adapters.length === 0 ? (
          <div className="grid place-items-center h-full text-[13px]" style={{ color: "var(--text-2)" }}>
            {loading ? t("statusLoading") : t("statusNoAdapters")}
          </div>
        ) : (
          adapters.map((a) => {
            const checked = selected.has(a.index);
            const tele = perNic[a.alias];
            const hasIp = a.ipv4 && a.ipv4 !== "0.0.0.0";
            return (
              <motion.div
                key={a.index}
                layout
                onClick={() => hasIp && !running && toggle(a.index)}
                className="grid items-center px-3 py-2.5 my-0.5 rounded-xl cursor-pointer transition-colors"
                style={{
                  gridTemplateColumns: "44px 1fr 140px 110px 70px",
                  background: checked ? "var(--surface-hover)" : "transparent",
                  border: checked ? "1px solid var(--border-strong)" : "1px solid transparent",
                  boxShadow: checked ? "inset 3px 0 0 var(--accent)" : "none",
                  cursor: hasIp && !running ? "pointer" : "not-allowed",
                  opacity: hasIp ? 1 : 0.5,
                }}
                onMouseEnter={(e) => {
                  if (!checked) e.currentTarget.style.background = "var(--surface)";
                }}
                onMouseLeave={(e) => {
                  if (!checked) e.currentTarget.style.background = "transparent";
                }}
              >
                {/* 复选框 */}
                <div
                  className="grid place-items-center w-[22px] h-[22px] rounded-md transition-colors"
                  style={{
                    background: checked ? "var(--accent)" : "transparent",
                    border: `1.5px solid ${checked ? "var(--accent)" : "var(--border-strong)"}`,
                  }}
                >
                  {checked && <Check size={14} color="#fff" strokeWidth={3} />}
                </div>

                {/* 别名 + 描述 */}
                <div className="flex flex-col leading-tight min-w-0 pr-2">
                  <span className="text-[13px] font-medium truncate">{a.alias}</span>
                  <span className="text-[10px] truncate" style={{ color: "var(--text-2)" }}>
                    {a.description}
                  </span>
                </div>

                {/* IPv4 */}
                <span className="text-[12px] tabular-nums truncate" style={{ color: hasIp ? "var(--text-1)" : "var(--text-2)" }}>
                  {hasIp ? a.ipv4 : t("noValidIp")}
                </span>

                {/* 速度 */}
                <span
                  className="text-[13px] font-semibold tabular-nums text-right"
                  style={{ color: tele && tele.downMbps > 0 ? "var(--accent-soft)" : "var(--text-2)" }}
                >
                  {tele ? tele.downMbps.toFixed(2) : "0.00"}
                </span>

                {/* 连接数 */}
                <span className="text-[13px] tabular-nums text-right" style={{ color: "var(--text-1)" }}>
                  {tele ? tele.connections : "—"}
                </span>
              </motion.div>
            );
          })
        )}
      </div>
    </div>
  );
}

function HeaderBtn({
  children,
  onClick,
  disabled,
}: {
  children: React.ReactNode;
  onClick: () => void;
  disabled?: boolean;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg text-[12px] font-medium transition-colors"
      style={{
        background: "var(--surface-strong)",
        border: "1px solid var(--border)",
        color: "var(--text-1)",
        opacity: disabled ? 0.4 : 1,
        cursor: disabled ? "not-allowed" : "pointer",
      }}
    >
      {children}
    </button>
  );
}
