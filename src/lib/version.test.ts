// Feature: network-capability-expansion, Property 16
import { describe, it, expect } from "vitest";
import fc from "fast-check";
import { versionGt } from "./version";

// Property 16: 版本比较与逐段数值序一致（versionGt）
// versionGt(a,b) 当且仅当 a 在逐段数值字典序上大于 b（缺位按 0）；据此反自反、反对称。
// Validates: Requirements 6.4, 7.4
const verArb = (): fc.Arbitrary<string> =>
  fc.array(fc.nat(50), { minLength: 1, maxLength: 4 }).map((a) => a.join("."));

describe("versionGt - Property 16: 版本比较逐段数值序", () => {
  it("反自反：versionGt(a, a) 恒为 false", () => {
    fc.assert(
      fc.property(verArb(), (a) => {
        expect(versionGt(a, a)).toBe(false);
      }),
      { numRuns: 100 },
    );
  });

  it("与逐段数值序一致，且反对称", () => {
    fc.assert(
      fc.property(verArb(), verArb(), (a, b) => {
        const pa = a.split(".").map(Number);
        const pb = b.split(".").map(Number);
        const n = Math.max(pa.length, pb.length);
        let cmp = 0;
        for (let i = 0; i < n; i++) {
          const x = pa[i] ?? 0;
          const y = pb[i] ?? 0;
          if (x !== y) {
            cmp = x > y ? 1 : -1;
            break;
          }
        }
        expect(versionGt(a, b)).toBe(cmp > 0);
        // 反对称：不可同时 a>b 且 b>a
        expect(versionGt(a, b) && versionGt(b, a)).toBe(false);
      }),
      { numRuns: 100 },
    );
  });
});
