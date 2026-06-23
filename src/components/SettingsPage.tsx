import { useEffect, useState, type ReactNode } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  Gamepad2,
  Globe,
  Info,
  Languages,
  MonitorDown,
  Palette,
  Plug,
  ServerCog,
} from "lucide-react";
import { useSettings, type Theme } from "../store";
import { type Lang } from "../i18n";
import { api } from "../lib/api";
import { useToast } from "./Toast";

const REPO = "https://github.com/Hypostasis-Cat/HypoMux";

interface Props {
  running: boolean;
}

export function SettingsPage({ running }: Props) {
  const { t, lang, theme, socksPort, httpPort, closeToTray, set } = useSettings();
  const toast = useToast();
  const [admin, setAdmin] = useState(true);

  useEffect(() => {
    api.checkAdmin().then(setAdmin).catch(() => setAdmin(true));
  }, []);

  const handleAppConfig = async (
    app: "steam" | "idm",
    enable: boolean,
  ) => {
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
      <div className="max-w-[820px] mx-auto flex flex-col gap-5">
        {!admin && (
          <div
            className="glass px-4 py-3 text-[12.5px] leading-relaxed"
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
            <div className="flex items-center gap-3">
              <PortInput
                label={t("portHttp")}
                value={httpPort}
                disabled={running}
                onChange={(v) => set("httpPort", v)}
              />
              <PortInput
                label={t("portSocks")}
                value={socksPort}
                disabled={running}
                onChange={(v) => set("socksPort", v)}
              />
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

        {/* 关于 */}
        <Section icon={<Info size={16} />} title={t("aboutTitle")}>
          <Row label={t("aboutVersion")}>
            <span className="text-[13px]" style={{ color: "var(--text-1)" }}>
              v1.0.0
            </span>
          </Row>
          <Row label={t("aboutAuthor")}>
            <span className="text-[13px]" style={{ color: "var(--text-1)" }}>
              青云制作_彭明航
            </span>
          </Row>
          <Row label={t("aboutLicense")}>
            <span className="text-[13px]" style={{ color: "var(--text-1)" }}>
              AGPL-3.0
            </span>
          </Row>
          <Row icon={<Globe size={15} />} label={t("aboutOriginal")}>
            <button
              onClick={() => openUrl(REPO)}
              className="text-[13px] hover:underline"
              style={{ color: "var(--accent-soft)" }}
            >
              {REPO}
            </button>
          </Row>
          <p className="text-[12px] leading-relaxed mt-1 px-1" style={{ color: "var(--text-2)" }}>
            {t("aboutDesc")}
          </p>
        </Section>
      </div>
    </div>
  );
}

function Section({
  icon,
  title,
  hint,
  children,
}: {
  icon: ReactNode;
  title: string;
  hint?: string;
  children: ReactNode;
}) {
  return (
    <div className="glass p-5" style={{ boxShadow: "var(--shadow)" }}>
      <div className="flex items-center gap-2 mb-1">
        <span style={{ color: "var(--accent-soft)" }}>{icon}</span>
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

function Row({ icon, label, children }: { icon?: ReactNode; label: string; children: ReactNode }) {
  return (
    <div
      className="flex items-center justify-between py-3"
      style={{ borderTop: "1px solid var(--border)" }}
    >
      <div className="flex items-center gap-2 text-[13px]" style={{ color: "var(--text-1)" }}>
        {icon && <span style={{ color: "var(--text-2)" }}>{icon}</span>}
        {label}
      </div>
      {children}
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
      style={{ background: "var(--surface-strong)", border: "1px solid var(--border)" }}
    >
      {options.map((o) => {
        const active = o.value === value;
        return (
          <button
            key={String(o.value)}
            onClick={() => onChange(o.value)}
            className="px-3.5 py-1.5 rounded-md text-[12.5px] font-medium transition-colors"
            style={{
              background: active ? "var(--accent)" : "transparent",
              color: active ? "#fff" : "var(--text-1)",
            }}
          >
            {o.label}
          </button>
        );
      })}
    </div>
  );
}

function PortInput({
  label,
  value,
  disabled,
  onChange,
}: {
  label: string;
  value: number;
  disabled?: boolean;
  onChange: (v: number) => void;
}) {
  return (
    <div className="flex items-center gap-2">
      <span className="text-[11px]" style={{ color: "var(--text-2)" }}>
        {label}
      </span>
      <input
        type="number"
        min={1}
        max={65534}
        value={value}
        disabled={disabled}
        onChange={(e) => onChange(Math.min(65534, Math.max(1, Number(e.target.value) || 1)))}
        className="w-[88px] px-2.5 py-1.5 rounded-lg text-[13px] tabular-nums outline-none"
        style={{
          background: "var(--surface-strong)",
          border: "1px solid var(--border)",
          color: "var(--text-0)",
          opacity: disabled ? 0.5 : 1,
        }}
      />
    </div>
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
          style={{ background: "linear-gradient(135deg, var(--accent), var(--accent-soft))" }}
        >
          {applyText}
        </button>
        <button
          onClick={onRestore}
          className="px-3.5 py-1.5 rounded-lg text-[12.5px] font-medium transition-colors"
          style={{ background: "var(--surface-strong)", border: "1px solid var(--border)", color: "var(--text-1)" }}
        >
          {restoreText}
        </button>
      </div>
    </Row>
  );
}
