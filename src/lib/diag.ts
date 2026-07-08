// 诊断页纯逻辑：趋势历史追加/裁剪与文本报告生成。
// 本模块不依赖 DOM、React 或 Tauri `invoke`，便于单元测试与属性测试直接导入。

/** 单个诊断趋势采样点（每张网卡按时间追加，key = 网卡 index） */
export interface DiagTrendPoint {
  ts: number;
  latencyMs: number;
  jitterMs: number;
  lossPct: number;
  mbps: number;
  ok: boolean;
}

/** 诊断趋势历史：网卡 index -> 采样点序列（按时间升序） */
export type DiagTrend = Record<number, DiagTrendPoint[]>;

/**
 * 向趋势序列末尾追加一个新采样点。
 * 纯函数：返回新数组，不修改入参。追加后长度加一且末元素为新点。
 */
export function appendTrendPoint(trend: DiagTrendPoint[], point: DiagTrendPoint): DiagTrendPoint[] {
  return [...trend, point];
}

/**
 * 裁剪趋势序列，仅保留最近 `max` 个点（按时间顺序的末尾若干）。
 * 纯函数：返回新数组，不修改入参。结果长度不超过 `max`。
 * `max <= 0` 时返回空数组。
 */
export function capTrend(trend: DiagTrendPoint[], max: number): DiagTrendPoint[] {
  if (max <= 0) return [];
  if (trend.length <= max) return trend.slice();
  return trend.slice(trend.length - max);
}

/** 报告中一张网卡的已格式化指标（由调用方按当前语言/数据预先算好） */
export interface DiagReportRow {
  alias: string;
  ipv4: string;
  /** 已格式化的延迟文本，如 "23 ms" 或超时占位 */
  latency: string;
  /** 已格式化的抖动文本，如 "3 ms" 或不可用占位 */
  jitter: string;
  /** 已格式化的丢包率文本，如 "0%" 或不可用占位 */
  loss: string;
  /** 已格式化的吞吐文本，如 "18.5 MB/s" 或超时占位 */
  speed: string;
  /** 已翻译的评级文本 */
  grade: string;
}

/** 报告所需的标签文案（由调用方按当前语言注入，保持模块与 i18n 解耦） */
export interface DiagReportLabels {
  title: string;
  latency: string;
  jitter: string;
  loss: string;
  speed: string;
  grade: string;
}

/**
 * 生成纯文本体检报告的行数组。
 * 纯函数：不访问 DOM / store / 剪贴板，仅根据入参拼装文本行。
 * 结构：标题、时间戳、空行，随后每张网卡两行（名称+IP 行、指标行）。
 * 指标行含 RTT / 抖动 / 丢包 / 吞吐四项标签及评级。
 */
export function buildReportLines(rows: DiagReportRow[], labels: DiagReportLabels, timestamp: string): string[] {
  const lines: string[] = [labels.title, timestamp, ""];
  for (const r of rows) {
    lines.push(`• ${r.alias} (${r.ipv4})`);
    lines.push(
      `    ${labels.latency}: ${r.latency}  ${labels.jitter}: ${r.jitter}  ${labels.loss}: ${r.loss}  ${labels.speed}: ${r.speed}  ${labels.grade}: ${r.grade}`,
    );
  }
  return lines;
}
