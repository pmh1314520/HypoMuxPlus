import { useEffect, useState } from "react";
import { motion } from "framer-motion";
import { ArrowDownUp, Bookmark, Cable, Check, CheckSquare, Clipboard, Copy, Layers, Pencil, Plus, RefreshCw, Server, Smartphone, Square, Wifi, X } from "lucide-react";
import { useSettings } from "../store";
import { useToast } from "./Toast";
import { Tooltip } from "./Tooltip";
import type { AdapterInfo, NicTelemetry } from "../lib/api";

/** 依据网卡别名/描述推断链路类型图标（仅作直观标识，不影响调度） */
function LinkIcon({ alias, description }: { alias: string; description: string }) {
  const s = `${alias} ${description}`.toLowerCase();
  let Icon = Cable;
  let color = "var(--text-2)";
  if (/wi-?fi|wlan|wireless|无线/.test(s)) {
    Icon = Wifi;
    color = "var(--accent-soft)";
  } else if (/usb|cellular|rndis|tether|蜂窝|移动宽带|手机/.test(s)) {
    Icon = Smartphone;
    color = "var(--series-3)";
  } else if (/virtual|vpn|tap|tun|hyper-v|vethernet|loopback|虚拟/.test(s)) {
    Icon = Server;
    color = "var(--series-4)";
  }
  return <Icon size={13} style={{ color }} className="shrink-0" />;
}

interface NicProfile {
  name: string;
  indices: number[];
}

const PROFILES_KEY = "hmx-nic-profiles";
const NOTES_KEY = "hmx-nic-notes";

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

