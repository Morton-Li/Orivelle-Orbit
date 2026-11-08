import { chromium } from "@playwright/test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdir, mkdtemp } from "node:fs/promises";
import path from "node:path";
import { setTimeout as delay } from "node:timers/promises";

if (
  process.platform !== "win32" ||
  process.arch !== "x64" ||
  process.env.RUNNER_ENVIRONMENT !== "github-hosted"
) {
  throw new Error("Run this test on a GitHub-hosted Windows x64 runner.");
}

const executable = path.resolve(
  "runtime/target/x86_64-pc-windows-msvc/release/orivelle-orbit.exe",
);
const evidence = path.resolve("runtime/logs/windows-integration");
await mkdir(evidence, { recursive: true });
const profile = await mkdtemp(
  path.join(process.env.RUNNER_TEMP, "orbit-webview-"),
);
const endpoint = "http://127.0.0.1:9222";
const app = spawn(executable, {
  stdio: "inherit",
  env: {
    ...process.env,
    WEBVIEW2_USER_DATA_FOLDER: profile,
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:
      "--remote-debugging-address=127.0.0.1 --remote-debugging-port=9222",
  },
});
app.unref();
let launchError;
app.on("error", (error) => {
  launchError = error;
});

async function waitForWebView() {
  const deadline = Date.now() + 60_000;
  while (Date.now() < deadline) {
    if (launchError) throw launchError;
    if (app.exitCode !== null || app.signalCode !== null)
      throw new Error(
        "Orbit exited before WebView2 was ready: " +
          (app.exitCode ?? app.signalCode),
      );
    const response = await fetch(endpoint + "/json/version", {
      signal: AbortSignal.timeout(1_000),
    }).catch(() => null);
    if (response?.ok) {
      await response.json();
      return;
    }
    await delay(250);
  }
  throw new Error("Orbit's WebView2 did not start within 60 seconds.");
}

function invoke(page, command, request = {}) {
  return page.evaluate(
    ({ command, request }) =>
      window.__TAURI_INTERNALS__.invoke(command, { request }),
    { command, request },
  );
}

let browser;
let page;
try {
  await waitForWebView();
  browser = await chromium.connectOverCDP(endpoint, { timeout: 15_000 });
  const context = browser.contexts()[0];
  if (!context) throw new Error("WebView2 did not expose a browser context.");
  page =
    context.pages()[0] ??
    (await context.waitForEvent("page", { timeout: 15_000 }));
  page.setDefaultTimeout(15_000);
  page.on("pageerror", (error) => console.error(error));

  const navigation = page.getByRole("navigation");
  await navigation.getByRole("link", { name: "首页", exact: true }).click();
  const initial = await invoke(page, "bootstrap");
  assert.equal(initial.platform, "windows");
  assert.deepEqual(initial.connections, []);
  assert.equal(initial.clients.length, 2);
  assert(initial.clients.every((client) => !client.platform_supported));
  for (const name of ["接入", "图片工作台", "设置"]) {
    await navigation.getByRole("link", { name, exact: true }).click();
    await page.getByRole("heading", { level: 1 }).waitFor();
  }

  const nextTheme = initial.settings.theme === "dark" ? "light" : "dark";
  await page.locator("#setting-theme").selectOption(nextTheme);
  await page.waitForFunction(
    async (theme) => {
      const state = await window.__TAURI_INTERNALS__.invoke("bootstrap", {
        request: {},
      });
      return state.settings.theme === theme;
    },
    nextTheme,
    { polling: 250 },
  );
  await page.locator("#setting-locale").selectOption("en");
  await navigation
    .getByRole("link", { name: "Settings", exact: true })
    .waitFor();
  assert.equal((await invoke(page, "bootstrap")).settings.locale, "en");
  assert.equal((await invoke(page, "check_update")).status, "disabled");

  await page.getByRole("button", { name: "Preview diagnostic report" }).click();
  assert.match(
    await page.getByRole("dialog").locator("pre").textContent(),
    /Platform: windows \/ x86_64/,
  );
  await page.screenshot({ path: path.join(evidence, "diagnostics.png") });
  console.log(
    "Orbit WebView2, native settings, diagnostics and Windows platform gates verified.",
  );
} catch (error) {
  await page
    ?.screenshot({ path: path.join(evidence, "failure.png"), timeout: 5_000 })
    .catch(() => {});
  throw error;
} finally {
  await browser?.close().catch(() => {});
  if (app.pid && app.exitCode === null && app.signalCode === null) app.kill();
}
