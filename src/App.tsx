import { useCallback, useEffect, useRef, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { register, unregister } from "@tauri-apps/plugin-global-shortcut";
import { isPermissionGranted, requestPermission, sendNotification } from "@tauri-apps/plugin-notification";
import { Sidebar } from "./components/Sidebar";
import { TopBar } from "./components/TopBar";
import { StatusBar } from "./components/StatusBar";
import { Dashboard } from "./components/Dashboard";
import { StatsPage } from "./components/StatsPage";
import { DiagnosticsPage } from "./components/DiagnosticsPage";
import { TutorialPage } from "./components/TutorialPage";
import { AboutPage } from "./components/AboutPage";
import { SettingsPage } from "./components/SettingsPage";
import { Onboarding } from "./components/Onboarding";
import { UpdateDialog } from "./components/UpdateDialog";
import { ToastProvider, useToast } from "./components/Toast";
import type { View } from "./components/shell-types";
import { useSettings, ACCENTS } from "./store";
import {
  api,
  emitHudConfig,
  onBoostState,
  onConnections,
  onConnClosed,
  onLog,
  onSpeedTest,
  onTelemetry,
  onTrayToggle,
  onNicAlert,
  win,
  type AdapterInfo,
  type ConnInfo,
  type LatencyResult,
  type NicTelemetry,
  type SelectedNic,
  type TelemetryPayload,
  type UpdateInfo,
} from "./lib/api";

export interface ClosedConn {
  proto: string;
  target: string;
  nic: string;
  at: number;
}

const HISTORY_LEN = 60;
const NIC_SPARK_LEN = 24;
const LOG_CAP = 300;
const CONN_HISTORY_CAP = 200;
const SELECTED_KEY = "hmx-plus-selected";
const LIFETIME_KEY = "hmx-lifetime-mb";
const LIFE_PEAK_KEY = "hmx-lifetime-peak";
const LIFE_SECS_KEY = "hmx-lifetime-secs";
const DAILY_KEY = "hmx-daily-mb";

function todayKey(): string {
  const d = new Date();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${d.getFullYear()}-${m}-${day}`;
}

function loadDaily(): Record<string, number> {
  try {
    const raw = localStorage.getItem(DAILY_KEY);
    if (raw) {
      const obj = JSON.parse(raw);
      if (obj && typeof obj === "object") return obj;
    }
  } catch {
    /* ignore */
  }
  return {};
}

function loadSelected(): Set<number> {
  try {
    const raw = localStorage.getItem(SELECTED_KEY);
    if (raw) return new Set<number>(JSON.parse(raw));
  } catch {
    /* ignore */
  }
  return new Set();
}

function AppInner() {
  const { t, lang, socksPort, httpPort, closeToTray, launchMinimized, autoBoost, strategy, globalHotkey, notifications, hotkeyCombo, hotkeyStop, downLimit, bypassList, alwaysOnTop, theme, accent, hudEnabled, hudOpacity, hudLocked, hudUnit, hudShowDown, hudShowUp, hudShowConns, hudShowNics } =
    useSettings();
  const toast = useToast();

  const [view, setView] = useState<View>(() => {
    const v = localStorage.getItem("hmx-view");
    const valid = ["dashboard", "stats", "diagnostics", "tutorial", "settings", "about"];
    return (v && valid.includes(v) ? v : "dashboard") as View;
  });
  const [adapters, setAdapters] = useState<AdapterInfo[]>([]);
  const [selected, setSelected] = useState<Set<number>>(loadSelected);
  const [loading, setLoading] = useState(true);
  const [admin, setAdmin] = useState(true);

  const [running, setRunning] = useState(false);
  const [busy, setBusy] = useState(false);

  const [telemetry, setTelemetry] = useState<TelemetryPayload | null>(null);
  const [perNic, setPerNic] = useState<Record<string, NicTelemetry>>({});
  const [nicHistory, setNicHistory] = useState<Record<string, number[]>>({});
  const [history, setHistory] = useState<number[]>(new Array(HISTORY_LEN).fill(0));
  const [peak, setPeak] = useState(0);
  const [uptime, setUptime] = useState(0);
  const [sessionMB, setSessionMB] = useState(0);
  const [logs, setLogs] = useState<string[]>([]);
  const [latencies, setLatencies] = useState<Record<number, LatencyResult>>({});
  const [testing, setTesting] = useState(false);
  const [speedResults, setSpeedResults] = useState<Record<number, { mbps: number; ok: boolean }>>({});
  const [benchmarking, setBenchmarking] = useState(false);
  const [connections, setConnections] = useState<ConnInfo[]>([]);
  const [connHistory, setConnHistory] = useState<ClosedConn[]>([]);
  const [showOnboarding, setShowOnboarding] = useState(() => !localStorage.getItem("hmx-onboarded"));
  const [update, setUpdate] = useState<UpdateInfo | null>(null);
  const [lifetimeMB, setLifetimeMB] = useState<number>(() => Number(localStorage.getItem(LIFETIME_KEY)) || 0);
  const [lifetimePeak, setLifetimePeak] = useState<number>(() => Number(localStorage.getItem(LIFE_PEAK_KEY)) || 0);
  const [lifetimeSeconds, setLifetimeSeconds] = useState<number>(
    () => Number(localStorage.getItem(LIFE_SECS_KEY)) || 0,
  );
  const [dailyMB, setDailyMB] = useState<Record<string, number>>(loadDaily);

  const onBoostRef = useRef<() => void>(() => {});
  const runningRef = useRef(false);
  useEffect(() => {
    runningRef.current = running;
  }, [running]);

  const booted = useRef(false);

  const scan = useCallback(async () => {
    setLoading(true);
    try {
      const list = await api.scanAdapters();
      setAdapters(list);
      setSelected((prev) => new Set([...prev].filter((i) => list.some((a) => a.index === i))));
    } catch (e) {
      toast("error", t("msgScanFailed", { err: String(e) }));
    } finally {
      setLoading(false);
    }
  }, [t, toast]);

  useEffect(() => {
    scan();
    api.getBoostState().then(setRunning).catch(() => {});
    api.checkAdmin().then(setAdmin).catch(() => setAdmin(true));

    const unlisteners: Array<() => void> = [];
    onLog((line) => {
      setLogs((prev) => {
        const next = [...prev, line];
        return next.length > LOG_CAP ? next.slice(next.length - LOG_CAP) : next;
      });
    }).then((u) => unlisteners.push(u));

    onTelemetry((p) => {
      setTelemetry(p);
      const map: Record<string, NicTelemetry> = {};
      for (const n of p.perNic) map[n.name] = n;
      setPerNic(map);
      setNicHistory((prev) => {
        const next: Record<string, number[]> = {};
        for (const n of p.perNic) {
          const base = prev[n.name] ?? new Array(NIC_SPARK_LEN).fill(0);
          next[n.name] = [...base.slice(-(NIC_SPARK_LEN - 1)), n.downMbps];
        }
        return next;
      });
      setHistory((prev) => [...prev.slice(1), p.total.downMbps]);
      setPeak((prev) => Math.max(prev, p.total.downMbps));
      // 每秒一次采样，downMbps(MB/s) × 1s ≈ 本秒下载量(MB)
      setSessionMB((prev) => prev + p.total.downMbps);
      // 累计加速流量（跨会话持久化）
      setLifetimeMB((prev) => {
        const n = prev + p.total.downMbps;
        localStorage.setItem(LIFETIME_KEY, String(n));
        return n;
      });
      setLifetimePeak((prev) => {
        if (p.total.downMbps > prev) {
          localStorage.setItem(LIFE_PEAK_KEY, String(p.total.downMbps));
          return p.total.downMbps;
        }
        return prev;
      });
      // 每日加速流量（跨会话持久化，仅保留最近 60 天）
      if (p.total.downMbps > 0) {
        setDailyMB((prev) => {
          const key = todayKey();
          const next = { ...prev, [key]: (prev[key] ?? 0) + p.total.downMbps };
          const keys = Object.keys(next).sort();
          while (keys.length > 60) {
            delete next[keys.shift() as string];
          }
          localStorage.setItem(DAILY_KEY, JSON.stringify(next));
          return next;
        });
      }
    }).then((u) => unlisteners.push(u));

    onBoostState((r) => setRunning(r)).then((u) => unlisteners.push(u));
    onConnections((c) => setConnections(c)).then((u) => unlisteners.push(u));
    onConnClosed((c) =>
      setConnHistory((prev) => {
        const next = [{ proto: c.proto, target: c.target, nic: c.nic, at: Date.now() }, ...prev];
        return next.length > CONN_HISTORY_CAP ? next.slice(0, CONN_HISTORY_CAP) : next;
      }),
    ).then((u) => unlisteners.push(u));
    onSpeedTest((r) =>
      setSpeedResults((prev) => ({ ...prev, [r.index]: { mbps: r.mbps, ok: r.ok } })),
    ).then((u) => unlisteners.push(u));

    // 托盘菜单「切换加速」：触发与主界面一致的一键加速 / 停止流程
    onTrayToggle(() => onBoostRef.current()).then((u) => unlisteners.push(u));

    // 网卡掉线守护：失联 / 恢复时提示用户
    onNicAlert((a) =>
      toast(a.alive ? "success" : "warning", t(a.alive ? "nicUpToast" : "nicDownToast", { name: a.name })),
    ).then((u) => unlisteners.push(u));

    return () => unlisteners.forEach((u) => u());
  }, [scan]);

  useEffect(() => {
    api.setCloseToTray(closeToTray).catch(() => {});
  }, [closeToTray]);

  // 窗口置顶开关
  useEffect(() => {
    win.setAlwaysOnTop(alwaysOnTop).catch(() => {});
  }, [alwaysOnTop]);

  // 持久化当前分页，避免意外重载后回到默认页
  useEffect(() => {
    localStorage.setItem("hmx-view", view);
  }, [view]);

  // 启动时静默检查更新
  useEffect(() => {
    api
      .checkUpdate()
      .then((info) => {
        if (info.hasUpdate) setUpdate(info);
      })
      .catch(() => {});
  }, []);

  // 手动检查更新（关于页按钮）
  const onCheckUpdate = async () => {
    try {
      const info = await api.checkUpdate();
      if (info.hasUpdate) setUpdate(info);
      else toast("success", t("updLatest", { v: info.current }));
    } catch (e) {
      toast("error", t("updCheckFailed", { err: String(e) }));
    }
  };

  // 同步 HUD 启用状态到后端（控制托盘模式下是否显示悬浮窗）
  useEffect(() => {
    api.setHudEnabled(hudEnabled).catch(() => {});
  }, [hudEnabled]);

  // 推送 HUD 配置到悬浮窗（透明度 / 锁定 / 单位 / 显示项 / 配色 / 主题）
  useEffect(() => {
    const a = ACCENTS[accent] ?? ACCENTS.blue;
    emitHudConfig({
      opacity: hudOpacity,
      locked: hudLocked,
      unit: hudUnit,
      showDown: hudShowDown,
      showUp: hudShowUp,
      showConns: hudShowConns,
      showNics: hudShowNics,
      accent: a.accent,
      accentSoft: a.soft,
      theme,
    }).catch(() => {});
  }, [hudOpacity, hudLocked, hudUnit, hudShowDown, hudShowUp, hudShowConns, hudShowNics, accent, theme]);

  // 持久化已选网卡（供"启动后自动加速"复用）
  useEffect(() => {
    localStorage.setItem(SELECTED_KEY, JSON.stringify([...selected]));
  }, [selected]);

  // 首次扫描完成后的启动自动化：最小化到托盘 / 自动加速
  useEffect(() => {
    if (booted.current || loading) return;
    booted.current = true;
    if (launchMinimized) api.hideToTray().catch(() => {});
    if (autoBoost && selected.size > 0 && !running) {
      void onBoost();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loading]);

  useEffect(() => {
    if (!running) {
      setUptime(0);
      setPerNic({});
      setNicHistory({});
      setTelemetry(null);
      setHistory(new Array(HISTORY_LEN).fill(0));
      setPeak(0);
      setSessionMB(0);
      setConnections([]);
      return;
    }
    const start = Date.now();
    const timer = setInterval(() => {
      setUptime((Date.now() - start) / 1000);
      setLifetimeSeconds((prev) => {
        const n = prev + 1;
        localStorage.setItem(LIFE_SECS_KEY, String(n));
        return n;
      });
    }, 1000);
    return () => clearInterval(timer);
  }, [running]);

  const toggle = (index: number) =>
    setSelected((prev) => {
      const next = new Set(prev);
      next.has(index) ? next.delete(index) : next.add(index);
      return next;
    });

  const selectAll = () =>
    setSelected(new Set(adapters.filter((a) => a.ipv4 && a.ipv4 !== "0.0.0.0").map((a) => a.index)));
  const deselectAll = () => setSelected(new Set());
  // 套用网卡方案：仅纳入仍存在且具备有效 IPv4 的网卡
  const applySelection = (indices: number[]) =>
    setSelected(
      new Set(
        indices.filter((i) => adapters.some((a) => a.index === i && a.ipv4 && a.ipv4 !== "0.0.0.0")),
      ),
    );

  // 重置全部累计统计与每日趋势
  const resetStats = () => {
    setLifetimeMB(0);
    setLifetimePeak(0);
    setLifetimeSeconds(0);
    setDailyMB({});
    localStorage.removeItem(LIFETIME_KEY);
    localStorage.removeItem(LIFE_PEAK_KEY);
    localStorage.removeItem(LIFE_SECS_KEY);
    localStorage.removeItem(DAILY_KEY);
    toast("success", t("msgStatsReset"));
  };

  // 清空连接历史
  const clearConnHistory = () => setConnHistory([]);

  // 链路体检：逐张网卡探测出口延迟
  const onTest = async () => {
    const valid: SelectedNic[] = adapters
      .filter((a) => a.ipv4 && a.ipv4 !== "0.0.0.0")
      .map((a) => ({ index: a.index, name: a.alias, ip: a.ipv4 }));
    if (valid.length === 0) {
      toast("warning", t("msgLatencyNoSel"));
      return;
    }
    setTesting(true);
    try {
      const res = await api.testLatency(valid);
      const map: Record<number, LatencyResult> = {};
      for (const r of res) map[r.index] = r;
      setLatencies(map);
    } catch (e) {
      toast("error", String(e));
    } finally {
      setTesting(false);
    }
  };

  // 各网卡下载测速跑分
  const onBench = async () => {
    const valid: SelectedNic[] = adapters
      .filter((a) => a.ipv4 && a.ipv4 !== "0.0.0.0")
      .map((a) => ({ index: a.index, name: a.alias, ip: a.ipv4 }));
    if (valid.length === 0) {
      toast("warning", t("msgLatencyNoSel"));
      return;
    }
    setBenchmarking(true);
    setSpeedResults({});
    try {
      await api.speedTest(valid, 6);
    } catch (e) {
      toast("error", String(e));
    } finally {
      setBenchmarking(false);
    }
  };

  // 一键诊断：先测延迟，再测吞吐
  const onDiagnose = async () => {
    await onTest();
    await onBench();
  };

  // 单张网卡重测（诊断页卡片用）
  const onTestOne = async (a: AdapterInfo) => {
    if (!a.ipv4 || a.ipv4 === "0.0.0.0") return;
    const nic: SelectedNic = { index: a.index, name: a.alias, ip: a.ipv4 };
    try {
      const res = await api.testLatency([nic]);
      setLatencies((prev) => {
        const next = { ...prev };
        for (const r of res) next[r.index] = r;
        return next;
      });
      await api.speedTest([nic], 6);
    } catch (e) {
      toast("error", String(e));
    }
  };

  // 系统通知
  const notify = async (body: string) => {
    if (!notifications) return;
    try {
      let granted = await isPermissionGranted();
      if (!granted) granted = (await requestPermission()) === "granted";
      if (granted) sendNotification({ title: t("notifyTitle"), body });
    } catch {
      /* ignore */
    }
  };

  const onBoost = async () => {
    if (busy) return;
    if (running) {
      setBusy(true);
      try {
        await api.stopBoost();
        toast("info", t("msgBoostStopped"));
        notify(t("msgBoostStopped"));
      } catch (e) {
        toast("error", String(e));
      } finally {
        setBusy(false);
      }
      return;
    }

    const chosen: SelectedNic[] = adapters
      .filter((a) => selected.has(a.index) && a.ipv4 && a.ipv4 !== "0.0.0.0")
      .map((a) => ({ index: a.index, name: a.alias, ip: a.ipv4 }));

    if (chosen.length === 0) {
      toast("warning", t("warnNoSelection"));
      return;
    }

    setBusy(true);
    setLogs([]);
    try {
      // 启动前端口预检：被占用则给出明确提示与可用端口建议，避免难懂的绑定失败
      const [socksFree, httpFree] = await Promise.all([
        api.isPortFree(socksPort).catch(() => true),
        api.isPortFree(httpPort).catch(() => true),
      ]);
      if (!socksFree || !httpFree) {
        const busyPort = !socksFree ? socksPort : httpPort;
        const suggest = await api.suggestFreePort(busyPort + 1).catch(() => 0);
        toast(
          "error",
          t("msgPortBusy", { port: busyPort, suggest: suggest || busyPort + 1 }),
        );
        setBusy(false);
        return;
      }

      // 与原项目一致：开启前提醒 Steam 重启
      const steam = await api.checkSteamRunning().catch(() => false);
      if (steam) toast("warning", t("warnSteamRunning"));

      const bypass = bypassList
        .split(/[\s,;]+/)
        .map((s) => s.trim())
        .filter(Boolean);
      await api.startBoost(chosen, socksPort, httpPort, strategy, lang, downLimit, bypass);
      toast("success", t("msgBoostStarted"));
      notify(t("msgBoostStarted"));
    } catch (e) {
      toast("error", t("msgStartFailed", { err: String(e) }));
    } finally {
      setBusy(false);
    }
  };

  onBoostRef.current = onBoost;

  // 全局热键：分别绑定「加速」与「停止」两组快捷键
  useEffect(() => {
    if (!globalHotkey) return;
    let cancelled = false;
    const combos = [hotkeyCombo, hotkeyStop].filter(Boolean);
    (async () => {
      try {
        for (const c of combos) await unregister(c).catch(() => {});
        if (cancelled) return;
        if (hotkeyCombo) {
          await register(hotkeyCombo, (e) => {
            if ((!e || e.state === "Pressed") && !runningRef.current) onBoostRef.current();
          });
        }
        if (hotkeyStop && hotkeyStop !== hotkeyCombo) {
          await register(hotkeyStop, (e) => {
            if ((!e || e.state === "Pressed") && runningRef.current) onBoostRef.current();
          });
        }
      } catch (err) {
        toast("error", t("msgHotkeyFailed", { err: String(err) }));
      }
    })();
    return () => {
      cancelled = true;
      for (const c of combos) unregister(c).catch(() => {});
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [globalHotkey, hotkeyCombo, hotkeyStop]);

  const canBoost = running || selected.size > 0;
  const totalConn = telemetry?.total.connections ?? 0;

  return (
    <div className="h-screen flex flex-col">
      <div className="flex-1 min-h-0 flex">
        <Sidebar view={view} setView={setView} running={running} />

        <main className="flex-1 min-w-0 flex flex-col">
          <TopBar view={view} running={running} loading={loading} onRefresh={scan} />

          <div className="flex-1 min-h-0 px-5 pb-5">
            <AnimatePresence mode="wait">
              <motion.div
                key={view}
                initial={{ opacity: 0, y: 10 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0, y: -10 }}
                transition={{ duration: 0.2, ease: "easeOut" }}
                className="h-full"
              >
                {view === "dashboard" ? (
                  <Dashboard
                    telemetry={telemetry}
                    history={history}
                    peak={peak}
                    uptime={uptime}
                    sessionMB={sessionMB}
                    running={running}
                    busy={busy}
                    canBoost={canBoost}
                    onBoost={onBoost}
                    adapters={adapters}
                    selected={selected}
                    toggle={toggle}
                    selectAll={selectAll}
                    deselectAll={deselectAll}
                    applySelection={applySelection}
                    refresh={scan}
                    perNic={perNic}
                    nicHistory={nicHistory}
                    loading={loading}
                    logs={logs}
                    clearLogs={() => setLogs([])}
                    connections={connections}
                    connHistory={connHistory}
                    clearHistory={clearConnHistory}
                  />
                ) : view === "tutorial" ? (
                  <TutorialPage />
                ) : view === "diagnostics" ? (
                  <DiagnosticsPage
                    adapters={adapters}
                    latencies={latencies}
                    speedResults={speedResults}
                    diagnosing={testing || benchmarking}
                    onDiagnose={onDiagnose}
                    onTestOne={onTestOne}
                  />
                ) : view === "stats" ? (
                  <StatsPage
                    lifetimeMB={lifetimeMB}
                    lifetimePeak={lifetimePeak}
                    lifetimeSeconds={lifetimeSeconds}
                    sessionMB={sessionMB}
                    sessionPeak={peak}
                    uptime={uptime}
                    totalConn={totalConn}
                    running={running}
                    dailyMB={dailyMB}
                    onReset={resetStats}
                  />
                ) : view === "about" ? (
                  <AboutPage lifetimeMB={lifetimeMB} admin={admin} onReplayGuide={() => setShowOnboarding(true)} onCheckUpdate={onCheckUpdate} />
                ) : (
                  <SettingsPage running={running} />
                )}
              </motion.div>
            </AnimatePresence>
          </div>
        </main>
      </div>

      <StatusBar
        running={running}
        admin={admin}
        selectedCount={selected.size}
        socksPort={socksPort}
        httpPort={httpPort}
        totalConn={totalConn}
      />

      {showOnboarding && (
        <Onboarding
          onClose={() => {
            localStorage.setItem("hmx-onboarded", "1");
            setShowOnboarding(false);
          }}
        />
      )}

      {update && <UpdateDialog info={update} onClose={() => setUpdate(null)} />}
    </div>
  );
}

export default function App() {
  return (
    <ToastProvider>
      <AppInner />
    </ToastProvider>
  );
}
