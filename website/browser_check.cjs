"use strict";

if (process.env.GITHUB_ACTIONS !== "true") {
  throw new Error("Run this browser check in GitHub Actions; use static checks on resource-limited hosts.");
}

const assert = require("node:assert/strict");
const fs = require("node:fs");
const http = require("node:http");
const path = require("node:path");
const { execFileSync } = require("node:child_process");
const { chromium } = require("playwright");

execFileSync("python3", ["-B", path.join(__dirname, "build.py")], { stdio: "inherit" });
const root = path.join(__dirname, "dist");
const output = path.join(__dirname, "browser-report");
const releases = JSON.parse(fs.readFileSync(path.join(__dirname, "src/content/releases.json"), "utf8"));
const mime = {
  ".html": "text/html; charset=utf-8", ".css": "text/css", ".js": "text/javascript",
  ".woff2": "font/woff2", ".webp": "image/webp", ".png": "image/png",
  ".svg": "image/svg+xml",
};
const headers = {};
for (const line of fs.readFileSync(path.join(root, "_headers"), "utf8").split("\n")) {
  if (!line.startsWith("  ")) continue;
  const separator = line.indexOf(":");
  if (separator > 0) headers[line.slice(0, separator).trim()] = line.slice(separator + 1).trim();
}
const server = http.createServer((request, response) => {
  let target;
  try {
    const route = decodeURIComponent(new URL(request.url, "http://localhost").pathname);
    const pathname = route.endsWith("/") ? route + "index.html"
      : path.extname(route) ? route : route + ".html";
    target = path.resolve(root, "." + pathname);
    if (!target.startsWith(root + path.sep) || !fs.statSync(target).isFile()) throw new Error("Not a page");
  } catch {
    response.writeHead(404).end();
    return;
  }
  response.writeHead(200, { ...headers, "Content-Type": mime[path.extname(target)] || "text/plain" });
  const stream = fs.createReadStream(target);
  stream.on("error", error => response.destroy(error));
  stream.pipe(response);
});

const report = { status: "running", pages: [] };
let browser;
let stopping = false;

async function stop() {
  if (stopping) return;
  stopping = true;
  if (browser) await browser.close();
  await new Promise(resolve => {
    server.close(resolve);
    server.closeAllConnections();
  });
}

async function settleImages(page) {
  await page.evaluate(async () => {
    await document.fonts.ready;
    const images = [...document.images].filter(image => image.hasAttribute("src"));
    for (const image of images) image.loading = "eager";
    await Promise.all(images.map(image => image.decode()));
  });
}

