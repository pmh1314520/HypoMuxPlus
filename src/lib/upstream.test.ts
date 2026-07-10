// Feature: nic-upstream-proxy-chain, Property 11
// 补齐既有可选属性测试（pro-differentiation-and-hardening, Task 11.2 / Req 11.5）。
//
// Property 11: 上游条目校验综合正确性（validateUpstream）
// For any 上游条目输入（随机 host 长度、端口值、类型字符串、可选认证），
// validateUpstream 通过当且仅当：host 非空且长度 ≤253、port 为整数且 ∈ [1,65535]、
// kind ∈ {socks5,http}、（若配置认证）用户名与密码长度均 ∈ [1,255]；
// 任一条件违反时校验失败并标记出对应的失败字段。
// Validates: Requirements 1.2, 1.6, 9.6
import { describe, it, expect } from "vitest";
import fc from "fast-check";
import {
  validateUpstream,
  HOST_MAX_LEN,
  PORT_MIN,
  PORT_MAX,
  CRED_MIN_LEN,
  CRED_MAX_LEN,
  type UpstreamValidationInput,
  type UpstreamValidationResult,
} from "./upstream";

// 独立于被测实现、直接依据验收标准推导的期望结果 oracle。
function expectedResult(input: UpstreamValidationInput): UpstreamValidationResult {
  const fields: UpstreamValidationResult["fields"] = {};

  const host = input.host;
  if (typeof host !== "string" || host.length < 1 || host.length > HOST_MAX_LEN) {
    fields.host = true;
  }

  const port = input.port;
  if (
    typeof port !== "number" ||
    !Number.isInteger(port) ||
    port < PORT_MIN ||
    port > PORT_MAX
  ) {
    fields.port = true;
  }

  if (input.kind !== "socks5" && input.kind !== "http") {
    fields.kind = true;
  }

  const hasUsername = typeof input.username === "string" && input.username.length > 0;
  const hasPassword = typeof input.password === "string" && input.password.length > 0;
  if (hasUsername || hasPassword) {
    if (
      typeof input.username !== "string" ||
      input.username.length < CRED_MIN_LEN ||
      input.username.length > CRED_MAX_LEN
    ) {
      fields.username = true;
    }
    if (
      typeof input.password !== "string" ||
      input.password.length < CRED_MIN_LEN ||
      input.password.length > CRED_MAX_LEN
    ) {
      fields.password = true;
    }
  }

  return { ok: Object.keys(fields).length === 0, fields };
}

// 智能生成器：把输入约束到「合法 / 各类越界」的关键分区，充分覆盖边界。
const hostArb = fc.oneof(
  fc.string({ minLength: 1, maxLength: HOST_MAX_LEN }), // 合法长度
  fc.constant(""), // 空 -> 非法
  fc.string({ minLength: HOST_MAX_LEN + 1, maxLength: HOST_MAX_LEN + 40 }), // 超长 -> 非法
  fc.constant(undefined), // 缺失 -> 非法
);

const portArb = fc.oneof(
  fc.integer({ min: PORT_MIN, max: PORT_MAX }), // 合法
  fc.integer({ min: -200, max: 0 }), // 过小 -> 非法
  fc.integer({ min: PORT_MAX + 1, max: PORT_MAX + 10000 }), // 过大 -> 非法
  fc.constantFrom(1.5, 80.5, 0.5, Number.NaN), // 非整数 -> 非法
  fc.constant(undefined), // 缺失 -> 非法
);

const kindArb = fc.oneof(
  fc.constantFrom("socks5", "http"), // 合法
  fc.constantFrom("SOCKS5", "HTTP", "https", "socks", "tcp", ""), // 非法字符串
  fc.constant(undefined), // 缺失 -> 非法
);

const credArb = fc.oneof(
  fc.constant(undefined), // 未配置
  fc.constant(""), // 空串（视为未配置该字段）
  fc.string({ minLength: CRED_MIN_LEN, maxLength: CRED_MAX_LEN }), // 合法长度
  fc.string({ minLength: CRED_MAX_LEN + 1, maxLength: CRED_MAX_LEN + 40 }), // 超长 -> 非法
);

const inputArb: fc.Arbitrary<UpstreamValidationInput> = fc.record({
  host: hostArb,
  port: portArb,
  kind: kindArb,
  username: credArb,
  password: credArb,
});

describe("validateUpstream 上游条目校验 (Property 11 / Req 1.2,1.6,9.6)", () => {
  it("对任意输入：ok 与失败字段标记均与验收标准一致", () => {
    fc.assert(
      fc.property(inputArb, (input) => {
        const actual = validateUpstream(input);
        const expected = expectedResult(input);
        expect(actual).toEqual(expected);
      }),
      { numRuns: 100 },
    );
  });

  it("对任意输入：ok 当且仅当无任何失败字段被标记", () => {
    fc.assert(
      fc.property(inputArb, (input) => {
        const { ok, fields } = validateUpstream(input);
        expect(ok).toBe(Object.keys(fields).length === 0);
      }),
      { numRuns: 100 },
    );
  });

  // 示例用例：固定典型输入，锚定核心语义与关键边界。
  it("合法条目（无认证）通过校验", () => {
    const res = validateUpstream({ kind: "socks5", host: "proxy.example.com", port: 1080 });
    expect(res).toEqual({ ok: true, fields: {} });
  });

  it("合法条目（含合法认证）通过校验", () => {
    const res = validateUpstream({
      kind: "http",
      host: "127.0.0.1",
      port: 8080,
      username: "user",
      password: "pass",
    });
    expect(res).toEqual({ ok: true, fields: {} });
  });

  it("host 为空时标记 host 失败", () => {
    const res = validateUpstream({ kind: "http", host: "", port: 8080 });
    expect(res.ok).toBe(false);
    expect(res.fields.host).toBe(true);
  });

  it("host 超过 253 长度时标记 host 失败", () => {
    const res = validateUpstream({
      kind: "http",
      host: "a".repeat(HOST_MAX_LEN + 1),
      port: 8080,
    });
    expect(res.ok).toBe(false);
    expect(res.fields.host).toBe(true);
  });

  it("port 越界（下界/上界）时标记 port 失败", () => {
    expect(validateUpstream({ kind: "http", host: "h", port: 0 }).fields.port).toBe(true);
    expect(
      validateUpstream({ kind: "http", host: "h", port: PORT_MAX + 1 }).fields.port,
    ).toBe(true);
  });

  it("port 非整数时标记 port 失败", () => {
    const res = validateUpstream({ kind: "http", host: "h", port: 80.5 });
    expect(res.fields.port).toBe(true);
  });

  it("kind 非法时标记 kind 失败", () => {
    const res = validateUpstream({ kind: "https", host: "h", port: 8080 });
    expect(res.fields.kind).toBe(true);
  });

  it("仅提供用户名时要求密码也合法（缺密码 -> 标记 password 失败）", () => {
    const res = validateUpstream({ kind: "http", host: "h", port: 8080, username: "user" });
    expect(res.ok).toBe(false);
    expect(res.fields.password).toBe(true);
    expect(res.fields.username).toBeUndefined();
  });

  it("认证凭据超长（>255）时标记对应字段失败", () => {
    const long = "x".repeat(CRED_MAX_LEN + 1);
    const res = validateUpstream({
      kind: "http",
      host: "h",
      port: 8080,
      username: long,
      password: long,
    });
    expect(res.fields.username).toBe(true);
    expect(res.fields.password).toBe(true);
  });
});
