// 分流决策模拟器前端纯逻辑模块（Feature: pro-differentiation-and-hardening）。
//
// 本模块仅包含不依赖 DOM / Tauri `invoke` 的纯函数与类型定义，
// 可被 vitest 直接导入做单元 / 属性测试（Req 3.1 / 3.5，Property 9）。
//
// 语义与后端 `compute_route_decision` 严格一致（见 design.md 第 7 节）：
//   优先级：bypass 最高 > 进程规则 > 域名规则 > 调度回退；
//   走上游 vs 直连的判定与 `decide_egress` 对同一输入的结果一致。
// 内部的 `patternMatch` / `matchProcRule` / `pickUpstreamForNic` / 规则分派逻辑
// 分别镜像后端 `engine.rs` 的 `pattern_match` / `match_proc_rule` /
// `pick_upstream_for_nic` 与 `engine::start` 的规则分派，保证前端「纯 TS 复算展示」
// 的结果与运行期后端选路一致，且不发起任何真实网络连接、不改变引擎状态（Req 3.6）。

import type { RouteRule } from "../App";
import type { UpstreamBinding, UpstreamProxy } from "./upstream";

/** 端口取值下界（与 upstream.ts / 后端一致）。 */
export const PORT_MIN = 1;
/** 端口取值上界。 */
export const PORT_MAX = 65535;
/** Windows IfIndex（u32）取值上界，用于解析 `nic:<idx>` 动作。 */
const U32_MAX = 4294967295;

// ============================================================================
// 输入校验（Req 3.5，Property 9）
// ============================================================================

/**
 * 模拟器输入校验结果：`ok` 为整体是否通过；`hostError` / `portError`
 * 分别标记 host / port 字段校验失败，供调用方定位失败字段。
 */
export interface SimInputValidation {
  ok: boolean;
  hostError?: boolean;
  portError?: boolean;
}

/**
 * 校验模拟器输入（Req 3.5，Property 9）。
 *
 * 通过当且仅当：host 为非空字符串，且 port 为整数且 ∈ [1, 65535]。
 * 任一条件违反时 `ok` 为 false，并把对应字段标记为 true。
 */
export function validateSimInput(host: string, port: number): SimInputValidation {
  const result: SimInputValidation = { ok: true };

  if (typeof host !== "string" || host.length === 0) {
    result.hostError = true;
    result.ok = false;
  }

  if (
    typeof port !== "number" ||
    !Number.isInteger(port) ||
    port < PORT_MIN ||
    port > PORT_MAX
  ) {
    result.portError = true;
    result.ok = false;
  }

  return result;
}

// ============================================================================
// 分流决策数据模型（前端等价 Route_Decision，与后端 RouteDecision 契约一致）
// ============================================================================

/** 命中的规则类别：进程规则 / 域名规则 / 无规则（回退调度策略）。 */
export type MatchedRuleKind = "process" | "domain" | "none";

/**
 * 命中的规则描述。
 * - `kind`：规则类别；
 * - `pattern`：命中的域名 pattern 或进程可执行文件名（`none` 时为空）。
 */
export interface MatchedRule {
  kind: MatchedRuleKind;
  pattern?: string;
}

/**
 * 前端等价的分流判定结果（对应后端 `RouteDecision`）。
 * - `bypassHit`：是否命中 bypass 直连白名单（命中则直连、不展示上游）；
 * - `matchedRule`：命中的规则（进程 / 域名 / 无规则回退）；
 * - `nicIfIndex`：承载网卡 IfIndex；命中 bypass 时为 null；
 * - `viaUpstream`：走上游时选中的上游 id；直连为 null。
 */
export interface RouteDecision {
  bypassHit: boolean;
  matchedRule: MatchedRule;
  nicIfIndex: number | null;
  viaUpstream: string | null;
}

/**
 * 模拟器当前配置（纯 TS 复算所需的路由上下文）。
 * 语义与后端 `engine::start` 的规则分派 + `decide_egress` 输入一致。
 */
