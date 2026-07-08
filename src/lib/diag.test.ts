// Feature: network-capability-expansion, Property 23
import { describe, it, expect } from "vitest";
import fc from "fast-check";
import { appendTrendPoint, capTrend, type DiagTrendPoint } from "./diag";

/** 生成单个诊断趋势采样点的 fast-check arbitrary */
const trendPointArb: fc.Arbitrary<DiagTrendPoint> = fc.record({
  ts: fc.integer(),
  latencyMs: fc.integer(),
  jitterMs: fc.integer(),
  lossPct: fc.double({ min: 0, max: 1, noNaN: true }),
  mbps: fc.double({ noNaN: true }),
  ok: fc.boolean(),
});

const trendArb: fc.Arbitrary<DiagTrendPoint[]> = fc.array(trendPointArb);

describe("diag 趋势历史追加/裁剪 (Property 23)", () => {
  // Property A：追加一个新点后数组长度加一且末元素为新点，且不修改入参
  // Validates: Requirements 10.1
  it("appendTrendPoint 追加后长度+1、末元素为新点且不改动入参", () => {
    fc.assert(
      fc.property(trendArb, trendPointArb, (arr, point) => {
        const before = arr.length;
        const result = appendTrendPoint(arr, point);
        // 长度加一
        expect(result.length).toBe(before + 1);
        // 末元素为新点（深比较）
        expect(result[result.length - 1]).toEqual(point);
        // 不修改入参
        expect(arr.length).toBe(before);
      }),
      { numRuns: 100 }
    );
  });

  // Property B：capTrend 结果长度不超过 max 且恰好保留最近的 max 个点（末尾若干，顺序保持）
  // Validates: Requirements 10.2
  it("capTrend 结果长度受 max 约束并保留最近的点", () => {
    fc.assert(
      fc.property(trendArb, fc.integer({ min: -2, max: 60 }), (arr, max) => {
        const result = capTrend(arr, max);
        if (max <= 0) {
          // max<=0 返回空数组
          expect(result).toEqual([]);
        } else {
          // 长度 = min(arr.length, max)，且永不超过 max
          expect(result.length).toBe(Math.min(arr.length, max));
          expect(result.length).toBeLessThanOrEqual(max);
          if (arr.length > max) {
            // 保留最近的 max 个点（末尾若干，顺序保持）
            expect(result).toEqual(arr.slice(arr.length - max));
          } else {
            // 长度不超过 max 时为原数组的拷贝
            expect(result).toEqual(arr);
          }
        }
      }),
      { numRuns: 100 }
    );
  });
});
