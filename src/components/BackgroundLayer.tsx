// 自定义背景图层：图片底层 + 明暗蒙版上层
//
// 设计要点：
// - 两层分离：图片层只负责「显示什么」（不透明度 / 模糊），蒙版层只负责「压多暗」。
//   这样用户调图片不透明度时不会牵动文字对比度，反之亦然，两个滑块语义互不干扰。
// - 蒙版跟随主题：深色主题压向黑、浅色主题压向白，且用径向 + 线性双层渐变而非纯色平铺，
//   使中心通透、四周收拢，形成景深与高级感，也顺带保证顶栏 / 侧栏区域的文字对比度。
// - 固定定位铺满视口且 pointer-events: none，绝不拦截任何交互；层级置于内容之下。
// - 图片模糊后边缘会透出底色，故用 scale 轻微放大溢出裁掉模糊边，避免四周出现亮边。

import { useSettings } from "../store";

export function BackgroundLayer() {
  const { bgEnabled, bgImageUrl, bgOpacity, bgMask, bgBlur, theme } = useSettings();

  // 未启用或尚无图片：不渲染任何东西，完全回退到 #root 的默认渐变基底
  if (!bgEnabled || !bgImageUrl) return null;

  const light = theme === "light";
  // 蒙版基色：深色主题压向近黑（与 --bg-0 同族），浅色主题压向近白，避免染色偏移
  const maskRgb = light ? "244, 246, 250" : "10, 12, 16";
  // 中心较通透、边缘更浓：中心浓度按 60% 折算，边缘取满值，形成自然的暗角收拢
  const centerAlpha = (bgMask * 0.6).toFixed(3);
  const edgeAlpha = bgMask.toFixed(3);

  return (
    <div aria-hidden className="fixed inset-0 pointer-events-none" style={{ zIndex: 0 }}>
      {/* 图片层：cover 铺满并居中，模糊时放大以裁掉模糊产生的透明边缘 */}
      <div
        className="absolute inset-0"
        style={{
          backgroundImage: `url("${bgImageUrl}")`,
          backgroundSize: "cover",
          backgroundPosition: "center",
          backgroundRepeat: "no-repeat",
          opacity: bgOpacity,
          filter: bgBlur > 0 ? `blur(${bgBlur}px)` : undefined,
          transform: bgBlur > 0 ? `scale(${1 + bgBlur / 100})` : undefined,
          transition: "opacity 0.35s ease, filter 0.35s ease, transform 0.35s ease",
        }}
      />
      {/* 明暗蒙版层：径向（中心通透→边缘收拢）叠加线性（顶部略压，衬托顶栏文字） */}
      <div
        className="absolute inset-0"
        style={{
          background: `
            radial-gradient(120% 120% at 50% 40%,
              rgba(${maskRgb}, ${centerAlpha}) 0%,
              rgba(${maskRgb}, ${edgeAlpha}) 100%),
            linear-gradient(180deg,
              rgba(${maskRgb}, ${centerAlpha}) 0%,
              rgba(${maskRgb}, 0) 30%)
          `,
          transition: "background 0.35s ease",
        }}
      />
    </div>
  );
}