export interface RouteSimConfig {
  /** 上游代理链总开关；关闭时恒直连（零回归语义）。 */
  upstreamChain: boolean;
  /** 显式 bypass 直连白名单（域名 pattern）。 */
  bypass: string[];
  /** 分流规则（域名 / 进程），沿用 App.tsx 的 RouteRule 结构。 */
  rules: RouteRule[];
  /** 网卡↔上游映射。 */
  bindings: UpstreamBinding[];
  /** 由调度策略预选的承载网卡 IfIndex（未命中 Nic 规则时的回退承载网卡）。 */
  chosenIfIndex: number;
  /** 调度序号（复用承载网卡既有连接计数），用于一网卡多上游轮转选择。 */
  schedIdx: number;
}

/** 模拟目标：域名 / 进程名 + 端口（可选进程名）。 */
export interface RouteSimTarget {
  host: string;
  port: number;
  procName?: string;
}

// ============================================================================
// 内部：规则动作解析与匹配（镜像后端 engine.rs 语义）
// ============================================================================

type RuleAction =
  | { type: "direct" }
  | { type: "aggregate" }
  | { type: "nic"; ifIndex: number };

/**
 * 解析规则动作字符串（镜像后端 `parse_rule_action`）：
 * `"direct"` / `"aggregate"` / `"nic:<ifindex>"`（trim + 小写；ifindex 需可解析为 u32）。
 * 非法或未知形式返回 null。
 */
function parseRuleAction(s: string): RuleAction | null {
  const act = s.trim().toLowerCase();
  if (act === "direct") {
    return { type: "direct" };
  }
  if (act === "aggregate") {
    return { type: "aggregate" };
  }
  if (act.startsWith("nic:")) {
    const idx = act.slice(4).trim();
    if (idx.length > 0 && /^[0-9]+$/.test(idx)) {
      const n = Number(idx);
      if (Number.isInteger(n) && n >= 0 && n <= U32_MAX) {
        return { type: "nic", ifIndex: n };
      }
    }
  }
  return null;
}

/**
 * 规则匹配（镜像后端 `pattern_match`）：pattern 可为 "域名" 或 "域名:port"。
 * 域名支持精确 / 子域 / `*` 通配。host 与 pattern 均应已小写。
 */
export function patternMatch(pattern: string, host: string, port: number): boolean {
  let patHost = pattern;
  let patPort: number | null = null;

  // 以最后一个 ':' 分割；仅当端口段全为数字且非空时视为端口。
  const idx = pattern.lastIndexOf(":");
  if (idx !== -1) {
    const p = pattern.slice(idx + 1);
    if (p.length > 0 && /^[0-9]+$/.test(p)) {
      // 端口段全为数字：主机段恒取冒号前部分；端口仅在可解析为 u16 时生效。
      patHost = pattern.slice(0, idx);
      const parsed = Number(p);
      patPort = parsed <= PORT_MAX ? parsed : null;
    }
  }

  if (patPort !== null) {
    // port==0 表示忽略端口约束（与后端 is_bypass 传入 0 的语义一致）。
    if (port !== 0 && patPort !== port) {
      return false;
    }
  }

  // trim_start_matches("*.") 后再 trim。
  let ph = patHost;
  while (ph.startsWith("*.")) {
    ph = ph.slice(2);
  }
  ph = ph.trim();

  if (ph === "*" || ph.length === 0) {
    return patPort !== null;
  }
  return host === ph || host.endsWith("." + ph);
}

/**
 * 进程规则匹配（镜像后端 `match_proc_rule`）：大小写不敏感精确匹配可执行文件名。
 * 返回首个命中规则的动作与 pattern；无命中返回 null。
 */
function matchProcRule(
  rulesProc: Array<{ pattern: string; action: RuleAction }>,
  procName: string,
): { action: RuleAction; pattern: string } | null {
  const name = procName.toLowerCase();
  for (const r of rulesProc) {
    if (r.pattern === name) {
      return { action: r.action, pattern: r.pattern };
    }
  }
  return null;
}

/**
 * 承载网卡多上游选择（镜像后端 `pick_upstream_for_nic`）：
 * 该网卡无绑定 / 绑定为空 => null；单个 => 该上游；多个 => `list[schedIdx % len]`。
 */
