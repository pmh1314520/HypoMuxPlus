// Feature: pro-differentiation-and-hardening, Property 4（前端镜像）
//
// 前端订阅解析属性测试：验证 `parseSubscription` / `parseShareLink` 的健壮性与
// 受支持节点 round-trip，以及非受支持节点计入 ignoredUnsupported 计数。
// 语义与后端 `subscription.rs` 镜像一致（仅保留 socks5 / http 节点）。
//
// 覆盖（对齐 Task 3.4）：
//   1. 健壮性：任意字符串输入下 parseSubscription 不抛异常；纯非受支持内容时 candidates 为空。
//   2. round-trip：为 socks5:// / http(s):// / socks(5h):// 构造合法分享链接
//      （含可选 user:pass@host:port#name），parse 后还原等价字段。
//   3. 非受支持类型（ss / vmess / trojan / hysteria 等）计入 ignoredUnsupported。
//
// Validates: Requirements 4.1, 4.4, 4.8
import { describe, it, expect } from "vitest";
import fc from "fast-check";
import { parseSubscription, parseShareLink } from "./subscription";
import type { UpstreamKind } from "./upstream";

// ---- 智能生成器：约束到「可安全 round-trip」的输入分区 ----

// host 允许字符：字母数字与 . -（不含会破坏解析的 : @ # / ? 与空白）。
const HOST_CHARS = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789.-".split("");
// 认证 / label 允许字符：字母数字与 . _ -（同样排除分隔符与空白）。
const TOKEN_CHARS = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789._-".split("");

function segArb(chars: string[], min = 1, max = 20): fc.Arbitrary<string> {
  return fc
    .array(fc.constantFrom(...chars), { minLength: min, maxLength: max })
    .map((a) => a.join(""));
}

const hostArb = segArb(HOST_CHARS, 1, 30);
const portArb = fc.integer({ min: 1, max: 65535 });
const tokenArb = segArb(TOKEN_CHARS, 1, 20);

// 受支持 scheme 及其归一化后的 kind。
const SUPPORTED_SCHEMES: Array<[string, UpstreamKind]> = [
  ["socks5", "socks5"],
  ["socks", "socks5"],
  ["socks5h", "socks5"],
  ["http", "http"],
  ["https", "http"],
];
const supportedSchemeArb = fc.constantFrom(...SUPPORTED_SCHEMES);

// 认证段：无认证 / 仅用户名 / 用户名+密码。
type Auth = { username?: string; password?: string } | undefined;
const authArb: fc.Arbitrary<Auth> = fc.oneof(
  fc.constant<Auth>(undefined),
  tokenArb.map((username) => ({ username })),
  fc.record({ username: tokenArb, password: tokenArb }),
);

// label：可选片段（#name）。
const labelArb = fc.option(tokenArb, { nil: undefined });

interface SupportedSpec {
  scheme: string;
  kind: UpstreamKind;
  auth: Auth;
  host: string;
  port: number;
  label: string | undefined;
}

const supportedSpecArb: fc.Arbitrary<SupportedSpec> = fc
  .tuple(supportedSchemeArb, authArb, hostArb, portArb, labelArb)
  .map(([[scheme, kind], auth, host, port, label]) => ({
    scheme,
    kind,
    auth,
    host,
    port,
    label,
  }));

function buildSupportedLink(spec: SupportedSpec): string {
  let s = `${spec.scheme}://`;
  if (spec.auth) {
    if (spec.auth.password !== undefined) {
      s += `${spec.auth.username}:${spec.auth.password}@`;
    } else {
      s += `${spec.auth.username}@`;
    }
  }
  s += `${spec.host}:${spec.port}`;
  if (spec.label !== undefined) {
    s += `#${spec.label}`;
  }
  return s;
}

// 非受支持 scheme：解析应返回 null 并计入 ignored。
const unsupportedSchemeArb = fc.constantFrom(
  "ss",
  "ssr",
  "vmess",
  "vless",
  "trojan",
  "hysteria",
  "hysteria2",
  "tuic",
);
const unsupportedLinkArb: fc.Arbitrary<string> = fc
  .tuple(unsupportedSchemeArb, tokenArb)
  .map(([scheme, rest]) => `${scheme}://${rest}`);

describe("parseSubscription 健壮性 (Property 4 / Req 4.1,4.8)", () => {
  it("对任意字符串输入均不抛异常且返回结构良好的结果", () => {
    fc.assert(
      fc.property(fc.string(), (input) => {
        const res = parseSubscription(input);
        expect(Array.isArray(res.candidates)).toBe(true);
        expect(Number.isInteger(res.ignoredUnsupported)).toBe(true);
        expect(res.ignoredUnsupported).toBeGreaterThanOrEqual(0);
      }),
      { numRuns: 100 },
    );
  });

  it("纯非受支持链接集合：candidates 为空且 ignoredUnsupported 等于非空行数", () => {
    fc.assert(
      fc.property(
        fc.array(unsupportedLinkArb, { minLength: 1, maxLength: 20 }),
        (lines) => {
          const res = parseSubscription(lines.join("\n"));
          expect(res.candidates).toEqual([]);
          expect(res.ignoredUnsupported).toBe(lines.length);
        },
      ),
      { numRuns: 100 },
    );
  });
});

