// 订阅式上游导入解析前端镜像（Feature: pro-differentiation-and-hardening，Req 4）。
//
// 本模块把一段 Import_Source（Clash 订阅 YAML / base64 编码正文 /
// 节点分享链接集合）解析为一组 `UpstreamProxy` 候选。全部为**不依赖 DOM / invoke 的纯函数**，
// 语义与后端 `src-tauri/src/subscription.rs` 完全一致，可被 vitest 直接导入做单元 / 属性测试。
//
// 设计约束（见 design.md 与 subscription.rs）：
// - 仅识别并保留类型为 `socks5` / `http` 的节点（含 socks/socks5h => socks5，https => http）。
// - 非 socks5 / http 协议（ss / vmess / trojan / hysteria 等）或畸形节点被跳过、不写入候选，
//   并计入 `ignoredUnsupported`。
// - 对任意输入绝不抛异常；无受支持节点返回空候选。
// - 采用最小内联解析（Clash proxies 段行式解析 + 分享链接解析 + base64 预解码），不引入重依赖。
// - 生成候选时用与后端一致的稳定确定性 id（FNV-1a 64 位内容哈希）。

import type { UpstreamKind, UpstreamProxy } from "./upstream";

/**
 * 一次订阅导入的解析结果。
 *
 * `candidates` 仅含类型为 `socks5` / `http` 的受支持上游候选；
 * `ignoredUnsupported` 统计被识别为「不支持」或「畸形」而跳过的节点数量，供 UI 提示。
 */
export interface ImportResult {
  /** 受支持的上游候选（仅 socks5 / http）。 */
  candidates: UpstreamProxy[];
  /** 被忽略（不支持协议或畸形）的节点计数。 */
  ignoredUnsupported: number;
}

function emptyResult(): ImportResult {
  return { candidates: [], ignoredUnsupported: 0 };
}

/**
 * 顶层解析：自动识别 Clash YAML / base64 正文 / 分享链接集合（Req 4.1/4.2/4.5）。
 *
 * 识别顺序：
 * 1. 若正文含 `proxies:` 段 => 按 Clash 订阅解析；
 * 2. 否则尝试 base64 解码，成功后对解码正文再按 Clash / 分享链接解析（Req 4.2）；
 * 3. 否则按明文分享链接集合逐行解析。
 *
 * 对任意输入均不抛异常；解析不出受支持节点时返回空候选。
 */
export function parseSubscription(input: string): ImportResult {
  const trimmed = typeof input === "string" ? input.trim() : "";
  if (trimmed.length === 0) {
    return emptyResult();
  }

  // 1) 明文即为 Clash YAML。
  if (looksLikeClash(trimmed)) {
    return parseClashProxies(trimmed);
  }

  // 2) base64 正文预解码后再解析（Req 4.2）。
  const decoded = tryBase64Decode(trimmed);
  if (decoded !== null) {
    if (looksLikeClash(decoded)) {
      return parseClashProxies(decoded);
    }
    return parseShareLinksBlock(decoded);
  }

  // 3) 明文分享链接集合。
  return parseShareLinksBlock(trimmed);
}

/** 是否疑似 Clash 订阅（存在以 `proxies:` 开头的行）。 */
function looksLikeClash(text: string): boolean {
  return text.split("\n").some((l) => l.replace(/^\s+/, "").startsWith("proxies:"));
}

/** 逐行解析分享链接集合：受支持链接入候选，其余非空行计入忽略计数。 */
function parseShareLinksBlock(text: string): ImportResult {
  const candidates: UpstreamProxy[] = [];
  let ignored = 0;
  for (const raw of text.split("\n")) {
    const line = raw.trim();
    if (line.length === 0) {
      continue;
    }
    const proxy = parseShareLink(line);
    if (proxy) {
      candidates.push(proxy);
    } else {
      ignored += 1;
    }
  }
  return { candidates, ignoredUnsupported: ignored };
}

/**
 * base64 正文预解码（Req 4.2）。
 *
 * 兼容标准（`+/`）与 URL-safe（`-_`）字母表，忽略空白与 `=` 补位，容忍缺省补位。
 * 解码失败、含非法字符、结果非合法 UTF-8 或结果为空时返回 `null`（交由后续按明文尝试）。
 */
