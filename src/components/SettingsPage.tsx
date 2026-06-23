import { useEffect, useState, type ReactNode } from "react";
import { disable as autoDisable, enable as autoEnable, isEnabled as autoIsEnabled } from "@tauri-apps/plugin-autostart";
import {
  Bell,
  Gamepad2,
  KeyRound,
  Languages,
  MinusSquare,
  MonitorDown,
  Palette,
  Plug,
  Power,
  Rocket,
  ServerCog,
  Shuffle,
  Zap,
} from "lucide-react";
import { useSettings, type SchedStrategy, type Theme } from "../store";
import { type Lang } from "../i18n";
import { api } from "../lib/api";
import { useToast } from "./Toast";
import { NumberField } from "./NumberField";
import { Switch } from "./Switch";

interface Props {
  running: boolean;
}

export function SettingsPage({ running }: Props) {
  const { t, lang, theme, socksPort, httpPort, closeToTray, autostart, launchMinimized, autoBoost, strategy, globalHotkey, notifications, hotkeyCombo, set } =
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

  return (
    <div className="h-full overflow-y-auto px-1 pb-6">
      <div className="max-w-[860px] mx-auto flex flex-col gap-5">
        {!admin && (
          <div
            className="panel px-4 py-3 text-[12.5px] leading-relaxed"
            style={{ borderLeft: "3px solid var(--warn)", color: "var(--text-1)" }}
          >
            {t("adminWarn")}
          </div>
        )}

        {/* 通用 */}
        <Section icon={<ServerCog size={16} />} title={t("settingsGeneral")}>
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
              onChange={(v) => set("theme", v)}
            />
          </Row>
          <Row icon={<Plug size={15} />} label={t("settingPorts")}>
            <div className="flex items-center gap-4">
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
        <Section icon={<Rocket size={16} />} title={t("settingsAutomation")}>
          <Row icon={<Power size={15} />} label={t("settingAutostart")} hint={t("settingAutostartHint")}>
            <Switch checked={autostart} onChange={toggleAutostart} />
          </Row>
          <Row icon={<MinusSquare size={15} />} label={t("settingLaunchMin")}>
            <Switch checked={launchMinimized} onChange={(v) => set("launchMinimized", v)} />
          </Row>
          <Row icon={<Zap size={15} />} label={t("settingAutoBoost")} hint={t("settingAutoBoostHint")}>
            <Switch checked={autoBoost} onChange={(v) => set("autoBoost", v)} />
          </Row>
          <Row
            icon={<KeyRound size={15} />}
            label={t("settingHotkey")}
            hint={t("settingHotkeyHint")}
          >
            <Switch checked={globalHotkey} onChange={(v) => set("globalHotkey", v)} />
          </Row>
          {globalHotkey && (
            <Row label={t("settingHotkeyCombo")}>
              <HotkeyCapture
                value={hotkeyCombo}
                onChange={(v) => set("hotkeyCombo", v)}
                recordingLabel={t("hotkeyRecording")}
              />
            </Row>
          )}
          <Row icon={<Bell size={15} />} label={t("settingNotify")} hint={t("settingNotifyHint")}>
            <Switch checked={notifications} onChange={(v) => set("notifications", v)} />
          </Row>
        </Section>

        {/* 调度引擎（Plus 专属） */}
        <Section icon={<Shuffle size={16} />} title={t("schedTitle")} hint={t("schedHint")}>
          <div className="pt-1 flex items-center justify-between gap-4 flex-wrap">
            <Segmented<SchedStrategy>
              value={strategy}
              options={[
                { value: "rr", label: t("schedRR") },
                { value: "least", label: t("schedLeast") },
                { value: "weighted", label: t("schedWeighted") },
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

        {/* 应用兼容性 */}
        <Section icon={<Plug size={16} />} title={t("appcompatTitle")} hint={t("appcompatHint")}>
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

function Section({ icon, title, hint, children }: { icon: ReactNode; title: string; hint?: string; children: ReactNode }) {
  return (
    <div className="panel p-5">
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
