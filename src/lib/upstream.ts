// 上游代理链前端纯逻辑模块（Feature: nic-upstream-proxy-chain）。
//
// 本模块仅包含不依赖 DOM / Tauri `invoke` 的纯函数与类型定义，
// 可被 vitest 直接导入做单元 / 属性测试。
// 类型与后端 serde camelCase 契约保持一致（见 design.md 前后端类型契约表）。

/** 上游代理条目类型。 */
export type UpstreamKind = "socks5" | "http";

/**
 * 一条上游代理节点条目。
 * 与后端 `UpstreamProxy`（serde camelCase）契约一致。
 */
export interface UpstreamProxy {
  /** 同组唯一、稳定、不复用的上游标识（前端以 crypto.randomUUID() 生成）。 */
  id: string;
  /** 上游类型：socks5 / http。 */
  kind: UpstreamKind;
  /** 上游主机地址（域名或 IP），长度 ≤ 253。 */
  host: string;
  /** 上游端口，取值 1..=65535。 */
  port: number;
  /** 可选认证用户名（配置认证时长度 1..=255）。 */
  username?: string;
  /** 可选认证密码（配置认证时长度 1..=255）。 */
  password?: string;
  /** 备注名，长度 ≤ 64，供日志 / UI 展示。 */
  label: string;
}

/**
 * 一条网卡↔上游映射。
 * 与后端 `UpstreamBinding`（serde camelCase：ifIndex / upstreamIds）契约一致。
 */
export interface UpstreamBinding {
  /** 网卡权威标识 IfIndex。 */
  ifIndex: number;
  /** 该网卡绑定的上游 id 列表（引用 UpstreamProxy.id）。 */
  upstreamIds: string[];
}

/** 校验失败字段标记：某字段为 true 表示该字段校验未通过。 */
export interface UpstreamValidationFields {
  host?: boolean;
  port?: boolean;
  kind?: boolean;
  username?: boolean;
  password?: boolean;
}

/** 上游条目校验结果：ok 为整体是否通过；fields 标记各失败字段。 */
export interface UpstreamValidationResult {
  ok: boolean;
  fields: UpstreamValidationFields;
}

/** validateUpstream 的输入：id / label 不参与校验，故与展示字段解耦。 */
export interface UpstreamValidationInput {
  kind?: string;
  host?: string;
  port?: number;
  username?: string;
  password?: string;
}

/** 主机名最大长度（域名整体上限）。 */
export const HOST_MAX_LEN = 253;
/** 端口取值下界。 */
export const PORT_MIN = 1;
/** 端口取值上界。 */
export const PORT_MAX = 65535;
/** 认证凭据长度下界。 */
export const CRED_MIN_LEN = 1;
/** 认证凭据长度上界。 */
export const CRED_MAX_LEN = 255;
/** 上游条目数量上限。 */
export const UPSTREAM_MAX_COUNT = 128;

const VALID_KINDS: ReadonlySet<string> = new Set<string>(["socks5", "http"]);

/**
 * 校验一条上游代理条目（Req 1.2 / 1.5 / 1.6）。
 *
 * 通过当且仅当：
 * - host 非空且长度 ≤ 253；
 * - port 为整数且 ∈ [1, 65535]；
 * - kind ∈ {socks5, http}；
 * - 若配置了认证（username 或 password 任一非空），则用户名与密码长度均 ∈ [1, 255]。
 *
 * 任一条件违反时，`ok` 为 false，并在 `fields` 中把对应字段标记为 true。
 */
export function validateUpstream(input: UpstreamValidationInput): UpstreamValidationResult {
  const fields: UpstreamValidationFields = {};

  // host：非空且长度 ≤ 253
  const host = input.host;
  if (typeof host !== "string" || host.length < 1 || host.length > HOST_MAX_LEN) {
    fields.host = true;
  }

  // port：整数且 ∈ [1, 65535]
  const port = input.port;
  if (
    typeof port !== "number" ||
    !Number.isInteger(port) ||
    port < PORT_MIN ||
    port > PORT_MAX
  ) {
    fields.port = true;
  }

  // kind：∈ {socks5, http}
  if (typeof input.kind !== "string" || !VALID_KINDS.has(input.kind)) {
    fields.kind = true;
  }

  // 认证：当配置了认证时，用户名与密码长度均须 ∈ [1, 255]
  const username = input.username;
  const password = input.password;
  const hasUsername = typeof username === "string" && username.length > 0;
  const hasPassword = typeof password === "string" && password.length > 0;
  const authConfigured = hasUsername || hasPassword;
  if (authConfigured) {
    if (
      typeof username !== "string" ||
      username.length < CRED_MIN_LEN ||
      username.length > CRED_MAX_LEN
    ) {
      fields.username = true;
    }
    if (
      typeof password !== "string" ||
      password.length < CRED_MIN_LEN ||
      password.length > CRED_MAX_LEN
    ) {
      fields.password = true;
    }
  }

  return { ok: Object.keys(fields).length === 0, fields };
}

/**
 * 删除一条上游条目时，从所有网卡映射中清理对该条目的引用（Req 2.6）。
 *
 * 纯函数：返回一份新的映射列表，其中每条 UpstreamBinding.upstreamIds 已移除给定 id。
 * 保留网卡条目本身（清理后 upstreamIds 为空的网卡等价于未绑定上游），
 * 且返回结果中不再包含对该 id 的任何悬空引用。
 */
export function removeUpstreamRef(
  bindings: UpstreamBinding[],
  id: string,
): UpstreamBinding[] {
  return bindings.map((binding) => ({
    ifIndex: binding.ifIndex,
    upstreamIds: binding.upstreamIds.filter((uid) => uid !== id),
  }));
}
