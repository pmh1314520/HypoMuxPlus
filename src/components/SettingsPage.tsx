import { useEffect, useRef, useState, type ReactNode } from "react";
import { disable as autoDisable, enable as autoEnable, isEnabled as autoIsEnabled } from "@tauri-apps/plugin-autostart";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import {
  Bell,
  Contrast,
  Droplet,
  Gamepad2,
  Gauge,
  KeyRound,
  Languages,
  MinusSquare,
  MonitorCog,
  MonitorDown,
  Network,
  Palette,
  PictureInPicture2,
  Plug,
  Plus,
  Power,
  Rocket,
  Save,
  ServerCog,
  Shuffle,
  Trash2,
  Wand2,
  Zap,
} from "lucide-react";
import { ACCENTS, useSettings, type AccentKey, type SchedStrategy, type Theme } from "../store";
import { type Lang } from "../i18n";
import { api } from "../lib/api";
import type { AdapterInfo } from "../lib/api";
import { useToast } from "./Toast";
import { NumberField } from "./NumberField";
import { Switch } from "./Switch";
import { Tooltip } from "./Tooltip";

interface Props {
  running: boolean;
  adapters: AdapterInfo[];
  routeRules: { pattern: string; action: string }[];
  setRouteRules: (rules: { pattern: string; action: string }[]) => void;
  onStopBoost: () => void;
}

