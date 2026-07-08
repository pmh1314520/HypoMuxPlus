// Feature: network-capability-expansion, Property 24
import { describe, it, expect } from "vitest";
import fc from "fast-check";
import { buildReportLines, type DiagReportRow, type DiagReportLabels } from "./diag";

// Property 24: 文本报告包含全部指标（buildReportLines）
// 每张网卡对应行同时包含 RTT / 抖动 / 丢包 / 吞吐四项指标标签。
// Validates: Requirements 10.4

// 固定标签：使用互不相同且不会互相包含的文案，避免子串误判。
const LABELS: DiagReportLabels = {
  title: "HypoMuxPlus 链路体检报告",
  latency: "RTT",
  jitter: "抖动",
  loss: "丢包",
  speed: "吞吐",
  grade: "评级",
};

/** 单张网卡报告行的 fast-check arbitrary（已格式化的指标文本） */
const rowArb: fc.Arbitrary<DiagReportRow> = fc.record({
  alias: fc.string({ minLength: 1, maxLength: 12 }),
  ipv4: fc.tuple(fc.nat(255), fc.nat(255), fc.nat(255), fc.nat(255)).map((o) => o.join(".")),
  latency: fc.integer({ min: 0, max: 9999 }).map((n) => `${n} ms`),
  jitter: fc.integer({ min: 0, max: 999 }).map((n) => `${n} ms`),
  loss: fc.integer({ min: 0, max: 100 }).map((n) => `${n}%`),
  speed: fc.double({ min: 0, max: 10000, noNaN: true }).map((n) => `${n.toFixed(1)} MB/s`),
  grade: fc.constantFrom("优秀", "良好", "一般", "较慢", "不可用"),
});

describe("buildReportLines - Property 24: 报告包含全部指标", () => {
  it("每张网卡的指标行同时含 RTT / 抖动 / 丢包 / 吞吐 四项标签", () => {
    fc.assert(
      fc.property(fc.array(rowArb, { minLength: 1, maxLength: 8 }), (rows) => {
        const lines = buildReportLines(rows, LABELS, "2024-01-01 00:00:00");

        // 找出所有"完整覆盖四项标签"的行
        const fullyCovered = lines.filter(
          (line) =>
            line.includes(`${LABELS.latency}:`) &&
            line.includes(`${LABELS.jitter}:`) &&
            line.includes(`${LABELS.loss}:`) &&
            line.includes(`${LABELS.speed}:`),
        );

        // 恰好每张网卡一行完整覆盖
        expect(fullyCovered.length).toBe(rows.length);
      }),
      { numRuns: 100 },
    );
  });
});
