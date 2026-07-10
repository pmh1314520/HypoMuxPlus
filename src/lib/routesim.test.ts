// Feature: pro-differentiation-and-hardening, Property 9
//
// Property 9: 模拟器输入校验（validateSimInput，前端）
// For any host 与 port：validateSimInput 通过当且仅当 host 为非空字符串
// 且 port 为整数且 ∈ [1, 65535]；任一条件违反时 ok 为 false，并把对应字段
// （hostError / portError）标记为 true。对任意输入均不抛异常。
//
// 语义严格对齐 design.md「Property 9」与 requirements.md Req 3.5 中的验收标准：
// 「host 非空且 port ∈ [1,65535]」（此处「非空」即 length > 0，不做去空白处理，
// 与被测实现 validateSimInput 一致）。
//
// Validates: Requirements 3.5
import { describe, it, expect } from "vitest";
import fc from "fast-check";
import {
  validateSimInput,
  PORT_MIN,
  PORT_MAX,
  type SimInputValidation,
} from "./routesim";

// 独立于被测实现、直接依据验收标准（Property 9 / Req 3.5）推导的期望 oracle。
function expectedResult(host: unknown, port: unknown): SimInputValidation {
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

// 智能生成器：把输入约束到「合法 / 各类越界」的关键分区，充分覆盖边界。
const hostArb = fc.oneof(
  fc.string({ minLength: 1, maxLength: 60 }), // 一般非空串（含可能的空白字符）
  fc.constant(""), // 空串 -> 非法
  fc.constantFrom(" ", "   ", "\t", "\n", " \t "), // 纯空白 -> 依 spec 视为非空（有效 host 段）
  fc.constantFrom("example.com", "*.foo.bar", "chrome.exe"), // 典型域名 / 进程名
);

const portArb = fc.oneof(
  fc.integer({ min: PORT_MIN, max: PORT_MAX }), // 合法
  fc.constantFrom(PORT_MIN, PORT_MAX), // 边界值 1 / 65535
  fc.integer({ min: -500, max: PORT_MIN - 1 }), // 过小（含 0、负数）-> 非法
  fc.integer({ min: PORT_MAX + 1, max: PORT_MAX + 10000 }), // 过大 -> 非法
  fc.constantFrom(1.5, 80.5, 0.5, Number.NaN, Number.POSITIVE_INFINITY), // 非整数 -> 非法
);

describe("validateSimInput 模拟器输入校验 (Property 9 / Req 3.5)", () => {
  it("对任意输入：ok 与字段错误标记均与验收标准一致", () => {
    fc.assert(
      fc.property(hostArb, portArb, (host, port) => {
        const actual = validateSimInput(host, port);
        const expected = expectedResult(host, port);
        expect(actual).toEqual(expected);
      }),
      { numRuns: 100 },
    );
  });

  it("有效性等价：ok 为真 当且仅当 host 非空 且 port 为 [1,65535] 内整数", () => {
    fc.assert(
      fc.property(hostArb, portArb, (host, port) => {
        const { ok } = validateSimInput(host, port);
        const hostOk = typeof host === "string" && host.length > 0;
        const portOk = Number.isInteger(port) && port >= PORT_MIN && port <= PORT_MAX;
        expect(ok).toBe(hostOk && portOk);
      }),
      { numRuns: 100 },
    );
  });

  it("对任意输入均不抛异常", () => {
    fc.assert(
      fc.property(hostArb, portArb, (host, port) => {
        expect(() => validateSimInput(host, port)).not.toThrow();
      }),
      { numRuns: 100 },
    );
  });

  // 示例用例：锚定核心语义与关键边界。
  it("合法输入（非空 host + 端口在范围内）通过校验", () => {
    expect(validateSimInput("example.com", 443)).toEqual({ ok: true });
    expect(validateSimInput("chrome.exe", PORT_MIN)).toEqual({ ok: true });
    expect(validateSimInput("h", PORT_MAX)).toEqual({ ok: true });
  });

  it("空 host 时标记 hostError 且整体失败", () => {
    const res = validateSimInput("", 8080);
    expect(res.ok).toBe(false);
    expect(res.hostError).toBe(true);
    expect(res.portError).toBeUndefined();
  });

  it("端口过小（< 1）时标记 portError", () => {
    expect(validateSimInput("h", 0).portError).toBe(true);
    expect(validateSimInput("h", 0).ok).toBe(false);
    expect(validateSimInput("h", -1).portError).toBe(true);
  });

  it("端口过大（> 65535）时标记 portError", () => {
    const res = validateSimInput("h", PORT_MAX + 1);
    expect(res.ok).toBe(false);
    expect(res.portError).toBe(true);
  });

  it("端口非整数时标记 portError", () => {
    expect(validateSimInput("h", 80.5).portError).toBe(true);
    expect(validateSimInput("h", Number.NaN).portError).toBe(true);
  });

  it("host 与 port 同时非法时两个字段均被标记", () => {
    const res = validateSimInput("", 0);
    expect(res).toEqual({ ok: false, hostError: true, portError: true });
  });
});