export function pickUpstreamForNic(
  bindings: UpstreamBinding[],
  ifIndex: number,
  schedIdx: number,
): string | null {
  const binding = bindings.find((b) => b.ifIndex === ifIndex);
  const list = binding ? binding.upstreamIds : [];
  if (list.length === 0) {
    return null;
  }
  if (list.length === 1) {
    return list[0];
  }
  const mod = ((schedIdx % list.length) + list.length) % list.length;
  return list[mod];
}

// ============================================================================
// 分流决策复算（镜像后端 compute_route_decision，Req 3.1/3.2/3.3/3.4/3.6）
// ============================================================================

/**
 * 依据当前配置以纯 TS 复算一条 Route_Decision（Req 3.1/3.2/3.3/3.4/3.6）。
 *
 * 优先级严格一致：bypass 最高 > 进程规则 > 域名规则 > 调度回退。
 * 走上游 vs 直连与后端 `decide_egress` 一致（总开关关 / 命中 bypass / 网卡无绑定 => 直连）。
 * 纯函数，不发起真实连接、不改引擎状态。
 */
export function computeRouteDecision(
  config: RouteSimConfig,
  target: RouteSimTarget,
): RouteDecision {
  const hostLower = target.host.toLowerCase();
  const procNameLower =
    typeof target.procName === "string" && target.procName.length > 0
      ? target.procName
      : null;

  // 规则分派（镜像 engine::start）：
  // - 显式 bypass + 域名规则 action=direct => bypass 白名单；
  // - 域名规则 action=nic:<idx>            => rulesNic；
  // - 进程规则（kind=process）             => rulesProc（parse_rule_action）。
  const bypass: string[] = [];
  for (const b of config.bypass) {
    const pat = b.trim().toLowerCase();
    if (pat.length > 0) {
      bypass.push(pat);
    }
  }

  const rulesNic: Array<{ pattern: string; ifIndex: number }> = [];
  const rulesProc: Array<{ pattern: string; action: RuleAction }> = [];
  for (const r of config.rules) {
    const pat = (r.pattern ?? "").trim().toLowerCase();
    if (pat.length === 0) {
      continue;
    }
    const kind = (r.kind ?? "domain").trim().toLowerCase();
    if (kind === "process") {
      const action = parseRuleAction(r.action ?? "");
      if (action) {
        rulesProc.push({ pattern: pat, action });
      }
      continue;
    }
    // 域名规则（默认）
    const act = (r.action ?? "").trim().toLowerCase();
    if (act === "direct") {
      bypass.push(pat);
    } else if (act.startsWith("nic:")) {
      const parsed = parseRuleAction(act);
      if (parsed && parsed.type === "nic") {
        rulesNic.push({ pattern: pat, ifIndex: parsed.ifIndex });
      }
    }
    // "aggregate" 即默认行为，无需处理
  }

  // 1) bypass 最高优先（镜像 is_bypass：仅按 host 匹配，端口忽略 => 传入 0）。
  const bypassHit = bypass.some((pat) => patternMatch(pat, hostLower, 0));
  if (bypassHit) {
    return {
      bypassHit: true,
      matchedRule: { kind: "none" },
      nicIfIndex: null,
      viaUpstream: null,
    };
  }

  // 2) 规则决策（镜像 decide_rule_action）：进程规则优先于域名规则。
  let matchedRule: MatchedRule = { kind: "none" };
  let carrierIfIndex = config.chosenIfIndex;

  let ruleAction: RuleAction | null = null;
  let ruleSource: "process" | "domain" | null = null;
  let rulePattern: string | undefined;

  if (procNameLower) {
    const hit = matchProcRule(rulesProc, procNameLower);
    if (hit) {
      ruleAction = hit.action;
      ruleSource = "process";
      rulePattern = hit.pattern;
    }
  }
  if (!ruleAction) {
    for (const r of rulesNic) {
      if (patternMatch(r.pattern, hostLower, target.port)) {
        ruleAction = { type: "nic", ifIndex: r.ifIndex };
        ruleSource = "domain";
        rulePattern = r.pattern;
        break;
      }
    }
  }

  // 仅 Nic 动作会钉死承载网卡（镜像 pick_nic：仅对 RuleAction::Nic 生效）；
  // 进程规则的 direct/aggregate 动作不改变承载网卡，回退调度预选网卡。
  if (ruleAction && ruleAction.type === "nic") {
    carrierIfIndex = ruleAction.ifIndex;
    matchedRule = { kind: ruleSource as MatchedRuleKind, pattern: rulePattern };
  } else {
    matchedRule = { kind: "none" };
  }

  // 3) 出口决策（镜像 decide_egress）：总开关关 / 网卡无绑定 => 直连；否则走上游。
  let viaUpstream: string | null = null;
  if (config.upstreamChain) {
    viaUpstream = pickUpstreamForNic(config.bindings, carrierIfIndex, config.schedIdx);
  }

  return {
    bypassHit: false,
    matchedRule,
    nicIfIndex: carrierIfIndex,
    viaUpstream,
  };
}

