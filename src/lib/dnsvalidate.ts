// 每网卡 DNS / DoH 端点校验前端纯逻辑模块（Feature: pro-differentiation-and-hardening）。
//
// 本模块仅包含不依赖 DOM / Tauri `invoke` 的纯函数，可被 vitest 直接导入做单元 / 属性测试。
// 语义与后端 `validate_dns_endpoint` 严格一致（见 design.md 第 3 节 / Property 6）：
//   - plain 通过当且仅当 endpoint 是合法 IPv4 或 IPv6 地址；
//   - doh   通过当且仅当 endpoint 是 `https://` 开头且主机段非空的 URL；
//   - 其余一律不通过。
//
// 为保证浏览器 / Node 环境下的确定性，IPv4 / IPv6 与 DoH 主机段均采用手写解析，
// 不依赖内置 URL 解析器（不同运行时对畸形输入的容错存在差异）。

/** DNS 端点类型：plain（明文 DNS 服务器 IP）| doh（DoH 端点 URL）。 */
export type DnsKind = "plain" | "doh";

/**
 * 校验单张网卡的 DNS 端点是否合法（Req 7.5，Property 6）。
 *
 * @param kind     端点类型：`plain` 或 `doh`。
 * @param endpoint 待校验的端点字符串。
 * @returns 合法返回 true，否则 false。
 */
export function validateDnsEndpoint(kind: DnsKind, endpoint: string): boolean {
  if (typeof endpoint !== "string") {
    return false;
  }
  if (kind === "plain") {
    return isIpv4(endpoint) || isIpv6(endpoint);
  }
  if (kind === "doh") {
    return isHttpsUrlWithHost(endpoint);
  }
  return false;
}

/**
 * 判定字符串是否为合法 IPv4 地址（点分四段，每段 0-255，无前导零歧义）。
 *
 * 采用严格解析：恰好四段、每段为 1..=3 位十进制数字、数值 ∈ [0, 255]，
 * 且不接受形如 "01" 的多位前导零（与标准库 `Ipv4Addr::from_str` 行为一致）。
 */
export function isIpv4(input: string): boolean {
  const parts = input.split(".");
  if (parts.length !== 4) {
    return false;
  }
  for (const part of parts) {
    if (!isValidIpv4Octet(part)) {
      return false;
    }
  }
  return true;
}

function isValidIpv4Octet(part: string): boolean {
  // 长度 1..=3，且全为 ASCII 数字。
  if (part.length < 1 || part.length > 3) {
    return false;
  }
  for (let i = 0; i < part.length; i++) {
    const c = part.charCodeAt(i);
    if (c < 0x30 || c > 0x39) {
      return false;
    }
  }
  // 拒绝多位前导零（"0" 合法，"00" / "01" 非法）。
  if (part.length > 1 && part[0] === "0") {
    return false;
  }
  const value = Number(part);
  return value >= 0 && value <= 255;
}

/**
 * 判定字符串是否为合法 IPv6 地址。
 *
 * 支持：完整八段形式、`::` 压缩形式（至多出现一次）、以及末段内嵌 IPv4
 * （如 `::ffff:192.168.0.1`）。每个 16 位段为 1..=4 位十六进制。
 */
