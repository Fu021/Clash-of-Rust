"use strict";

if (process.env.GITHUB_ACTIONS !== "true") {
  throw new Error("Run this browser check in GitHub Actions; use static checks on resource-limited hosts.");
}

const assert = require("node:assert/strict");
const fs = require("node:fs");
const http = require("node:http");
const path = require("node:path");
const { chromium } = require("playwright");

const root = path.join(__dirname, "dist");
const output = path.join(__dirname, "browser-report");
const releases = JSON.parse(fs.readFileSync(path.join(__dirname, "src/content/releases.json"), "utf8"));
const mime = {
  ".html": "text/html; charset=utf-8", ".css": "text/css", ".js": "text/javascript",
  ".woff2": "font/woff2", ".webp": "image/webp", ".png": "image/png",
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
    for (const image of document.images) image.loading = "eager";
    await Promise.all([...document.images].map(image => image.decode()));
  });
}

async function checkPage(context, origin, route, width) {
  const page = await context.newPage();
  page.setDefaultTimeout(15000);
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
      broken: [...document.images].filter(image => !image.naturalWidth).length,
    }));
    assert.ok(state.heading && !state.broken && state.width <= state.viewport + 1,
      `${width} ${route}: ${JSON.stringify(state)}`);
    if (route.endsWith("/download")) {
      for (const channel of ["preview", "stable"]) {
        await page.locator(`[data-channel="${channel}"]`).click();
        assert.equal(await page.locator("[data-pkgs] a").count(), 18, `${channel} downloads`);
        assert.equal((await page.locator("[data-version]").textContent()).trim(),
          releases.downloads[channel].tag.slice(1));
      }
    }
    if (route.endsWith("/guide")) {
      await page.locator('[data-guide-select="tui"]').click();
      assert.equal(await page.locator("#tui").isVisible(), true);
      assert.equal(await page.locator("#gui").isVisible(), false);
      assert.equal(await page.locator('[data-guide-for="gui"]:visible').count(), 0);
      assert.ok((await page.locator("[data-set-lang]").getAttribute("href")).endsWith("#tui"));
      if (route === "/guide") {
        await page.screenshot({ path: path.join(output, `${width}-guide-tui.png`), fullPage: true });
      }
      await page.locator('[data-guide-select="gui"]').click();
      await settleImages(page);
      assert.equal(await page.locator("#gui").isVisible(), true);
      assert.equal(await page.locator("#tui").isVisible(), false);
      await page.goto(origin + route + "#tui-pages", { waitUntil: "load" });
      assert.equal(await page.locator("#tui-pages").isVisible(), true);
      await page.locator('[data-guide-select="gui"]').click();
    }
    if (route === "/" || route === "/en/") {
      const shots = page.locator("[data-demo-frame]");
      for (let index = 0; index < await shots.count(); index++) {
        await page.locator(`[data-demo-step="${index}"]`).click();
        assert.equal(await shots.nth(index).isVisible(), true);
        assert.equal(await page.locator("[data-demo-frame]:visible").count(), 1);
      }
      await page.locator('[data-demo-step="0"]').click();
    }
    assert.deepEqual(errors, [], `${width} ${route}`);
    if (["/", "/download", "/tools", "/guide", "/releases"].includes(route)) {
      const name = route === "/" ? "home" : route.slice(1);
      await page.screenshot({ path: path.join(output, `${width}-${name}.png`), fullPage: true });
    }
    report.pages.push({ route, width, status: "passed" });
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
            return { scene: scene.width, picture: picture.width, left: picture.left - scene.left };
          }, zoom);
          assert.ok(geometry.scene > 0 && Math.abs(geometry.scene - geometry.picture) < 2 &&
            Math.abs(geometry.left) < 2, `Hero resize ${route} ${width}px ${zoom}: ${JSON.stringify(geometry)}`);
        }
      }
      await page.evaluate(() => { document.documentElement.style.zoom = ""; });
    }
    report.hero_resize_and_zoom = "passed";
  } finally {
    await page.close();
  }
}

async function run() {
  fs.mkdirSync(output, { recursive: true });
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  const origin = `http://127.0.0.1:${server.address().port}`;
  browser = await chromium.launch({ headless: true });
  for (const viewport of [{ width: 1280, height: 800 }, { width: 360, height: 640 }]) {
    const context = await browser.newContext({ viewport, reducedMotion: "reduce", serviceWorkers: "block" });
    await context.route("**/*", route => new URL(route.request().url()).origin === origin
      ? route.continue() : route.abort());
    try {
      for (const route of ["/", "/tools", "/guide", "/download", "/releases",
        "/en/", "/en/tools", "/en/guide", "/en/download", "/en/releases"]) {
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
  const context = await browser.newContext({
    viewport: { width: 360, height: 640 }, javaScriptEnabled: false, serviceWorkers: "block",
  });
  try {
    const page = await context.newPage();
    await page.goto(origin + "/download", { waitUntil: "load", timeout: 20000 });
    assert.equal(await page.locator("[data-pkgs] a").count(), 18);
    assert.equal(await page.locator('[data-channel="preview"]').isVisible(), false);
    await page.goto(origin + "/guide", { waitUntil: "load", timeout: 20000 });
    assert.equal(await page.locator("#gui").isVisible(), true);
    assert.equal(await page.locator("#tui").isVisible(), true);
    await page.goto(origin + "/en/guide", { waitUntil: "load", timeout: 20000 });
    assert.equal(await page.locator("html").getAttribute("lang"), "en");
    assert.equal(await page.locator("#tui").isVisible(), true);
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