export function SettingsPage({ running, adapters, routeRules, setRouteRules, onStopBoost }: Props) {
  const { t, lang, theme, autoTheme, highContrast, accent, socksPort, httpPort, closeToTray, autostart, launchMinimized, autoBoost, autoBoostOnApp, strategy, globalHotkey, notifications, hotkeyCombo, hotkeyStop, downLimit, bypassList, hudEnabled, hudOpacity, hudLocked, hudUnit, hudShowDown, hudShowUp, hudShowConns, hudShowNics, set } =
    useSettings();
  const toast = useToast();
  const [admin, setAdmin] = useState(true);

  useEffect(() => {
    api.checkAdmin().then(setAdmin).catch(() => setAdmin(true));
    // 同步真实的开机自启状态到 UI
    autoIsEnabled()
      .then((v) => set("autostart", v))
      .catch(() => {});
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const toggleAutostart = async (v: boolean) => {
    try {
      if (v) await autoEnable();
      else await autoDisable();
      set("autostart", v);
    } catch (e) {
      toast("error", t("msgAutostartFailed", { err: String(e) }));
    }
  };

  const handleAppConfig = async (app: "steam" | "idm", enable: boolean) => {
    try {
      if (app === "steam") await api.configureSteam(enable, socksPort);
      else await api.configureIdm(enable, socksPort);
      const key =
        app === "steam"
          ? enable
            ? "msgSteamApplied"
            : "msgSteamRestored"
          : enable
          ? "msgIdmApplied"
          : "msgIdmRestored";
      toast("success", t(key));
    } catch (e) {
      toast("error", t("msgConfigFailed", { err: String(e) }));
    }
  };

  // 导出全部配置（设置 + 网卡方案 + 已选网卡）为 JSON 文件
  const exportConfig = async () => {
    try {
      const data = {
        app: "HypoMuxPlus",
        version: 1,
        exportedAt: new Date().toISOString(),
        settings: JSON.parse(localStorage.getItem("hmx-plus-settings") || "{}"),
        profiles: JSON.parse(localStorage.getItem("hmx-nic-profiles") || "[]"),
        selected: JSON.parse(localStorage.getItem("hmx-plus-selected") || "[]"),
      };
      const path = await saveDialog({
        defaultPath: "hypomuxplus-config.json",
        filters: [{ name: "JSON", extensions: ["json"] }],
      });
      if (!path) return;
      await api.writeTextFile(path, JSON.stringify(data, null, 2));
      toast("success", t("msgExported"));
    } catch (e) {
      toast("error", String(e));
    }
  };

  // 从 JSON 文件导入配置，写回 localStorage 后重新加载界面以全量生效
  const importConfig = async () => {
    try {
      const path = await openDialog({
        multiple: false,
        directory: false,
        filters: [{ name: "JSON", extensions: ["json"] }],
      });
      if (!path || typeof path !== "string") return;
      const text = await api.readTextFile(path);
      const data = JSON.parse(text);
      if (data?.app !== "HypoMuxPlus" || typeof data.settings !== "object" || !data.settings) {
        throw new Error("invalid");
      }
      localStorage.setItem("hmx-plus-settings", JSON.stringify(data.settings));
      if (Array.isArray(data.profiles)) localStorage.setItem("hmx-nic-profiles", JSON.stringify(data.profiles));
      if (Array.isArray(data.selected)) localStorage.setItem("hmx-plus-selected", JSON.stringify(data.selected));
      toast("success", t("msgImported"));
      setTimeout(() => window.location.reload(), 900);
    } catch {
      toast("error", t("msgImportFailed"));
    }
  };

  // 自动选择两个互不相同的可用端口并填入
  const autoPickPorts = async () => {
    try {
      const http = await api.suggestFreePort(httpPort);
      let socks = await api.suggestFreePort(socksPort === http ? http + 1 : socksPort);
      if (socks === http) socks = await api.suggestFreePort(http + 1);
      set("httpPort", http);
      set("socksPort", socks);
      toast("success", t("msgPortsPicked", { http, socks }));
    } catch (e) {
      toast("error", String(e));
    }
  };

  const sectionNav = [
    { id: "sec-general", label: t("settingsGeneral") },
    { id: "sec-auto", label: t("settingsAutomation") },
    { id: "sec-sched", label: t("schedTitle") },
    { id: "sec-hud", label: t("settingsHud") },
    { id: "sec-traffic", label: t("settingsTraffic") },
    { id: "sec-backup", label: t("settingsBackup") },
    { id: "sec-appcompat", label: t("appcompatTitle") },
  ];
  const jumpTo = (id: string) => document.getElementById(id)?.scrollIntoView({ behavior: "smooth", block: "start" });

  const scrollRef = useRef<HTMLDivElement>(null);
  const [activeSec, setActiveSec] = useState("sec-general");
  const onScroll = () => {
    const c = scrollRef.current;
    if (!c) return;
    const ct = c.getBoundingClientRect().top;
    let cur = sectionNav[0].id;
    for (const s of sectionNav) {
      const el = document.getElementById(s.id);
      if (el && el.getBoundingClientRect().top - ct <= 80) cur = s.id;
    }
    setActiveSec(cur);
  };

  return (
    <div ref={scrollRef} onScroll={onScroll} className="h-full overflow-y-auto px-1 pb-6">
      <div className="max-w-[860px] mx-auto flex flex-col gap-5">
        {/* 分组快速跳转 */}
        <div
          className="sticky top-0 z-20 -mx-1 px-1 py-2 flex items-center gap-2 flex-wrap"
          style={{ background: "color-mix(in srgb, var(--bg-0) 82%, transparent)", backdropFilter: "blur(8px)" }}
        >
          {sectionNav.map((s) => {
            const active = activeSec === s.id;
            return (
              <button
                key={s.id}
                onClick={() => {
                  setActiveSec(s.id);
                  jumpTo(s.id);
                }}
                className="px-2.5 py-1 rounded-lg text-[11.5px] font-medium transition-colors whitespace-nowrap"
                style={{
                  background: active ? "var(--accent)" : "var(--surface-2)",
                  border: `1px solid ${active ? "var(--accent)" : "var(--border)"}`,
                  color: active ? "#fff" : "var(--text-1)",
                }}
              >
                {s.label}
              </button>
            );
          })}
        </div>

        {!admin && (
          <div
            className="panel px-4 py-3 text-[12.5px] leading-relaxed"
            style={{ borderLeft: "3px solid var(--warn)", color: "var(--text-1)" }}
          >
            {t("adminWarn")}
          </div>
        )}

        {/* 通用 */}
        <Section id="sec-general" icon={<ServerCog size={16} />} title={t("settingsGeneral")}>
          <Row icon={<Languages size={15} />} label={t("settingLanguage")}>
            <Segmented<Lang>
              value={lang}
              options={[
                { value: "zh", label: "中文" },
                { value: "en", label: "English" },
              ]}
              onChange={(v) => set("lang", v)}
            />
          </Row>
          <Row icon={<Palette size={15} />} label={t("settingTheme")}>
            <Segmented<Theme>
              value={theme}
              options={[
                { value: "dark", label: t("themeDark") },
                { value: "light", label: t("themeLight") },
              ]}
              onChange={(v) => {
                set("autoTheme", false);
                set("theme", v);
              }}
            />
          </Row>
          <Row icon={<MonitorCog size={15} />} label={t("settingAutoTheme")} hint={t("settingAutoThemeHint")}>
            <Switch checked={autoTheme} onChange={(v) => set("autoTheme", v)} />
          </Row>
          <Row icon={<Contrast size={15} />} label={t("settingHighContrast")} hint={t("settingHighContrastHint")}>
            <Switch checked={highContrast} onChange={(v) => set("highContrast", v)} />
          </Row>
          <Row icon={<Droplet size={15} />} label={t("settingAccent")}>
            <div className="flex items-center gap-2">
              {(Object.keys(ACCENTS) as AccentKey[]).map((k) => {
                const active = accent === k;
                return (
                  <button
                    key={k}
                    onClick={() => set("accent", k)}
                    aria-label={k}
                    className="rounded-full transition-transform hover:scale-110"
                    style={{
                      width: 22,
                      height: 22,
                      background: ACCENTS[k].accent,
                      border: active ? "2px solid var(--text-0)" : "2px solid transparent",
                      boxShadow: active ? `0 0 0 2px var(--surface-1), 0 0 8px ${ACCENTS[k].glow}` : "none",
                    }}
                  />
                );
              })}
            </div>
          </Row>
          <Row icon={<Plug size={15} />} label={t("settingPorts")}>
            <div className="flex items-center gap-3">
              <div className="flex items-center gap-2">
                <span className="text-[11px]" style={{ color: "var(--text-2)" }}>
                  {t("portHttp")}
                </span>
                <NumberField value={httpPort} disabled={running} onChange={(v) => set("httpPort", v)} />
              </div>
              <div className="flex items-center gap-2">
                <span className="text-[11px]" style={{ color: "var(--text-2)" }}>
                  {t("portSocks")}
                </span>
                <NumberField value={socksPort} disabled={running} onChange={(v) => set("socksPort", v)} />
              </div>
              <Tooltip label={t("btnAutoPort")} placement="top">
                <button
                  onClick={autoPickPorts}
                  disabled={running}
                  className="grid place-items-center w-8 h-8 rounded-lg transition-colors"
                  style={{
                    background: "var(--surface-2)",
                    border: "1px solid var(--border)",
                    color: "var(--accent-soft)",
                    opacity: running ? 0.4 : 1,
                    cursor: running ? "not-allowed" : "pointer",
                  }}
                >
                  <Wand2 size={15} />
                </button>
              </Tooltip>
            </div>
          </Row>
          <Row icon={<MonitorDown size={15} />} label={t("settingCloseBehavior")}>
            <Segmented<boolean>
              value={closeToTray}
              options={[
                { value: true, label: t("closeToTray") },
                { value: false, label: t("closeToExit") },
              ]}
              onChange={(v) => set("closeToTray", v)}
            />
          </Row>
        </Section>

        {/* 自动化（Plus 专属） */}
        <Section id="sec-auto" icon={<Rocket size={16} />} title={t("settingsAutomation")}>
          <Row icon={<Power size={15} />} label={t("settingAutostart")} hint={t("settingAutostartHint")}>
            <Switch checked={autostart} onChange={toggleAutostart} />
          </Row>
          <Row icon={<MinusSquare size={15} />} label={t("settingLaunchMin")}>
            <Switch checked={launchMinimized} onChange={(v) => set("launchMinimized", v)} />
          </Row>
          <Row icon={<Zap size={15} />} label={t("settingAutoBoost")} hint={t("settingAutoBoostHint")}>
            <Switch checked={autoBoost} onChange={(v) => set("autoBoost", v)} />
          </Row>
          <Row icon={<Gamepad2 size={15} />} label={t("settingAutoBoostApp")} hint={t("settingAutoBoostAppHint")}>
            <Switch checked={autoBoostOnApp} onChange={(v) => set("autoBoostOnApp", v)} />
          </Row>
          <Row
            icon={<KeyRound size={15} />}
            label={t("settingHotkey")}
            hint={t("settingHotkeyHint")}
          >
            <Switch checked={globalHotkey} onChange={(v) => set("globalHotkey", v)} />
          </Row>
          {globalHotkey && (
            <>
              <Row label={t("settingHotkeyStart")}>
                <HotkeyCapture
                  value={hotkeyCombo}
                  onChange={(v) => set("hotkeyCombo", v)}
                  recordingLabel={t("hotkeyRecording")}
                />
              </Row>
              <Row label={t("settingHotkeyStop")}>
                <HotkeyCapture
                  value={hotkeyStop}
                  onChange={(v) => set("hotkeyStop", v)}
                  recordingLabel={t("hotkeyRecording")}
                />
              </Row>
            </>
          )}
          <Row icon={<Bell size={15} />} label={t("settingNotify")} hint={t("settingNotifyHint")}>
            <Switch checked={notifications} onChange={(v) => set("notifications", v)} />
          </Row>
        </Section>

        {/* 调度引擎（Plus 专属） */}
        <Section id="sec-sched" icon={<Shuffle size={16} />} title={t("schedTitle")} hint={t("schedHint")}>
          <div className="pt-1 flex items-center justify-between gap-4 flex-wrap">
            <Segmented<SchedStrategy>
              value={strategy}
              options={[
                { value: "weighted", label: t("schedWeighted") },
                { value: "least", label: t("schedLeast") },
                { value: "rr", label: t("schedRR") },
              ]}
              onChange={(v) => set("strategy", v)}
            />
          </div>
          <p className="text-[12px] leading-relaxed mt-3" style={{ color: "var(--text-2)" }}>
            {strategy === "rr"
              ? t("schedRRDesc")
              : strategy === "least"
              ? t("schedLeastDesc")
              : t("schedWeightedDesc")}
          </p>
        </Section>

        {/* 情景模式（Plus 专属） */}
        <Section id="sec-scene" icon={<Wand2 size={16} />} title={t("sceneTitle")} hint={t("sceneHint")}>
          <div className="pt-1 flex flex-wrap gap-2">
            {[
              { key: "sceneFull", apply: () => { set("strategy", "weighted"); set("downLimit", 0); } },
              { key: "sceneBalanced", apply: () => { set("strategy", "least"); set("downLimit", 0); } },
              { key: "sceneSaver", apply: () => { set("strategy", "weighted"); set("downLimit", 5); } },
              { key: "sceneGame", apply: () => { onStopBoost(); } },
            ].map((s) => (
              <button
                key={s.key}
                onClick={() => {
                  s.apply();
                  toast("success", t("sceneApplied", { name: t(s.key) }));
                }}
                className="px-3.5 py-2 rounded-xl text-[12.5px] font-medium transition-transform hover:scale-105"
                style={{ background: "var(--surface-2)", border: "1px solid var(--border)", color: "var(--text-1)" }}
              >
                {t(s.key)}
              </button>
            ))}
          </div>
        </Section>

        {/* 应用分流规则（Plus 专属） */}
        <Section id="sec-rules" icon={<Shuffle size={16} />} title={t("rulesTitle")} hint={t("rulesHint")}>
          <RouteRulesEditor adapters={adapters} rules={routeRules} setRules={setRouteRules} />
        </Section>

        {/* 悬浮窗 HUD（Plus 专属） */}
        <Section id="sec-hud" icon={<PictureInPicture2 size={16} />} title={t("settingsHud")} hint={t("settingsHudHint")}>
          <Row icon={<PictureInPicture2 size={15} />} label={t("settingHudEnable")} hint={t("settingHudEnableHint")}>
            <Switch checked={hudEnabled} onChange={(v) => set("hudEnabled", v)} />
          </Row>
          {hudEnabled && (
            <>
              <Row label={t("settingHudOpacity")}>
                <div className="flex items-center gap-3">
                  <input
                    type="range"
                    min={0.4}
                    max={1}
                    step={0.02}
                    value={hudOpacity}
                    onChange={(e) => set("hudOpacity", parseFloat(e.target.value))}
                    style={{ accentColor: "var(--accent)", width: 150 }}
                  />
                  <span className="text-[12px] mono w-[42px] text-right" style={{ color: "var(--text-1)" }}>
                    {Math.round(hudOpacity * 100)}%
                  </span>
                </div>
              </Row>
              <Row label={t("settingHudLock")} hint={t("settingHudLockHint")}>
                <Switch checked={hudLocked} onChange={(v) => set("hudLocked", v)} />
              </Row>
              <Row label={t("settingHudUnit")}>
                <Segmented<string>
                  value={hudUnit}
                  options={[
                    { value: "mbps", label: "MB/s" },
                    { value: "mbit", label: "Mbps" },
                  ]}
                  onChange={(v) => set("hudUnit", v as typeof hudUnit)}
                />
              </Row>
              <Row label={t("settingHudMetrics")}>
                <div className="flex items-center gap-2">
                  <ToggleChip active={hudShowDown} onClick={() => set("hudShowDown", !hudShowDown)}>
                    {t("hudMetricDown")}
                  </ToggleChip>
                  <ToggleChip active={hudShowUp} onClick={() => set("hudShowUp", !hudShowUp)}>
                    {t("hudMetricUp")}
                  </ToggleChip>
                  <ToggleChip active={hudShowConns} onClick={() => set("hudShowConns", !hudShowConns)}>
                    {t("hudMetricConns")}
                  </ToggleChip>
                </div>
              </Row>
              <Row label={t("settingHudNics")} hint={t("settingHudNicsHint")}>
                <Switch checked={hudShowNics} onChange={(v) => set("hudShowNics", v)} />
              </Row>
              <div className="pt-3" style={{ borderTop: "1px solid var(--border)" }}>
                <div className="eyebrow mb-2.5">{t("hudPreview")}</div>
                <div className="grid place-items-center py-4 rounded-xl" style={{ background: "var(--surface)", border: "1px dashed var(--border)" }}>
                  <HudPreview
                    opacity={hudOpacity}
                    unit={hudUnit}
                    showDown={hudShowDown}
                    showUp={hudShowUp}
                    showConns={hudShowConns}
                    showNics={hudShowNics}
                    theme={theme}
                  />
                </div>
              </div>
              <p className="text-[11px] mt-3" style={{ color: "var(--text-2)" }}>
                {t("hudTipDrag")}
              </p>
            </>
          )}
        </Section>

        {/* 流量控制（Plus 专属） */}
        <Section id="sec-traffic" icon={<Gauge size={16} />} title={t("settingsTraffic")} hint={t("settingsTrafficHint")}>
          <Row icon={<Gauge size={15} />} label={t("settingDownLimit")} hint={t("settingDownLimitHint")}>
            <div className="flex items-center gap-2">
              <NumberField value={downLimit} min={0} max={100000} disabled={running} onChange={(v) => set("downLimit", v)} />
              <span className="text-[11px]" style={{ color: "var(--text-2)" }}>
                {t("unitMbps")}
              </span>
            </div>
          </Row>
          <div className="flex flex-col gap-2 py-3" style={{ borderTop: "1px solid var(--border)" }}>
            <div className="flex items-center gap-2 text-[13px]" style={{ color: "var(--text-1)" }}>
              <span style={{ color: "var(--text-2)" }}>
                <Network size={15} />
              </span>
              {t("settingBypass")}
            </div>
            <div className="text-[11px] ml-[22px]" style={{ color: "var(--text-2)" }}>
              {t("settingBypassHint")}
            </div>
            <textarea
              value={bypassList}
              disabled={running}
              onChange={(e) => set("bypassList", e.target.value)}
              placeholder={t("bypassPlaceholder")}
              spellCheck={false}
              rows={4}
              className="mt-1 ml-[22px] px-3 py-2 rounded-lg text-[12px] mono resize-none outline-none"
              style={{
                background: "var(--surface-2)",
                border: "1px solid var(--border)",
                color: "var(--text-0)",
                opacity: running ? 0.5 : 1,
              }}
            />
          </div>
        </Section>

        {/* 配置备份（Plus 专属） */}
        <Section id="sec-backup" icon={<Save size={16} />} title={t("settingsBackup")} hint={t("settingsBackupHint")}>
          <div className="pt-1 flex items-center gap-2.5">
            <button
              onClick={exportConfig}
              className="px-3.5 py-1.5 rounded-lg text-[12.5px] font-medium text-white transition-transform hover:scale-105"
              style={{ background: "linear-gradient(135deg, var(--accent-deep), var(--accent))" }}
            >
              {t("btnExportConfig")}
            </button>
            <button
              onClick={importConfig}
              className="px-3.5 py-1.5 rounded-lg text-[12.5px] font-medium transition-colors"
              style={{ background: "var(--surface-2)", border: "1px solid var(--border)", color: "var(--text-1)" }}
            >
              {t("btnImportConfig")}
            </button>
          </div>
        </Section>

        {/* 应用兼容性 */}
        <Section id="sec-appcompat" icon={<Plug size={16} />} title={t("appcompatTitle")} hint={t("appcompatHint")}>
          <CompatRow
            icon={<Gamepad2 size={15} />}
            label={t("steamConfig")}
            onApply={() => handleAppConfig("steam", true)}
            onRestore={() => handleAppConfig("steam", false)}
            applyText={t("btnApply")}
            restoreText={t("btnRestore")}
          />
          <CompatRow
            icon={<MonitorDown size={15} />}
            label={t("idmConfig")}
            onApply={() => handleAppConfig("idm", true)}
            onRestore={() => handleAppConfig("idm", false)}
            applyText={t("btnApply")}
            restoreText={t("btnRestore")}
          />
        </Section>
      </div>
    </div>
  );
}

function Section({ id, icon, title, hint, children }: { id?: string; icon: ReactNode; title: string; hint?: string; children: ReactNode }) {
  return (
    <div id={id} className="panel p-5" style={{ scrollMarginTop: 56 }}>
      <div className="flex items-center gap-2 mb-1">
        <span style={{ color: "var(--cyan)" }}>{icon}</span>
        <h3 className="font-semibold text-[14px]">{title}</h3>
      </div>
      {hint && (
        <p className="text-[11.5px] mb-3" style={{ color: "var(--text-2)" }}>
          {hint}
        </p>
      )}
      <div className="flex flex-col">{children}</div>
    </div>
  );
}

function Row({ icon, label, hint, children }: { icon?: ReactNode; label: string; hint?: string; children: ReactNode }) {
  return (
    <div className="flex items-center justify-between py-3 gap-4" style={{ borderTop: "1px solid var(--border)" }}>
      <div className="min-w-0">
        <div className="flex items-center gap-2 text-[13px]" style={{ color: "var(--text-1)" }}>
          {icon && <span style={{ color: "var(--text-2)" }}>{icon}</span>}
          {label}
        </div>
        {hint && (
          <div className="text-[11px] mt-1 ml-[22px]" style={{ color: "var(--text-2)" }}>
            {hint}
          </div>
        )}
      </div>
      <div className="shrink-0">{children}</div>
    </div>
  );
}

function Segmented<T extends string | boolean>({
  value,
  options,
  onChange,
}: {
  value: T;
  options: { value: T; label: string }[];
  onChange: (v: T) => void;
}) {
  return (
    <div
      className="flex items-center p-0.5 rounded-lg gap-0.5"
      style={{ background: "var(--surface-2)", border: "1px solid var(--border)" }}
    >
      {options.map((o) => {
        const active = o.value === value;
        return (
          <button
            key={String(o.value)}
            onClick={() => onChange(o.value)}
            className="px-3.5 py-1.5 rounded-md text-[12.5px] font-medium transition-colors"
            style={{ background: active ? "var(--accent)" : "transparent", color: active ? "#fff" : "var(--text-1)" }}
          >
            {o.label}
          </button>
        );
      })}
    </div>
  );
}

function HudPreview({
  opacity,
  unit,
  showDown,
  showUp,
  showConns,
  showNics,
  theme,
}: {
  opacity: number;
  unit: string;
  showDown: boolean;
  showUp: boolean;
  showConns: boolean;
  showNics: boolean;
  theme: Theme;
}) {
  const light = theme === "light";
  const txt0 = light ? "#111722" : "#e7eaee";
  const txt2 = light ? "#8995a4" : "#5b636d";
  const cardBg = (light ? "rgba(255,255,255," : "rgba(16,19,26,") + opacity + ")";
  const d = unit === "mbit" ? { v: "188.5", u: "Mbps" } : { v: "23.6", u: "MB/s" };
  const up = unit === "mbit" ? "12.4" : "1.6";
  const sample = [
    { n: "以太网", w: 78 },
    { n: "WLAN", w: 52 },
  ];
  return (
    <div
      className="rounded-2xl px-3.5 py-3 flex flex-col gap-1.5"
      style={{
        width: 232,
        background: cardBg,
        border: `1px solid ${light ? "rgba(15,30,60,0.12)" : "rgba(255,255,255,0.1)"}`,
        boxShadow: "0 12px 34px -14px rgba(0,0,0,0.6)",
        backdropFilter: "blur(14px)",
      }}
    >
      <div className="flex items-center gap-2">
        <span className="w-2 h-2 rounded-full" style={{ background: "#3ecf8e", boxShadow: "0 0 7px #3ecf8e" }} />
        <span className="text-[11px] font-bold tracking-tight" style={{ color: txt0 }}>
          HypoMux<span style={{ color: "var(--accent-soft)" }}>Plus</span>
        </span>
        <div className="flex-1" />
        <span className="grid place-items-center w-[22px] h-[22px] rounded-md" style={{ background: "var(--accent)", color: "#fff" }}>
          <Power size={12} />
        </span>
      </div>
      <svg width="100%" height={22} viewBox="0 0 200 22" preserveAspectRatio="none" className="block">
        <polyline
          points="0,18 24,10 48,14 72,6 96,11 120,4 144,9 168,5 200,8"
          fill="none"
          stroke="var(--accent-soft)"
          strokeWidth="1.6"
          strokeLinejoin="round"
        />
      </svg>
      {(showDown || showUp || showConns) && (
        <div className="flex items-end justify-between gap-2">
          {showDown && <PreviewMetric label="↓" v={d.v} u={d.u} color="var(--accent-soft)" txt2={txt2} />}
          {showUp && <PreviewMetric label="↑" v={up} u={d.u} color={txt0} txt2={txt2} />}
          {showConns && <PreviewMetric label="⇄" v="32" u="conns" color={txt0} txt2={txt2} />}
        </div>
      )}
      {showNics && (
        <div className="flex flex-col gap-1 mt-0.5 pt-1.5" style={{ borderTop: `1px solid ${light ? "rgba(15,30,60,0.08)" : "rgba(255,255,255,0.06)"}` }}>
          {sample.map((s) => (
            <div key={s.n} className="flex items-center gap-2">
              <span className="text-[9px] truncate flex-1" style={{ color: txt2 }}>
                {s.n}
              </span>
              <div className="w-[64px] h-[3px] rounded-full overflow-hidden" style={{ background: light ? "rgba(15,30,60,0.1)" : "rgba(255,255,255,0.1)" }}>
                <div className="h-full rounded-full" style={{ width: `${s.w}%`, background: "var(--accent-soft)" }} />
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function PreviewMetric({ label, v, u, color, txt2 }: { label: string; v: string; u: string; color: string; txt2: string }) {
  return (
    <div className="flex flex-col leading-none min-w-0">
      <span className="text-[9px] mono" style={{ color: txt2 }}>
        {label} {u}
      </span>
      <span className="text-[16px] font-bold mono mt-0.5" style={{ color }}>
        {v}
      </span>
    </div>
  );
}

function ToggleChip({ active, onClick, children }: { active: boolean; onClick: () => void; children: ReactNode }) {
  return (
    <button
      onClick={onClick}
      className="px-3 py-1.5 rounded-lg text-[12px] font-medium transition-colors"
      style={{
        background: active ? "var(--accent)" : "var(--surface-2)",
        color: active ? "#fff" : "var(--text-1)",
        border: `1px solid ${active ? "var(--accent)" : "var(--border)"}`,
      }}
    >
      {children}
    </button>
  );
}

function HotkeyCapture({
  value,
  onChange,
  recordingLabel,
}: {
  value: string;
  onChange: (v: string) => void;
  recordingLabel: string;
}) {
  const [rec, setRec] = useState(false);
  const fmt = (c: string) =>
    c.replace("Control", "Ctrl").replace("Super", "Win").split("+").join(" + ");
  const onKey = (e: React.KeyboardEvent) => {
    if (!rec) return;
    e.preventDefault();
    const key = e.key;
    if (["Control", "Alt", "Shift", "Meta", "OS"].includes(key)) return; // 等待非修饰键
    const mods: string[] = [];
    if (e.ctrlKey) mods.push("Control");
    if (e.altKey) mods.push("Alt");
    if (e.shiftKey) mods.push("Shift");
    if (e.metaKey) mods.push("Super");
    if (mods.length === 0) return; // 必须含至少一个修饰键
    let main = key.length === 1 ? key.toUpperCase() : key;
    if (key === " ") main = "Space";
    onChange([...mods, main].join("+"));
    setRec(false);
  };
  return (
    <button
      onClick={() => setRec(true)}
      onKeyDown={onKey}
      onBlur={() => setRec(false)}
      className="px-3.5 py-1.5 rounded-lg text-[12.5px] font-semibold mono transition-colors"
      style={{
        background: rec ? "var(--accent)" : "var(--surface-2)",
        color: rec ? "#fff" : "var(--text-0)",
        border: `1px solid ${rec ? "var(--accent)" : "var(--border)"}`,
        minWidth: 130,
      }}
    >
      {rec ? recordingLabel : fmt(value)}
    </button>
  );
}

function CompatRow({
  icon,
  label,
  onApply,
  onRestore,
  applyText,
  restoreText,
}: {
  icon: ReactNode;
  label: string;
  onApply: () => void;
  onRestore: () => void;
  applyText: string;
  restoreText: string;
}) {
  return (
    <Row icon={icon} label={label}>
      <div className="flex items-center gap-2">
        <button
          onClick={onApply}
          className="px-3.5 py-1.5 rounded-lg text-[12.5px] font-medium text-white transition-transform hover:scale-105"
          style={{ background: "linear-gradient(135deg, var(--accent-deep), var(--accent))" }}
        >
          {applyText}
        </button>
        <button
          onClick={onRestore}
          className="px-3.5 py-1.5 rounded-lg text-[12.5px] font-medium transition-colors"
          style={{ background: "var(--surface-2)", border: "1px solid var(--border)", color: "var(--text-1)" }}
        >
          {restoreText}
        </button>
      </div>
    </Row>
  );
}

/** 应用分流规则编辑器：域名/端口 → 直连 / 聚合 / 指定网卡 */
function RouteRulesEditor({
  adapters,
  rules,
  setRules,
}: {
  adapters: AdapterInfo[];
  rules: { pattern: string; action: string }[];
  setRules: (r: { pattern: string; action: string }[]) => void;
}) {
  const { t } = useSettings();
  const nics = adapters.filter((a) => a.ipv4 && a.ipv4 !== "0.0.0.0");

  const update = (i: number, patch: Partial<{ pattern: string; action: string }>) => {
    setRules(rules.map((r, idx) => (idx === i ? { ...r, ...patch } : r)));
  };
  const add = () => setRules([...rules, { pattern: "", action: "aggregate" }]);
  const remove = (i: number) => setRules(rules.filter((_, idx) => idx !== i));

  return (
    <div className="pt-1 flex flex-col gap-2">
      {rules.length === 0 && (
        <div className="text-[12px] py-2" style={{ color: "var(--text-2)" }}>
          {t("rulesEmpty")}
        </div>
      )}
      {rules.map((r, i) => (
        <div key={i} className="flex items-center gap-2">
          <input
            value={r.pattern}
            onChange={(e) => update(i, { pattern: e.target.value })}
            placeholder={t("rulesPatternPh")}
            className="flex-1 px-2.5 py-1.5 rounded-lg text-[12.5px] outline-none"
            style={{ background: "var(--surface-2)", border: "1px solid var(--border)", color: "var(--text-0)" }}
          />
          <select
            value={r.action}
            onChange={(e) => update(i, { action: e.target.value })}
            className="px-2 py-1.5 rounded-lg text-[12.5px] outline-none shrink-0"
            style={{ background: "var(--surface-2)", border: "1px solid var(--border)", color: "var(--text-1)", maxWidth: 180 }}
          >
            <option value="aggregate">{t("ruleAggregate")}</option>
            <option value="direct">{t("ruleDirect")}</option>
            {nics.map((n) => (
              <option key={n.index} value={`nic:${n.index}`}>
                {t("ruleViaNic", { name: n.alias })}
              </option>
            ))}
          </select>
          <button
            onClick={() => remove(i)}
            className="grid place-items-center w-8 h-8 rounded-lg shrink-0 transition-colors hover:[background:var(--surface-hover)]"
            style={{ color: "var(--text-2)" }}
          >
            <Trash2 size={14} />
          </button>
        </div>
      ))}
      <button
        onClick={add}
        className="self-start flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[12.5px] font-medium transition-colors hover:[background:var(--surface-hover)]"
        style={{ background: "var(--surface-2)", border: "1px solid var(--border)", color: "var(--accent-soft)" }}
      >
        <Plus size={14} /> {t("rulesAdd")}
      </button>
    </div>
  );
}