async function checkPage(context, origin, route, width) {
  const page = await context.newPage();
  page.setDefaultTimeout(15000);
  const activate = locator => width === 360 ? locator.tap() : locator.click();
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("response", response => {
    if (new URL(response.url()).origin === origin && response.status() >= 400) {
      errors.push(`${response.status()} ${new URL(response.url()).pathname}`);
    }
  });
  try {
    const response = await page.goto(origin + route, { waitUntil: "load", timeout: 20000 });
    assert.equal(response.status(), 200, route);
    await settleImages(page);
    const state = await page.evaluate(() => ({
      width: document.documentElement.scrollWidth,
      viewport: innerWidth,
      heading: document.querySelector("h1")?.textContent.trim(),
      broken: [...document.images].filter(image => image.hasAttribute("src") && !image.naturalWidth).length,
      illustration: document.querySelector(".scene img")?.naturalWidth,
    }));
    assert.ok(state.heading && state.illustration && !state.broken && state.width <= state.viewport + 1,
      `${width} ${route}: ${JSON.stringify(state)}`);
    const menu = page.locator(".mobile-menu");
    if (width <= 1000) {
      assert.equal(await menu.locator("nav").isVisible(), false);
      await activate(menu.locator("summary"));
      assert.equal(await menu.locator("nav").isVisible(), true);
      const overflow = await page.evaluate(() => document.documentElement.scrollWidth > innerWidth + 1);
      assert.equal(overflow, false, `Open menu overflow: ${route}`);
      await page.keyboard.press("Escape");
      assert.equal(await menu.locator("nav").isVisible(), false);
    }
    const toc = page.locator("[data-toc]");
    if (await toc.count()) {
      if (width <= 900) {
        assert.equal(await toc.locator("nav").isVisible(), false);
        await activate(toc.locator("summary"));
      }
      assert.equal(await toc.locator("nav").isVisible(), true);
      await activate(toc.locator("a").first());
      if (width <= 900) assert.equal(await toc.locator("nav").isVisible(), false);
      await page.evaluate(() => scrollTo(0, 0));
    }
    if (route.endsWith("/download")) {
      for (const channel of ["preview", "stable"]) {
        await activate(page.locator(`[data-channel="${channel}"]`));
        assert.equal(await page.locator("[data-pkgs] a").count(), 18, `${channel} downloads`);
        assert.equal((await page.locator("[data-version]").textContent()).trim(),
          releases.downloads[channel].tag.slice(1));
      }
    }
    if (["guide", "tui", "cli"].some(name => route.endsWith("/" + name))) {
      assert.equal(await page.locator(".manual-nav a").count(), 3);
      assert.equal(await page.locator(".manual-nav [aria-current]").count(), 1);
    }
    const imageLink = page.locator("[data-enlarge]").first();
    if (await imageLink.count()) {
      await activate(imageLink);
      assert.equal(await page.locator("[data-image-view]").isVisible(), true);
      const enlarged = page.locator("[data-image-view] img");
      await enlarged.evaluate(image => image.decode());
      await page.keyboard.press("Escape");
      assert.equal(await page.locator("[data-image-view]").isVisible(), false);
      assert.equal(await imageLink.evaluate(link => link === document.activeElement), true);
      await activate(imageLink);
      await activate(page.locator("[data-image-close]"));
      assert.equal(await page.locator("[data-image-view]").isVisible(), false);
    }
    if (route === "/" || route === "/en/") {
      const shots = page.locator("[data-demo-frame]");
      const height = await page.locator(".demo-view").evaluate(view => view.getBoundingClientRect().height);
      for (let index = 0; index < await shots.count(); index++) {
        await activate(page.locator(`[data-demo-step="${index}"]`));
        assert.equal(await shots.nth(index).isVisible(), true);
        assert.equal(await page.locator("[data-demo-frame]:visible").count(), 1);
        assert.ok(Math.abs(await page.locator(".demo-view").evaluate(view => view.getBoundingClientRect().height) - height) < 2,
          `Tour height changed: ${route} frame ${index}`);
      }
      await activate(page.locator('[data-demo-step="0"]'));
    }
    assert.deepEqual(errors, [], `${width} ${route}`);
    if (width !== 800 || !route.startsWith("/en/")) {
      const name = route === "/" ? "home" : route === "/en/" ? "en-home" : route.slice(1).replaceAll("/", "-");
      await page.evaluate(() => { document.activeElement?.blur(); scrollTo(0, 0); });
      await page.screenshot({ path: path.join(output, `${width}-${name}.png`), fullPage: true });
    }
    report.pages.push({ route, width, input: width === 360 ? "touch" : "mouse", status: "passed" });
    console.log(`Verified ${width}px ${route}`);
  } finally {
    await page.close();
  }
}

async function checkHeroResizing(context, origin) {
  const page = await context.newPage();
  try {
    for (const route of ["/", "/en/"]) {
      await page.goto(origin + route, { waitUntil: "load" });
      for (const width of [1920, 1280, 800, 360, 1920]) {
        await page.setViewportSize({ width, height: 900 });
        for (const zoom of [.8, 1, 1.25, 1.5, 2]) {
          const geometry = await page.evaluate(zoom => {
            document.documentElement.style.zoom = String(zoom);
            const scene = document.querySelector(".hero .scene").getBoundingClientRect();
            const picture = document.querySelector(".hero .scene img").getBoundingClientRect();
            const image = document.querySelector(".hero .scene img");
            return { scene: scene.width, picture: picture.width, left: picture.left - scene.left,
              ratio: picture.width / picture.height, original: image.naturalWidth / image.naturalHeight,
              overflow: document.documentElement.scrollWidth > innerWidth + 1 };
          }, zoom);
          assert.ok(geometry.scene > 0 && Math.abs(geometry.scene - geometry.picture) < 2 &&
            Math.abs(geometry.left) < 2 && Math.abs(geometry.ratio - geometry.original) < .01 && !geometry.overflow,
            `Hero resize ${route} ${width}px ${zoom}: ${JSON.stringify(geometry)}`);
        }
      }
      await page.evaluate(() => { document.documentElement.style.zoom = ""; });
    }
    report.hero_resize_and_zoom = "passed";
  } finally {
    await page.close();
  }
}

