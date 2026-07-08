// Feature: network-capability-expansion, Property 17
import { describe, it, expect } from "vitest";
import fc from "fast-check";
import { DICT } from "./i18n";

// Property 17: i18n 中英字典键集合完全一致
// 中文字典 zh 与英文字典 en 的键集合相等（对称差为空），不存在仅在一侧出现的键。
// Validates: Requirements 7.2
describe("i18n 字典键对齐 (Property 17)", () => {
  const zhKeys = Object.keys(DICT.zh);
  const enKeys = Object.keys(DICT.en);
  const zhSet = new Set(zhKeys);
  const enSet = new Set(enKeys);

  it("zh 与 en 键集合完全一致（对称差为空）", () => {
    const onlyZh = zhKeys.filter((k) => !enSet.has(k));
    const onlyEn = enKeys.filter((k) => !zhSet.has(k));
    expect(onlyZh).toEqual([]);
    expect(onlyEn).toEqual([]);
  });

  it("对任意键：存在于 zh 当且仅当存在于 en", () => {
    const allKeys = Array.from(new Set([...zhKeys, ...enKeys]));
    fc.assert(
      fc.property(fc.constantFrom(...allKeys), (key) => {
        expect(zhSet.has(key)).toBe(enSet.has(key));
      }),
      { numRuns: 100 },
    );
  });
});
