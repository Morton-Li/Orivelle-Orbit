import { chromium } from "@playwright/test";
import assert from "node:assert/strict";
import { execFile, spawn } from "node:child_process";
import { mkdir, mkdtemp, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { promisify } from "node:util";
import { startPanelFixture } from "./panel-fixture.mjs";

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
const fixture = await startPanelFixture();
const powershell = promisify(execFile);
const appEnvironment = { ...process.env };
delete appEnvironment.ORBIT_FIXTURE_PFX;
delete appEnvironment.ORBIT_FIXTURE_PASSPHRASE;
const app = spawn(executable, {
  stdio: "inherit",
  env: {
    ...appEnvironment,
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

async function fileDialog(button, filePath) {
  await button.click();
  await powershell(
    "powershell.exe",
    [
      "-NoProfile",
      "-NonInteractive",
      "-STA",
      "-File",
      "tests/windows/file-dialog.ps1",
      "-OrbitProcess",
      String(app.pid),
      "-FilePath",
      filePath,
    ],
    { timeout: 20_000 },
  );
}

async function authorizeAccount(page) {
  await page
    .getByRole("button", { name: "连接账户", exact: true })
    .last()
    .click();
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "继续", exact: true })
    .click();
  await page
    .getByText("请在系统浏览器完成登录和授权。", { exact: true })
    .waitFor();

  const external = await chromium.launch({
    channel: "chrome",
    args: ["--disable-background-networking", "--no-proxy-server"],
  });
  try {
    const approval = await external.newPage();
    await approval.goto(fixture.authorizationUrl);
    await approval.getByRole("link", { name: "Approve test account" }).click();
    await approval
      .getByText("Orbit authorization completed.", { exact: false })
      .waitFor();
  } finally {
    await external.close();
  }
  await page.getByText("Synthetic account", { exact: true }).first().waitFor();
  const connected = await invoke(page, "bootstrap");
  assert.equal(connected.session.status, "authenticated");
  assert.equal(connected.session.account.wallet.amount, "12.34567890");
  assert.equal(connected.connections[0].permission_state, "allowed");
  console.log(
    "Browser authorization, signed requests and Windows credential storage verified.",
  );
}

async function waitForImages(page, count) {
  await page.waitForFunction(
    async (expected) => {
      const state = await window.__TAURI_INTERNALS__.invoke("bootstrap", {
        request: {},
      });
      return (
        state.image_history.length === expected &&
        state.tasks.filter(
          (task) => task.state === "completed" && task.charges.length > 0,
        ).length === expected
      );
    },
    count,
    { polling: 250, timeout: 30_000 },
  );
}

async function exerciseImages(page, navigation) {
  await page
    .locator(".topbar")
    .getByRole("button", { name: "刷新", exact: true })
    .click();
  await page.waitForFunction(
    () => !document.querySelector(".topbar button")?.disabled,
  );
  assert((fixture.requests.get("GET /api/v1/orbit/bootstrap") ?? 0) >= 2);
  await navigation.getByRole("link", { name: "接入", exact: true }).click();
  await page.locator("#access-node").selectOption(fixture.nodeId);
  await page.locator("#model-codex").selectOption(fixture.textModelId);
  await page.locator("#model-claude").selectOption(fixture.textModelId);
  const configPreviews = page.getByRole("button", {
    name: "预览配置",
    exact: true,
  });
  assert.equal(await configPreviews.count(), 2);
  for (const preview of await configPreviews.all()) {
    assert(!(await preview.isEnabled()));
  }
  await navigation
    .getByRole("link", { name: "图片工作台", exact: true })
    .click();
  await page.locator("#image-model").selectOption(fixture.modelId);
  await page.locator("#image-prompt").fill("Orbit Windows fixture generation");
  await page.getByRole("button", { name: "生成图片", exact: true }).click();
  await waitForImages(page, 1);
  await page.waitForFunction(
    () => document.querySelector(".image-preview-button img")?.naturalWidth > 0,
  );
  const generated = (await invoke(page, "bootstrap")).image_history[0];

  await page
    .getByRole("button", { name: "复制图片", exact: true })
    .first()
    .click();
  await page.getByText("图片已复制", { exact: true }).waitFor();
  await powershell(
    "powershell.exe",
    [
      "-NoProfile",
      "-NonInteractive",
      "-STA",
      "-Command",
      "Add-Type -AssemblyName System.Windows.Forms; if (-not [Windows.Forms.Clipboard]::ContainsImage()) { throw 'Expected a Windows clipboard image.' }",
    ],
    { timeout: 10_000 },
  );
  const exported = path.join(profile, "fixture-export.png");
  await fileDialog(
    page.getByRole("button", { name: "保存", exact: true }).first(),
    exported,
  );
  await page.getByText("已保存", { exact: true }).waitFor();
  assert.deepEqual(await readFile(exported), fixture.png);

  await page
    .getByRole("button", { name: "再次编辑", exact: true })
    .first()
    .click();
  await page.getByRole("button", { name: "编辑图片", exact: true }).waitFor();
  await page.getByRole("button", { name: /^移除参考图: / }).click();
  const reference = path.join(profile, "reference.png");
  await writeFile(reference, fixture.png);
  await fileDialog(
    page.getByRole("button", { name: "添加参考图", exact: true }),
    reference,
  );
  await page.locator("#image-prompt").fill("Orbit Windows fixture edit");
  await page.getByRole("button", { name: "编辑图片", exact: true }).click();
  await waitForImages(page, 2);

  await page.getByRole("tab", { name: "本地历史", exact: true }).click();
  await page
    .getByRole("button", { name: "删除本地历史", exact: true })
    .first()
    .click();
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "确认", exact: true })
    .click();
  await page.getByRole("dialog").waitFor({ state: "hidden" });
  assert.equal((await invoke(page, "bootstrap")).image_history.length, 1);
  assert.equal(fixture.requests.get("POST /v1/images/generations"), 1);
  assert.equal(fixture.requests.get("POST /v1/images/edits"), 1);
  assert.equal(generated.mime, "image/png");
  await page.screenshot({ path: path.join(evidence, "image-history.png") });
  console.log(
    "Node and model selection, image generation and editing, usage reconciliation, clipboard, file dialogs and history verified.",
  );
}

