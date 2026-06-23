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
import { ToastProvider, useToast } from "./components/Toast";
import type { View } from "./components/shell-types";
import { useSettings } from "./store";
import {
  api,
  onBoostState,
  onConnections,
  onLog,
  onSpeedTest,
  onTelemetry,
  win,
  type AdapterInfo,
  type ConnInfo,
  type LatencyResult,
  type NicTelemetry,
  type SelectedNic,
  type TelemetryPayload,
} from "./lib/api";

const HISTORY_LEN = 60;
const LOG_CAP = 300;
const SELECTED_KEY = "hmx-plus-selected";
const LIFETIME_KEY = "hmx-lifetime-mb";
const LIFE_PEAK_KEY = "hmx-lifetime-peak";
const LIFE_SECS_KEY = "hmx-lifetime-secs";

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
  const { t, socksPort, httpPort, closeToTray, launchMinimized, autoBoost, strategy, globalHotkey, notifications, hotkeyCombo } =
    useSettings();
  const toast = useToast();

  const [view, setView] = useState<View>("dashboard");
  const [adapters, setAdapters] = useState<AdapterInfo[]>([]);
  const [selected, setSelected] = useState<Set<number>>(loadSelected);
  const [loading, setLoading] = useState(true);
  const [admin, setAdmin] = useState(true);

  const [running, setRunning] = useState(false);
  const [busy, setBusy] = useState(false);

  const [telemetry, setTelemetry] = useState<TelemetryPayload | null>(null);
  const [perNic, setPerNic] = useState<Record<string, NicTelemetry>>({});
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
  const [lifetimeMB, setLifetimeMB] = useState<number>(() => Number(localStorage.getItem(LIFETIME_KEY)) || 0);
  const [lifetimePeak, setLifetimePeak] = useState<number>(() => Number(localStorage.getItem(LIFE_PEAK_KEY)) || 0);
  const [lifetimeSeconds, setLifetimeSeconds] = useState<number>(
    () => Number(localStorage.getItem(LIFE_SECS_KEY)) || 0,
  );

  const onBoostRef = useRef<() => void>(() => {});

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
    }).then((u) => unlisteners.push(u));

    onBoostState((r) => setRunning(r)).then((u) => unlisteners.push(u));
    onConnections((c) => setConnections(c)).then((u) => unlisteners.push(u));
    onSpeedTest((r) =>
      setSpeedResults((prev) => ({ ...prev, [r.index]: { mbps: r.mbps, ok: r.ok } })),
    ).then((u) => unlisteners.push(u));

    return () => unlisteners.forEach((u) => u());
  }, [scan]);

  useEffect(() => {
    api.setCloseToTray(closeToTray).catch(() => {});
  }, [closeToTray]);

  // 持久化已选网卡（供"启动后自动加速"复用）
  useEffect(() => {
    localStorage.setItem(SELECTED_KEY, JSON.stringify([...selected]));
  }, [selected]);

  // 首次扫描完成后的启动自动化：最小化到托盘 / 自动加速
  useEffect(() => {
    if (booted.current || loading) return;
    booted.current = true;
    if (launchMinimized) win.hide().catch(() => {});
    if (autoBoost && selected.size > 0 && !running) {
      void onBoost();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loading]);

  useEffect(() => {
    if (!running) {
      setUptime(0);
      setPerNic({});
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
      // 与原项目一致：开启前提醒 Steam 重启
      const steam = await api.checkSteamRunning().catch(() => false);
      if (steam) toast("warning", t("warnSteamRunning"));

      await api.startBoost(chosen, socksPort, httpPort, strategy);
      toast("success", t("msgBoostStarted"));
      notify(t("msgBoostStarted"));
    } catch (e) {
      toast("error", t("msgStartFailed", { err: String(e) }));
    } finally {
      setBusy(false);
    }
  };

  onBoostRef.current = onBoost;

  // 全局热键：在任意界面一键加速 / 停止
  useEffect(() => {
    if (!globalHotkey || !hotkeyCombo) return;
    let cancelled = false;
    (async () => {
      try {
        await unregister(hotkeyCombo).catch(() => {});
        if (cancelled) return;
        await register(hotkeyCombo, (e) => {
          if (!e || e.state === "Pressed") onBoostRef.current();
        });
      } catch (err) {
        toast("error", t("msgHotkeyFailed", { err: String(err) }));
      }
    })();
    return () => {
      cancelled = true;
      unregister(hotkeyCombo).catch(() => {});
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [globalHotkey, hotkeyCombo]);

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
                    refresh={scan}
                    perNic={perNic}
                    loading={loading}
                    logs={logs}
                    clearLogs={() => setLogs([])}
                    connections={connections}
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
                  />
                ) : view === "about" ? (
                  <AboutPage lifetimeMB={lifetimeMB} />
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
