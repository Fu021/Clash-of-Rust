"use strict";
document.documentElement.classList.add("js");
const REPO = "Fu021/Clash-of-Rust";
const WEB = `https://github.com/${REPO}`;
const API = `https://api.github.com/repos/${REPO}`;
const ZH = document.documentElement.lang === "zh-CN";
const say = (english, chinese) => (ZH ? chinese : english);
const $ = (selector, root = document) => root.querySelector(selector);
const $$ = (selector, root = document) => [...root.querySelectorAll(selector)];

/* Remember an explicit language choice so English pages stop redirecting. */
for (const link of $$("[data-set-lang]")) {
  link.addEventListener("click", () => {
    try { localStorage.setItem("cor-lang", link.dataset.setLang); } catch (_) {}
  });
}

async function github(path) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), 5000);
  try {
    const response = await fetch(API + path, {
      signal: controller.signal,
      credentials: "omit",
      referrerPolicy: "no-referrer",
      headers: { Accept: "application/vnd.github+json" },
    });
    if (!response.ok) throw new Error(String(response.status));
    return await response.json();
  } finally {
    clearTimeout(timer);
  }
}

/* Star count: shown only when GitHub answers, never a hard-coded number. */
github("").then(repo => {
  if (!Number.isSafeInteger(repo.stargazers_count)) return;
  for (const slot of $$("[data-stars]")) slot.textContent = repo.stargazers_count.toLocaleString(ZH ? "zh-CN" : "en");
}).catch(() => {});

const agent = navigator.userAgent;
const tablet = /iPad/.test(agent) || (/Macintosh/.test(agent) && navigator.maxTouchPoints > 1);
const platform = tablet || /iPhone/.test(agent) ? "" : /Android/.test(agent) ? "android" : /Windows/.test(agent) ? "windows"
  : /Mac OS X|Macintosh/.test(agent) ? "macos" : /Linux/.test(agent) ? "linux" : "";
const PLATFORM_NAME = { windows: "Windows", linux: "Linux", macos: "macOS", android: "Android" };

for (const button of $$("[data-download-button]")) {
  if (!platform) continue;
  button.textContent = say(`Download for ${PLATFORM_NAME[platform]}`, `下载 ${PLATFORM_NAME[platform]} 版`);
  button.href = `${button.getAttribute("href").split("#")[0]}#${platform}`;
}

/* Light and dark screenshots */
for (const figure of $$("[data-shots]")) {
  const buttons = $$("[data-shot]", figure);
  for (const button of buttons) {
    button.addEventListener("click", () => {
      for (const other of buttons) other.setAttribute("aria-pressed", String(other === button));
      for (const image of $$("img[data-theme]", figure)) image.hidden = image.dataset.theme !== button.dataset.shot;
    });
  }
}

/* Guide: mark the section being read in the table of contents. */
const tocLinks = $$(".toc a[href^='#']");
if (tocLinks.length && "IntersectionObserver" in window) {
  const byId = new Map(tocLinks.map(link => [link.hash.slice(1), link]));
  const visible = new Set();
  const observer = new IntersectionObserver(entries => {
    for (const entry of entries) entry.isIntersecting ? visible.add(entry.target.id) : visible.delete(entry.target.id);
    const first = [...byId.keys()].find(id => visible.has(id));
    if (!first) return;
    for (const [id, link] of byId) id === first ? link.setAttribute("aria-current", "true") : link.removeAttribute("aria-current");
  }, { rootMargin: "-10% 0px -55% 0px" });
  for (const id of byId.keys()) {
    const section = document.getElementById(id);
    if (section) observer.observe(section);
  }
}