export function tryBase64Decode(input: string): string | null {
  let val = 0;
  let bits = 0;
  const out: number[] = [];
  let seen = false;

  for (let i = 0; i < input.length; i += 1) {
    const c = input.charCodeAt(i);
    // 空白：跳过（\t \n \v \f \r 空格）。
    if (c === 0x20 || (c >= 0x09 && c <= 0x0d)) {
      continue;
    }
    // 补位符 `=`：忽略，不参与解码。
    if (c === 0x3d) {
      continue;
    }
    seen = true;
    let d: number;
    if (c >= 0x41 && c <= 0x5a) {
      d = c - 0x41; // A-Z => 0..25
    } else if (c >= 0x61 && c <= 0x7a) {
      d = c - 0x61 + 26; // a-z => 26..51
    } else if (c >= 0x30 && c <= 0x39) {
      d = c - 0x30 + 52; // 0-9 => 52..61
    } else if (c === 0x2b || c === 0x2d) {
      d = 62; // + 或 -
    } else if (c === 0x2f || c === 0x5f) {
      d = 63; // / 或 _
    } else {
      return null; // 非法 base64 字符 => 判定为非 base64 正文
    }
    val = (val << 6) | d;
    bits += 6;
    if (bits >= 8) {
      bits -= 8;
      out.push((val >> bits) & 0xff);
    }
  }

  if (!seen || out.length === 0) {
    return null;
  }
  return decodeUtf8(out);
}

/**
 * 将字节数组按 UTF-8 严格解码；遇到非法序列返回 `null`（对齐后端 `String::from_utf8`）。
 */
function decodeUtf8(bytes: number[]): string | null {
  let result = "";
  let i = 0;
  const n = bytes.length;
  while (i < n) {
    const b0 = bytes[i];
    if (b0 < 0x80) {
      result += String.fromCharCode(b0);
      i += 1;
    } else if (b0 >= 0xc2 && b0 <= 0xdf) {
      if (i + 1 >= n) return null;
      const b1 = bytes[i + 1];
      if ((b1 & 0xc0) !== 0x80) return null;
      const cp = ((b0 & 0x1f) << 6) | (b1 & 0x3f);
      result += String.fromCharCode(cp);
      i += 2;
    } else if (b0 >= 0xe0 && b0 <= 0xef) {
      if (i + 2 >= n) return null;
      const b1 = bytes[i + 1];
      const b2 = bytes[i + 2];
      if ((b1 & 0xc0) !== 0x80 || (b2 & 0xc0) !== 0x80) return null;
      const cp = ((b0 & 0x0f) << 12) | ((b1 & 0x3f) << 6) | (b2 & 0x3f);
      // 拒绝过长编码与代理区间。
      if (cp < 0x800 || (cp >= 0xd800 && cp <= 0xdfff)) return null;
      result += String.fromCharCode(cp);
      i += 3;
    } else if (b0 >= 0xf0 && b0 <= 0xf4) {
      if (i + 3 >= n) return null;
      const b1 = bytes[i + 1];
      const b2 = bytes[i + 2];
      const b3 = bytes[i + 3];
      if ((b1 & 0xc0) !== 0x80 || (b2 & 0xc0) !== 0x80 || (b3 & 0xc0) !== 0x80) return null;
      const cp =
        ((b0 & 0x07) << 18) | ((b1 & 0x3f) << 12) | ((b2 & 0x3f) << 6) | (b3 & 0x3f);
      if (cp < 0x10000 || cp > 0x10ffff) return null;
      // 转为代理对。
      const off = cp - 0x10000;
      result += String.fromCharCode(0xd800 + (off >> 10), 0xdc00 + (off & 0x3ff));
      i += 4;
    } else {
      return null; // 非法起始字节（含 0x80..0xc1、0xf5..0xff）
    }
  }
  return result;
}