describe("parseShareLink round-trip (Property 4 / Req 4.1)", () => {
  it("受支持链接解析后还原等价字段", () => {
    fc.assert(
      fc.property(supportedSpecArb, (spec) => {
        const link = buildSupportedLink(spec);
        const proxy = parseShareLink(link);

        expect(proxy).not.toBeNull();
        if (!proxy) return;

        expect(proxy.kind).toBe(spec.kind);
        expect(proxy.host).toBe(spec.host);
        expect(proxy.port).toBe(spec.port);
        expect(proxy.username).toBe(spec.auth?.username);
        expect(proxy.password).toBe(spec.auth?.password);
        expect(proxy.label).toBe(spec.label ?? "");
        expect(proxy.id.startsWith("sub-")).toBe(true);
      }),
      { numRuns: 100 },
    );
  });

  it("经 parseSubscription 解析单条受支持链接得到唯一候选、无忽略", () => {
    fc.assert(
      fc.property(supportedSpecArb, (spec) => {
        const link = buildSupportedLink(spec);
        const res = parseSubscription(link);

        expect(res.ignoredUnsupported).toBe(0);
        expect(res.candidates).toHaveLength(1);

        const proxy = res.candidates[0];
        expect(proxy.kind).toBe(spec.kind);
        expect(proxy.host).toBe(spec.host);
        expect(proxy.port).toBe(spec.port);
        expect(proxy.username).toBe(spec.auth?.username);
        expect(proxy.password).toBe(spec.auth?.password);
        expect(proxy.label).toBe(spec.label ?? "");
      }),
      { numRuns: 100 },
    );
  });
});

describe("parseShareLink 非受支持类型计入忽略 (Property 4 / Req 4.4)", () => {
  it("ss/vmess/trojan/hysteria 等分享链接解析为 null", () => {
    fc.assert(
      fc.property(unsupportedLinkArb, (link) => {
        expect(parseShareLink(link)).toBeNull();
      }),
      { numRuns: 100 },
    );
  });

  it("受支持与非受支持混合：candidates 仅含受支持项，其余计入 ignoredUnsupported", () => {
    fc.assert(
      fc.property(
        fc.array(supportedSpecArb, { minLength: 0, maxLength: 8 }),
        fc.array(unsupportedLinkArb, { minLength: 0, maxLength: 8 }),
        (supported, unsupported) => {
          const supportedLinks = supported.map(buildSupportedLink);
          // 交错排列受支持与非受支持链接，逐行组成 Import_Source。
          const lines: string[] = [];
          const max = Math.max(supportedLinks.length, unsupported.length);
          for (let i = 0; i < max; i += 1) {
            if (i < supportedLinks.length) lines.push(supportedLinks[i]);
            if (i < unsupported.length) lines.push(unsupported[i]);
          }
          const res = parseSubscription(lines.join("\n"));
          expect(res.candidates).toHaveLength(supported.length);
          expect(res.ignoredUnsupported).toBe(unsupported.length);
        },
      ),
      { numRuns: 100 },
    );
  });
});

// ---- 示例用例：锚定核心语义与关键边界 ----
describe("parseSubscription 示例用例", () => {
  it("空输入返回空结果", () => {
    expect(parseSubscription("")).toEqual({ candidates: [], ignoredUnsupported: 0 });
    expect(parseSubscription("   \n  \t ")).toEqual({
      candidates: [],
      ignoredUnsupported: 0,
    });
  });

  it("socks5 含认证与备注的分享链接", () => {
    const res = parseSubscription("socks5://user:pass@1.2.3.4:1080#node-a");
    expect(res.ignoredUnsupported).toBe(0);
    expect(res.candidates).toHaveLength(1);
    const p = res.candidates[0];
    expect(p.kind).toBe("socks5");
    expect(p.host).toBe("1.2.3.4");
    expect(p.port).toBe(1080);
    expect(p.username).toBe("user");
    expect(p.password).toBe("pass");
    expect(p.label).toBe("node-a");
  });

  it("http 无认证链接，https 归一化为 http", () => {
    expect(parseShareLink("http://proxy.example.com:8080")?.kind).toBe("http");
    expect(parseShareLink("https://proxy.example.com:8080")?.kind).toBe("http");
  });

  it("不支持协议返回 null 并在批量中被计数", () => {
    expect(parseShareLink("ss://xxx@1.2.3.4:8388")).toBeNull();
    const res = parseSubscription(
      ["socks5://1.2.3.4:1080", "vmess://abc", "trojan://def@h:443"].join("\n"),
    );
    expect(res.candidates).toHaveLength(1);
    expect(res.ignoredUnsupported).toBe(2);
  });

  it("Clash YAML 仅提取 socks5/http 节点", () => {
    const yaml = [
      "proxies:",
      "  - {name: a, type: socks5, server: 1.1.1.1, port: 1080}",
      "  - {name: b, type: ss, server: 2.2.2.2, port: 8388}",
      "  - {name: c, type: http, server: 3.3.3.3, port: 8080}",
    ].join("\n");
    const res = parseSubscription(yaml);
    expect(res.candidates).toHaveLength(2);
    expect(res.ignoredUnsupported).toBe(1);
    expect(res.candidates.map((p) => p.kind)).toEqual(["socks5", "http"]);
  });
});
