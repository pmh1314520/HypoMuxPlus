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
import { AggregateSpeedTest } from "./components/AggregateSpeedTest";
import { SessionReport, type SessionStats } from "./components/SessionReport";
import { ToastProvider, useToast } from "./components/Toast";
import type { View } from "./components/shell-types";
import { useSettings, ACCENTS } from "./store";
import {
  api,
  emitHudConfig,
  emitHudNotice,
  onBoostState,
  onConnections,
  onConnClosed,
  onLog,
  onSpeedTest,
  onTelemetry,
  onTrayToggle,
  onNicAlert,
  onAutoBoost,
  onCli,
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
  id: number;
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

const NICCFG_KEY = "hmx-nic-config";
function loadNicConfig(): Record<number, { weight: number; limit: number }> {
  try {
    const raw = localStorage.getItem(NICCFG_KEY);
    if (raw) {
      const obj = JSON.parse(raw);
      if (obj && typeof obj === "object") return obj;
    }
  } catch {
    /* ignore */
  }
  return {};
}

export interface RouteRule {
  pattern: string;
  action: string; // "direct" | "aggregate" | "nic:<ifindex>"
  /** 规则类型：域名规则（默认）或按进程可执行文件名匹配；缺省视为 "domain"（向后兼容旧配置） */
  kind?: "domain" | "process";
}
const RULES_KEY = "hmx-route-rules";
function loadRouteRules(): RouteRule[] {
  try {
    const raw = localStorage.getItem(RULES_KEY);
    if (raw) {
      const arr = JSON.parse(raw);
      if (Array.isArray(arr)) return arr.filter((r) => r && typeof r.pattern === "string" && typeof r.action === "string");
    }
  } catch {
    /* ignore */
  }
  return [];
}

function AppInner() {
  const { t, lang, socksPort, httpPort, closeToTray, launchMinimized, autoBoost, autoBoostOnApp, strategy, globalHotkey, notifications, hotkeyCombo, hotkeyStop, downLimit, bypassList, tunMode, ipVersion, udpAssociate, upstreams, upstreamBindings, upstreamChain, upstreamFallback, healthCfg, connCap, taskCap, proxyGuardian, systemProxy, perNicDns, alwaysOnTop, theme, accent, hudEnabled, hudOpacity, hudLocked, hudUnit, hudShowDown, hudShowUp, hudShowConns, hudShowNics, hudClickThrough, sessionReport, set } =
    useSettings();
  const toast = useToast();

  const [view, setView] = useState<View>(() => {
    const v = localStorage.getItem("hmx-view");
    const valid = ["dashboard", "stats", "diagnostics", "tutorial", "settings", "about"];
    return (v && valid.includes(v) ? v : "dashboard") as View;
  });
  const [adapters, setAdapters] = useState<AdapterInfo[]>([]);
  const [selected, setSelected] = useState<Set<number>>(loadSelected);
  const [nicConfig, setNicConfig] = useState<Record<number, { weight: number; limit: number }>>(loadNicConfig);
  const [routeRules, setRouteRules] = useState<RouteRule[]>(loadRouteRules);
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
  const [aggOpen, setAggOpen] = useState(false);
  const [report, setReport] = useState<SessionStats | null>(null);
  const [lifetimeMB, setLifetimeMB] = useState<number>(() => Number(localStorage.getItem(LIFETIME_KEY)) || 0);
  const [lifetimePeak, setLifetimePeak] = useState<number>(() => Number(localStorage.getItem(LIFE_PEAK_KEY)) || 0);
  const [lifetimeSeconds, setLifetimeSeconds] = useState<number>(
    () => Number(localStorage.getItem(LIFE_SECS_KEY)) || 0,
  );
  const [dailyMB, setDailyMB] = useState<Record<string, number>>(loadDaily);

  const onBoostRef = useRef<() => void>(() => {});
  const runningRef = useRef(false);
  const lastTeleRef = useRef(0);
  const autoStartedRef = useRef(false);
  const sessRef = useRef({ mb: 0, peak: 0, secs: 0 });
  const sessNicsRef = useRef(0);
  const prevRunningRef = useRef(false);
  const sawConnRef = useRef(false);
  const hintedProxyRef = useRef(false);
  const hintedNicRef = useRef(false);
  useEffect(() => {
    runningRef.current = running;
  }, [running]);

  const booted = useRef(false);

  // 持久化节流：用 ref 暂存最新统计值，每 5 秒 / 隐藏 / 卸载时落盘，避免每秒写磁盘
  const persistRef = useRef({ mb: lifetimeMB, peak: lifetimePeak, secs: lifetimeSeconds, daily: dailyMB });
  useEffect(() => {
    persistRef.current = { mb: lifetimeMB, peak: lifetimePeak, secs: lifetimeSeconds, daily: dailyMB };
  }, [lifetimeMB, lifetimePeak, lifetimeSeconds, dailyMB]);
  useEffect(() => {
    const flush = () => {
      const p = persistRef.current;
      localStorage.setItem(LIFETIME_KEY, String(p.mb));
      localStorage.setItem(LIFE_PEAK_KEY, String(p.peak));
      localStorage.setItem(LIFE_SECS_KEY, String(p.secs));
      localStorage.setItem(DAILY_KEY, JSON.stringify(p.daily));
    };
    const id = setInterval(flush, 5000);
    const onHide = () => {
      if (document.hidden) flush();
    };
    window.addEventListener("beforeunload", flush);
    document.addEventListener("visibilitychange", onHide);
    return () => {
      clearInterval(id);
      flush();
      window.removeEventListener("beforeunload", flush);
      document.removeEventListener("visibilitychange", onHide);
    };
  }, []);

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
      // 记录本次会话出现过的最大活跃网卡数（供战报展示协同网卡数）
      const activeNics = p.perNic.filter((nn) => nn.downMbps > 0).length;
      if (activeNics > sessNicsRef.current) sessNicsRef.current = activeNics;
      if (p.total.connections > 0) sawConnRef.current = true;
      // 托盘图标实时显示合并下行速度
      api.updateTraySpeed(p.total.downMbps).catch(() => {});
      // 按真实时间间隔积分本次下载量(MB)，避免遥测抖动导致统计偏差
      const now = Date.now();
      const last = lastTeleRef.current;
      lastTeleRef.current = now;
      const dt = last ? Math.min(5, Math.max(0, (now - last) / 1000)) : 1;
      const deltaMB = p.total.downMbps * dt;
      setSessionMB((prev) => prev + deltaMB);
      // 累计加速流量（持久化由节流落盘统一处理）
      setLifetimeMB((prev) => prev + deltaMB);
      setLifetimePeak((prev) => (p.total.downMbps > prev ? p.total.downMbps : prev));
      // 每日加速流量（仅保留最近 60 天）
      if (deltaMB > 0) {
        setDailyMB((prev) => {
          const key = todayKey();
          const next = { ...prev, [key]: (prev[key] ?? 0) + deltaMB };
          const keys = Object.keys(next).sort();
          while (keys.length > 60) {
            delete next[keys.shift() as string];
          }
          return next;
        });
      }
    }).then((u) => unlisteners.push(u));

    onBoostState((r) => setRunning(r)).then((u) => unlisteners.push(u));
    onConnections((c) => setConnections(c)).then((u) => unlisteners.push(u));
    onConnClosed((c) =>
      setConnHistory((prev) => {
        const next = [{ id: c.id, proto: c.proto, target: c.target, nic: c.nic, at: Date.now() }, ...prev];
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

    // 进程感知自动加速：检测到下载类应用自动加速，全部退出自动停（仅停自动启动的）
    onAutoBoost((boost) => {
      if (boost) {
        if (!runningRef.current) {
          autoStartedRef.current = true;
          onBoostRef.current();
        }
      } else if (runningRef.current && autoStartedRef.current) {
        autoStartedRef.current = false;
        onBoostRef.current();
      }
    }).then((u) => unlisteners.push(u));

    // CLI 控制（第二个实例转发的命令）
    onCli((action) => {
      if (action === "start") {
        if (!runningRef.current) onBoostRef.current();
      } else if (action === "stop") {
        if (runningRef.current) onBoostRef.current();
      } else if (action === "toggle") {
        onBoostRef.current();
      }
    }).then((u) => unlisteners.push(u));

    return () => unlisteners.forEach((u) => u());
  }, [scan]);

  useEffect(() => {
    api.setCloseToTray(closeToTray).catch(() => {});
  }, [closeToTray]);

  // 托盘菜单语言跟随客户端所选语言
  useEffect(() => {
    api.setTrayLanguage(lang === "en").catch(() => {});
  }, [lang]);

  // 同步"进程感知自动加速"开关到后端
  useEffect(() => {
    api.setAppWatch(autoBoostOnApp).catch(() => {});
  }, [autoBoostOnApp]);

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
      clickThrough: hudClickThrough,
    }).catch(() => {});
  }, [hudOpacity, hudLocked, hudUnit, hudShowDown, hudShowUp, hudShowConns, hudShowNics, accent, theme, hudClickThrough]);

  // 持久化已选网卡（供"启动后自动加速"复用）
  useEffect(() => {
    localStorage.setItem(SELECTED_KEY, JSON.stringify([...selected]));
  }, [selected]);

  // 持久化每网卡权重/限速配置
  useEffect(() => {
    localStorage.setItem(NICCFG_KEY, JSON.stringify(nicConfig));
  }, [nicConfig]);

  // 持久化分流规则
  useEffect(() => {
    localStorage.setItem(RULES_KEY, JSON.stringify(routeRules));
  }, [routeRules]);

  const setNicCfg = (index: number, patch: Partial<{ weight: number; limit: number }>) =>
    setNicConfig((prev) => {
      const cur = prev[index] ?? { weight: 100, limit: 0 };
      return { ...prev, [index]: { ...cur, ...patch } };
    });

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

  // 镜像本次会话数据到 ref，供停止时生成战报快照
  useEffect(() => {
    sessRef.current = { mb: sessionMB, peak, secs: uptime };
  }, [sessionMB, peak, uptime]);

  // 加速停止时弹出本次战报；开始时重置协同网卡计数
  useEffect(() => {
    if (prevRunningRef.current && !running) {
      const s = sessRef.current;
      if (sessionReport && s.mb >= 1) {
        setReport({ mb: s.mb, peak: s.peak, secs: s.secs, nics: Math.max(sessNicsRef.current, 1) });
      }
    }
    if (!prevRunningRef.current && running) {
      sessNicsRef.current = 0;
      sawConnRef.current = false;
      hintedProxyRef.current = false;
      hintedNicRef.current = false;
    }
    prevRunningRef.current = running;
  }, [running]);

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
      lastTeleRef.current = 0;
      api.resetTrayIcon().catch(() => {});
      return;
    }
    const start = Date.now();
    const timer = setInterval(() => {
      const secs = (Date.now() - start) / 1000;
      setUptime(secs);
      setLifetimeSeconds((prev) => prev + 1);
      // 加速 15s 仍无任何经代理的连接 → 多半是下载工具没配代理，提示用户
      if (secs >= 15 && !sawConnRef.current && !hintedProxyRef.current && selected.size > 0) {
        hintedProxyRef.current = true;
        const msg = t("proxyUnusedHint", { port: socksPort });
        toast("warning", msg);
        emitHudNotice("warning", msg).catch(() => {});
      }
      // 有连接但流量只集中在单卡（且选了多卡）→ 提示检查多线程/独立出口
      if (secs >= 24 && sawConnRef.current && !hintedNicRef.current && selected.size > 1 && sessNicsRef.current <= 1) {
        hintedNicRef.current = true;
        toast("info", t("oneNicHint"));
      }
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

  // 一键聚合测速：对已选网卡并发跑分，展示合并速度与提升幅度
  const runAggregate = async () => {
    const valid: SelectedNic[] = adapters
      .filter((a) => selected.has(a.index) && a.ipv4 && a.ipv4 !== "0.0.0.0")
      .map((a) => ({ index: a.index, name: a.alias, ip: a.ipv4 }));
    if (valid.length === 0) {
      toast("warning", t("aggNoSel"));
      return;
    }
    setBenchmarking(true);
    setSpeedResults({});
    try {
      await api.speedTest(valid, 8);
    } catch (e) {
      toast("error", String(e));
    } finally {
      setBenchmarking(false);
    }
  };
  const onAggregate = () => {
    const hasSel = adapters.some((a) => selected.has(a.index) && a.ipv4 && a.ipv4 !== "0.0.0.0");
    if (!hasSel) {
      toast("warning", t("aggNoSel"));
      return;
    }
    setAggOpen(true);
    if (!benchmarking) void runAggregate();
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
    // 同步提示：既在主窗口弹 toast，也推送给 HUD（托盘模式下主窗口不可见时仍有反馈）
    const notify2 = (kind: "success" | "warning" | "error" | "info", msg: string) => {
      toast(kind, msg);
      emitHudNotice(kind, msg).catch(() => {});
    };
    if (busy) return;
    if (running) {
      setBusy(true);
      try {
        await api.stopBoost();
        notify2("info", t("msgBoostStopped"));
        notify(t("msgBoostStopped"));
      } catch (e) {
        notify2("error", String(e));
      } finally {
        setBusy(false);
      }
      return;
    }

    const chosen: SelectedNic[] = adapters
      .filter((a) => selected.has(a.index) && a.ipv4 && a.ipv4 !== "0.0.0.0")
      .map((a) => ({
        index: a.index,
        name: a.alias,
        ip: a.ipv4,
        weight: nicConfig[a.index]?.weight ?? 100,
        limit_mbps: nicConfig[a.index]?.limit ?? 0,
      }));

    if (chosen.length === 0) {
      notify2("warning", t("warnNoSelection"));
      return;
    }

    // SOCKS 与 HTTP 端口相同会导致第二个监听绑定失败，提前拦截并给出明确指引
    if (socksPort === httpPort) {
      notify2("error", t("msgSamePort", { port: socksPort }));
      return;
    }

    setBusy(true);
    setLogs([]);
    try {
      // 启动前端口自愈（方案 A）：首选端口可用就沿用；不可用（被程序占用，或被 Windows
      // 系统保留 —— 如 Hyper-V/WSL/Docker 申请的 excludedportrange，此时 netstat 查不到
      // 却仍 bind 失败）则自动回退到可用端口，持久化写回设置并 toast 告知，避免用户撞上
      // 「端口不可用」而无法启动。suggestFreePort(start) 会先探测 start 本身，可用即原样
      // 返回，否则向上寻找下一个可用端口。
      let finalSocks = await api.suggestFreePort(socksPort).catch(() => socksPort);
      let finalHttp = await api
        .suggestFreePort(finalSocks === httpPort ? httpPort + 1 : httpPort)
        .catch(() => httpPort);
      // 兜底：确保两个端口互不相同（相同端口会导致第二个监听绑定失败）
      if (finalHttp === finalSocks) {
        finalHttp = await api.suggestFreePort(finalSocks + 1).catch(() => finalHttp);
      }
      // 端口发生自动切换：持久化保存并显式告知用户新端口（手动填端口的用户需据此更新）
      if (finalSocks !== socksPort || finalHttp !== httpPort) {
        set("socksPort", finalSocks);
        set("httpPort", finalHttp);
        notify2("warning", t("msgPortAutoSwitched", { socks: finalSocks, http: finalHttp }));
      }

      // 与原项目一致：开启前提醒 Steam 重启
      const steam = await api.checkSteamRunning().catch(() => false);
      if (steam) notify2("warning", t("warnSteamRunning"));

      const bypass = bypassList
        .split(/[\s,;]+/)
        .map((s) => s.trim())
        .filter(Boolean);
      await api.startBoost(chosen, finalSocks, finalHttp, strategy, lang, downLimit, bypass, routeRules, tunMode, ipVersion, udpAssociate, upstreams, upstreamBindings, upstreamChain, upstreamFallback, healthCfg, perNicDns, connCap, taskCap, proxyGuardian, systemProxy);
      // 未接管系统代理（且非 TUN）时提示为「仅本地代理已启动」，并给出需手动配置的地址
      const okMsg =
        !tunMode && !systemProxy
          ? t("msgBoostStartedLocal", { socks: finalSocks, http: finalHttp })
          : t("msgBoostStarted");
      notify2("success", okMsg);
      notify(okMsg);
    } catch (e) {
      notify2("error", t("msgStartFailed", { err: String(e) }));
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
                    onAggregate={onAggregate}
                    nicConfig={nicConfig}
                    setNicCfg={setNicCfg}
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
                    onApplyHealthy={applySelection}
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
                  <SettingsPage running={running} adapters={adapters} routeRules={routeRules} setRouteRules={setRouteRules} onStopBoost={() => { if (running) onBoost(); }} />)}
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

      {aggOpen && (
        <AggregateSpeedTest
          adapters={adapters}
          selected={selected}
          speedResults={speedResults}
          running={benchmarking}
          onClose={() => setAggOpen(false)}
          onRun={runAggregate}
        />
      )}
      {report && <SessionReport stats={report} onClose={() => setReport(null)} />}
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