/**
 * Clash 订阅 `proxies:` 段最小提取（Req 4.3）。
 *
 * 仅取 `type ∈ {socks5, http}` 的 `name` / `server` / `port` / `username` / `password`
 * 映射为候选；其他类型或畸形节点计入 `ignoredUnsupported`。
 */
export function parseClashProxies(yaml: string): ImportResult {
  const candidates: UpstreamProxy[] = [];
  let ignored = 0;
  for (const item of extractProxyItems(yaml)) {
    const proxy = proxyFromMap(item);
    if (proxy) {
      candidates.push(proxy);
    } else {
      ignored += 1;
    }
  }
  return { candidates, ignoredUnsupported: ignored };
}

type KvList = Array<[string, string]>;

/**
 * 从 Clash YAML 的 `proxies:` 段抽取每个节点的键值对列表。
 *
 * 支持块状（多行缩进 `key: value`）与流式（`- {name: x, type: socks5, ...}`）两种写法。
 */
function extractProxyItems(yaml: string): KvList[] {
  const items: KvList[] = [];
  let inProxies = false;
  let proxiesIndent = 0;
  let current: KvList | null = null;

  for (const raw of yaml.split("\n")) {
    const trimmed = raw.trim();
    const indent = raw.length - raw.replace(/^\s+/, "").length;
    if (trimmed.length === 0 || trimmed.startsWith("#")) {
      continue;
    }

    if (!inProxies) {
      if (trimmed === "proxies:") {
        inProxies = true;
        proxiesIndent = indent;
      }
      continue;
    }

    // 已进入 proxies 段：遇到同级或更浅的普通键 => 退出该段。
    if (indent <= proxiesIndent && !trimmed.startsWith("-")) {
      if (current) {
        items.push(current);
        current = null;
      }
      inProxies = false;
      continue;
    }

    if (trimmed.startsWith("-")) {
      // 新列表项：先落盘上一项。
      if (current) {
        items.push(current);
        current = null;
      }
      const rest = trimmed.slice(1).trim();
      if (rest.startsWith("{")) {
        // 流式：单行完整节点。
        const map: KvList = [];
        parseFlowMap(rest, map);
        items.push(map);
        current = null;
      } else {
        // 块状：`- key: value` 的首键。
        const map: KvList = [];
        const kv = parseKv(rest);
        if (kv) {
          map.push(kv);
        }
        current = map;
      }
    } else if (current) {
      // 块状项的后续键。
      const kv = parseKv(trimmed);
      if (kv) {
        current.push(kv);
      }
    }
  }

  if (current) {
    items.push(current);
  }
  return items;
}

/** 解析流式内联映射 `{k1: v1, k2: v2, ...}` 为键值对。 */
function parseFlowMap(s: string, out: KvList): void {
  let inner = s.trim();
  if (inner.startsWith("{")) {
    inner = inner.slice(1);
  }
  if (inner.endsWith("}")) {
    inner = inner.slice(0, -1);
  }
  inner = inner.trim();
  for (const part of inner.split(",")) {
    const kv = parseKv(part.trim());
    if (kv) {
      out.push(kv);
    }
  }
}

/** 解析单个 `key: value`，键小写化、值去引号去空白。 */
function parseKv(s: string): [string, string] | null {
  const idx = s.indexOf(":");
  if (idx < 0) {
    return null;
  }
  const key = s.slice(0, idx).trim().toLowerCase();
  if (key.length === 0) {
    return null;
  }
  const value = unquote(s.slice(idx + 1).trim());
  return [key, value];
}

/** 去除首尾成对的单/双引号并去空白。 */
function unquote(s: string): string {
  const t = s.trim();
  if (t.length >= 2) {
    const first = t[0];
    const last = t[t.length - 1];
    if ((first === '"' && last === '"') || (first === "'" && last === "'")) {
      return t.slice(1, -1);
    }
  }
  return t;
}

/** 在键值对列表中取值（键已小写）。 */
function mapGet(map: KvList, key: string): string | undefined {
  for (const [k, v] of map) {
    if (k === key) {
      return v;
    }
  }
  return undefined;
}

