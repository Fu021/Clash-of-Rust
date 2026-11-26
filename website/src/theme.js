"use strict";

(() => {
  const root = document.documentElement;
  root.classList.add("theme-loading");
  setTimeout(() => root.classList.remove("theme-loading"), 1200);
  const key = "cor-site-theme";
  const modes = ["auto", "light", "dark"];
  const springAutumn = [7 * 60, 18 * 60];
  const summer = [6 * 60, 20 * 60];
  const winter = [7 * 60 + 30, 17 * 60 + 30];
  let mode = "auto";
  let timer;
  try {
    const saved = localStorage.getItem(key);
    if (modes.includes(saved)) mode = saved;
  } catch {}

  function daylightHours(month) {
    if (month === 11 || month < 2) return winter;
    if (month >= 5 && month < 8) return summer;
    return springAutumn;
  }

  function apply() {
    clearTimeout(timer);
    const now = new Date();
    const [dayStart, nightStart] = daylightHours(now.getMonth());
    const minute = now.getHours() * 60 + now.getMinutes();
    const dark = mode === "dark" || (mode === "auto" && (minute < dayStart || minute >= nightStart));
    const root = document.documentElement;
    root.dataset.theme = dark ? "dark" : "light";
    root.dataset.themeMode = mode;
    document.querySelector('meta[name="theme-color"]')?.setAttribute("content", dark ? "#101d23" : "#f8f3e7");
    document.dispatchEvent(new Event("site-theme-change"));
    if (mode === "auto" && !document.hidden) {
      const next = new Date(now);
      const boundary = minute < dayStart ? dayStart : minute < nightStart ? nightStart : 24 * 60;
      next.setHours(0, boundary, 0, 0);
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
