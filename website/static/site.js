import { releases } from "./releases.js";

const repository = "https://github.com/Fu021/Clash-of-Rust";
const dialog = document.querySelector(".image-dialog");
const fullImage = document.querySelector("[data-full-image]");
const imageTitle = document.querySelector("[data-image-title]");
const originalImage = document.querySelector("[data-original-image]");

for (const button of document.querySelectorAll("[data-zoom]")) {
  button.addEventListener("click", () => {
    const image = button.querySelector("img");
    fullImage.src = image.src;
    fullImage.alt = image.alt;
    imageTitle.textContent = button.dataset.zoom;
    originalImage.href = image.src;
    dialog.showModal();
  });
}
document.querySelector("[data-close-image]").addEventListener("click", () => dialog.close());
dialog.addEventListener("click", event => {
  if (event.target !== dialog) return;
  const bounds = dialog.getBoundingClientRect();
  if (event.clientX < bounds.left || event.clientX > bounds.right ||
      event.clientY < bounds.top || event.clientY > bounds.bottom) dialog.close();
});

const featureLinks = [...document.querySelectorAll(".feature-index a")];
if (featureLinks.length) {
  const updateFeatureIndex = () => {
    let current = featureLinks[0];
    for (const link of featureLinks) {
      const section = document.getElementById(link.hash.slice(1));
      if (section.getBoundingClientRect().top <= 180) current = link;
    }
    if (window.scrollY + window.innerHeight >= document.documentElement.scrollHeight - 4) {
      current = featureLinks[featureLinks.length - 1];
    }
    for (const link of featureLinks) {
      if (link === current) link.setAttribute("aria-current", "location");
      else link.removeAttribute("aria-current");
    }
  };
  window.addEventListener("scroll", updateFeatureIndex, { passive: true });
  updateFeatureIndex();
}

const channelButtons = [...document.querySelectorAll("[data-channel]")];
if (channelButtons.length) {
  const snapshots = { stable: releases.stable, preview: releases.preview };
  let channel = location.hash === "#preview" ? "preview" : "stable";
  let selection = "all";
  const variants = [...document.querySelectorAll("[data-suffix]")].map(row => row.dataset.suffix);
  const status = document.querySelector("[data-release-status]");

  function verify(record, prerelease) {
    if (!record || typeof record !== "object" || record.draft !== false ||
        record.prerelease !== prerelease || typeof record.tag_name !== "string" ||
        !/^v\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(record.tag_name) ||
        record.html_url !== `${repository}/releases/tag/${record.tag_name}` ||
        !Array.isArray(record.assets) || record.assets.some(asset =>
          !asset || typeof asset !== "object" || typeof asset.name !== "string")) return false;
    const names = record.assets.map(asset => asset.name);
    if (new Set(names).size !== names.length) return false;
    return variants.every(suffix => {
      const name = `Clash-of-Rust-${record.tag_name.slice(1)}-${suffix}`;
      return [name, `${name}.sha256`].every(filename => {
        const asset = record.assets.find(entry => entry.name === filename);
        return asset && asset.state === "uploaded" && Number.isSafeInteger(asset.size) &&
          asset.size > 0 &&
          asset.browser_download_url === `${repository}/releases/download/${record.tag_name}/${filename}`;
      });
    });
  }

  function render() {
    const record = snapshots[channel];
    const assets = new Map(record.assets.map(asset => [asset.name, asset]));
    for (const button of channelButtons) {
      button.setAttribute("aria-pressed", String(button.dataset.channel === channel));
    }
    document.querySelector("[data-release-label]").textContent = channel === "stable" ? "正式版" : "预发布版";
    document.querySelector("[data-release-version]").textContent = record.tag_name.slice(1);
    document.querySelector("[data-release-notes]").href = record.html_url;
    document.querySelector("[data-release-description]").textContent = channel === "stable"
      ? "推荐日常使用。安装包已包含内核与 Geo 数据。"
      : "率先体验网络工具箱与 TUI 等新功能。使用前建议备份配置。";
    for (const row of document.querySelectorAll("[data-suffix]")) {
      const name = `Clash-of-Rust-${record.tag_name.slice(1)}-${row.dataset.suffix}`;
      const asset = assets.get(name);
      row.querySelector("[data-download]").href = asset.browser_download_url;
      row.querySelector("[data-size]").textContent = `${(asset.size / 1048576).toFixed(1)} MiB`;
      row.querySelector("[data-checksum]").href = assets.get(`${name}.sha256`).browser_download_url;
    }
    for (const button of document.querySelectorAll("[data-filter]")) {
      button.setAttribute("aria-pressed", String(button.dataset.filter === selection));
    }
    for (const card of document.querySelectorAll("[data-platform]")) {
      card.hidden = selection !== "all" && card.dataset.platform !== selection;
    }
  }

  for (const button of channelButtons) {
    button.addEventListener("click", () => {
      channel = button.dataset.channel;
      history.replaceState(null, "", `#${channel}`);
      render();
    });
  }
  for (const button of document.querySelectorAll("[data-filter]")) {
    button.addEventListener("click", () => {
      selection = button.dataset.filter;
      render();
    });
  }
  window.addEventListener("hashchange", () => {
    channel = location.hash === "#preview" ? "preview" : "stable";
    render();
  });
  render();
  for (const control of document.querySelectorAll("[data-interactive]")) control.hidden = false;
  document.documentElement.dataset.downloadsReady = "true";

  async function fetchRelease(path) {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), 5000);
    try {
      const response = await fetch(`https://api.github.com/repos/Fu021/Clash-of-Rust/${path}`, {
        signal: controller.signal,
        credentials: "omit",
        headers: { Accept: "application/vnd.github+json" },
        referrerPolicy: "no-referrer",
      });
      if (!response.ok) throw new Error("Release metadata unavailable");
      return await response.json();
    } finally {
      clearTimeout(timer);
    }
  }

  const results = await Promise.allSettled([
    fetchRelease("releases/latest"),
    fetchRelease("releases?per_page=10"),
  ]);
  let verified = 0;
  if (results[0].status === "fulfilled" && verify(results[0].value, false)) {
    snapshots.stable = results[0].value;
    verified += 1;
  }
  if (results[1].status === "fulfilled" && Array.isArray(results[1].value)) {
    const candidates = results[1].value
      .filter(record => verify(record, true) && typeof record.published_at === "string")
      .sort((first, second) => second.published_at.localeCompare(first.published_at));
    if (candidates.length) {
      snapshots.preview = candidates[0];
      verified += 1;
    }
  }
  render();
  status.textContent = verified === 2
    ? "已载入 GitHub 的公开发行信息。"
    : "暂时无法确认最新发行，当前保留可用下载列表。";
  document.documentElement.dataset.releaseFetch = verified === 2 ? "complete" : "fallback";
}