/** 将一个 Clash 节点的键值对映射为受支持候选；不支持/畸形返回 `null`。 */
function proxyFromMap(map: KvList): UpstreamProxy | null {
  const rawType = mapGet(map, "type");
  if (rawType === undefined) {
    return null;
  }
  const kind = normalizeKind(rawType);
  if (kind === null) {
    return null;
  }
  const host = (mapGet(map, "server") ?? "").trim();
  if (host.length === 0) {
    return null;
  }
  const port = parsePort(mapGet(map, "port") ?? "");
  if (port === null) {
    return null;
  }
  const username = opt(mapGet(map, "username") ?? "");
  const password = opt(mapGet(map, "password") ?? "");
  const label = mapGet(map, "name") ?? "";
  return buildProxy(kind, host, port, username, password, label);
}

/**
 * 单条分享链接解析（Req 4.4）。
 *
 * `socks5://[user:pass@]host:port#name`、`http(s)://[user:pass@]host:port#name`
 * 映射为候选；`ss` / `vmess` / `trojan` / `hysteria` 等其他协议或畸形链接返回 `null`。
 */
export function parseShareLink(line: string): UpstreamProxy | null {
  const trimmedLine = line.trim();
  const schemeIdx = trimmedLine.indexOf("://");
  if (schemeIdx < 0) {
    return null;
  }
  const scheme = trimmedLine.slice(0, schemeIdx);
  const rest = trimmedLine.slice(schemeIdx + 3);
  const kind = normalizeKind(scheme);
  if (kind === null) {
    return null;
  }

  // 去除锚点 `#name` 作为 label。
  const hashIdx = rest.indexOf("#");
  let main = hashIdx >= 0 ? rest.slice(0, hashIdx) : rest;
  const frag = hashIdx >= 0 ? rest.slice(hashIdx + 1) : null;

  // 去除 host:port 之后的路径 / 查询串。
  const pathIdx = firstIndexOfAny(main, ["/", "?"]);
  if (pathIdx >= 0) {
    main = main.slice(0, pathIdx);
  }

  // 认证段（最后一个 `@` 之前）。
  const atIdx = main.lastIndexOf("@");
  const auth = atIdx >= 0 ? main.slice(0, atIdx) : null;
  const hostport = atIdx >= 0 ? main.slice(atIdx + 1) : main;

  // host:port（从右侧切分，兼容 IPv6 的 `[::1]:port`）。
  const colonIdx = hostport.lastIndexOf(":");
  if (colonIdx < 0) {
    return null;
  }
  const hostRaw = hostport.slice(0, colonIdx);
  const portStr = hostport.slice(colonIdx + 1);
  const port = parsePort(portStr);
  if (port === null) {
    return null;
  }
  const host = stripIpv6Brackets(hostRaw);
  if (host.length === 0) {
    return null;
  }

  let username: string | undefined;
  let password: string | undefined;
  if (auth !== null) {
    const acIdx = auth.indexOf(":");
    if (acIdx >= 0) {
      username = opt(auth.slice(0, acIdx));
      password = opt(auth.slice(acIdx + 1));
    } else {
      username = opt(auth);
      password = undefined;
    }
  }

  const label = frag ?? "";
  return buildProxy(kind, host, port, username, password, label);
}

/** 返回 `chars` 中任一字符在 `s` 里的最小索引，均不存在时返回 -1。 */
function firstIndexOfAny(s: string, chars: string[]): number {
  let min = -1;
  for (const ch of chars) {
    const idx = s.indexOf(ch);
    if (idx >= 0 && (min < 0 || idx < min)) {
      min = idx;
    }
  }
  return min;
}

/** 归一化上游类型：仅接受 socks5 / http 家族，其余（含空）返回 `null`。 */
function normalizeKind(raw: string): UpstreamKind | null {
  switch (raw.trim().toLowerCase()) {
    case "socks5":
    case "socks":
    case "socks5h":
      return "socks5";
    case "http":
    case "https":
      return "http";
    default:
      return null;
  }
}