async function revokeAndLogout(page, navigation) {
  await navigation.getByRole("link", { name: "接入", exact: true }).click();
  await page
    .getByRole("button", { name: "撤销本安装推理 key", exact: true })
    .click();
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "确认", exact: true })
    .click();
  await page.getByRole("dialog").waitFor({ state: "hidden" });
  assert.equal(
    fixture.requests.get(
      "POST /api/v1/orbit/installations/current/inference-key/revoke",
    ),
    1,
  );
  await navigation.getByRole("link", { name: "设置", exact: true }).click();
  await page.getByRole("button", { name: "退出登录", exact: true }).click();
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "确认", exact: true })
    .click();
  await page.getByRole("dialog").waitFor({ state: "hidden" });
  assert.equal((await invoke(page, "bootstrap")).session.status, "logged_out");
  assert.equal(
    fixture.requests.get("POST /api/v1/orbit/sessions/current/revoke"),
    1,
  );
  await page.getByRole("button", { name: "忘记此安装", exact: true }).click();
  const forget = page.getByRole("dialog");
  await forget.getByRole("checkbox").check();
  await forget.getByRole("button", { name: "确认", exact: true }).click();
  await forget.waitFor({ state: "hidden" });
  assert.deepEqual((await invoke(page, "bootstrap")).installations, []);
  console.log(
    "Inference credential revocation, logout and installation cleanup verified.",
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
  assert(initial.clients.every((client) => client.platform_supported));
  assert(
    initial.clients.every(
      (client) => client.configuration_access !== "allowed",
    ),
  );
  await authorizeAccount(page);
  await exerciseImages(page, navigation);
  await revokeAndLogout(page, navigation);

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
  const diagnosticPath = path.join(profile, "orbit-diagnostics.txt");
  await fileDialog(
    page
      .getByRole("dialog")
      .getByRole("button", { name: "Save diagnostic report" }),
    diagnosticPath,
  );
  await page.getByText("Saved", { exact: true }).waitFor();
  assert.match(
    await readFile(diagnosticPath, "utf8"),
    /Platform: windows \/ x86_64/,
  );
  assert.deepEqual(fixture.failures, []);
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
  fixture.close();
}
