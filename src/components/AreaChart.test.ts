// Feature: network-capability-expansion, Property 18
import { describe, it, expect } from "vitest";
import fc from "fast-check";
import { niceCeil } from "./AreaChart";

// Property 18: niceCeil 上界单调性
// 结果不小于输入，且对不减序列单调不减，并落在预期的“友好”刻度集合上。
// Validates: Requirements 7.3
describe("niceCeil - Property 18: 上界单调性", () => {
  const nonNegDouble = () =>
    fc.double({ min: 0, max: 1e12, noNaN: true, noDefaultInfinity: true });

  // Property A（上界）：对所有 v >= 0，niceCeil(v) >= v。
  it("结果不小于输入 (upper bound)", () => {
    fc.assert(
      fc.property(nonNegDouble(), (v) => {
        const result = niceCeil(v);
        // “友好”步长恒 >= 归一化值，故 step*mag >= v；容忍浮点误差使用相对 epsilon。
        const tolerance = Math.max(1e-6, Math.abs(v) * 1e-9);
        expect(result).toBeGreaterThanOrEqual(v - tolerance);
      }),
      { numRuns: 100 }
    );
  });

  // Property B（单调不减）：对任意 a <= b，niceCeil(a) <= niceCeil(b)。
  it("对不减序列单调不减 (monotonic non-decreasing)", () => {
    fc.assert(
      fc.property(nonNegDouble(), nonNegDouble(), (x, y) => {
        const a = Math.min(x, y);
        const b = Math.max(x, y);
        expect(niceCeil(a)).toBeLessThanOrEqual(niceCeil(b));
      }),
      { numRuns: 100 }
    );
  });

  // Property C（落在预期刻度集合上）：结果为刻度集合中的值，
  // 通过幂等性验证：niceCeil 作用于其自身输出应返回相同值。
  it("落在预期刻度集合上 (idempotent on the nice scale set)", () => {
    fc.assert(
      fc.property(nonNegDouble(), (v) => {
        const result = niceCeil(v);
        expect(niceCeil(result)).toBe(result);
        // 额外验证归一化尾数属于 {1, 2, 5, 10}。
        if (result > 1) {
          const mag = Math.pow(10, Math.floor(Math.log10(result)));
          const mantissa = Math.round(result / mag);
          expect([1, 2, 5, 10]).toContain(mantissa);
        } else {
          expect(result).toBe(1);
        }
      }),
      { numRuns: 100 }
    );
  });
});