async function checkTour(origin) {
  const context = await browser.newContext({ viewport: { width: 1280, height: 800 }, serviceWorkers: "block" });
  await context.route("**/*", route => new URL(route.request().url()).origin === origin ? route.continue() : route.abort());
  const page = await context.newPage();
  const active = () => page.locator('[data-demo-step][aria-pressed="true"]').getAttribute("data-demo-step");
  try {
    await page.goto(origin, { waitUntil: "load" });
    await page.locator("[data-demo]").scrollIntoViewIfNeeded();
    await page.mouse.move(1, 1);
    const first = await active();
    await page.waitForFunction(previous => document.querySelector('[data-demo-step][aria-pressed="true"]').dataset.demoStep !== previous,
      first, { timeout: 7000 });
    await page.locator("[data-demo-toggle]").focus();
    const focused = await active();
    await page.waitForTimeout(3900);
    assert.equal(await active(), focused, "Tour moved while keyboard focus was inside");
    await page.locator(".brand").first().evaluate(link => link.focus({ preventScroll: true }));
    await page.waitForFunction(previous => document.querySelector('[data-demo-step][aria-pressed="true"]').dataset.demoStep !== previous,
      focused, { timeout: 7000 });
    await page.locator("[data-demo]").hover();
    const hovering = await active();
    await page.waitForTimeout(3900);
    assert.equal(await active(), hovering, "Tour moved while hovered");
    await page.mouse.move(1, 1);
    await page.locator('[data-demo-step="2"]').click();
    assert.equal(await page.locator("[data-demo-toggle]").getAttribute("aria-label"), "继续自动切换");
    await page.locator(".brand").first().evaluate(link => link.focus({ preventScroll: true }));
    await page.mouse.move(1, 1);
    await page.waitForTimeout(3900);
    assert.equal(await active(), "2", "Manual selection did not pause");
    await page.locator("[data-demo-toggle]").click();
    assert.equal(await page.locator("[data-demo-toggle]").getAttribute("aria-label"), "暂停自动切换");
    await page.locator(".brand").first().evaluate(link => link.focus({ preventScroll: true }));
    await page.mouse.move(1, 1);
    await page.waitForFunction(() => document.querySelector('[data-demo-step][aria-pressed="true"]').dataset.demoStep !== "2",
      null, { timeout: 7000 });
    await page.locator('[data-demo-frame]:visible [data-enlarge]').click();
    const enlarged = await active();
    assert.equal(await page.locator("[data-image-view]").isVisible(), true);
    await page.mouse.move(1, 1);
    await page.waitForTimeout(3900);
    assert.equal(await active(), enlarged, "Tour moved while the enlarged image was open");
    await page.keyboard.press("Escape");
    assert.equal(await page.locator("[data-image-view]").isVisible(), false);
    assert.equal(await page.locator('[data-demo-frame]:visible [data-enlarge]').evaluate(link => link === document.activeElement), true);
    await page.emulateMedia({ reducedMotion: "reduce" });
    await page.waitForFunction(() => document.querySelector("[data-demo-toggle]").disabled);
    const reduced = await active();
    await page.waitForTimeout(3900);
    assert.equal(await active(), reduced, "Reduced motion did not stop autoplay");
    assert.equal(await page.locator("[data-demo-status]").getAttribute("aria-live"), "off");
    report.automatic_tour = "passed";
  } finally {
    await context.close();
  }
}

async function checkDownloadFallbacks(origin) {
  function record(channel) {
    const snapshot = releases.downloads[channel];
    const assets = Object.entries(snapshot.sizes).flatMap(([suffix, size]) => {
      const name = `Clash-of-Rust-${snapshot.tag.slice(1)}-${suffix}`;
      return [name, name + ".sha256"].map(file => ({ name: file, state: "uploaded", size: file === name ? size : 65,
        browser_download_url: `https://github.com/Fu021/Clash-of-Rust/releases/download/${snapshot.tag}/${file}` }));
    });
    return { tag_name: snapshot.tag, draft: false, prerelease: channel === "preview",
      published_at: snapshot.published, assets };
  }
  for (const mode of ["valid", "offline", "missing-checksum", "unsafe-url"]) {
    const context = await browser.newContext({ viewport: { width: 360, height: 640 }, serviceWorkers: "block" });
    await context.route("**/*", route => {
      const url = new URL(route.request().url());
      if (url.origin === origin) return route.continue();
      if (url.hostname !== "api.github.com" || mode === "offline") return route.abort();
      const stable = record("stable");
      const preview = record("preview");
      if (mode === "missing-checksum") preview.assets.pop();
      if (mode === "unsafe-url") preview.assets[0].browser_download_url = "https://example.invalid/installer";
      const json = url.pathname.endsWith("/releases/latest") ? stable
        : url.pathname.endsWith("/releases") ? [preview, stable] : { stargazers_count: 1 };
      return route.fulfill({ status: 200, json });
    });
    try {
      const page = await context.newPage();
      await page.goto(origin + "/download", { waitUntil: "load" });
      const expected = mode === "valid" ? "已与 GitHub 核对。" : `显示 ${releases.checked} 的列表`;
      await page.waitForFunction(text => document.querySelector("[data-fetch-state]").textContent.includes(text),
        expected, { timeout: 10000 });
      await page.locator('[data-channel="preview"]').click();
      assert.equal(await page.locator("[data-pkgs] a").count(), 18);
      assert.equal(await page.locator('[data-pkgs] a[href*="example.invalid"]').count(), 0);
      assert.equal((await page.locator("[data-version]").textContent()).trim(), releases.downloads.preview.tag.slice(1));
    } finally {
      await context.close();
    }
  }
  report.download_fallbacks = "passed";
}

