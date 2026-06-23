// HypoMux Plus 官网交互：中英切换 + 滚动揭示

const STORAGE = "hmx-site-lang";
let lang = localStorage.getItem(STORAGE) || "zh";

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
  localStorage.setItem(STORAGE, lang);
  applyLang();
});

applyLang();

// 滚动揭示
const io = new IntersectionObserver(
  (entries) => {
    entries.forEach((e) => {
      if (e.isIntersecting) {
        e.target.classList.add("in");
        io.unobserve(e.target);
      }
    });
  },
  { threshold: 0.12 },
);
document.querySelectorAll(".reveal").forEach((el) => io.observe(el));
