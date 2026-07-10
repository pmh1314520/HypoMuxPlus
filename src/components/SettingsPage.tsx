import { useEffect, useRef, useState, type ReactNode } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { disable as autoDisable, enable as autoEnable, isEnabled as autoIsEnabled } from "@tauri-apps/plugin-autostart";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import {
  Activity,
  AlertTriangle,
  Bell,
  Compass,
  Contrast,
  Download,
  Droplet,
  Gamepad2,
  Gauge,
  Globe,
  KeyRound,
  Languages,
  MinusSquare,
  MonitorCog,
  MonitorDown,
  Network,
  Palette,
  Pencil,
  PictureInPicture2,
  Plug,
  Plus,
  Power,
  Rocket,
  Save,
  ServerCog,
  Shield,
  ShieldCheck,
  Shuffle,
  Timer,
  Trash2,
  Wand2,
  Waypoints,
  X,
  Zap,
} from "lucide-react";
import { ACCENTS, useSettings, type AccentKey, type SchedStrategy, type Theme } from "../store";
import { type Lang } from "../i18n";
import { api } from "../lib/api";
import type { AdapterInfo } from "../lib/api";
import {
  UPSTREAM_MAX_COUNT,
  removeUpstreamRef,
  validateUpstream,
  type UpstreamKind,
  type UpstreamProxy,
  type UpstreamValidationFields,
} from "../lib/upstream";
import { parseSubscription } from "../lib/subscription";
import { validateDnsEndpoint, type DnsKind } from "../lib/dnsvalidate";
import {
  computeRouteDecision,
  formatRouteDecision,
  validateSimInput,
  type RouteDecisionDisplay,
  type RouteSimConfig,
  type RouteSimTarget,
} from "../lib/routesim";
import { useModal } from "../lib/useModal";
import { useToast } from "./Toast";
import { NumberField } from "./NumberField";
import { Switch } from "./Switch";
import { Tooltip } from "./Tooltip";

interface Props {
  running: boolean;
  adapters: AdapterInfo[];
  routeRules: { pattern: string; action: string; kind?: "domain" | "process" }[];
  setRouteRules: (rules: { pattern: string; action: string; kind?: "domain" | "process" }[]) => void;
  onStopBoost: () => void;
}

