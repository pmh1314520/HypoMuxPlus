import { useCallback, useEffect, useRef, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { TitleBar, type View } from "./components/TitleBar";
import { Dashboard } from "./components/Dashboard";
import { SettingsPage } from "./components/SettingsPage";
import { ToastProvider, useToast } from "./components/Toast";
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

  const [running, setRunning] = useState(false);
  const [busy, setBusy] = useState(false);

  const [telemetry, setTelemetry] = useState<TelemetryPayload | null>(null);
  const [perNic, setPerNic] = useState<Record<string, NicTelemetry>>({});
  const [history, setHistory] = useState<number[]>(new Array(HISTORY_LEN).fill(0));
  const [peak, setPeak] = useState(0);
  const [uptime, setUptime] = useState(0);
  const [logs, setLogs] = useState<string[]>([]);

  const runningRef = useRef(running);
  runningRef.current = running;

  // ---- 扫描网卡 ----
  const scan = useCallback(async () => {
    setLoading(true);
    try {
      const list = await api.scanAdapters();
      setAdapters(list);
      // 自动保留仍存在的已选项
      setSelected((prev) => new Set([...prev].filter((i) => list.some((a) => a.index === i))));
    } catch (e) {
      toast("error", t("msgScanFailed", { err: String(e) }));
    } finally {
      setLoading(false);
    }
  }, [t, toast]);

  // ---- 初始化：扫描 + 状态 + 事件订阅 ----
  useEffect(() => {
    scan();
    api.getBoostState().then(setRunning).catch(() => {});

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

  // ---- 同步关闭行为到后端 ----
  useEffect(() => {
    api.setCloseToTray(closeToTray).catch(() => {});
  }, [closeToTray]);

  // ---- 运行计时 + 停止时清零 ----
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

  // ---- 勾选操作 ----
  const toggle = (index: number) =>
    setSelected((prev) => {
      const next = new Set(prev);
      next.has(index) ? next.delete(index) : next.add(index);
      return next;
    });

  const selectAll = () =>
    setSelected(new Set(adapters.filter((a) => a.ipv4 && a.ipv4 !== "0.0.0.0").map((a) => a.index)));
  const deselectAll = () => setSelected(new Set());

  // ---- 加速 / 停止 ----
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
      await api.startBoost(chosen, socksPort, httpPort);
      toast("success", t("msgBoostStarted"));
    } catch (e) {
      toast("error", t("msgStartFailed", { err: String(e) }));
    } finally {
      setBusy(false);
    }
  };

  const canBoost = running || selected.size > 0;

  return (
    <div className="h-screen flex flex-col">
      <TitleBar view={view} setView={setView} running={running} />

      <div className="flex-1 min-h-0 p-4">
        <AnimatePresence mode="wait">
          <motion.div
            key={view}
            initial={{ opacity: 0, y: 12 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: -12 }}
            transition={{ duration: 0.22, ease: "easeOut" }}
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