function loadNotes(): Record<string, string> {
  try {
    const raw = localStorage.getItem(NOTES_KEY);
    if (raw) {
      const obj = JSON.parse(raw);
      if (obj && typeof obj === "object") return obj;
    }
  } catch {
    /* ignore */
  }
  return {};
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
  nicHistory: Record<string, number[]>;
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
  nicHistory,
  running,
  loading,
}: Props) {
  const { t } = useSettings();
  const toast = useToast();
  const [profiles, setProfiles] = useState<NicProfile[]>(loadProfiles);
  const [naming, setNaming] = useState(false);
  const [draftName, setDraftName] = useState("");
  const [notes, setNotes] = useState<Record<string, string>>(loadNotes);
  const [editingNote, setEditingNote] = useState<number | null>(null);
  const [noteDraft, setNoteDraft] = useState("");
  const [sort, setSort] = useState<"default" | "speed" | "conns">("default");
  const [ctxMenu, setCtxMenu] = useState<{ x: number; y: number; a: AdapterInfo } | null>(null);

  useEffect(() => {
    if (!ctxMenu) return;
    const close = () => setCtxMenu(null);
    window.addEventListener("click", close);
    window.addEventListener("scroll", close, true);
    window.addEventListener("resize", close);
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && close();
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("click", close);
      window.removeEventListener("scroll", close, true);
      window.removeEventListener("resize", close);
      window.removeEventListener("keydown", onKey);
    };
  }, [ctxMenu]);

  const copyText = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      toast("success", t("msgCopied"));
    } catch {
      /* ignore */
    }
  };

  const sortLabel = sort === "speed" ? t("sortSpeed") : sort === "conns" ? t("sortConns") : t("sortDefault");
  const cycleSort = () => setSort((s) => (s === "default" ? "speed" : s === "speed" ? "conns" : "default"));
  const sortedAdapters =
    sort === "default"
      ? adapters
      : [...adapters].sort((a, b) => {
          const ta = perNic[a.alias];
          const tb = perNic[b.alias];
          const va = sort === "speed" ? ta?.downMbps ?? 0 : ta?.connections ?? 0;
          const vb = sort === "speed" ? tb?.downMbps ?? 0 : tb?.connections ?? 0;
          return vb - va;
        });

  const startEditNote = (index: number) => {
    setEditingNote(index);
    setNoteDraft(notes[String(index)] ?? "");
  };
  const saveNote = (index: number) => {
    const v = noteDraft.trim();
    setNotes((prev) => {
      const next = { ...prev };
      if (v) next[String(index)] = v;
      else delete next[String(index)];
      localStorage.setItem(NOTES_KEY, JSON.stringify(next));
      return next;
    });
    setEditingNote(null);
    setNoteDraft("");
  };

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
        className="flex items-center gap-2 px-5 py-3.5 shrink-0"
        style={{ borderBottom: "1px solid var(--border)" }}
      >
        <Layers size={17} style={{ color: "var(--accent-soft)" }} className="shrink-0" />
        <div className="flex flex-col leading-tight min-w-0">
          <span className="font-semibold text-[14px] truncate">{t("adaptersTitle")}</span>
          <span className="text-[11px] truncate" style={{ color: "var(--text-2)" }}>
            {t("adaptersHint")}
          </span>
        </div>
        <div className="flex-1" />
        <span
          className="text-[11px] px-2 py-1 rounded-md whitespace-nowrap shrink-0"
          style={{ background: "var(--surface-2)", color: "var(--text-1)" }}
        >
          {t("selectedCount", { n: selected.size })}
        </span>
        <Tooltip label={t("sortTip")} placement="top">
          <button
            onClick={cycleSort}
            className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg text-[12px] font-medium transition-colors whitespace-nowrap shrink-0"
            style={{
              background: sort === "default" ? "var(--surface-strong)" : "color-mix(in srgb, var(--accent) 16%, transparent)",
              border: `1px solid ${sort === "default" ? "var(--border)" : "color-mix(in srgb, var(--accent) 35%, transparent)"}`,
              color: sort === "default" ? "var(--text-1)" : "var(--accent-soft)",
            }}
          >
            <ArrowDownUp size={13} /> {sortLabel}
          </button>
        </Tooltip>
        <Tooltip label={t("selectAll")} placement="top">
          <HeaderBtn onClick={selectAll} disabled={running}>
            <CheckSquare size={15} />
          </HeaderBtn>
        </Tooltip>
        <Tooltip label={t("deselectAll")} placement="top">
          <HeaderBtn onClick={deselectAll} disabled={running}>
            <Square size={15} />
          </HeaderBtn>
        </Tooltip>
        <Tooltip label={t("tipRefresh")} placement="top">
          <HeaderBtn onClick={refresh} disabled={running}>
            <RefreshCw size={15} className={loading ? "animate-spin" : ""} />
          </HeaderBtn>
        </Tooltip>
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
        style={{ gridTemplateColumns: "44px 1fr 130px 134px 60px", color: "var(--text-2)" }}
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
          loading ? (
            <div className="grid place-items-center h-full text-[13px]" style={{ color: "var(--text-2)" }}>
              {t("statusLoading")}
            </div>
          ) : (
            <div className="grid place-items-center h-full px-6">
              <div className="flex flex-col items-center text-center max-w-[360px]">
                <span
                  className="grid place-items-center w-12 h-12 rounded-2xl mb-3"
                  style={{ background: "var(--surface-2)", border: "1px solid var(--border)", color: "var(--text-2)" }}
                >
                  <Cable size={22} />
                </span>
                <div className="text-[14px] font-semibold mb-1.5">{t("statusNoAdapters")}</div>
                <p className="text-[12px] leading-relaxed mb-4" style={{ color: "var(--text-2)" }}>
                  {t("noNicHint")}
                </p>
                <button
                  onClick={refresh}
                  className="flex items-center gap-1.5 px-3.5 py-2 rounded-lg text-[12.5px] font-medium text-white transition-transform hover:scale-105"
                  style={{ background: "linear-gradient(135deg, var(--accent-deep), var(--accent))" }}
                >
                  <RefreshCw size={14} className={loading ? "animate-spin" : ""} /> {t("btnRescan")}
                </button>
              </div>
            </div>
          )
        ) : (
          sortedAdapters.map((a) => {
            const checked = selected.has(a.index);
            const tele = perNic[a.alias];
            const hasIp = a.ipv4 && a.ipv4 !== "0.0.0.0";
            return (
              <motion.div
                key={a.index}
                layout
                onClick={() => hasIp && !running && toggle(a.index)}
                onContextMenu={(e) => {
                  e.preventDefault();
                  e.stopPropagation();
                  setCtxMenu({ x: e.clientX, y: e.clientY, a });
                }}
                className="grid items-center px-3 py-2.5 my-0.5 rounded-xl cursor-pointer transition-colors"
                style={{
                  gridTemplateColumns: "44px 1fr 130px 134px 60px",
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

                {/* 别名 + 备注 + 描述 */}
                <div className="flex flex-col leading-tight min-w-0 pr-2">
                  {editingNote === a.index ? (
                    <input
                      autoFocus
                      value={noteDraft}
                      onClick={(e) => e.stopPropagation()}
                      onChange={(e) => setNoteDraft(e.target.value)}
                      onKeyDown={(e) => {
                        e.stopPropagation();
                        if (e.key === "Enter") saveNote(a.index);
                        if (e.key === "Escape") {
                          setEditingNote(null);
                          setNoteDraft("");
                        }
                      }}
                      onBlur={() => saveNote(a.index)}
                      maxLength={20}
                      placeholder={t("nicNotePlaceholder")}
                      className="px-2 py-0.5 rounded-md text-[12.5px] outline-none"
                      style={{ background: "var(--surface-2)", border: "1px solid var(--accent)", color: "var(--text-0)", width: "100%" }}
                    />
                  ) : (
                    <div className="flex items-center gap-1.5 min-w-0">
                      <LinkIcon alias={a.alias} description={a.description} />
                      <span className="text-[13px] font-medium truncate">
                        {notes[String(a.index)] || a.alias}
                      </span>
                      {notes[String(a.index)] && (
                        <span
                          className="text-[9px] px-1 py-px rounded mono shrink-0"
                          style={{ background: "var(--surface-2)", color: "var(--text-2)" }}
                        >
                          {a.alias}
                        </span>
                      )}
                      <Tooltip label={t("nicNoteTip")} placement="top">
                        <button
                          onClick={(e) => {
                            e.stopPropagation();
                            startEditNote(a.index);
                          }}
                          className="grid place-items-center w-4 h-4 rounded shrink-0 transition-colors hover:[color:var(--accent-soft)]"
                          style={{ color: "var(--text-2)" }}
                        >
                          <Pencil size={10} />
                        </button>
                      </Tooltip>
                    </div>
                  )}
                  <span className="text-[10px] truncate" style={{ color: "var(--text-2)" }}>
                    {a.description}
                  </span>
                </div>

                {/* IPv4 */}
                <span className="text-[12px] tabular-nums truncate" style={{ color: hasIp ? "var(--text-1)" : "var(--text-2)" }}>
                  {hasIp ? a.ipv4 : t("noValidIp")}
                </span>

                {/* 速度 + 迷你曲线 */}
                <div className="flex items-center justify-end gap-2">
                  <Sparkline data={nicHistory[a.alias]} active={!!(tele && tele.downMbps > 0)} />
                  <span
                    className="text-[13px] font-semibold tabular-nums text-right"
                    style={{ color: tele && tele.downMbps > 0 ? "var(--accent-soft)" : "var(--text-2)", minWidth: 42 }}
                  >
                    {tele ? tele.downMbps.toFixed(2) : "0.00"}
                  </span>
                </div>

                {/* 连接数 */}
                <span className="text-[13px] tabular-nums text-right" style={{ color: "var(--text-1)" }}>
                  {tele ? tele.connections : "—"}
                </span>
              </motion.div>
            );
          })
        )}
      </div>

      {/* 网卡行右键菜单（自研，非浏览器原生） */}
      {ctxMenu && (
        <div
          className="fixed z-[300] py-1 rounded-xl text-[12.5px]"
          style={{
            left: Math.min(ctxMenu.x, window.innerWidth - 180),
            top: Math.min(ctxMenu.y, window.innerHeight - 140),
            minWidth: 168,
            background: "var(--surface-strong)",
            border: "1px solid var(--border-strong)",
            boxShadow: "var(--shadow)",
            backdropFilter: "blur(10px)",
          }}
          onClick={(e) => e.stopPropagation()}
        >
          <CtxItem
            icon={<Copy size={13} />}
            label={t("ctxCopyIp")}
            disabled={!ctxMenu.a.ipv4 || ctxMenu.a.ipv4 === "0.0.0.0"}
            onClick={() => {
              copyText(ctxMenu.a.ipv4);
              setCtxMenu(null);
            }}
          />
          <CtxItem
            icon={<Clipboard size={13} />}
            label={t("ctxCopyName")}
            onClick={() => {
              copyText(ctxMenu.a.alias);
              setCtxMenu(null);
            }}
          />
          <CtxItem
            icon={<Pencil size={13} />}
            label={t("ctxEditNote")}
            onClick={() => {
              startEditNote(ctxMenu.a.index);
              setCtxMenu(null);
            }}
          />
        </div>
      )}
    </div>
  );
}