// ============================================================================
// 展示映射（Req 3.2/3.3/3.4，供 UI 结构化渲染，不硬编码中英文案）
// ============================================================================

/** 出口路径 i18n 键：直连聚合 / 走上游。 */
export type EgressKey = "routeSimEgressDirect" | "routeSimEgressUpstream";

/** 命中规则类别 i18n 键：bypass / 进程 / 域名 / 无规则回退调度。 */
export type RuleKey =
  | "routeSimRuleBypass"
  | "routeSimRuleProcess"
  | "routeSimRuleDomain"
  | "routeSimRuleFallback";

/**
 * Route_Decision 的结构化展示数据（i18n 键 + 原始数据），供 UI 渲染。
 * 文案不在此硬编码，`egressKey` / `ruleKey` 为 i18n 键，`rulePattern` /
 * `upstreamLabel` 为供展示的原始数据（用户输入 / 上游标签）。
 */
export interface RouteDecisionDisplay {
  bypassHit: boolean;
  egressKey: EgressKey;
  ruleKey: RuleKey;
  /** 命中的域名 pattern 或进程名；无命中规则时为 null。 */
  rulePattern: string | null;
  /** 承载网卡 IfIndex；命中 bypass 时为 null。 */
  nicIfIndex: number | null;
  /** 走上游时选中的上游标签（可用 upstreams 解析 id -> label）；直连为 null。 */
  upstreamLabel: string | null;
}

/**
 * 把一条 Route_Decision 转为用于 UI 展示的结构化数据 / i18n 键（Req 3.2/3.3/3.4）。
 *
 * @param decision  待展示的判定结果。
 * @param upstreams 可选：上游列表，用于把 `viaUpstream`（id）解析为可读标签。
 * @returns 结构化展示数据；不含任何硬编码中英文案。
 */
export function formatRouteDecision(
  decision: RouteDecision,
  upstreams?: UpstreamProxy[],
): RouteDecisionDisplay {
  const bypassHit = decision.bypassHit;

  let ruleKey: RuleKey;
  if (bypassHit) {
    ruleKey = "routeSimRuleBypass";
  } else if (decision.matchedRule.kind === "process") {
    ruleKey = "routeSimRuleProcess";
  } else if (decision.matchedRule.kind === "domain") {
    ruleKey = "routeSimRuleDomain";
  } else {
    ruleKey = "routeSimRuleFallback";
  }

  const egressKey: EgressKey = decision.viaUpstream
    ? "routeSimEgressUpstream"
    : "routeSimEgressDirect";

  let upstreamLabel: string | null = null;
  if (decision.viaUpstream) {
    const found = upstreams?.find((u) => u.id === decision.viaUpstream);
    if (found) {
      upstreamLabel =
        typeof found.label === "string" && found.label.length > 0
          ? found.label
          : found.host;
    } else {
      upstreamLabel = decision.viaUpstream;
    }
  }

  return {
    bypassHit,
    egressKey,
    ruleKey,
    rulePattern: bypassHit ? null : decision.matchedRule.pattern ?? null,
    nicIfIndex: decision.nicIfIndex,
    upstreamLabel,
  };
}