export function isIpv6(input: string): boolean {
  // 不允许包含区域标识（zone id，如 "%eth0"）——DNS 服务器地址无需之。
  if (input.length === 0 || input.indexOf("%") !== -1) {
    return false;
  }

  // 处理压缩符 "::"：至多出现一次。
  const doubleColonIndex = input.indexOf("::");
  if (doubleColonIndex !== -1 && input.indexOf("::", doubleColonIndex + 1) !== -1) {
    return false;
  }

  let head: string;
  let tail: string;
  if (doubleColonIndex !== -1) {
    head = input.slice(0, doubleColonIndex);
    tail = input.slice(doubleColonIndex + 2);
  } else {
    head = input;
    tail = "";
  }

  const headGroups = head.length > 0 ? head.split(":") : [];
  const tailGroups = tail.length > 0 ? tail.split(":") : [];

  // 存在空分段（非压缩位置的连续冒号 / 首尾单冒号）视为非法。
  for (const g of headGroups) {
    if (g.length === 0) {
      return false;
    }
  }
  for (const g of tailGroups) {
    if (g.length === 0) {
      return false;
    }
  }

  // 组合后的分段序列，统计 16 位段与末段内嵌 IPv4。
  const allGroups = headGroups.concat(tailGroups);
  let sixteenBitGroups = allGroups.length;
  let embeddedIpv4 = false;

  if (allGroups.length > 0) {
    const last = allGroups[allGroups.length - 1];
    if (last.indexOf(".") !== -1) {
      // 末段为内嵌 IPv4，占用 2 个 16 位段。
      if (!isIpv4(last)) {
        return false;
      }
      embeddedIpv4 = true;
      sixteenBitGroups = allGroups.length - 1;
    }
  }

  // 校验其余各段为合法 16 位十六进制段。
  const hexCount = embeddedIpv4 ? allGroups.length - 1 : allGroups.length;
  for (let i = 0; i < hexCount; i++) {
    if (!isValidIpv6Hextet(allGroups[i])) {
      return false;
    }
  }

  // 内嵌 IPv4 折算为 2 段，用于计数。
  const totalUnits = sixteenBitGroups + (embeddedIpv4 ? 2 : 0);

  if (doubleColonIndex !== -1) {
    // 压缩形式：`::` 至少代表 1 个省略段，故已写出的单元数必须严格小于 8。
    return totalUnits < 8;
  }
  // 完整形式：恰好 8 个 16 位单元。
  return totalUnits === 8;
}

function isValidIpv6Hextet(group: string): boolean {
  if (group.length < 1 || group.length > 4) {
    return false;
  }
  for (let i = 0; i < group.length; i++) {
    const c = group.charCodeAt(i);
    const isDigit = c >= 0x30 && c <= 0x39;
    const isLower = c >= 0x61 && c <= 0x66;
    const isUpper = c >= 0x41 && c <= 0x46;
    if (!isDigit && !isLower && !isUpper) {
      return false;
    }
  }
  return true;
}

/**
 * 判定字符串是否为 `https://` 开头且主机段非空的 URL（DoH 端点）。
 *
 * 手写解析而非依赖内置 URL 解析器：去掉 `https://` 前缀后，主机段为
 * 路径 `/`、查询 `?`、片段 `#` 之前的部分；若主机段还带用户信息 `@`，
 * 取其后的权威主机部分。要求主机段去除端口后非空。
 */
export function isHttpsUrlWithHost(input: string): boolean {
  const prefix = "https://";
  if (input.length <= prefix.length) {
    return false;
  }
  if (input.slice(0, prefix.length).toLowerCase() !== prefix) {
    return false;
  }
  let rest = input.slice(prefix.length);

  // 截断到主机段结束：路径 / 查询 / 片段之前。
  const stopChars = ["/", "?", "#"];
  let end = rest.length;
  for (const ch of stopChars) {
    const idx = rest.indexOf(ch);
    if (idx !== -1 && idx < end) {
      end = idx;
    }
  }
  let authority = rest.slice(0, end);

  // 去掉用户信息部分（user:pass@host）。
  const atIndex = authority.lastIndexOf("@");
  if (atIndex !== -1) {
    authority = authority.slice(atIndex + 1);
  }

  // 去掉端口部分（host:port），但需兼容 IPv6 字面量 [::1]:443。
  let host: string;
  if (authority.startsWith("[")) {
    const closeIndex = authority.indexOf("]");
    if (closeIndex === -1) {
      return false;
    }
    host = authority.slice(1, closeIndex);
  } else {
    const colonIndex = authority.indexOf(":");
    host = colonIndex === -1 ? authority : authority.slice(0, colonIndex);
  }

  return host.length > 0;
}
