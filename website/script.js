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
  const meta = document.querySelector('meta[name="theme-color"]');
  if (meta) meta.setAttribute("content", theme === "dark" ? "#0a0e18" : "#eef2f8");
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

/* ---------- 二维码全屏放大灯箱 ---------- */
(function () {
  const lb = document.getElementById("lightbox");
  const lbImg = document.getElementById("lightboxImg");
  const lbCap = document.getElementById("lightboxCap");
  const lbClose = document.getElementById("lightboxClose");
  if (!lb) return;

  function open(src, cap) {
    lbImg.src = src;
    lbCap.textContent = cap || "";
    lb.classList.add("open");
    lb.setAttribute("aria-hidden", "false");
  }
  function close() {
    lb.classList.remove("open");
    lb.setAttribute("aria-hidden", "true");
  }

  document.querySelectorAll(".qr").forEach((qr) => {
    const img = qr.querySelector("img");
    const nameEl = qr.querySelector(".name span");
    img?.addEventListener("click", () => open(img.src, nameEl ? nameEl.textContent : ""));
  });

  lbClose?.addEventListener("click", close);
  lb.addEventListener("click", (e) => {
    if (e.target === lb) close();
  });
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape") close();
  });
})();

/* ---------- Hero 实时演示动画（60fps 逐帧缓动，极致丝滑） ---------- */
(function () {
  const numEl = document.getElementById("shotNum");
  const areaEl = document.getElementById("sparkArea");
  const lineEl = document.getElementById("sparkLine");
  const bars = [0, 1, 2].map((i) => ({
    fill: document.getElementById("b" + i),
    val: document.getElementById("b" + i + "v"),
  }));
  if (!numEl || !lineEl) return;

  const MIN = 100,
    MAX = 260,
    N = 30,
    W = 320,
    TOP = 8,
    BOT = 58;

  // 目标值（随机游走，不规律变化）与显示值（每帧缓动逼近）
  let tTotal = 140 + Math.random() * 80;
  let dTotal = tTotal;
  const target = [];
  const disp = [];
  let seed = tTotal;
  for (let i = 0; i < N; i++) {
    seed += (Math.random() - 0.5) * 55;
    seed = Math.min(MAX, Math.max(MIN, seed));
    target.push(seed);
    disp.push(seed);
  }
  let tW = [0.36, 0.34, 0.3];
  let dW = tW.slice();

  const yOf = (v) => BOT - ((v - MIN) / (MAX - MIN)) * (BOT - TOP);

  // Catmull-Rom → 三次贝塞尔，平滑曲线
  function smoothPath(ys) {
    const step = W / (N - 1);
    const p = ys.map((y, i) => [i * step, yOf(y)]);
    let d = `M${p[0][0].toFixed(2)},${p[0][1].toFixed(2)}`;
    for (let i = 0; i < p.length - 1; i++) {
      const p0 = p[i - 1] || p[i],
        p1 = p[i],
        p2 = p[i + 1],
        p3 = p[i + 2] || p2;
      const c1x = p1[0] + (p2[0] - p0[0]) / 6,
        c1y = p1[1] + (p2[1] - p0[1]) / 6;
      const c2x = p2[0] - (p3[0] - p1[0]) / 6,
        c2y = p2[1] - (p3[1] - p1[1]) / 6;
      d += `C${c1x.toFixed(2)},${c1y.toFixed(2)} ${c2x.toFixed(2)},${c2y.toFixed(2)} ${p2[0].toFixed(2)},${p2[1].toFixed(2)}`;
    }
    return d;
  }

  let raf = 0;
  function render() {
    dTotal += (tTotal - dTotal) * 0.075;
    for (let i = 0; i < N; i++) disp[i] += (target[i] - disp[i]) * 0.12;
    for (let i = 0; i < 3; i++) dW[i] += (tW[i] - dW[i]) * 0.08;

    const line = smoothPath(disp);
    lineEl.setAttribute("d", line);
    areaEl.setAttribute("d", `${line} L${W},60 L0,60 Z`);
    numEl.textContent = dTotal.toFixed(1);
    bars.forEach((b, i) => {
      if (b.fill) b.fill.style.width = (dW[i] * 100).toFixed(2) + "%";
      if (b.val) b.val.textContent = (dTotal * dW[i]).toFixed(1) + " MB/s";
    });
    raf = requestAnimationFrame(render);
  }

  // 不规律地推进目标：曲线左移一格 + 新样本，速度随机游走，权重抖动
  function mutate() {
    // 均值回归随机游走：向中心 180 拉回 + 随机扰动，避免长时间贴边
    const CENTER = 180;
    const kick = Math.random() < 0.15 ? 95 : 48; // 偶尔大幅跳动
    tTotal += (CENTER - tTotal) * 0.2 + (Math.random() - 0.5) * 2 * kick;
    tTotal = Math.min(MAX, Math.max(MIN, tTotal));
    target.shift();
    target.push(tTotal + (Math.random() - 0.5) * 40);
    target[N - 1] = Math.min(MAX, Math.max(MIN, target[N - 1]));

    let w = tW.map((x) => Math.max(0.14, x + (Math.random() - 0.5) * 0.26));
    const s = w[0] + w[1] + w[2];
    tW = w.map((x) => x / s);

    timer = setTimeout(mutate, 620 + Math.random() * 760);
  }

  let timer = 0;
  function start() {
    if (!raf) raf = requestAnimationFrame(render);
    if (!timer) timer = setTimeout(mutate, 600);
  }
  function stop() {
    cancelAnimationFrame(raf);
    raf = 0;
    clearTimeout(timer);
    timer = 0;
  }

  const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  if (reduce) {
    // 静态快照
    lineEl.setAttribute("d", smoothPath(disp));
    areaEl.setAttribute("d", `${smoothPath(disp)} L${W},60 L0,60 Z`);
    numEl.textContent = dTotal.toFixed(1);
    bars.forEach((b, i) => {
      if (b.fill) b.fill.style.width = (dW[i] * 100).toFixed(2) + "%";
      if (b.val) b.val.textContent = (dTotal * dW[i]).toFixed(1) + " MB/s";
    });
    return;
  }

  // 标签页不可见时暂停，省电且回到时不跳变
  document.addEventListener("visibilitychange", () => (document.hidden ? stop() : start()));
  start();
})();


