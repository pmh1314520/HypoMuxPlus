import { useCallback, useEffect, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { Sidebar } from "./components/Sidebar";
import { TopBar } from "./components/TopBar";
import { StatusBar } from "./components/StatusBar";
import { Dashboard } from "./components/Dashboard";
import { SettingsPage } from "./components/SettingsPage";
import { ToastProvider, useToast } from "./components/Toast";
import type { View } from "./components/shell-types";
import { useSettings } from "./store";
import {
  api,
  onBoostState,
  onLog,
  onTelemetry,
  type AdapterInfo,
  type NicTelemetry,
  type SelectedNic,
  type TelemetryPayload,
} from "./lib/api";

const HISTORY_LEN = 60;
const LOG_CAP = 300;

function AppInner() {
  const { t, socksPort, httpPort, closeToTray } = useSettings();
  const toast = useToast();

  const [view, setView] = useState<View>("dashboard");
  const [adapters, setAdapters] = useState<AdapterInfo[]>([]);
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [loading, setLoading] = useState(true);
  const [admin, setAdmin] = useState(true);

  const [running, setRunning] = useState(false);
  const [busy, setBusy] = useState(false);

  const [telemetry, setTelemetry] = useState<TelemetryPayload | null>(null);
  const [perNic, setPerNic] = useState<Record<string, NicTelemetry>>({});
  const [history, setHistory] = useState<number[]>(new Array(HISTORY_LEN).fill(0));
  const [peak, setPeak] = useState(0);
  const [uptime, setUptime] = useState(0);
  const [logs, setLogs] = useState<string[]>([]);

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
    }).then((u) => unlisteners.push(u));

    onBoostState((r) => setRunning(r)).then((u) => unlisteners.push(u));

    return () => unlisteners.forEach((u) => u());
  }, [scan]);

  useEffect(() => {
    api.setCloseToTray(closeToTray).catch(() => {});
  }, [closeToTray]);

  useEffect(() => {
    if (!running) {
      setUptime(0);
      setPerNic({});
      setTelemetry(null);
      setHistory(new Array(HISTORY_LEN).fill(0));
      setPeak(0);
      return;
    }
    const start = Date.now();
    const timer = setInterval(() => setUptime((Date.now() - start) / 1000), 1000);
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

  const onBoost = async () => {
    if (busy) return;
    if (running) {
      setBusy(true);
      try {
        await api.stopBoost();
        toast("info", t("msgBoostStopped"));
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

      await api.startBoost(chosen, socksPort, httpPort);
      toast("success", t("msgBoostStarted"));
    } catch (e) {
      toast("error", t("msgStartFailed", { err: String(e) }));
    } finally {
      setBusy(false);
    }
  };

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
                  />
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