export function SettingsPage({ running, adapters, routeRules, setRouteRules, onStopBoost }: Props) {
  const { t, lang, theme, autoTheme, highContrast, accent, socksPort, httpPort, closeToTray, autostart, launchMinimized, autoBoost, autoBoostOnApp, strategy, globalHotkey, notifications, hotkeyCombo, hotkeyStop, downLimit, bypassList, tunMode, ipVersion, udpAssociate, hudEnabled, hudOpacity, hudLocked, hudUnit, hudShowDown, hudShowUp, hudShowConns, hudShowNics, hudClickThrough, sessionReport, set } =
    useSettings();
  const toast = useToast();
  const [admin, setAdmin] = useState(true);
  const [svc, setSvc] = useState<{ installed: boolean; available: boolean }>({ installed: false, available: false });
  const [svcBusy, setSvcBusy] = useState(false);

  const refreshSvc = () =>
    api
      .tunServiceStatus()
      .then(([installed, available]) => setSvc({ installed, available }))
      .catch(() => {});

  useEffect(() => {
    refreshSvc();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const installSvc = async () => {
    setSvcBusy(true);
    try {
      await api.installTunService();
      toast("success", t("tunSvcInstalled"));
      await refreshSvc();
    } catch (e) {
      toast("error", t("tunSvcInstallFailed", { err: String(e) }));
    } finally {
      setSvcBusy(false);
    }
  };

  const uninstallSvc = async () => {
    setSvcBusy(true);
    try {
      await api.uninstallTunService();
      toast("success", t("tunSvcUninstalled"));
      await refreshSvc();
    } catch (e) {
      toast("error", t("tunSvcUninstallFailed", { err: String(e) }));
    } finally {
      setSvcBusy(false);
    }
  };

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
  // 在系统文件管理器中打开本地日志文件夹；失败经既有 toast 反馈，不崩溃
  const openLogFolder = async () => {
    try {
      await api.openLogDir();
    } catch (e) {
      toast("error", String(e));
    }
  };

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
    { id: "sec-upstream", label: t("upstreamTitle") },
    { id: "sec-health", label: t("healthProbeTitle") },
    { id: "sec-subimport", label: t("subImportTitle") },
    { id: "sec-pernicdns", label: t("perNicDnsTitle") },
    { id: "sec-stability", label: t("stabilityTitle") },
    { id: "sec-routesim", label: t("routeSimTitle") },
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
          <Row icon={<Network size={15} />} label={t("ipVersion")}>
            <Segmented<"auto" | "v4first" | "v6first" | "v4only">
              value={ipVersion}
              options={[
                { value: "auto", label: t("ipVerAuto") },
                { value: "v4first", label: t("ipVerV4First") },
                { value: "v6first", label: t("ipVerV6First") },
                { value: "v4only", label: t("ipVerV4Only") },
              ]}
              onChange={(v) => set("ipVersion", v)}
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
            <div className="flex flex-col items-end gap-1.5">
              <div className="flex items-center gap-3">
                <div className="flex items-center gap-2">
                  <span className="text-[11px]" style={{ color: "var(--text-2)" }}>
                    {t("portHttp")}
                  </span>
                  <NumberField value={httpPort} disabled={running} onChange={(v) => set("httpPort", v)} ariaLabel={`${t("portHttp")} ${t("settingPorts")}`} />
                </div>
                <div className="flex items-center gap-2">
                  <span className="text-[11px]" style={{ color: "var(--text-2)" }}>
                    {t("portSocks")}
                  </span>
                  <NumberField value={socksPort} disabled={running} onChange={(v) => set("socksPort", v)} ariaLabel={`${t("portSocks")} ${t("settingPorts")}`} />
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
              {socksPort === httpPort && (
                <span className="text-[11px] font-medium" style={{ color: "var(--danger)" }}>
                  {t("portSameWarn")}
                </span>
              )}
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
          <Row icon={<Zap size={15} />} label={t("settingSessionReport")} hint={t("settingSessionReportHint")}>
            <Switch checked={sessionReport} onChange={(v) => set("sessionReport", v)} />
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
              <Row label={t("settingHudClickThrough")} hint={t("settingHudClickThroughHint")}>
                <Switch checked={hudClickThrough} onChange={(v) => set("hudClickThrough", v)} />
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
          <Row icon={<Network size={15} />} label={t("settingTunMode")} hint={t("settingTunModeHint")}>
            <Switch checked={tunMode} onChange={(v) => set("tunMode", v)} />
          </Row>
          {tunMode && (
            <div className="flex flex-col gap-2 py-3" style={{ borderTop: "1px solid var(--border)" }}>
              <div className="flex items-center gap-2 flex-wrap">
                <span className="flex items-center gap-1.5 text-[13px]" style={{ color: "var(--text-1)" }}>
                  <ServerCog size={15} style={{ color: "var(--accent-soft)" }} />
                  {t("tunSvcTitle")}
                </span>
                <span
                  className="text-[11px] px-2 py-0.5 rounded-md"
                  style={{
                    background: svc.available ? "color-mix(in srgb, var(--series-2) 20%, transparent)" : "var(--surface-2)",
                    color: svc.available ? "var(--series-2)" : "var(--text-2)",
                  }}
                >
                  {svc.available ? t("tunSvcRunning") : svc.installed ? t("tunSvcInstalledNotRunning") : t("tunSvcNotInstalled")}
                </span>
                <div className="flex-1" />
                {!svc.installed ? (
                  <button
                    onClick={installSvc}
                    disabled={svcBusy}
                    className="px-3 py-1.5 rounded-lg text-[12px] font-medium text-white transition-transform hover:scale-105"
                    style={{ background: "var(--accent)", opacity: svcBusy ? 0.5 : 1 }}
                  >
                    {svcBusy ? t("tunSvcWorking") : t("tunSvcInstall")}
                  </button>
                ) : (
                  <button
                    onClick={uninstallSvc}
                    disabled={svcBusy}
                    className="px-3 py-1.5 rounded-lg text-[12px] font-medium transition-colors"
                    style={{ background: "var(--surface-strong)", border: "1px solid var(--border)", color: "var(--text-1)", opacity: svcBusy ? 0.5 : 1 }}
                  >
                    {svcBusy ? t("tunSvcWorking") : t("tunSvcUninstall")}
                  </button>
                )}
              </div>
              <p className="text-[11px] leading-relaxed" style={{ color: "var(--text-2)" }}>
                {svc.available ? t("tunSvcHintReady") : t("tunSvcHint")}
              </p>
            </div>
          )}
          <Row icon={<Gauge size={15} />} label={t("settingDownLimit")} hint={t("settingDownLimitHint")}>
            <div className="flex items-center gap-2">
              <NumberField value={downLimit} min={0} max={100000} disabled={running} onChange={(v) => set("downLimit", v)} ariaLabel={t("settingDownLimit")} />
              <span className="text-[11px]" style={{ color: "var(--text-2)" }}>
                {t("unitMbps")}
              </span>
            </div>
          </Row>
          <Row icon={<Network size={15} />} label={t("udpAssociate")} hint={t("udpAssociateHint")}>
            <Switch checked={udpAssociate} onChange={(v) => set("udpAssociate", v)} ariaLabel={t("udpAssociate")} />
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

        {/* 上游代理链 / 多节点聚合（Plus 专属） */}
        <Section id="sec-upstream" icon={<Waypoints size={16} />} title={t("upstreamTitle")} hint={t("upstreamHint")}>
          <UpstreamChainEditor adapters={adapters} />
        </Section>

        {/* 健康探测与优选（Plus 专属） */}
        <Section id="sec-health" icon={<Activity size={16} />} title={t("healthProbeTitle")} hint={t("healthProbeHint")}>
          <HealthProbeSection />
        </Section>

        {/* 订阅导入（Plus 专属） */}
        <Section id="sec-subimport" icon={<Download size={16} />} title={t("subImportTitle")} hint={t("subImportHint")}>
          <SubscriptionImportSection />
        </Section>

        {/* 每网卡 DNS / DoH（Plus 专属） */}
        <Section id="sec-pernicdns" icon={<Globe size={16} />} title={t("perNicDnsTitle")} hint={t("perNicDnsHint")}>
          <PerNicDnsSection adapters={adapters} />
        </Section>

        {/* 稳定性上限与防泄漏看门狗（Plus 专属） */}
        <Section id="sec-stability" icon={<Shield size={16} />} title={t("stabilityTitle")} hint={t("stabilityHint")}>
          <StabilitySection />
        </Section>

        {/* 分流决策模拟器（Plus 专属，纯展示不改引擎状态） */}
        <Section id="sec-routesim" icon={<Compass size={16} />} title={t("routeSimTitle")} hint={t("routeSimHint")}>
          <RouteSimSection adapters={adapters} routeRules={routeRules} />
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
            <button
              onClick={openLogFolder}
              aria-label={t("openLogDir")}
              className="px-3.5 py-1.5 rounded-lg text-[12.5px] font-medium transition-colors"
              style={{ background: "var(--surface-2)", border: "1px solid var(--border)", color: "var(--text-1)" }}
            >
              {t("openLogDir")}
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

/** 应用分流规则编辑器：域名/端口 或 进程名 → 直连 / 聚合 / 指定网卡 */
type RouteRuleItem = { pattern: string; action: string; kind?: "domain" | "process" };
function RouteRulesEditor({
  adapters,
  rules,
  setRules,
}: {
  adapters: AdapterInfo[];
  rules: RouteRuleItem[];
  setRules: (r: RouteRuleItem[]) => void;
}) {
  const { t } = useSettings();
  const toast = useToast();
  const [importUrl, setImportUrl] = useState("");
  const [importing, setImporting] = useState(false);
  const nics = adapters.filter((a) => a.ipv4 && a.ipv4 !== "0.0.0.0");

  const update = (i: number, patch: Partial<RouteRuleItem>) => {
    setRules(rules.map((r, idx) => (idx === i ? { ...r, ...patch } : r)));
  };
  const add = () => setRules([...rules, { pattern: "", action: "aggregate", kind: "domain" }]);
  const remove = (i: number) => setRules(rules.filter((_, idx) => idx !== i));

  // 规则订阅：从 URL 拉取规则列表并合并（每行：pattern 或 pattern,action）
  const importFromUrl = async () => {
    const url = importUrl.trim();
    if (!url || importing) return;
    setImporting(true);
    try {
      const text = await api.fetchText(url);
      const parsed: RouteRuleItem[] = [];
      for (const raw of text.split(/\r?\n/)) {
        const line = raw.trim();
        if (!line || line.startsWith("#") || line.startsWith("//")) continue;
        const [pat, act] = line.split(/[,\s]+/);
        if (!pat) continue;
        const action = act && /^(direct|aggregate|nic:\d+)$/.test(act) ? act : "direct";
        parsed.push({ pattern: pat.toLowerCase(), action, kind: "domain" });
      }
      if (parsed.length === 0) {
        toast("warning", t("rulesEmpty"));
        return;
      }
      // 合并去重（按 pattern+action）
      const seen = new Set(rules.map((r) => `${r.pattern}|${r.action}`));
      const merged = [...rules];
      for (const r of parsed) {
        const k = `${r.pattern}|${r.action}`;
        if (!seen.has(k)) {
          seen.add(k);
          merged.push(r);
        }
      }
      setRules(merged);
      setImportUrl("");
      toast("success", t("msgRulesImported", { n: parsed.length }));
    } catch (e) {
      toast("error", t("msgRulesImportFailed", { err: String(e) }));
    } finally {
      setImporting(false);
    }
  };

  return (
    <div className="pt-1 flex flex-col gap-2">
      {rules.length === 0 && (
        <div className="text-[12px] py-2" style={{ color: "var(--text-2)" }}>
          {t("rulesEmpty")}
        </div>
      )}
      {rules.map((r, i) => (
        <div key={i} className="flex items-center gap-2">
          <select
            value={r.kind ?? "domain"}
            onChange={(e) => update(i, { kind: e.target.value as "domain" | "process" })}
            aria-label={t("ruleKind")}
            className="px-2 py-1.5 rounded-lg text-[12.5px] outline-none shrink-0"
            style={{ background: "var(--surface-2)", border: "1px solid var(--border)", color: "var(--text-1)", maxWidth: 120 }}
          >
            <option value="domain">{t("ruleKindDomain")}</option>
            <option value="process">{t("ruleKindProcess")}</option>
          </select>
          <input
            value={r.pattern}
            onChange={(e) => update(i, { pattern: e.target.value })}
            placeholder={(r.kind ?? "domain") === "process" ? t("procNamePlaceholder") : t("rulesPatternPh")}
            aria-label={(r.kind ?? "domain") === "process" ? t("procNamePlaceholder") : t("rulesPattern")}
            className="flex-1 px-2.5 py-1.5 rounded-lg text-[12.5px] outline-none"
            style={{ background: "var(--surface-2)", border: "1px solid var(--border)", color: "var(--text-0)" }}
          />
          <select
            value={r.action}
            aria-label={t("rulesAction")}
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
      <div className="flex items-center gap-2 mt-1 pt-3" style={{ borderTop: "1px solid var(--border)" }}>
        <input
          value={importUrl}
          onChange={(e) => setImportUrl(e.target.value)}
          placeholder={t("rulesImportPh")}
          className="flex-1 px-2.5 py-1.5 rounded-lg text-[12px] outline-none"
          style={{ background: "var(--surface-2)", border: "1px solid var(--border)", color: "var(--text-0)" }}
        />
        <button
          onClick={importFromUrl}
          disabled={importing || !importUrl.trim()}
          className="px-3 py-1.5 rounded-lg text-[12.5px] font-medium shrink-0 transition-colors"
          style={{
            background: "var(--surface-2)",
            border: "1px solid var(--border)",
            color: "var(--accent-soft)",
            opacity: importing || !importUrl.trim() ? 0.5 : 1,
            cursor: importing || !importUrl.trim() ? "not-allowed" : "pointer",
          }}
        >
          {t("rulesImport")}
        </button>
      </div>
    </div>
  );
}

/** 判断某上游主机地址是否为本机回环（经它转发无法叠加多网卡带宽，仅提示不阻断）。 */
function isLoopbackHost(host: string): boolean {
  const h = host.trim().toLowerCase();
  return h === "127.0.0.1" || h === "localhost" || h === "::1";
}

/** 上游代理链编辑器：总开关 + 上游节点增删改 + 网卡↔上游映射 + 回退策略。 */
function UpstreamChainEditor({ adapters }: { adapters: AdapterInfo[] }) {
  const { t, upstreamChain, upstreamFallback, upstreams, setUpstreams, upstreamBindings, setUpstreamBindings, set } =
    useSettings();
  const toast = useToast();
  const [editorOpen, setEditorOpen] = useState(false);
  // null=新增；UpstreamProxy=编辑现有条目
  const [editing, setEditing] = useState<UpstreamProxy | null>(null);

  // 参与聚合的网卡：沿用既有「具备有效 IPv4」的可用网卡口径（与规则编辑器一致），index 即 IfIndex
  const nics = adapters.filter((a) => a.ipv4 && a.ipv4 !== "0.0.0.0");
  const atLimit = upstreams.length >= UPSTREAM_MAX_COUNT;

  const openCreate = () => {
    if (atLimit) return;
    setEditing(null);
    setEditorOpen(true);
  };
  const openEdit = (node: UpstreamProxy) => {
    setEditing(node);
    setEditorOpen(true);
  };

  // 保存节点：id 已存在则更新，否则追加（id 由弹窗以 crypto.randomUUID() 生成且删除后不复用）
  const saveNode = (node: UpstreamProxy) => {
    const exists = upstreams.some((u) => u.id === node.id);
    setUpstreams(exists ? upstreams.map((u) => (u.id === node.id ? node : u)) : [...upstreams, node]);
    setEditorOpen(false);
    toast("success", t("upstreamNodeSaved"));
  };

  // 删除节点：同步清理所有网卡映射对该上游的引用（Req 2.6）
  const deleteNode = (id: string) => {
    setUpstreams(upstreams.filter((u) => u.id !== id));
    setUpstreamBindings(removeUpstreamRef(upstreamBindings, id));
    toast("success", t("upstreamNodeDeleted"));
  };

  const boundIds = (ifIndex: number): string[] =>
    upstreamBindings.find((b) => b.ifIndex === ifIndex)?.upstreamIds ?? [];

  // 切换某网卡对某上游的绑定（多选，允许多网卡共享同一上游）
  const toggleBinding = (ifIndex: number, id: string) => {
    const existing = upstreamBindings.find((b) => b.ifIndex === ifIndex);
    if (!existing) {
      setUpstreamBindings([...upstreamBindings, { ifIndex, upstreamIds: [id] }]);
      return;
    }
    const has = existing.upstreamIds.includes(id);
    const nextIds = has ? existing.upstreamIds.filter((x) => x !== id) : [...existing.upstreamIds, id];
    setUpstreamBindings(upstreamBindings.map((b) => (b.ifIndex === ifIndex ? { ...b, upstreamIds: nextIds } : b)));
  };

  const nodeLabel = (u: UpstreamProxy) => (u.label.trim() ? u.label.trim() : `${u.host}:${u.port}`);

  return (
    <>
      {/* 总开关 */}
      <Row icon={<Waypoints size={15} />} label={t("upstreamEnable")} hint={t("upstreamEnableHint")}>
        <Switch checked={upstreamChain} onChange={(v) => set("upstreamChain", v)} ariaLabel={t("upstreamEnable")} />
      </Row>

      {/* 上游节点列表 */}
      <div className="flex flex-col gap-2 py-3" style={{ borderTop: "1px solid var(--border)" }}>
        <div className="flex items-center gap-2 text-[13px]" style={{ color: "var(--text-1)" }}>
          <span style={{ color: "var(--text-2)" }}>
            <ServerCog size={15} />
          </span>
          {t("upstreamNodesTitle")}
        </div>
        {upstreams.length === 0 ? (
          <div className="text-[12px] py-1 ml-[22px]" style={{ color: "var(--text-2)" }}>
            {t("upstreamNodesEmpty")}
          </div>
        ) : (
          <div className="flex flex-col gap-1.5 mt-1">
            {upstreams.map((u) => (
              <div
                key={u.id}
                className="flex flex-col gap-1 px-3 py-2 rounded-lg"
                style={{ background: "var(--surface-2)", border: "1px solid var(--border)" }}
              >
                <div className="flex items-center gap-2">
                  <span
                    className="text-[10.5px] font-semibold px-1.5 py-0.5 rounded shrink-0"
                    style={{ background: "var(--surface-strong)", color: "var(--accent-soft)" }}
                  >
                    {u.kind === "http" ? t("upstreamKindHttp") : t("upstreamKindSocks5")}
                  </span>
                  <span className="text-[12.5px] font-medium truncate" style={{ color: "var(--text-0)" }}>
                    {nodeLabel(u)}
                  </span>
                  <span className="text-[11px] mono truncate" style={{ color: "var(--text-2)" }}>
                    {u.host}:{u.port}
                  </span>
                  <div className="flex-1" />
                  <button
                    onClick={() => openEdit(u)}
                    aria-label={`${t("upstreamEdit")} ${nodeLabel(u)}`}
                    className="grid place-items-center w-7 h-7 rounded-lg shrink-0 transition-colors hover:[background:var(--surface-hover)]"
                    style={{ color: "var(--text-2)" }}
                  >
                    <Pencil size={13} />
                  </button>
                  <button
                    onClick={() => deleteNode(u.id)}
                    aria-label={`${t("upstreamDelete")} ${nodeLabel(u)}`}
                    className="grid place-items-center w-7 h-7 rounded-lg shrink-0 transition-colors hover:[background:var(--surface-hover)]"
                    style={{ color: "var(--text-2)" }}
                  >
                    <Trash2 size={13} />
                  </button>
                </div>
                {isLoopbackHost(u.host) && (
                  <div
                    className="flex items-start gap-1.5 text-[11px] leading-relaxed"
                    style={{ color: "var(--warn)" }}
                  >
                    <AlertTriangle size={13} className="shrink-0 mt-0.5" />
                    <span>{t("upstreamLoopbackWarn")}</span>
                  </div>
                )}
              </div>
            ))}
          </div>
        )}
        <button
          onClick={openCreate}
          disabled={atLimit}
          className="self-start flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[12.5px] font-medium mt-1 transition-colors hover:[background:var(--surface-hover)]"
          style={{
            background: "var(--surface-2)",
            border: "1px solid var(--border)",
            color: "var(--accent-soft)",
            opacity: atLimit ? 0.5 : 1,
            cursor: atLimit ? "not-allowed" : "pointer",
          }}
        >
          <Plus size={14} /> {t("upstreamAddNode")}
        </button>
        {atLimit && (
          <span className="text-[11px] font-medium" style={{ color: "var(--danger)" }}>
            {t("upstreamLimitReached")}
          </span>
        )}
      </div>

      {/* 网卡 ↔ 上游映射 */}
      <div className="flex flex-col gap-2 py-3" style={{ borderTop: "1px solid var(--border)" }}>
        <div className="flex items-center gap-2 text-[13px]" style={{ color: "var(--text-1)" }}>
          <span style={{ color: "var(--text-2)" }}>
            <Network size={15} />
          </span>
          {t("upstreamBinding")}
        </div>
        <div className="text-[11px] ml-[22px]" style={{ color: "var(--text-2)" }}>
          {t("upstreamBindingHint")}
        </div>
        {upstreams.length === 0 ? (
          <div className="text-[12px] py-1 ml-[22px]" style={{ color: "var(--text-2)" }}>
            {t("upstreamNodesEmpty")}
          </div>
        ) : (
          <div className="flex flex-col gap-2 mt-1">
            {nics.map((n) => {
              const ids = boundIds(n.index);
              return (
                <div
                  key={n.index}
                  className="flex flex-col gap-1.5 px-3 py-2 rounded-lg"
                  style={{ background: "var(--surface-2)", border: "1px solid var(--border)" }}
                >
                  <div className="flex items-center gap-2">
                    <span className="text-[12.5px] font-medium truncate" style={{ color: "var(--text-0)" }}>
                      {n.alias}
                    </span>
                    {ids.length === 0 && (
                      <span className="text-[11px]" style={{ color: "var(--text-2)" }}>
                        {t("upstreamBindingNone")}
                      </span>
                    )}
                  </div>
                  <div className="flex items-center gap-1.5 flex-wrap">
                    {upstreams.map((u) => {
                      const active = ids.includes(u.id);
                      return (
                        <button
                          key={u.id}
                          onClick={() => toggleBinding(n.index, u.id)}
                          aria-label={`${n.alias} · ${nodeLabel(u)}`}
                          aria-pressed={active}
                          className="px-2.5 py-1 rounded-lg text-[11.5px] font-medium transition-colors"
                          style={{
                            background: active ? "var(--accent)" : "var(--surface-strong)",
                            color: active ? "#fff" : "var(--text-1)",
                            border: `1px solid ${active ? "var(--accent)" : "var(--border)"}`,
                          }}
                        >
                          {nodeLabel(u)}
                        </button>
                      );
                    })}
                  </div>
                  {ids.length > 1 && (
                    <span className="text-[10.5px]" style={{ color: "var(--text-2)" }}>
                      {t("upstreamBindingMulti")}
                    </span>
                  )}
                </div>
              );
            })}
          </div>
        )}
      </div>

      {/* 回退策略 */}
      <Row icon={<Shuffle size={15} />} label={t("upstreamFallback")} hint={t("upstreamFallbackHint")}>
        <Segmented<"direct" | "fail">
          value={upstreamFallback}
          options={[
            { value: "direct", label: t("upstreamFallbackDirect") },
            { value: "fail", label: t("upstreamFallbackFail") },
          ]}
          onChange={(v) => set("upstreamFallback", v)}
        />
      </Row>

      <AnimatePresence>
        {editorOpen && (
          <UpstreamNodeDialog
            key="upstream-node-dialog"
            initial={editing}
            onSave={saveNode}
            onClose={() => setEditorOpen(false)}
          />
        )}
      </AnimatePresence>
    </>
  );
}

/** 上游节点编辑弹窗：字段级校验（保留输入 + 高亮失败字段），保存时经 validateUpstream 把关。 */
function UpstreamNodeDialog({
  initial,
  onSave,
  onClose,
}: {
  initial: UpstreamProxy | null;
  onSave: (node: UpstreamProxy) => void;
  onClose: () => void;
}) {
  const { t } = useSettings();
  const dialogRef = useModal(onClose);
  const [kind, setKind] = useState<UpstreamKind>(initial?.kind ?? "socks5");
  const [host, setHost] = useState(initial?.host ?? "");
  const [port, setPort] = useState<number>(initial?.port ?? 1080);
  const [authOn, setAuthOn] = useState<boolean>(!!(initial?.username || initial?.password));
  const [username, setUsername] = useState(initial?.username ?? "");
  const [password, setPassword] = useState(initial?.password ?? "");
  const [label, setLabel] = useState(initial?.label ?? "");
  const [fields, setFields] = useState<UpstreamValidationFields>({});

  const title = initial ? t("upstreamEdit") : t("upstreamAddNode");
  const showLoopback = isLoopbackHost(host);
  const clearField = (k: keyof UpstreamValidationFields) =>
    setFields((f) => (f[k] ? { ...f, [k]: false } : f));

  const submit = () => {
    const trimmedHost = host.trim();
    const res = validateUpstream({
      kind,
      host: trimmedHost,
      port,
      username: authOn ? username : undefined,
      password: authOn ? password : undefined,
    });
    if (!res.ok) {
      // 保留用户输入，仅高亮失败字段（Req 1.6）
      setFields(res.fields);
      return;
    }
    onSave({
      id: initial?.id ?? crypto.randomUUID(),
      kind,
      host: trimmedHost,
      port,
      username: authOn ? username : undefined,
      password: authOn ? password : undefined,
      label: label.trim().slice(0, 64),
    });
  };

  const inputStyle = (bad?: boolean) => ({
    background: "var(--surface-2)",
    border: `1px solid ${bad ? "var(--danger)" : "var(--border)"}`,
    color: "var(--text-0)",
  });

  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      className="fixed inset-0 z-[400] grid place-items-center p-6"
      style={{ background: "rgba(0,0,0,0.55)", backdropFilter: "blur(6px)" }}
      onClick={onClose}
    >
      <motion.div
        initial={{ opacity: 0, y: 20, scale: 0.97 }}
        animate={{ opacity: 1, y: 0, scale: 1 }}
        exit={{ opacity: 0, y: 12, scale: 0.98 }}
        transition={{ type: "spring", stiffness: 260, damping: 26 }}
        onClick={(e) => e.stopPropagation()}
        ref={dialogRef}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-label={title}
        className="panel w-[440px] max-w-[92vw] p-5 outline-none"
        style={{ boxShadow: "var(--shadow)" }}
      >
        <div className="flex items-center gap-3 mb-4">
          <span
            className="grid place-items-center w-9 h-9 rounded-xl shrink-0"
            style={{ background: "var(--surface-2)", border: "1px solid var(--border)", color: "var(--accent-soft)" }}
          >
            <Waypoints size={17} />
          </span>
          <h2 className="text-[15px] font-bold flex-1">{title}</h2>
          <button
            onClick={onClose}
            aria-label={t("upstreamCancel")}
            className="grid place-items-center w-8 h-8 rounded-lg transition-colors hover:[background:var(--surface-hover)]"
            style={{ color: "var(--text-2)" }}
          >
            <X size={16} />
          </button>
        </div>

        <div className="flex flex-col gap-3">
          {/* 类型 */}
          <DialogField label={t("upstreamKind")}>
            <Segmented<UpstreamKind>
              value={kind}
              options={[
                { value: "socks5", label: t("upstreamKindSocks5") },
                { value: "http", label: t("upstreamKindHttp") },
              ]}
              onChange={(v) => {
                setKind(v);
                clearField("kind");
              }}
            />
          </DialogField>

          {/* 主机地址 */}
          <DialogField label={t("upstreamHost")} error={fields.host ? t("upstreamInvalidHost") : undefined}>
            <input
              value={host}
              onChange={(e) => {
                setHost(e.target.value);
                clearField("host");
              }}
              placeholder={t("upstreamHostPlaceholder")}
              aria-label={t("upstreamHost")}
              spellCheck={false}
              className="w-full px-2.5 py-1.5 rounded-lg text-[12.5px] outline-none"
              style={inputStyle(fields.host)}
            />
          </DialogField>

          {/* 端口 */}
          <DialogField label={t("upstreamPort")} error={fields.port ? t("upstreamInvalidPort") : undefined}>
            <NumberField
              value={port}
              min={1}
              max={65535}
              onChange={(v) => {
                setPort(v);
                clearField("port");
              }}
              ariaLabel={t("upstreamPort")}
            />
          </DialogField>

          {/* 认证开关 */}
          <div className="flex items-center justify-between gap-3">
            <span className="text-[12.5px]" style={{ color: "var(--text-1)" }}>
              {t("upstreamAuthEnable")}
            </span>
            <Switch checked={authOn} onChange={setAuthOn} ariaLabel={t("upstreamAuthEnable")} />
          </div>
          {authOn && (
            <>
              <DialogField label={t("upstreamUser")} error={fields.username ? t("upstreamInvalidUser") : undefined}>
                <input
                  value={username}
                  onChange={(e) => {
                    setUsername(e.target.value);
                    clearField("username");
                  }}
                  placeholder={t("upstreamUserPlaceholder")}
                  aria-label={t("upstreamUser")}
                  autoComplete="off"
                  spellCheck={false}
                  className="w-full px-2.5 py-1.5 rounded-lg text-[12.5px] outline-none"
                  style={inputStyle(fields.username)}
                />
              </DialogField>
              <DialogField label={t("upstreamPass")} error={fields.password ? t("upstreamInvalidPass") : undefined}>
                <input
                  type="password"
                  value={password}
                  onChange={(e) => {
                    setPassword(e.target.value);
                    clearField("password");
                  }}
                  placeholder={t("upstreamPassPlaceholder")}
                  aria-label={t("upstreamPass")}
                  autoComplete="off"
                  spellCheck={false}
                  className="w-full px-2.5 py-1.5 rounded-lg text-[12.5px] outline-none"
                  style={inputStyle(fields.password)}
                />
              </DialogField>
            </>
          )}

          {/* 备注名 */}
          <DialogField label={t("upstreamLabel")}>
            <input
              value={label}
              onChange={(e) => setLabel(e.target.value)}
              placeholder={t("upstreamLabelPlaceholder")}
              aria-label={t("upstreamLabel")}
              spellCheck={false}
              className="w-full px-2.5 py-1.5 rounded-lg text-[12.5px] outline-none"
              style={inputStyle(false)}
            />
          </DialogField>

          {showLoopback && (
            <div className="flex items-start gap-1.5 text-[11px] leading-relaxed" style={{ color: "var(--warn)" }}>
              <AlertTriangle size={13} className="shrink-0 mt-0.5" />
              <span>{t("upstreamLoopbackWarn")}</span>
            </div>
          )}
        </div>

        <div className="flex items-center justify-end gap-2.5 mt-5">
          <button
            onClick={onClose}
            className="px-3.5 py-1.5 rounded-lg text-[12.5px] font-medium transition-colors"
            style={{ background: "var(--surface-2)", border: "1px solid var(--border)", color: "var(--text-1)" }}
          >
            {t("upstreamCancel")}
          </button>
          <button
            onClick={submit}
            className="px-3.5 py-1.5 rounded-lg text-[12.5px] font-medium text-white transition-transform hover:scale-105"
            style={{ background: "linear-gradient(135deg, var(--accent-deep), var(--accent))" }}
          >
            {t("upstreamSave")}
          </button>
        </div>
      </motion.div>
    </motion.div>
  );
}

/** 弹窗内单个字段行：标签 + 控件 + 可选错误提示（校验失败时以 danger 色展示）。 */
function DialogField({ label, error, children }: { label: string; error?: string; children: ReactNode }) {
  return (
    <div className="flex flex-col gap-1">
      <span className="text-[11.5px] font-medium" style={{ color: "var(--text-2)" }}>
        {label}
      </span>
      {children}
      {error && (
        <span className="text-[11px]" style={{ color: "var(--danger)" }}>
          {error}
        </span>
      )}
    </div>
  );
}

/** 健康探测与优选配置分区：总开关 + 间隔/超时/阈值/冷却参数（数值输入带合理范围校验）。
 *  参数绑定 store 的 `healthCfg`（时间字段以毫秒持久化，界面以秒展示）。默认关闭，零回归。 */
function HealthProbeSection() {
  const { t, healthCfg, set } = useSettings();
  // 局部合并写回：仅改动传入的字段，其余保持不变。
  const setCfg = (patch: Partial<typeof healthCfg>) => set("healthCfg", { ...healthCfg, ...patch });
  // 毫秒 -> 秒（展示用），至少 1 秒避免出现 0。
  const toSec = (ms: number) => Math.max(1, Math.round(ms / 1000));

  return (
    <>
      <Row icon={<Activity size={15} />} label={t("healthProbeEnable")} hint={t("healthProbeEnableHint")}>
        <Switch checked={healthCfg.enabled} onChange={(v) => setCfg({ enabled: v })} ariaLabel={t("healthProbeEnable")} />
      </Row>
      {healthCfg.enabled && (
        <>
          <Row icon={<Timer size={15} />} label={t("healthProbeInterval")} hint={t("healthProbeIntervalHint")}>
            <div className="flex items-center gap-2">
              <NumberField
                value={toSec(healthCfg.intervalMs)}
                min={5}
                max={3600}
                onChange={(v) => setCfg({ intervalMs: v * 1000 })}
                ariaLabel={t("healthProbeInterval")}
              />
              <span className="text-[11px]" style={{ color: "var(--text-2)" }}>
                {t("unitSeconds")}
              </span>
            </div>
          </Row>
          <Row icon={<Timer size={15} />} label={t("healthProbeTimeout")} hint={t("healthProbeTimeoutHint")}>
            <div className="flex items-center gap-2">
              <NumberField
                value={toSec(healthCfg.timeoutMs)}
                min={1}
                max={60}
                onChange={(v) => setCfg({ timeoutMs: v * 1000 })}
                ariaLabel={t("healthProbeTimeout")}
              />
              <span className="text-[11px]" style={{ color: "var(--text-2)" }}>
                {t("unitSeconds")}
              </span>
            </div>
          </Row>
          <Row icon={<Gauge size={15} />} label={t("healthProbeThreshold")} hint={t("healthProbeThresholdHint")}>
            <NumberField
              value={healthCfg.failThreshold}
              min={1}
              max={20}
              onChange={(v) => setCfg({ failThreshold: v })}
              ariaLabel={t("healthProbeThreshold")}
            />
          </Row>
          <Row icon={<Timer size={15} />} label={t("healthProbeCooldown")} hint={t("healthProbeCooldownHint")}>
            <div className="flex items-center gap-2">
              <NumberField
                value={toSec(healthCfg.cooldownMs)}
                min={5}
                max={3600}
                onChange={(v) => setCfg({ cooldownMs: v * 1000 })}
                ariaLabel={t("healthProbeCooldown")}
              />
              <span className="text-[11px]" style={{ color: "var(--text-2)" }}>
                {t("unitSeconds")}
              </span>
            </div>
          </Row>
          {/* 上游质量优选随健康探测自动生效：以只读开关如实反映其状态（不可单独关闭）。 */}
          <Row icon={<Shuffle size={15} />} label={t("upstreamSelectorEnable")} hint={t("upstreamSelectorEnableHint")}>
            <Switch checked={healthCfg.enabled} onChange={() => {}} disabled ariaLabel={t("upstreamSelectorEnable")} />
          </Row>
        </>
      )}
    </>
  );
}

/** 稳定性上限与防泄漏看门狗分区：connCap / taskCap 数值上限 + proxyGuardian 开关。
 *  分别绑定 store 的 `connCap` / `taskCap` / `proxyGuardian`（0 表示使用内置默认值）。 */
function StabilitySection() {
  const { t, connCap, taskCap, proxyGuardian, set } = useSettings();
  return (
    <>
      <Row icon={<Network size={15} />} label={t("connCapLabel")} hint={t("connCapHint")}>
        <NumberField
          value={connCap}
          min={0}
          max={65535}
          onChange={(v) => set("connCap", v)}
          ariaLabel={t("connCapLabel")}
        />
      </Row>
      <Row icon={<Activity size={15} />} label={t("taskCapLabel")} hint={t("taskCapHint")}>
        <NumberField
          value={taskCap}
          min={0}
          max={4096}
          onChange={(v) => set("taskCap", v)}
          ariaLabel={t("taskCapLabel")}
        />
      </Row>
      <Row icon={<ShieldCheck size={15} />} label={t("proxyGuardianEnable")} hint={t("proxyGuardianEnableHint")}>
        <Switch checked={proxyGuardian} onChange={(v) => set("proxyGuardian", v)} ariaLabel={t("proxyGuardianEnable")} />
      </Row>
    </>
  );
}

/** 分流决策模拟器分区：输入目标（域名/进程名 + 可选端口）后，以纯 TS 复算展示其将命中的
 *  分流路径（命中 bypass / 规则 / 承载网卡 / 直连或上游 / 选中上游标签）。语义与后端选路一致
 *  （Req 3.1/3.6）；输入经 `validateSimInput` 校验（Req 3.5）。仅做展示，不发起任何真实连接、
 *  不改变引擎 / 加速状态。配置从 store 与 props 读取：upstreamChain / bypassList / upstreams /
 *  upstreamBindings（store）、routeRules / adapters（props）。 */
function RouteSimSection({ adapters, routeRules }: { adapters: AdapterInfo[]; routeRules: RouteRuleItem[] }) {
  const { t, upstreamChain, bypassList, upstreams, upstreamBindings } = useSettings();

  const [host, setHost] = useState("");
  const [proc, setProc] = useState("");
  const [portStr, setPortStr] = useState("");
  const [result, setResult] = useState<RouteDecisionDisplay | null>(null);
  const [hostErr, setHostErr] = useState(false);
  const [portErr, setPortErr] = useState(false);

  // 参与聚合的网卡口径与其余分区一致（具备有效 IPv4）；index 即 IfIndex。
  const nics = adapters.filter((a) => a.ipv4 && a.ipv4 !== "0.0.0.0");

  // 承载网卡 IfIndex → 网卡别名（找不到时回退展示 IfIndex）。
  const nicAlias = (ifIndex: number): string => {
    const found = adapters.find((a) => a.index === ifIndex);
    return found ? found.alias : `#${ifIndex}`;
  };

  const onSimulate = () => {
    const h = host.trim();
    const p = proc.trim();
    const portTrim = portStr.trim();
    // 端口可选：留空时以 443（HTTPS）复算；填写则按输入解析并交由 validateSimInput 校验。
    const port = portTrim === "" ? 443 : Number(portTrim);

    const v = validateSimInput(h, port);
    setHostErr(Boolean(v.hostError));
    setPortErr(Boolean(v.portError));
    if (!v.ok) {
      setResult(null);
      return;
    }

    // 纯 TS 复算所需路由上下文（与后端 engine::start 规则分派 + decide_egress 输入一致）。
    const config: RouteSimConfig = {
      upstreamChain,
      bypass: bypassList.split("\n"),
      rules: routeRules,
      bindings: upstreamBindings,
      chosenIfIndex: nics[0]?.index ?? 0,
      schedIdx: 0,
    };
    const target: RouteSimTarget = { host: h, port, procName: p.length > 0 ? p : undefined };
    setResult(formatRouteDecision(computeRouteDecision(config, target), upstreams));
  };

  return (
    <div className="pt-1 flex flex-col gap-3">
      {/* 目标输入：域名（必填）+ 进程名（可选）+ 端口（可选） */}
      <div className="flex flex-col gap-1.5">
        <input
          type="text"
          value={host}
          onChange={(e) => setHost(e.target.value)}
          placeholder={t("routeSimHostPlaceholder")}
          aria-label={t("routeSimHostPlaceholder")}
          spellCheck={false}
          className="px-3 py-2 rounded-lg text-[12.5px] mono outline-none"
          style={{
            background: "var(--surface-2)",
            border: `1px solid ${hostErr ? "var(--danger, #e5484d)" : "var(--border)"}`,
            color: "var(--text-0)",
          }}
        />
        {hostErr && (
          <div className="text-[11px] ml-0.5" style={{ color: "var(--danger, #e5484d)" }}>
            {t("routeSimErrHost")}
          </div>
        )}
        <div className="flex items-center gap-2 flex-wrap">
          <input
            type="text"
            value={proc}
            onChange={(e) => setProc(e.target.value)}
            placeholder={t("routeSimProcPlaceholder")}
            aria-label={t("routeSimProcPlaceholder")}
            spellCheck={false}
            className="flex-1 min-w-[160px] px-3 py-2 rounded-lg text-[12.5px] mono outline-none"
            style={{ background: "var(--surface-2)", border: "1px solid var(--border)", color: "var(--text-0)" }}
          />
          <input
            type="text"
            inputMode="numeric"
            value={portStr}
            onChange={(e) => setPortStr(e.target.value)}
            placeholder={t("routeSimPortPlaceholder")}
            aria-label={t("routeSimPortPlaceholder")}
            spellCheck={false}
            className="w-[140px] px-3 py-2 rounded-lg text-[12.5px] mono outline-none"
            style={{
              background: "var(--surface-2)",
              border: `1px solid ${portErr ? "var(--danger, #e5484d)" : "var(--border)"}`,
              color: "var(--text-0)",
            }}
          />
        </div>
        {portErr && (
          <div className="text-[11px] ml-0.5" style={{ color: "var(--danger, #e5484d)" }}>
            {t("routeSimErrPort")}
          </div>
        )}
      </div>

      <div>
        <button
          onClick={onSimulate}
          aria-label={t("routeSimRun")}
          className="px-3.5 py-1.5 rounded-lg text-[12.5px] font-medium text-white transition-transform hover:scale-105"
          style={{ background: "linear-gradient(135deg, var(--accent-deep), var(--accent))" }}
        >
          {t("routeSimRun")}
        </button>
      </div>

      {/* 决策结果（纯展示） */}
      {result && (
        <div className="flex flex-col gap-2 pt-2" style={{ borderTop: "1px solid var(--border)" }}>
          <div className="flex items-center gap-2 text-[13px]" style={{ color: "var(--text-1)" }}>
            <span style={{ color: "var(--text-2)" }}>
              <Compass size={15} />
            </span>
            {t("routeSimResultTitle")}
          </div>
          <div className="flex flex-col gap-1.5 mt-1">
            {/* 命中规则 */}
            <RouteSimResultRow
              icon={<Shuffle size={14} />}
              label={t("routeSimFieldRule")}
              value={
                result.rulePattern
                  ? `${t(result.ruleKey)} · ${result.rulePattern}`
                  : t(result.ruleKey)
              }
            />
            {/* 承载网卡（命中 bypass 直连时无承载网卡） */}
            {result.nicIfIndex != null && (
              <RouteSimResultRow
                icon={<Network size={14} />}
                label={t("routeSimFieldNic")}
                value={nicAlias(result.nicIfIndex)}
              />
            )}
            {/* 出口方式 */}
            <RouteSimResultRow
              icon={<Waypoints size={14} />}
              label={t("routeSimFieldEgress")}
              value={t(result.egressKey)}
            />
            {/* 选中上游（仅走上游时展示） */}
            {result.upstreamLabel != null && (
              <RouteSimResultRow
                icon={<ServerCog size={14} />}
                label={t("routeSimFieldUpstream")}
                value={result.upstreamLabel}
              />
            )}
          </div>
        </div>
      )}
    </div>
  );
}

/** 分流决策结果的单行展示：左侧字段名（i18n），右侧对应取值。 */
function RouteSimResultRow({ icon, label, value }: { icon: ReactNode; label: string; value: string }) {
  return (
    <div
      className="flex items-center justify-between gap-3 px-3 py-2 rounded-lg"
      style={{ background: "var(--surface-2)", border: "1px solid var(--border)" }}
    >
      <div className="flex items-center gap-2 text-[12px] shrink-0" style={{ color: "var(--text-2)" }}>
        <span>{icon}</span>
        {label}
      </div>
      <div className="text-[12.5px] font-medium text-right truncate" style={{ color: "var(--text-0)" }}>
        {value}
      </div>
    </div>
  );
}

/** 每网卡 DNS / DoH 配置分区：为每张参与聚合的网卡单独配置 plain / doh + endpoint。
 *  经 `validateDnsEndpoint` 校验，非法端点提示且不持久化（禁止保存）；有效配置绑定 store 的 `perNicDns`。 */
function PerNicDnsSection({ adapters }: { adapters: AdapterInfo[] }) {
  const { t, perNicDns, setPerNicDns } = useSettings();
  // 参与聚合的网卡口径与其余分区一致（具备有效 IPv4），index 即 IfIndex。
  const nics = adapters.filter((a) => a.ipv4 && a.ipv4 !== "0.0.0.0");

  // 本地草稿：ifIndex -> { kind, endpoint }。草稿允许暂存非法输入以供编辑，
  // 仅当校验通过时才写入持久化 store（禁止保存非法端点，Req 7.5）。
  const [drafts, setDrafts] = useState<Record<number, { kind: DnsKind; endpoint: string }>>(() => {
    const init: Record<number, { kind: DnsKind; endpoint: string }> = {};
    for (const d of perNicDns) init[d.ifIndex] = { kind: d.kind, endpoint: d.endpoint };
    return init;
  });

  // 将有效配置写入 store；非法则从 store 移除该网卡条目（不持久化非法值）。
  const persist = (ifIndex: number, kind: DnsKind, endpoint: string) => {
    const others = perNicDns.filter((d) => d.ifIndex !== ifIndex);
    if (validateDnsEndpoint(kind, endpoint)) {
      setPerNicDns([...others, { ifIndex, kind, endpoint }]);
    } else {
      setPerNicDns(others);
    }
  };

  const toggle = (ifIndex: number, on: boolean) => {
    if (on) {
      setDrafts((d) => ({ ...d, [ifIndex]: { kind: "plain", endpoint: "" } }));
      // 空端点非法，暂不写入 store，待用户填入有效值后再持久化。
    } else {
      setDrafts((d) => {
        const next = { ...d };
        delete next[ifIndex];
        return next;
      });
      setPerNicDns(perNicDns.filter((x) => x.ifIndex !== ifIndex));
    }
  };

  const changeKind = (ifIndex: number, kind: DnsKind) => {
    const cur = drafts[ifIndex] ?? { kind: "plain", endpoint: "" };
    const next = { kind, endpoint: cur.endpoint };
    setDrafts((d) => ({ ...d, [ifIndex]: next }));
    persist(ifIndex, next.kind, next.endpoint);
  };

  const changeEndpoint = (ifIndex: number, endpoint: string) => {
    const cur = drafts[ifIndex] ?? { kind: "plain" as DnsKind, endpoint: "" };
    const next = { kind: cur.kind, endpoint };
    setDrafts((d) => ({ ...d, [ifIndex]: next }));
    persist(ifIndex, next.kind, next.endpoint);
  };

  if (nics.length === 0) {
    return (
      <div className="text-[12px] py-1" style={{ color: "var(--text-2)" }}>
        {t("upstreamNodesEmpty")}
      </div>
    );
  }

  return (
    <div className="pt-1 flex flex-col gap-2">
      {nics.map((n) => {
        const draft = drafts[n.index];
        const enabled = draft !== undefined;
        const invalid = enabled && draft.endpoint.length > 0 && !validateDnsEndpoint(draft.kind, draft.endpoint);
        return (
          <div
            key={n.index}
            className="flex flex-col gap-2 px-3 py-2.5 rounded-lg"
            style={{ background: "var(--surface-2)", border: "1px solid var(--border)" }}
          >
            <div className="flex items-center gap-2">
              <span className="text-[12.5px] font-medium truncate flex-1" style={{ color: "var(--text-0)" }}>
                {n.alias}
              </span>
              <Switch checked={enabled} onChange={(v) => toggle(n.index, v)} ariaLabel={`${n.alias} · ${t("perNicDnsTitle")}`} />
            </div>
            {enabled && (
              <div className="flex flex-col gap-2">
                <div className="flex items-center gap-2 flex-wrap">
                  <span className="text-[11px]" style={{ color: "var(--text-2)" }}>
                    {t("perNicDnsMode")}
                  </span>
                  <Segmented<DnsKind>
                    value={draft.kind}
                    options={[
                      { value: "plain", label: t("perNicDnsModePlain") },
                      { value: "doh", label: t("perNicDnsModeDoh") },
                    ]}
                    onChange={(v) => changeKind(n.index, v)}
                  />
                </div>
                <input
                  value={draft.endpoint}
                  onChange={(e) => changeEndpoint(n.index, e.target.value)}
                  placeholder={draft.kind === "doh" ? t("perNicDnsEndpointPlaceholderDoh") : t("perNicDnsEndpointPlaceholderPlain")}
                  aria-label={`${n.alias} · ${t("perNicDnsEndpoint")}`}
                  spellCheck={false}
                  className="w-full px-2.5 py-1.5 rounded-lg text-[12px] mono outline-none"
                  style={{
                    background: "var(--surface-strong)",
                    border: `1px solid ${invalid ? "var(--danger)" : "var(--border)"}`,
                    color: "var(--text-0)",
                  }}
                />
                {invalid && (
                  <span className="text-[11px]" style={{ color: "var(--danger)" }}>
                    {draft.kind === "doh" ? t("perNicDnsInvalidDoh") : t("perNicDnsInvalidPlain")}
                  </span>
                )}
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}

// 订阅测速：单次探测的超时与并发上限。
const SUB_PROBE_TIMEOUT_MS = 5000;
const SUB_PROBE_CONCURRENCY = 6;

/**
 * 对单个上游候选做尽力而为的连通性延迟探测（Req 4.6）。
 *
 * 说明：受浏览器 / WebView 能力限制，前端无法发起原始 TCP 连接。此处以带超时的 `fetch`
 * 探测候选 host:port 的可达性并计时——请求在超时内完成（无论以何种方式结束，如响应 / 连接被拒
 * / 协议不符）均视为已产生一次网络往返，取其耗时作为连通性延迟近似；超时中止则视为不可达（返回 null）。
 * 真实公网节点的精确测速由人工 / 集成验证兜底（见 spec 验证清单 Req 4.6）。
 */
async function probeUpstreamLatency(host: string, port: number, timeoutMs: number): Promise<number | null> {
  const controller = new AbortController();
  const timer = window.setTimeout(() => controller.abort(), timeoutMs);
  const target = host.includes(":") ? `http://[${host}]:${port}` : `http://${host}:${port}`;
  const start = performance.now();
  try {
    await fetch(target, { mode: "no-cors", cache: "no-store", signal: controller.signal });
    return Math.round(performance.now() - start);
  } catch {
    if (controller.signal.aborted) return null; // 超时 => 判定不可达
    return Math.round(performance.now() - start); // 非超时异常：已产生往返，取耗时近似
  } finally {
    window.clearTimeout(timer);
  }
}

/** 订阅导入分区：粘贴 Import_Source -> 即时解析预览候选与忽略计数 -> 一键测速排序 -> 确认并入上游列表。
 *  解析走 `parseSubscription` 纯逻辑；并入遵守既有 128 上限与去重（Req 4.6 / 4.7）。 */
function SubscriptionImportSection() {
  const { t, upstreams, setUpstreams } = useSettings();
  const toast = useToast();
  const [text, setText] = useState("");
  const [candidates, setCandidates] = useState<UpstreamProxy[]>([]);
  const [ignored, setIgnored] = useState(0);
  const [parsed, setParsed] = useState(false);
  const [testing, setTesting] = useState(false);
  // id -> 延迟(ms) 或 null(不可达)；键不存在表示尚未测速。
  const [latencies, setLatencies] = useState<Record<string, number | null>>({});

  const nodeLabel = (u: UpstreamProxy) => (u.label.trim() ? u.label.trim() : `${u.host}:${u.port}`);

  const onParse = () => {
    const res = parseSubscription(text);
    setCandidates(res.candidates);
    setIgnored(res.ignoredUnsupported);
    setLatencies({});
    setParsed(true);
    if (res.candidates.length === 0) {
      toast("warning", t("subImportEmptyResult"));
    } else {
      toast("success", t("subImportParsed", { n: res.candidates.length }));
    }
  };

  const onSpeedTest = async () => {
    if (candidates.length === 0 || testing) return;
    setTesting(true);
    setLatencies({});
    const results: Record<string, number | null> = {};
    const queue = [...candidates];
    const runWorker = async () => {
      for (;;) {
        const c = queue.shift();
        if (!c) break;
        results[c.id] = await probeUpstreamLatency(c.host, c.port, SUB_PROBE_TIMEOUT_MS);
        setLatencies({ ...results }); // 渐进更新已完成结果
      }
    };
    const workers = Array.from({ length: Math.min(SUB_PROBE_CONCURRENCY, candidates.length) }, runWorker);
    await Promise.all(workers);
    // 排序：可达（延迟升序）在前，不可达（null）排末尾并标记不可用（Req 4.6）。
    const sorted = [...candidates].sort((a, b) => {
      const la = results[a.id] == null ? Number.POSITIVE_INFINITY : (results[a.id] as number);
      const lb = results[b.id] == null ? Number.POSITIVE_INFINITY : (results[b.id] as number);
      return la - lb;
    });
    setCandidates(sorted);
    setLatencies(results);
    setTesting(false);
  };

  const onConfirm = () => {
    if (candidates.length === 0) return;
    const existingIds = new Set(upstreams.map((u) => u.id));
    const fresh = candidates.filter((c) => !existingIds.has(c.id));
    const remaining = Math.max(0, UPSTREAM_MAX_COUNT - upstreams.length);
    const toAdd = fresh.slice(0, remaining);
    if (fresh.length > remaining) {
      // 超出 128 上限：截断并提示（Req 4.7）。
      toast("warning", t("subImportLimitHint"));
    }
    if (toAdd.length === 0) return;
    setUpstreams([...upstreams, ...toAdd]);
    toast("success", t("subImportConfirmed", { n: toAdd.length }));
    // 清空一次性草稿。
    setText("");
    setCandidates([]);
    setIgnored(0);
    setParsed(false);
    setLatencies({});
  };

  const latencyLabel = (id: string): { text: string; unavailable: boolean } => {
    if (testing && !(id in latencies)) return { text: t("subImportSpeedTesting"), unavailable: false };
    if (!(id in latencies)) return { text: "", unavailable: false };
    const v = latencies[id];
    if (v == null) return { text: t("upstreamFallbackFail"), unavailable: true };
    return { text: `${v} ms`, unavailable: false };
  };

  return (
    <div className="pt-1 flex flex-col gap-3">
      <textarea
        value={text}
        onChange={(e) => setText(e.target.value)}
        placeholder={t("subImportPastePlaceholder")}
        aria-label={t("subImportTitle")}
        spellCheck={false}
        rows={5}
        className="px-3 py-2 rounded-lg text-[12px] mono resize-none outline-none"
        style={{ background: "var(--surface-2)", border: "1px solid var(--border)", color: "var(--text-0)" }}
      />
      <div className="flex items-center gap-2 flex-wrap">
        <button
          onClick={onParse}
          disabled={!text.trim()}
          className="px-3.5 py-1.5 rounded-lg text-[12.5px] font-medium text-white transition-transform hover:scale-105"
          style={{
            background: "linear-gradient(135deg, var(--accent-deep), var(--accent))",
            opacity: text.trim() ? 1 : 0.5,
            cursor: text.trim() ? "pointer" : "not-allowed",
          }}
        >
          {t("subImportParse")}
        </button>
        <button
          onClick={onSpeedTest}
          disabled={candidates.length === 0 || testing}
          className="px-3.5 py-1.5 rounded-lg text-[12.5px] font-medium transition-colors"
          style={{
            background: "var(--surface-2)",
            border: "1px solid var(--border)",
            color: "var(--accent-soft)",
            opacity: candidates.length === 0 || testing ? 0.5 : 1,
            cursor: candidates.length === 0 || testing ? "not-allowed" : "pointer",
          }}
        >
          {testing ? t("subImportSpeedTesting") : t("subImportSpeedTest")}
        </button>
        <button
          onClick={onConfirm}
          disabled={candidates.length === 0}
          className="px-3.5 py-1.5 rounded-lg text-[12.5px] font-medium transition-colors"
          style={{
            background: "var(--surface-2)",
            border: "1px solid var(--border)",
            color: "var(--text-1)",
            opacity: candidates.length === 0 ? 0.5 : 1,
            cursor: candidates.length === 0 ? "not-allowed" : "pointer",
          }}
        >
          {t("subImportConfirm")}
        </button>
      </div>

      {/* 解析预览 */}
      <div className="flex flex-col gap-2 pt-2" style={{ borderTop: "1px solid var(--border)" }}>
        <div className="flex items-center gap-2 text-[13px]" style={{ color: "var(--text-1)" }}>
          <span style={{ color: "var(--text-2)" }}>
            <ServerCog size={15} />
          </span>
          {t("subImportPreviewTitle")}
        </div>
        {!parsed ? (
          <div className="text-[12px] py-1 ml-[22px]" style={{ color: "var(--text-2)" }}>
            {t("subImportPreviewEmpty")}
          </div>
        ) : candidates.length === 0 ? (
          <div className="text-[12px] py-1 ml-[22px]" style={{ color: "var(--text-2)" }}>
            {t("subImportEmptyResult")}
          </div>
        ) : (
          <>
            <div className="text-[11.5px] ml-[22px]" style={{ color: "var(--text-2)" }}>
              {t("subImportParsed", { n: candidates.length })}
              {ignored > 0 ? ` · ${t("subImportIgnored", { n: ignored })}` : ""}
            </div>
            <div className="flex flex-col gap-1.5 mt-1">
              {candidates.map((u) => {
                const lat = latencyLabel(u.id);
                return (
                  <div
                    key={u.id}
                    className="flex items-center gap-2 px-3 py-2 rounded-lg"
                    style={{ background: "var(--surface-2)", border: "1px solid var(--border)" }}
                  >
                    <span
                      className="text-[10.5px] font-semibold px-1.5 py-0.5 rounded shrink-0"
                      style={{ background: "var(--surface-strong)", color: "var(--accent-soft)" }}
                    >
                      {u.kind === "http" ? t("upstreamKindHttp") : t("upstreamKindSocks5")}
                    </span>
                    <span className="text-[12.5px] font-medium truncate" style={{ color: "var(--text-0)" }}>
                      {nodeLabel(u)}
                    </span>
                    <span className="text-[11px] mono truncate" style={{ color: "var(--text-2)" }}>
                      {u.host}:{u.port}
                    </span>
                    <div className="flex-1" />
                    {lat.text && (
                      <span
                        className="text-[11px] mono shrink-0"
                        style={{ color: lat.unavailable ? "var(--danger)" : "var(--series-2)" }}
                      >
                        {lat.text}
                      </span>
                    )}
                  </div>
                );
              })}
            </div>
            <div className="text-[11px] mt-1 ml-[22px]" style={{ color: "var(--text-2)" }}>
              {t("subImportLimitHint")}
            </div>
          </>
        )}
      </div>
    </div>
  );
}