/** 解析端口：必须为 1..=65535 的十进制整数。 */
function parsePort(s: string): number | null {
  const t = s.trim();
  // 对齐 Rust `u16::parse`：仅接受纯十进制数字串（可含前导 0）。
  if (t.length === 0 || !/^[0-9]+$/.test(t)) {
    return null;
  }
  const p = Number.parseInt(t, 10);
  if (!Number.isInteger(p) || p < 1 || p > 65535) {
    return null;
  }
  return p;
}

/** 去除 IPv6 字面量的方括号。 */
function stripIpv6Brackets(host: string): string {
  const h = host.trim();
  if (h.length >= 2 && h.startsWith("[") && h.endsWith("]")) {
    return h.slice(1, -1);
  }
  return h;
}

/** 空字符串归一为 `undefined`，否则原值。 */
function opt(s: string): string | undefined {
  return s.length === 0 ? undefined : s;
}

/** 构造候选，生成稳定唯一 id（内容哈希，纯确定性）。 */
function buildProxy(
  kind: UpstreamKind,
  host: string,
  port: number,
  username: string | undefined,
  password: string | undefined,
  label: string,
): UpstreamProxy {
  const id = stableId(kind, host, port, username, password, label);
  const proxy: UpstreamProxy = { id, kind, host, port, label };
  if (username !== undefined) {
    proxy.username = username;
  }
  if (password !== undefined) {
    proxy.password = password;
  }
  return proxy;
}

// FNV-1a 64 位常量（与后端一致），以 BigInt 表达并按 64 位截断。
const FNV_OFFSET = 0xcbf29ce484222325n;
const FNV_PRIME = 0x00000100000001b3n;
const MASK64 = 0xffffffffffffffffn;

/**
 * 基于节点内容生成稳定唯一 id（FNV-1a 64 位哈希，纯函数、可复现）。
 *
 * 逐字节混入并在每个字段后追加分隔符 `0x1f`，字段顺序与字节化方式与后端 `stable_id` 完全一致：
 * kind / host / port(小端 u16) / username / password / label。
 */
function stableId(
  kind: string,
  host: string,
  port: number,
  username: string | undefined,
  password: string | undefined,
  label: string,
): string {
  let hash = FNV_OFFSET;

  const mix = (bytes: number[]): void => {
    for (const b of bytes) {
      hash ^= BigInt(b);
      hash = (hash * FNV_PRIME) & MASK64;
    }
    // 字段分隔符，避免不同拼接产生同一序列。
    hash ^= 0x1fn;
    hash = (hash * FNV_PRIME) & MASK64;
  };

  mix(utf8Bytes(kind));
  mix(utf8Bytes(host));
  mix([port & 0xff, (port >> 8) & 0xff]); // u16 小端
  mix(utf8Bytes(username ?? ""));
  mix(utf8Bytes(password ?? ""));
  mix(utf8Bytes(label));

  return `sub-${hash.toString(16).padStart(16, "0")}`;
}

/** 将字符串编码为 UTF-8 字节序列（与 Rust `str::as_bytes` 一致）。 */
function utf8Bytes(s: string): number[] {
  const out: number[] = [];
  for (let i = 0; i < s.length; i += 1) {
    let cp = s.charCodeAt(i);
    // 处理代理对，还原为完整码点。
    if (cp >= 0xd800 && cp <= 0xdbff && i + 1 < s.length) {
      const lo = s.charCodeAt(i + 1);
      if (lo >= 0xdc00 && lo <= 0xdfff) {
        cp = 0x10000 + ((cp - 0xd800) << 10) + (lo - 0xdc00);
        i += 1;
      }
    }
    if (cp < 0x80) {
      out.push(cp);
    } else if (cp < 0x800) {
      out.push(0xc0 | (cp >> 6), 0x80 | (cp & 0x3f));
    } else if (cp < 0x10000) {
      out.push(0xe0 | (cp >> 12), 0x80 | ((cp >> 6) & 0x3f), 0x80 | (cp & 0x3f));
    } else {
      out.push(
        0xf0 | (cp >> 18),
        0x80 | ((cp >> 12) & 0x3f),
        0x80 | ((cp >> 6) & 0x3f),
        0x80 | (cp & 0x3f),
      );
    }
  }
  return out;
}
