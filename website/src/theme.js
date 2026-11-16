"use strict";

(() => {
  const key = "cor-site-theme";
  const modes = ["auto", "light", "dark"];
  let mode = "auto";
  let timer;
  try {
    const saved = localStorage.getItem(key);
    if (modes.includes(saved)) mode = saved;
  } catch {}

  function apply() {
    clearTimeout(timer);
    const now = new Date();
    const hour = now.getHours();
    const dark = mode === "dark" || (mode === "auto" && (hour < 7 || hour >= 19));
    const root = document.documentElement;
    root.dataset.theme = dark ? "dark" : "light";
    root.dataset.themeMode = mode;
    document.querySelector('meta[name="theme-color"]')?.setAttribute("content", dark ? "#101d23" : "#f8f3e7");
    document.dispatchEvent(new Event("site-theme-change"));
    if (mode === "auto" && !document.hidden) {
      const next = new Date(now);
      next.setHours(hour < 7 ? 7 : hour < 19 ? 19 : 31, 0, 0, 0);
      timer = setTimeout(apply, Math.min(next - now, 3600000));
    }
  }

  window.siteTheme = {
    setMode(value) {
      if (!modes.includes(value)) return;
      mode = value;
      try { localStorage.setItem(key, mode); } catch {}
      apply();
    },
  };
  window.addEventListener("storage", event => {
    if (event.key !== key && event.key !== null) return;
    mode = modes.includes(event.newValue) ? event.newValue : "auto";
    apply();
  });
  window.addEventListener("pageshow", apply);
  window.addEventListener("focus", apply);
  document.addEventListener("visibilitychange", apply);
  apply();
})();
