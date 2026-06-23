// HypoMuxPlus 官网交互：主题(跟随系统/手动) · 中英切换 · 导航滚动态 · 滚动揭示

/* ---------- 主题 ---------- */
const TKEY = "hmx-site-theme";
const darkMq = window.matchMedia("(prefers-color-scheme: dark)");
const SUN =
  '<circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4"/>';
const MOON = '<path d="M21 12.8A9 9 0 1 1 11.2 3 7 7 0 0 0 21 12.8z"/>';

function applyTheme(theme) {
  document.documentElement.setAttribute("data-theme", theme);
  const icon = document.getElementById("themeIcon");
  if (icon) icon.innerHTML = theme === "dark" ? SUN : MOON; // 图标表示“点击后切换到的模式”
}

// 初始：优先用户手动选择，否则跟随系统
let theme = localStorage.getItem(TKEY) || (darkMq.matches ? "dark" : "light");
applyTheme(theme);

document.getElementById("themeBtn")?.addEventListener("click", () => {
  theme = document.documentElement.getAttribute("data-theme") === "dark" ? "light" : "dark";
  localStorage.setItem(TKEY, theme);
  applyTheme(theme);
});

// 未手动选择时，实时跟随系统主题变化
darkMq.addEventListener("change", (e) => {
  if (!localStorage.getItem(TKEY)) applyTheme(e.matches ? "dark" : "light");
});

/* ---------- 中英切换 ---------- */
const LKEY = "hmx-site-lang";
let lang = localStorage.getItem(LKEY) || "zh";

function applyLang() {
  document.documentElement.lang = lang === "zh" ? "zh-CN" : "en";
  document.querySelectorAll("[data-zh]").forEach((el) => {
    const txt = el.getAttribute(lang === "zh" ? "data-zh" : "data-en");
    if (txt !== null) el.textContent = txt;
  });
  const btn = document.getElementById("langBtn");
  if (btn) btn.textContent = lang === "zh" ? "EN" : "中文";
}

document.getElementById("langBtn")?.addEventListener("click", () => {
  lang = lang === "zh" ? "en" : "zh";
  localStorage.setItem(LKEY, lang);
  applyLang();
});

applyLang();

/* ---------- 导航滚动态 ---------- */
const nav = document.getElementById("nav");
const onScroll = () => nav?.classList.toggle("scrolled", window.scrollY > 12);
window.addEventListener("scroll", onScroll, { passive: true });
onScroll();

/* ---------- 滚动揭示 ---------- */
const io = new IntersectionObserver(
  (entries) => {
    entries.forEach((e) => {
      if (e.isIntersecting) {
        e.target.classList.add("in");
        io.unobserve(e.target);
      }
    });
  },
  { threshold: 0.1 },
);
document.querySelectorAll(".reveal").forEach((el) => io.observe(el));

/* ---------- Hero 实时演示动画（随机游走，非循环） ---------- */
(function () {
  const numEl = document.getElementById("shotNum");
  const areaEl = document.getElementById("sparkArea");
  const lineEl = document.getElementById("sparkLine");
  const bars = [0, 1, 2].map((i) => ({
    fill: document.getElementById("b" + i),
    val: document.getElementById("b" + i + "v"),
  }));
  if (!numEl || !lineEl) return;

  const MIN = 100;
  const MAX = 500;
  const N = 28;
  const W = 320;
  const TOP = 8;
  const BOT = 58;

  // 初始用一段随机游走填满曲线
  let total = 240 + Math.random() * 120;
  const series = [];
  let seed = total;
  for (let i = 0; i < N; i++) {
    seed += (Math.random() - 0.5) * 90;
    seed = Math.min(MAX, Math.max(MIN, seed));
    series.push(seed);
  }
  total = series[N - 1];

  // 三网卡基础权重（每次抖动，制造自然的此消彼长）
  let weights = [0.36, 0.34, 0.3];

  function y(v) {
    const f = (v - MIN) / (MAX - MIN);
    return BOT - f * (BOT - TOP);
  }
  function draw() {
    const step = W / (N - 1);
    let line = "";
    for (let i = 0; i < N; i++) {
      line += (i ? "L" : "M") + (i * step).toFixed(1) + "," + y(series[i]).toFixed(1) + " ";
    }
    lineEl.setAttribute("d", line.trim());
    areaEl.setAttribute("d", line.trim() + " L" + W + ",60 L0,60 Z");
  }

  function animateNumber(from, to, dur) {
    const t0 = performance.now();
    function frame(now) {
      const p = Math.min(1, (now - t0) / dur);
      const e = 1 - Math.pow(1 - p, 3); // easeOutCubic
      numEl.textContent = (from + (to - from) * e).toFixed(1);
      if (p < 1) requestAnimationFrame(frame);
    }
    requestAnimationFrame(frame);
  }

  function updateBars() {
    // 权重随机抖动后归一化
    let w = weights.map((x) => Math.max(0.12, x + (Math.random() - 0.5) * 0.28));
    const sum = w[0] + w[1] + w[2];
    w = w.map((x) => x / sum);
    weights = w;
    bars.forEach((b, i) => {
      if (!b.fill) return;
      const v = total * w[i];
      b.fill.style.width = (w[i] * 100).toFixed(1) + "%";
      if (b.val) b.val.textContent = v.toFixed(1) + " MB/s";
    });
  }

  function tick() {
    const prev = total;
    // 随机游走：步长与方向都随机，偶尔大跳，营造"无规律"的真实感
    const jump = Math.random() < 0.18 ? 1 : 0.4;
    total += (Math.random() - 0.5) * 170 * jump;
    total = Math.min(MAX, Math.max(MIN, total));
    series.push(total);
    series.shift();
    draw();
    animateNumber(parseFloat(numEl.textContent) || prev, total, 700);
    updateBars();
    // 不规则刷新间隔
    setTimeout(tick, 820 + Math.random() * 900);
  }

  draw();
  updateBars();
  numEl.textContent = total.toFixed(1);

  const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  if (!reduce) setTimeout(tick, 900);
})();