/* ---------- FAQ 手风琴 ---------- */
(function () {
  const items = document.querySelectorAll(".faq-item");
  const setH = (item) => {
    const a = item.querySelector(".faq-a");
    if (item.classList.contains("open")) a.style.maxHeight = a.scrollHeight + "px";
  };
  items.forEach((item) => {
    const q = item.querySelector(".faq-q");
    const a = item.querySelector(".faq-a");
    q?.addEventListener("click", () => {
      const open = item.classList.toggle("open");
      a.style.maxHeight = open ? a.scrollHeight + "px" : null;
    });
  });
  // 视口变化 / 语言切换后重算已展开项高度
  window.addEventListener("resize", () => items.forEach(setH));
  document
    .getElementById("langBtn")
    ?.addEventListener("click", () => setTimeout(() => items.forEach(setH), 0));
})();

/* ---------- 防复制 / 防盗版（合理威慑，非绝对防护） ---------- */
(function () {
  const block = (e) => {
    e.preventDefault();
    return false;
  };
  ["contextmenu", "dragstart", "selectstart", "copy", "cut"].forEach((ev) =>
    document.addEventListener(ev, block),
  );
  document.addEventListener("keydown", (e) => {
    const k = (e.key || "").toLowerCase();
    if (e.key === "F12") return block(e);
    if ((e.ctrlKey || e.metaKey) && !e.shiftKey && ["s", "u", "p", "c", "a", "x"].includes(k)) return block(e);
    if ((e.ctrlKey || e.metaKey) && e.shiftKey && ["i", "j", "c"].includes(k)) return block(e);
  });
})();

/* ---------- 联系方式一键复制 ---------- */
(function () {
  let tipTimer;
  function showCopied(btn) {
    let tip = document.getElementById("copyTip");
    if (!tip) {
      tip = document.createElement("div");
      tip.id = "copyTip";
      tip.className = "copy-toast";
      document.body.appendChild(tip);
    }
    const isEn = document.documentElement.getAttribute("data-lang") === "en";
    tip.textContent = isEn ? "Copied" : "已复制";
    tip.classList.add("show");
    clearTimeout(tipTimer);
    tipTimer = setTimeout(() => tip.classList.remove("show"), 1500);
    btn.classList.add("copied");
    setTimeout(() => btn.classList.remove("copied"), 800);
  }
  function copyText(text) {
    if (navigator.clipboard && navigator.clipboard.writeText) {
      return navigator.clipboard.writeText(text);
    }
    // 回退方案：file:// 或无 clipboard API 时
    const ta = document.createElement("textarea");
    ta.value = text;
    ta.style.position = "fixed";
    ta.style.opacity = "0";
    document.body.appendChild(ta);
    ta.select();
    try {
      document.execCommand("copy");
    } catch (e) {
      /* ignore */
    }
    document.body.removeChild(ta);
    return Promise.resolve();
  }
  document.querySelectorAll("[data-copy]").forEach((btn) => {
    btn.addEventListener("click", () => {
      copyText(btn.getAttribute("data-copy") || "").then(() => showCopied(btn));
    });
  });
})();
