import assert from "node:assert/strict";
import {readFile} from "node:fs/promises";
import {createServer} from "node:http";
import path from "node:path";
import {fileURLToPath, pathToFileURL} from "node:url";

// Use an existing Playwright installation without adding a runtime dependency to Naro.
const playwright = process.env.PLAYWRIGHT_MODULE;
const {chromium, webkit} = await import(playwright ? pathToFileURL(path.resolve(playwright)).href : "playwright");
const publicDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../public");
const csp = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' blob:; media-src 'self' blob:; connect-src 'self'; manifest-src 'self'; worker-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'";
const server = createServer(async (request, response) => {
  const pathname = new URL(request.url, "http://localhost").pathname;
  response.setHeader("Content-Security-Policy", csp);
  if (pathname.startsWith("/api/")) {
    const data = pathname === "/api/access" ? {paired: true, online: true, name: "Theme fixture"}
      : pathname === "/api/activity" ? {running: [], finished: []}
      : pathname.startsWith("/api/sessions/") ? {sessions: []}
      : pathname.startsWith("/api/models/") ? {models: []} : null;
    response.setHeader("Content-Type", "application/json");
    response.end(JSON.stringify(data));
    return;
  }
  const file = pathname === "/" ? "index.html" : pathname.slice(1);
  if (file.includes("..")) return response.writeHead(404).end();
  try {
    const data = await readFile(path.join(publicDir, file));
    const type = path.extname(file);
    response.setHeader("Content-Type", {".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".json": "application/json", ".png": "image/png", ".webmanifest": "application/manifest+json"}[type] ?? "application/octet-stream");
    response.end(data);
  } catch {
    response.writeHead(404).end();
  }
});
await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
const origin = "http://127.0.0.1:" + server.address().port;
const artifacts = process.env.NARO_THEME_SCREENSHOTS;

async function expectTheme(page, theme, preference) {
  await page.waitForFunction(({theme, preference}) =>
    document.documentElement.dataset.theme === theme && document.documentElement.dataset.appearance === preference,
  {theme, preference});
  assert.equal(await page.locator("html").evaluate(node => getComputedStyle(node).colorScheme), theme);
  assert.equal(await page.locator('meta[name="theme-color"]').getAttribute("content"), theme === "dark" ? "#171b22" : "#ffffff");
}
async function choose(page, value) {
  await page.locator("#appearance").click();
  await page.locator("#appearance-" + value).click();
  assert.equal(await page.locator("#appearance-" + value).getAttribute("aria-selected"), "true");
}
async function verifyContrast(page) {
  const ratios = await page.evaluate(() => {
    const css = getComputedStyle(document.documentElement);
    const color = name => {
      const value = css.getPropertyValue(name).trim().slice(1);
      return [0, 2, 4].map(index => parseInt(value.slice(index, index + 2), 16) / 255.)
        .map(channel => channel <= .04045 ? channel / 12.92 : ((channel + .055) / 1.055) ** 2.4)
        .reduce((sum, channel, index) => sum + channel * [.2126, .7152, .0722][index], 0);
    };
    return [["--ink", "--paper"], ["--muted", "--soft"], ["--accent", "--paper"],
      ["--on-accent", "--accent"], ["--danger", "--danger-surface"],
      ["--diff-added-ink", "--diff-added-bg"], ["--diff-removed-ink", "--diff-removed-bg"]]
      .map(([foreground, background]) => {
        const a = color(foreground), b = color(background);
        return {foreground, background, ratio: (Math.max(a, b) + .05) / (Math.min(a, b) + .05)};
      });
  });
  for (const item of ratios) assert.ok(item.ratio >= 4.5, JSON.stringify(item));
}

try {
  const browsers = [["chromium", chromium, {channel: "chrome"}]];
  if (process.env.WEBKIT_EXECUTABLE) browsers.push(["webkit", webkit, {executablePath: process.env.WEBKIT_EXECUTABLE}]);
  for (const [name, engine, launch] of browsers) {
    const browser = await engine.launch({headless: true, ...launch});
    try {
      const context = await browser.newContext({colorScheme: "dark", viewport: {width: 1200, height: 900}});
      const page = await context.newPage();
      const errors = [];
      let takingScreenshot = false;
      page.on("pageerror", error => errors.push(error.message));
      page.on("console", message => {
        // Playwright injects temporary screenshot styles in WebKit; the production CSP
        // correctly rejects those. Keep all warnings from normal page operation fatal.
        if (/Content Security Policy|Refused to/.test(message.text()) && !takingScreenshot) errors.push(message.text());
      });
      await page.goto(origin);
      await page.locator("#workspace").waitFor({state: "visible"});
      await expectTheme(page, "dark", "system");
      await verifyContrast(page);
      await page.locator("#open-settings").click();
      await choose(page, "light");
      await expectTheme(page, "light", "light");
      await page.emulateMedia({colorScheme: "light"});
      await page.emulateMedia({colorScheme: "dark"});
      await expectTheme(page, "light", "light");
      await page.reload();
      await expectTheme(page, "light", "light");
      await page.locator("#open-settings").click();
      assert.equal(await page.locator("#appearance-value").textContent(), "浅色");
      await choose(page, "dark");
      await page.emulateMedia({colorScheme: "light"});
      await expectTheme(page, "dark", "dark");
      await choose(page, "system");
      await expectTheme(page, "light", "system");
      await page.emulateMedia({colorScheme: "dark"});
      await expectTheme(page, "dark", "system");
      await page.locator("#interface-language").waitFor({state: "visible"});
      await page.waitForFunction(() => !document.getElementById("interface-language").disabled);
      for (const [locale, caption] of [["en", "System"], ["ja", "システムに合わせる"], ["zh", "跟随系统"]]) {
        await page.locator("#interface-language").click();
        await page.locator("#interface-language-" + locale).click();
        assert.equal(await page.locator("#appearance-value").textContent(), caption);
      }
      // Keyboard selection uses the same accessible combobox behavior as the other settings.
      await page.locator("#appearance").focus();
      await page.keyboard.press("ArrowDown");
      await page.keyboard.press("End");
      await page.keyboard.press("Enter");
      await expectTheme(page, "dark", "dark");
      const other = await context.newPage();
      await other.goto(origin);
      await expectTheme(other, "dark", "dark");
      await choose(page, "light");
      await expectTheme(other, "light", "light");
      await choose(page, "dark");
      await expectTheme(other, "dark", "dark");
      await page.evaluate(() => localStorage.clear());
      await expectTheme(other, "dark", "system");
      await other.close();
      // The inline-free bootstrap runs even while the main module is delayed.
      await page.evaluate(() => localStorage.setItem("narolink.appearance", "dark"));
      await page.route("**/app.js", route => new Promise(resolve => setTimeout(resolve, 300)).then(() => route.continue()));
      await page.reload({waitUntil: "commit"});
      await expectTheme(page, "dark", "dark");
      await page.waitForLoadState();
      await page.unroute("**/app.js");
      await page.locator("#open-settings").click();
      for (const width of [1200, 390, 320]) {
        await page.setViewportSize({width, height: 844});
        for (const value of ["light", "dark"]) {
          await choose(page, value);
          const geometry = await page.locator("#settings-dialog").evaluate(node => {
            const bounds = node.getBoundingClientRect();
            return {left: bounds.left, right: bounds.right, width: innerWidth, scroll: node.scrollWidth, client: node.clientWidth};
          });
          assert.ok(geometry.left >= 0 && geometry.right <= geometry.width && geometry.scroll <= geometry.client + 1, JSON.stringify(geometry));
          if (value === "dark") {
            await verifyContrast(page);
            await page.waitForFunction(() => [...document.querySelectorAll(".settings-dropdown .picker-trigger")].every(node => {
              const style = getComputedStyle(node);
              const channels = style.backgroundColor.match(/\d+/g).slice(0, 3).map(Number);
              return channels.every(value => value < 80) && style.color === "rgb(230, 237, 245)" && style.opacity === "1";
            }));
            await page.locator("#interface-language").evaluate(node => { node.disabled = true; });
            await page.waitForFunction(() => {
              const style = getComputedStyle(document.getElementById("interface-language"));
              return style.opacity === "1" && style.color === "rgb(166, 179, 197)";
            });
            await page.locator("#interface-language").evaluate(node => { node.disabled = false; });
            await page.waitForFunction(() => getComputedStyle(document.getElementById("interface-language")).color === "rgb(230, 237, 245)");
          }
          if (artifacts) {
            takingScreenshot = true;
            try { await page.screenshot({caret: "initial", path: path.join(artifacts, `${name}-${width}-${value}.png`)}); }
            finally { takingScreenshot = false; }
          }
        }
      }
      await context.close();
      const blocked = await browser.newContext({colorScheme: "dark"});
      await blocked.addInitScript(() => Object.defineProperty(window, "localStorage", {get() { throw new DOMException("Blocked", "SecurityError"); }}));
      const blockedPage = await blocked.newPage();
      await blockedPage.goto(origin);
      assert.equal(await blockedPage.evaluate(() => window.naroAppearance.preference), "system");
      assert.equal(await blockedPage.locator("html").getAttribute("data-theme"), "dark");
      await blockedPage.evaluate(() => window.naroAppearance.set("light"));
      await expectTheme(blockedPage, "light", "light");
      await blocked.close();
      assert.deepEqual(errors, []);
      console.log(`${name}: theme switching, persistence, system changes, cross-tab sync, keyboard, translations, early bootstrap, contrast and mobile layout passed`);
    } finally {
      await browser.close();
    }
  }
} finally {
  await new Promise(resolve => server.close(resolve));
}
