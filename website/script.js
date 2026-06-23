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