function CtxItem({
  icon,
  label,
  onClick,
  disabled,
}: {
  icon: React.ReactNode;
  label: string;
  onClick: () => void;
  disabled?: boolean;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      className="w-full flex items-center gap-2.5 px-3.5 py-2 text-left transition-colors hover:[background:var(--surface-hover)]"
      style={{ color: disabled ? "var(--text-2)" : "var(--text-1)", opacity: disabled ? 0.5 : 1, cursor: disabled ? "not-allowed" : "pointer" }}
    >
      <span style={{ color: "var(--text-2)" }}>{icon}</span>
      {label}
    </button>
  );
}

function Sparkline({ data, active }: { data?: number[]; active: boolean }) {
  const w = 56;
  const h = 18;
  const series = data && data.length > 1 ? data : null;
  if (!series) {
    return (
      <svg width={w} height={h} style={{ opacity: 0.35 }}>
        <line x1="0" y1={h - 1} x2={w} y2={h - 1} stroke="var(--border-strong)" strokeWidth="1" />
      </svg>
    );
  }
  const max = Math.max(...series, 0.001);
  const n = series.length;
  const pts = series.map((v, i) => {
    const x = (i / (n - 1)) * w;
    const y = h - 1 - (v / max) * (h - 2);
    return `${x.toFixed(1)},${y.toFixed(1)}`;
  });
  const line = pts.join(" ");
  const area = `0,${h} ${line} ${w},${h}`;
  const color = active ? "var(--accent-soft)" : "var(--text-2)";
  return (
    <svg width={w} height={h} className="shrink-0">
      <polygon points={area} fill={color} opacity={0.12} />
      <polyline points={line} fill="none" stroke={color} strokeWidth="1.5" strokeLinejoin="round" strokeLinecap="round" />
    </svg>
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
      className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg text-[12px] font-medium transition-colors whitespace-nowrap shrink-0"
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
