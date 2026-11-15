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
    await page.evaluate(async () => {
      await document.fonts.ready;
      for (const image of document.images) image.loading = "eager";
      await Promise.all([...document.images].map(image => image.decode()));
    });
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
    assert.deepEqual(errors, [], `${width} ${route}`);
    if (["/zh/", "/zh/download", "/zh/tools"].includes(route)) {
      const name = route === "/zh/" ? "home" : route.split("/").pop();
      await page.screenshot({ path: path.join(output, `${width}-${name}.png`), fullPage: true });
    }
    report.pages.push({ route, width, status: "passed" });
    console.log(`Verified ${width}px ${route}`);
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
        "/zh/", "/zh/tools", "/zh/guide", "/zh/download", "/zh/releases"]) {
        await checkPage(context, origin, route, viewport.width);
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
    await page.goto(origin + "/zh/download", { waitUntil: "load", timeout: 20000 });
    assert.equal(await page.locator("[data-pkgs] a").count(), 18);
    assert.equal(await page.locator('[data-channel="preview"]').isVisible(), false);
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