/* Download page */
const downloads = $("[data-downloads]");
if (downloads) {
  const PACKAGES = [
    { os: "windows", suffix: "windows-x64-setup.exe", label: say("x64 installer", "x64 安装程序"), hint: say("Most PCs", "多数电脑") },
    { os: "windows", suffix: "windows-arm64-setup.exe", label: say("ARM64 installer", "ARM64 安装程序"), hint: say("Snapdragon and other ARM PCs", "骁龙等 ARM 电脑") },
    { os: "linux", suffix: "linux-amd64.deb", label: "amd64 DEB", hint: say("Most PCs and servers", "多数电脑和服务器") },
    { os: "linux", suffix: "linux-arm64.deb", label: "arm64 DEB", hint: say("ARM servers and boards", "ARM 服务器和开发板") },
    { os: "macos", suffix: "macos-arm64.dmg", label: "Apple Silicon DMG", hint: say("M-series Macs", "M 系列芯片") },
    { os: "macos", suffix: "macos-x64.dmg", label: "Intel DMG", hint: say("Intel Macs", "Intel 芯片") },
    { os: "android", suffix: "android-arm64.apk", label: "ARM64 APK", hint: say("Most phones and tablets", "多数手机和平板") },
    { os: "android", suffix: "android-armv7.apk", label: "ARMv7 APK", hint: say("Older 32-bit devices", "较旧的 32 位设备") },
    { os: "android", suffix: "android-x64.apk", label: "x86_64 APK", hint: say("Emulators and x86 devices", "模拟器和 x86 设备") },
  ];
  const snapshot = JSON.parse(document.getElementById("release-data").textContent);
  const releases = Object.fromEntries(Object.entries(snapshot.downloads).map(([channel,record]) =>
    [channel, {tag:record.tag, published:record.published, sizes:PACKAGES.map(item => record.sizes[item.suffix])}]));
  let channel = location.hash === "#preview" ? "preview" : "stable";
  let state = "loading";
  let previewIsStable = false;

  const mine = platform && $(`[data-os="${platform}"]`);
  if (mine) {
    mine.parentNode.prepend(mine);
    $("[data-yours]", mine).textContent = say("Your device", "你的设备");
  }

  /* A release counts only when all nine installers and nine checksum files are public. */
  function accept(record, prerelease) {
    if (!record || record.draft !== false || record.prerelease !== prerelease || typeof record.tag_name !== "string" ||
        !/^v\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(record.tag_name) || !Array.isArray(record.assets)) return null;
    const sizes = [];
    for (const item of PACKAGES) {
      const name = `Clash-of-Rust-${record.tag_name.slice(1)}-${item.suffix}`;
      const find = file => record.assets.find(asset => asset && asset.name === file && asset.state === "uploaded" &&
        Number.isSafeInteger(asset.size) && asset.size > 0 &&
        asset.browser_download_url === `${WEB}/releases/download/${record.tag_name}/${file}`);
      const installer = find(name);
      if (!installer || !find(`${name}.sha256`)) return null;
      sizes.push(installer.size);
    }
    return { tag: record.tag_name, sizes, published: String(record.published_at || "") };
  }

  function render() {
    const active = document.activeElement;
    const focusOs = active?.closest("[data-os]")?.dataset.os;
    const focusIndex = focusOs ? [...$$("a", $(`[data-os="${focusOs}"] [data-pkgs]`))].indexOf(active) : -1;
    const release = releases[channel];
    const version = release.tag.slice(1);
    for (const button of $$("[data-channel]")) button.setAttribute("aria-pressed", String(button.dataset.channel === channel));
    $("[data-version]").textContent = version;
    $("[data-notes]").href = `${WEB}/releases/tag/${release.tag}`;
    $("[data-channel-note]").textContent = channel === "stable" ? say("For everyday use.", "日常使用选这个。")
      : previewIsStable ? say("No preview is newer than stable right now, so this is the stable release.", "目前没有比正式版更新的预发布版，这里给出的是正式版。")
      : say("Newer features with a narrower validation scope. Back up your settings first.", "新功能先行，验证范围较小。建议先备份配置。");
    $("[data-fetch-state]").textContent = state === "loading" ? say("Checking GitHub…", "正在与 GitHub 核对…")
      : state === "confirmed" ? say("Checked against GitHub just now.", "已与 GitHub 核对。")
      : say(`Could not fully verify GitHub. Snapshot: ${snapshot.checked}; it may be out of date.`, `暂未完整核对 GitHub，显示 ${snapshot.checked} 的列表，可能已过期。`);
    for (const list of $$("[data-pkgs]")) {
      const os = list.closest("[data-os]").dataset.os;
      const first = PACKAGES.findIndex(item => item.os === os);
      list.replaceChildren(...PACKAGES.map((item, index) => ({ item, index })).filter(entry => entry.item.os === os).map(({ item, index }) => {
        const file = `Clash-of-Rust-${version}-${item.suffix}`;
        const url = `${WEB}/releases/download/${release.tag}/${file}`;
        const row = document.createElement("li");
        const link = document.createElement("a");
        const size = document.createElement("span");
        const sum = document.createElement("a");
        const hint = document.createElement("em");
        link.className = index === first ? "btn" : "btn ghost";
        link.href = url;
        size.textContent = `${(release.sizes[index] / 1048576).toFixed(1)} MiB`;
        link.append(item.label, size);
        sum.className = "sum";
        sum.href = `${url}.sha256`;
        sum.textContent = say("SHA256 file", "SHA256 文件");
        sum.setAttribute("aria-label", `${item.label} SHA256`);
        hint.textContent = item.hint;
        row.append(link, sum, hint);
        return row;
      }));
    }
    if (focusIndex >= 0) $$('a', $(`[data-os="${focusOs}"] [data-pkgs]`))[focusIndex]?.focus({preventScroll:true});
  }

  for (const button of $$("[data-channel]")) {
    button.addEventListener("click", () => {
      channel = button.dataset.channel;
      try { history.replaceState(null, "", channel === "preview" ? "#preview" : location.pathname + location.search); } catch (_) { /* Embedded previews may not permit history changes. */ }
      render();
    });
  }
  window.addEventListener("hashchange", () => {
    if (location.hash === "#preview" || location.hash === "") { channel = location.hash ? "preview" : "stable"; render(); }
  });
  render();

  Promise.allSettled([github("/releases/latest"), github("/releases?per_page=15")]).then(([latest, recent]) => {
    const stable = latest.status === "fulfilled" && accept(latest.value, false);
    if (stable) releases.stable = stable;
    let confirmed = Boolean(stable);
    if (recent.status === "fulfilled" && Array.isArray(recent.value)) {
      const newest = recent.value.filter(record => record && record.draft === false && record.prerelease === true)
        .sort((first, second) => String(second.published_at || "").localeCompare(String(first.published_at || "")))[0];
      if (newest && (!stable || String(newest.published_at || "") > stable.published)) {
        const preview = accept(newest, true);
        if (preview) releases.preview = preview; else confirmed = false;
      } else if (stable) {
        /* A preview older than stable is no longer worth offering. */
        releases.preview = stable;
        previewIsStable = true;
      } else confirmed = false;
    } else confirmed = false;
    state = confirmed ? "confirmed" : "fallback";
    render();
  });
}
/* A user-started tour of genuine component screenshots. No network or proxy actions. */
for (const tour of $$("[data-demo]")) {
  const frames = $$("[data-demo-frame]", tour);
  const steps = $$("[data-demo-step]", tour);
  const play = $("[data-demo-play]", tour);
  let current = 0;
  let timer = null;
  function pause() {
    clearInterval(timer); timer = null;
    play.setAttribute("aria-pressed", "false");
    play.textContent = say("Play tour", "播放导览");
  }
  function show(index, announce = true) {
    current = index;
    frames.forEach((frame,i) => { frame.hidden = i !== current; });
    steps.forEach((button,i) => button.setAttribute("aria-pressed", String(i === current)));
    const status = $("[data-demo-status]",tour);
    status.setAttribute("aria-live", announce ? "polite" : "off");
    status.textContent = `${current+1} / ${frames.length} · ${steps[current].textContent}`;
  }
  steps.forEach((button,index) => button.addEventListener("click", () => { pause(); show(index); }));
  play.addEventListener("click", () => {
    if (timer !== null) { pause(); return; }
    play.textContent = say("Pause tour", "暂停导览"); play.setAttribute("aria-pressed", "true");
    show((current+1)%frames.length);
    timer = setInterval(() => show((current+1)%frames.length, false), 5000);
  });
  document.addEventListener("visibilitychange", () => { if (document.hidden) pause(); });
  window.addEventListener("pagehide", pause);
  if ("IntersectionObserver" in window) new IntersectionObserver(entries => {
    if (!entries[0].isIntersecting) pause();
  }).observe(tour);
  show(0, false);
}
