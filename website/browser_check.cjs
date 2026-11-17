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
  await page.waitForFunction(() => !document.documentElement.classList.contains("theme-loading"));
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
      const name = route.split("/").filter(Boolean).pop();
      const current = menu.locator('a[aria-current="page"]');
      if (name && name !== "en") {
        assert.equal(await current.count(), 1, `Current mobile navigation: ${route}`);
        assert.equal(await current.getAttribute("href"), `${["tui", "cli"].includes(name) ? "guide" : name}.html`);
        assert.notEqual(await current.evaluate(link => getComputedStyle(link).boxShadow), "none",
          `Missing current-page underline: ${route}`);
      } else {
        assert.equal(await current.count(), 0);
      }
      await page.keyboard.press("Escape");
      await page.waitForFunction(() => !document.querySelector(".mobile-menu").open);
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
      assert.equal(await page.locator("[data-channel]").count(), 0);
      assert.equal(await page.locator("[data-pkgs] a").count(), 18, "Stable downloads");
      assert.equal((await page.locator("[data-version]").textContent()).trim(),
        releases.downloads.stable.tag.slice(1));
    }
    if (["guide", "tui", "cli"].some(name => route.endsWith("/" + name))) {
      assert.equal(await page.locator(".manual-nav a").count(), 3);
      assert.equal(await page.locator(".manual-nav [aria-current]").count(), 1);
    }
    for (const details of await page.locator(".benchmark-details").all()) {
      assert.equal(await details.locator(".benchmark-body").isVisible(), false);
      await activate(details.locator("summary"));
      assert.equal(await details.locator(".benchmark-body").isVisible(), true);
      await activate(details.locator("summary"));
      assert.equal(await details.locator(".benchmark-body").isVisible(), false);
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
      assert.equal(await page.locator("#what + #cli + #features").count(), 1,
        "Terminal examples must immediately follow the desktop tour");
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
    const overflowedTerminals = await page.locator(".term, .terminal-capture").evaluateAll(terminals =>
      terminals.filter(terminal => terminal.scrollWidth > terminal.clientWidth + 1).length);
    assert.equal(overflowedTerminals, 0, `Horizontally scrolling terminal: ${width} ${route}`);
    if (width !== 800 || !route.startsWith("/en/")) {
      const name = route === "/" ? "home" : route === "/en/" ? "en-home" : route.slice(1).replaceAll("/", "-");
      for (const theme of width === 800 ? ["light"] : ["light", "dark"]) {
        await page.evaluate(theme => {
          window.siteTheme.setMode(theme);
          document.activeElement?.blur();
          scrollTo(0, 0);
        }, theme);
        await settleImages(page);
        assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth + 1), false,
          `Theme overflow: ${width} ${route} ${theme}`);
        const colors = await page.locator("body").evaluate(body => {
          const style = getComputedStyle(body);
          return { ink: style.color, paper: style.backgroundColor };
        });
        assert.notEqual(colors.ink, colors.paper, `Unreadable theme: ${route} ${theme}`);
        await page.screenshot({ path: path.join(output, `${width}-${name}${theme === "dark" ? "-dark" : ""}.png`), fullPage: true });
      }
      await page.evaluate(() => window.siteTheme.setMode("light"));
    }
    assert.deepEqual(errors, [], `${width} ${route}`);
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
      await page.evaluate(() => document.fonts.ready);
      for (const width of [1920, 1280, 800, 641, 640, 360, 320, 1920]) {
        await page.setViewportSize({ width, height: 900 });
        for (const zoom of [.8, 1, 1.25, 1.5, 2]) {
          await page.evaluate(zoom => {
            document.documentElement.style.zoom = String(zoom);
            const image = document.querySelector(".hero .scene img");
            image.getBoundingClientRect();
          }, zoom);
          await page.waitForFunction(() => {
            const image = document.querySelector(".hero .scene img");
            return image.complete && image.naturalWidth > 0 && image.naturalHeight > 0;
          });
          const geometry = await page.evaluate(() => {
            const image = document.querySelector(".hero .scene img");
            const scene = document.querySelector(".hero .scene").getBoundingClientRect();
            const picture = image.getBoundingClientRect();
            const tags = [...document.querySelectorAll(".hero .tag")].map(tag => {
              const box = tag.getBoundingClientRect();
              const description = tag.querySelector("span");
              return { width: box.width, height: box.height, left: box.left, right: box.right,
                top: box.top, bottom: box.bottom, description: description.innerText.trim(),
                descriptionHeight: description.getBoundingClientRect().height };
            });
            return { scene: scene.width, picture: picture.width, left: picture.left - scene.left,
              ratio: picture.width / picture.height, original: image.naturalWidth / image.naturalHeight,
              sceneTop: scene.top, sceneBottom: scene.bottom, tags,
              mask: getComputedStyle(document.querySelector(".hero .scene")).maskImage,
              overflow: document.documentElement.scrollWidth > innerWidth + 1 };
          });
          assert.ok(geometry.scene > 0 && Math.abs(geometry.scene - geometry.picture) < 2 &&
            Math.abs(geometry.left) < 2 && Math.abs(geometry.ratio - geometry.original) < .01 && !geometry.overflow,
            `Hero resize ${route} ${width}px ${zoom}: ${JSON.stringify(geometry)}`);
          assert.equal(geometry.tags.length, 3, `Hero routing labels: ${route}`);
          assert.equal(geometry.mask, "none", `Hero routing labels masked: ${route}`);
          for (const tag of geometry.tags) {
            assert.ok(tag.width > 0 && tag.height > 0 && tag.description && tag.descriptionHeight > 0 &&
              tag.left >= -1 && tag.right <= width + 1 && tag.top >= geometry.sceneTop - 1 &&
              tag.bottom <= geometry.sceneBottom + 1,
              `Hero routing label ${route} ${width}px ${zoom}: ${JSON.stringify(tag)}`);
          }
          for (let index = 0; index < geometry.tags.length; index++) {
            for (const other of geometry.tags.slice(index + 1)) {
              const tag = geometry.tags[index];
              assert.ok(tag.right <= other.left || other.right <= tag.left ||
                tag.bottom <= other.top || other.bottom <= tag.top,
                `Hero routing labels overlap ${route} ${width}px ${zoom}: ${JSON.stringify(geometry.tags)}`);
            }
          }
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
    await page.waitForFunction(previous => document.querySelector('[data-demo-step][aria-pressed="true"]').dataset.demoStep !== previous,
      hovering, { timeout: 5000 });
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
  const latestTag = releases.downloads.stable.tag.replace(/^v(\d+)\.(\d+)\.(\d+)$/, (_, major, minor, patch) =>
    `v${major}.${minor}.${Number(patch) + 1}`);
  function record(tag = latestTag) {
    const snapshot = releases.downloads.stable;
    const assets = Object.entries(snapshot.sizes).flatMap(([suffix, size]) => {
      const name = `Clash-of-Rust-${tag.slice(1)}-${suffix}`;
      return [name, name + ".sha256"].map(file => ({ name: file, state: "uploaded", size: file === name ? size : 65,
        browser_download_url: `https://github.com/Fu021/Clash-of-Rust/releases/download/${tag}/${file}` }));
    });
    return { tag_name: tag, draft: false, prerelease: false, published_at: snapshot.published, assets };
  }
  for (const language of ["", "/en"]) {
    for (const mode of ["valid", "offline", "missing-checksum", "unsafe-url", "prerelease", "prerelease-tag", "draft"]) {
      const context = await browser.newContext({ viewport: { width: 360, height: 640 }, serviceWorkers: "block" });
      const releaseRequests = [];
      await context.route("**/*", route => {
        const url = new URL(route.request().url());
        if (url.origin === origin) return route.continue();
        if (url.pathname.includes("/releases")) releaseRequests.push(url.pathname + url.search);
        if (url.hostname !== "api.github.com" || mode === "offline") return route.abort();
        const stable = record(mode.startsWith("prerelease") ? latestTag + "-test.1" : latestTag);
        if (mode === "missing-checksum") stable.assets.pop();
        if (mode === "unsafe-url") stable.assets[0].browser_download_url = "https://example.invalid/installer";
        if (mode === "prerelease") stable.prerelease = true;
        if (mode === "draft") stable.draft = true;
        const json = url.pathname.endsWith("/releases/latest") ? stable
          : { stargazers_count: 1 };
        return route.fulfill({ status: 200, json });
      });
      try {
        const page = await context.newPage();
        await page.goto(origin + language + "/download#preview", { waitUntil: "load" });
        const expected = mode === "valid" ? "confirmed" : "fallback";
        await page.waitForFunction(state => document.querySelector("[data-downloads]").dataset.releaseState === state,
          expected, { timeout: 10000 });
        assert.equal(await page.locator("[data-download-notice]").isVisible(), mode !== "valid");
        assert.equal(await page.locator("[data-channel]").count(), 0);
        assert.equal(await page.locator("[data-pkgs] a").count(), 18);
        assert.equal(await page.locator('[data-pkgs] a[href*="example.invalid"]').count(), 0);
        const expectedTag = mode === "valid" ? latestTag : releases.downloads.stable.tag;
        assert.equal((await page.locator("[data-version]").textContent()).trim(), expectedTag.slice(1));
        assert.deepEqual(releaseRequests, ["/repos/Fu021/Clash-of-Rust/releases/latest"]);
        for (const href of await page.locator("[data-pkgs] a").evaluateAll(links => links.map(link => link.href))) {
          assert.ok(href.startsWith(`https://github.com/Fu021/Clash-of-Rust/releases/download/${expectedTag}/`));
        }
      } finally {
        await context.close();
      }
    }
  }
  report.download_fallbacks = "passed";
}

async function checkBenchmarkDetails(origin) {
  for (const width of [1280, 393]) {
    for (const reducedMotion of ["no-preference", "reduce"]) {
      const context = await browser.newContext({
        viewport: { width, height: 852 }, reducedMotion, serviceWorkers: "block",
        isMobile: width === 393, hasTouch: width === 393,
      });
      await context.route("**/*", route => new URL(route.request().url()).origin === origin
        ? route.continue() : route.abort());
      await context.addInitScript(() => { localStorage.setItem("cor-site-theme", "light"); });
      try {
        const page = await context.newPage();
        page.setDefaultTimeout(15000);
        for (const route of ["/", "/en/"]) {
          await page.goto(origin + route, { waitUntil: "load" });
          await settleImages(page);
          const details = page.locator(".benchmark-details");
          const summary = details.locator("summary");
          const body = details.locator(".benchmark-body");
          assert.equal(await body.isVisible(), false);
          const closed = await details.evaluate(element => element.getBoundingClientRect().height);
          await summary.evaluate(element => {
            element.addEventListener("click", () => {
              window.benchmarkMotionObserved =
                element.parentElement.getAnimations({ subtree: true }).length > 0;
            });
          });
          await summary.focus();
          await page.keyboard.press("Enter");
          assert.equal(await page.evaluate(() => window.benchmarkMotionObserved), reducedMotion === "no-preference",
            `Benchmark opening motion: ${route} ${width} ${reducedMotion}`);
          await page.waitForFunction(() => !document.querySelector(".benchmark-details").getAnimations({ subtree: true }).length);
          assert.equal(await body.isVisible(), true);
          assert.equal(await body.locator("a").isVisible(), true);
          assert.ok(await details.evaluate(element => element.getBoundingClientRect().height) > closed);
          const spacing = await page.evaluate(() => {
            const memory = document.querySelector("#memory");
            const edge = getComputedStyle(memory, "::after");
            return {
              padding: memory.getBoundingClientRect().bottom -
                memory.querySelector("details").getBoundingClientRect().bottom,
              gap: document.querySelector("#what h2").getBoundingClientRect().top -
                memory.getBoundingClientRect().bottom,
              mask: edge.maskImage,
              reference: getComputedStyle(document.querySelector("#cli"), "::after").maskImage,
              overflow: document.documentElement.scrollWidth > innerWidth + 1,
            };
          });
          assert.ok(spacing.padding >= 31 && spacing.gap >= 48 && !spacing.overflow &&
            spacing.mask.includes("dither-out") && spacing.mask === spacing.reference,
            `Benchmark section transition: ${route} ${width} ${JSON.stringify(spacing)}`);
          if (route === "/" && reducedMotion === "no-preference") {
            await page.screenshot({ path: path.join(output, `${width}-home-benchmark-open.png`), fullPage: true });
          }
          await page.keyboard.press("Space");
          assert.equal(await page.evaluate(() => window.benchmarkMotionObserved),
            reducedMotion === "no-preference", `Benchmark closing motion: ${route} ${width}`);
          await page.waitForFunction(() => !document.querySelector(".benchmark-details").open);
          assert.equal(await body.isVisible(), false);
          await summary.evaluate(element => {
            element.click();
            element.click();
            element.click();
          });
          await page.waitForFunction(() => !document.querySelector(".benchmark-details").getAnimations({ subtree: true }).length);
          assert.equal(await body.isVisible(), true, `Rapid disclosure toggles: ${route} ${width}`);
          if (reducedMotion === "no-preference") {
            await page.keyboard.press("Space");
            await page.emulateMedia({ reducedMotion: "reduce" });
            await page.waitForFunction(() => !document.querySelector(".benchmark-details").open);
            assert.equal(await details.evaluate(element => element.getAnimations({ subtree: true }).length), 0);
            await page.emulateMedia({ reducedMotion });
            await page.keyboard.press("Enter");
          }
          await page.keyboard.press("Escape");
          await page.waitForFunction(() => !document.querySelector(".benchmark-details").open);
          assert.equal(await summary.evaluate(element => element === document.activeElement), true);
        }
      } finally {
        await context.close();
      }
    }
  }
  report.benchmark_disclosure_and_spacing = "passed";
}

async function checkThemes(origin) {
  for (const fixture of [
    { timezone: "Asia/Shanghai", before: "2026-11-16T18:59:49+08:00", pause: "2026-11-16T18:59:50+08:00", initial: "light", after: "dark" },
    { timezone: "America/Los_Angeles", before: "2026-11-16T06:59:49-08:00", pause: "2026-11-16T06:59:50-08:00", initial: "dark", after: "light" },
  ]) {
    const context = await browser.newContext({
      viewport: { width: 360, height: 640 }, timezoneId: fixture.timezone, colorScheme: "dark",
      isMobile: true, hasTouch: true, reducedMotion: "reduce", serviceWorkers: "block",
    });
    await context.route("**/*", route => new URL(route.request().url()).origin === origin ? route.continue() : route.abort());
    try {
      const page = await context.newPage();
      await page.clock.install({ time: new Date(fixture.before) });
      await page.clock.pauseAt(new Date(fixture.pause));
      await page.goto(origin + "/releases", { waitUntil: "load" });
      const theme = () => page.locator("html").getAttribute("data-theme");
      const mode = () => page.locator("html").getAttribute("data-theme-mode");
      assert.equal(await mode(), "auto");
      assert.equal(await theme(), fixture.initial, `Local time: ${fixture.timezone}`);
      await page.clock.fastForward(10000);
      assert.equal(await theme(), fixture.after, `Local day/night boundary: ${fixture.timezone}`);
      const button = page.locator("[data-theme-toggle]");
      for (const expected of ["light", "dark", "auto"]) {
        await button.tap();
        assert.equal(await mode(), expected);
        assert.equal(await theme(), expected === "auto" ? fixture.after : expected);
        assert.equal(await button.locator("[data-theme-icon]:not([hidden])").getAttribute("data-theme-icon"), expected);
      }
      await button.tap();
      await page.locator("[data-set-lang]").tap();
      assert.equal(await page.locator("html").getAttribute("lang"), "en");
      assert.equal(await mode(), "light", "Manual theme lost on language navigation");
      assert.equal(await theme(), "light", "System preference overrode manual choice");
      await page.reload({ waitUntil: "load" });
      assert.equal(await mode(), "light", "Manual theme lost on reload");
      const other = await context.newPage();
      await other.goto(origin + "/start", { waitUntil: "load" });
      await other.evaluate(() => window.siteTheme.setMode("dark"));
      await page.bringToFront();
      await page.clock.runFor(50);
      assert.equal(await mode(), "dark", "Theme change did not reach another tab");
      await other.close();
      await page.evaluate(() => {
        localStorage.removeItem("cor-site-theme");
        dispatchEvent(new StorageEvent("storage", { key: "cor-site-theme", newValue: null }));
      });
      assert.equal(await mode(), "auto", "Cleared preference did not restore automatic mode");
    } finally {
      await context.close();
    }
  }
  const context = await browser.newContext({ viewport: { width: 360, height: 640 }, serviceWorkers: "block" });
  await context.route("**/*", route => new URL(route.request().url()).origin === origin ? route.continue() : route.abort());
  await context.addInitScript(() => {
    Object.defineProperty(window, "localStorage", { get() { throw new DOMException("Storage blocked", "SecurityError"); } });
  });
  try {
    const page = await context.newPage();
    const errors = [];
    page.on("pageerror", error => errors.push(error.message));
    await page.goto(origin + "/releases", { waitUntil: "load" });
    for (const expected of ["light", "dark", "auto"]) {
      await page.locator("[data-theme-toggle]").click();
      assert.equal(await page.locator("html").getAttribute("data-theme-mode"), expected);
    }
    assert.deepEqual(errors, [], "Blocked storage broke theme switching");
  } finally {
    await context.close();
  }
  report.themes_and_local_time = "passed";
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
    await context.addInitScript(() => { localStorage.setItem("cor-site-theme", "light"); });
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
  await checkBenchmarkDetails(origin);
  await checkThemes(origin);
  const context = await browser.newContext({
    viewport: { width: 360, height: 640 }, javaScriptEnabled: false, colorScheme: "dark", serviceWorkers: "block",
  });
  try {
    const page = await context.newPage();
    await context.route("**/*", route => new URL(route.request().url()).origin === origin ? route.continue() : route.abort());
    await page.goto(origin + "/download", { waitUntil: "load", timeout: 20000 });
    assert.equal(await page.locator("[data-pkgs] a").count(), 18);
    assert.equal(await page.locator("[data-channel]").count(), 0);
    assert.equal(await page.locator("[data-theme-toggle]").isVisible(), false);
    assert.equal(await page.locator("body").evaluate(body => getComputedStyle(body).backgroundColor), "rgb(16, 29, 35)");
    assert.ok((await page.locator(".scene img").evaluate(image => image.currentSrc)).includes("-night"));
    for (const language of ["", "/en"]) {
      await page.goto(origin + language + "/", { waitUntil: "load", timeout: 20000 });
      const summary = page.locator(".benchmark-details summary");
      await summary.click();
      assert.equal(await page.locator(".benchmark-body a").isVisible(), true);
      await summary.press("Space");
      assert.equal(await page.locator(".benchmark-body").isVisible(), false);
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