async function run() {
  fs.mkdirSync(output, { recursive: true });
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  const origin = `http://127.0.0.1:${server.address().port}`;
  browser = await chromium.launch({ headless: true });
  for (const viewport of [{ width: 1280, height: 800 }, { width: 800, height: 1280 }, { width: 360, height: 640 }]) {
    const context = await browser.newContext({
      viewport, reducedMotion: "reduce", serviceWorkers: "block",
      isMobile: viewport.width === 360, hasTouch: viewport.width === 360,
    });
    await context.route("**/*", route => new URL(route.request().url()).origin === origin
      ? route.continue() : route.abort());
    try {
      for (const route of ["/", "/start", "/guide", "/tui", "/cli", "/tools", "/download", "/releases",
        "/en/", "/en/start", "/en/guide", "/en/tui", "/en/cli", "/en/tools", "/en/download", "/en/releases"]) {
        await checkPage(context, origin, route, viewport.width);
      }
      if (viewport.width === 1280) {
        await checkHeroResizing(context, origin);
        const page = await context.newPage();
        await page.goto(origin, { waitUntil: "load" });
        assert.equal(await page.locator("html").getAttribute("lang"), "zh-CN");
        await page.locator(".top [data-set-lang]").click();
        assert.equal(await page.locator("html").getAttribute("lang"), "en");
        await page.locator(".top [data-set-lang]").click();
        assert.equal(await page.locator("html").getAttribute("lang"), "zh-CN");
        assert.equal(await page.locator(".hero-copy").evaluate(element => getComputedStyle(element).animationName), "none");
        report.language_and_reduced_motion = "passed";
        await page.close();
      }
    } finally {
      await context.close();
    }
  }
  await checkTour(origin);
  await checkDownloadFallbacks(origin);
  const context = await browser.newContext({
    viewport: { width: 360, height: 640 }, javaScriptEnabled: false, serviceWorkers: "block",
  });
  try {
    const page = await context.newPage();
    await context.route("**/*", route => new URL(route.request().url()).origin === origin ? route.continue() : route.abort());
    await page.goto(origin + "/download", { waitUntil: "load", timeout: 20000 });
    assert.equal(await page.locator("[data-pkgs] a").count(), 18);
    assert.equal(await page.locator('[data-channel="preview"]').isVisible(), false);
    for (const language of ["", "/en"]) {
      for (const name of ["start", "guide", "tui", "cli"]) {
        await page.goto(origin + language + "/" + name, { waitUntil: "load", timeout: 20000 });
        assert.equal(await page.locator("html").getAttribute("lang"), language ? "en" : "zh-CN");
        assert.equal(await page.locator("[data-toc] nav").isVisible(), true);
        assert.equal(await page.locator(".doc section").first().isVisible(), true);
      }
      await page.goto(origin + language + "/guide#tui-pages", { waitUntil: "load", timeout: 20000 });
      assert.equal(await page.locator("#tui-pages").isVisible(), true);
      assert.equal(await page.locator('#tui a[href="tui.html"]').count(), 1);
      await page.locator(".mobile-menu summary").click();
      assert.equal(await page.locator(".mobile-menu nav").isVisible(), true);
    }
    report.no_javascript = "passed";
  } finally {
    await context.close();
  }
  report.status = "passed";
}

const deadline = setTimeout(() => {
  report.status = "failed";
  report.error = "Browser checks exceeded the five-minute budget";
  fs.mkdirSync(output, { recursive: true });
  fs.writeFileSync(path.join(output, "report.json"), JSON.stringify(report, null, 2));
  console.error(report.error);
  stop().finally(() => process.exit(1));
}, 300000);

run().catch(error => {
  report.status = "failed";
  report.error = error.message;
  console.error(error);
  process.exitCode = 1;
}).finally(async () => {
  clearTimeout(deadline);
  fs.mkdirSync(output, { recursive: true });
  fs.writeFileSync(path.join(output, "report.json"), JSON.stringify(report, null, 2));
  await stop();
});
