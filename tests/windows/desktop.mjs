import { chromium, expect } from "@playwright/test";
import assert from "node:assert/strict";
import { execFile, spawn } from "node:child_process";
import { access, mkdir, mkdtemp, readFile, writeFile } from "node:fs/promises";
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
  process.env.ORBIT_WINDOWS_EXECUTABLE ??
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
delete appEnvironment.WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS;
let app;
let launchError;
let baseline;
const receipt = {
  source: process.env.GITHUB_SHA,
  build_source:
    process.env.ORBIT_WINDOWS_BUILD_SOURCE ?? process.env.GITHUB_SHA,
  artifact_run:
    process.env.ORBIT_WINDOWS_ARTIFACT_RUN ?? process.env.GITHUB_RUN_ID,
  baseline_tag: process.env.ORBIT_BASELINE_TAG ?? null,
  scope:
    "Real NSIS installation and version transition, native OS behavior and controlled HTTPS peer",
  checks: [],
};

function launch(binary) {
  launchError = undefined;
  app = spawn(binary, {
    stdio: "inherit",
    env: { ...appEnvironment, WEBVIEW2_USER_DATA_FOLDER: profile },
  });
  app.unref();
  app.on("error", (error) => {
    launchError = error;
  });
}

async function native(action, item, screenshot) {
  const args = [
    "-NoProfile",
    "-NonInteractive",
    "-STA",
    "-File",
    "tests/windows/lifecycle.ps1",
    "-OrbitProcess",
    String(app.pid),
    "-Action",
    action,
  ];
  if (item) args.push(action === "menu" ? "-MenuCommand" : "-Item", item);
  if (screenshot) args.push("-Screenshot", path.join(evidence, screenshot));
  const { stdout } = await powershell("powershell.exe", args, {
    timeout: 25_000,
  });
  return JSON.parse(stdout.trim());
}

async function visible(expected) {
  await expect
    .poll(async () => (await native("state")).main_visible, {
      timeout: 15_000,
    })
    .toBe(expected);
  assert.equal(app.exitCode, null, "The primary process must remain alive.");
}

async function exited() {
  await expect.poll(() => app.exitCode, { timeout: 15_000 }).not.toBe(null);
  assert.equal(app.exitCode, 0);
}

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
  const database = path.join(
    process.env.APPDATA,
    "io.orivelle.orbit",
    "orbit.sqlite3",
  );
  console.error(
    "Orbit data store exists:",
    await access(database).then(
      () => true,
      () => false,
    ),
  );
  throw new Error(
    "Orbit's WebView2 debugging endpoint did not answer within 60 seconds.",
  );
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
  await expect
    .poll(
      async () => {
        const state = await invoke(page, "bootstrap");
        return {
          images: state.image_history.length,
          reconciled: state.tasks.filter(
            (task) => task.state === "completed" && task.charges.length > 0,
          ).length,
        };
      },
      { timeout: 30_000 },
    )
    .toEqual({ images: count, reconciled: count });
}

async function choose(page, id, label) {
  await page.locator(`#${id}`).click();
  await page.getByRole("option", { name: label, exact: true }).click();
}

async function exerciseImages(page, navigation) {
  const imagesBefore = (await invoke(page, "bootstrap")).image_history.length;
  const generationsBefore =
    fixture.requests.get("POST /v1/images/generations") ?? 0;
  await page
    .locator(".topbar")
    .getByRole("button", { name: "刷新数据", exact: true })
    .click();
  await page.waitForFunction(
    () => !document.querySelector(".topbar button")?.disabled,
  );
  assert((fixture.requests.get("GET /api/v1/orbit/bootstrap") ?? 0) >= 2);
  await navigation.getByRole("link", { name: "接入", exact: true }).click();
  await choose(page, "access-node", fixture.nodeLabel);
  const configPreviews = page.getByRole("button", {
    name: "接入 Orbit",
    exact: true,
  });
  assert.equal(await configPreviews.count(), 2);
  for (const preview of await configPreviews.all()) {
    assert(!(await preview.isEnabled()));
  }
  await navigation
    .getByRole("link", { name: "图片工作台", exact: true })
    .click();
  await choose(page, "image-model", fixture.modelLabel);
  const advanced = page.getByRole("button", { name: "高级选项", exact: true });
  await expect(advanced).toHaveAttribute("aria-expanded", "false");
  await expect(page.locator(".parameter-details")).toHaveText("高级选项");
  await advanced.click();
  await expect(page.locator("#image-background")).toContainText("不透明");
  await page.locator("#image-background").click();
  await expect(page.getByRole("option")).toHaveText(["不透明", "透明"]);
  await page.keyboard.press("Escape");
  for (const id of [
    "image-n",
    "image-size",
    "image-width",
    "image-height",
    "image-quality",
    "image-output_format",
    "image-compression",
    "image-complexity",
  ])
    await expect(page.locator("#" + id)).toHaveCount(0);
  if (baseline) {
    await page
      .getByRole("button", { name: "复用参数", exact: true })
      .first()
      .click();
    await expect(advanced).toHaveAttribute("aria-expanded", "false");
    await expect(page.locator("#image-prompt")).toHaveValue(
      "Orbit v1.0.1 compatibility history",
    );
    await advanced.click();
    await expect(page.locator("#image-background")).toContainText("透明");
    await expect(page.locator("#image-quality")).toHaveCount(0);
    await expect(page.locator("#image-size")).toHaveCount(0);
    await expect(page.locator(".task-facts")).toContainText("1024x1024");
    await page.screenshot({
      path: path.join(evidence, "legacy-history-reuse.png"),
    });
    await choose(page, "image-background", "不透明");
    receipt.checks.push(
      "v1.0.1 history facts retained and unsupported parameters removed on reuse",
    );
  }
  await advanced.click();
  await expect(page.locator(".parameter-details")).toHaveText("高级选项");
  await page.locator("#image-prompt").fill("Orbit Windows fixture generation");
  const pending = fixture.holdNextImage();
  await page.getByRole("button", { name: "生成图片", exact: true }).click();
  try {
    await Promise.race([
      pending.received,
      delay(15_000).then(() => {
        throw new Error("The generation did not reach the controlled peer.");
      }),
    ]);
    await native("close");
    await visible(false);
    await native("menu", "quit", "tray-quit-menu.png");
    await native("dialog", "继续运行", "pending-quit-guard.png");
    if ((await native("state")).main_visible) await native("close");
    await visible(false);
    pending.release();
    await waitForImages(page, imagesBefore + 1);
    await visible(false);
    await native("double-click");
    await visible(true);
    receipt.checks.push(
      "pending request continues while hidden, native Quit cancellation retains process, tray double-click restores window",
    );
  } finally {
    pending.release();
  }
  assert.deepEqual(fixture.imageBodies.at(-1).body, {
    model: (await invoke(page, "bootstrap")).tasks.find(
      (task) => task.prompt === "Orbit Windows fixture generation",
    ).requested_model,
    prompt: "Orbit Windows fixture generation",
    background: "opaque",
    response_format: "b64_json",
  });
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
  await expect(advanced).toHaveAttribute("aria-expanded", "false");
  await advanced.click();
  await choose(page, "image-background", "透明");
  await advanced.click();
  await page.getByRole("button", { name: "编辑图片", exact: true }).click();
  await waitForImages(page, imagesBefore + 2);
  assert.equal(fixture.imageBodies.at(-1).body.background, "transparent");
  for (const key of [
    "size",
    "quality",
    "output_format",
    "output_compression",
    "complexity",
  ])
    assert.equal(fixture.imageBodies.at(-1).body[key], undefined);
  receipt.checks.push(
    "opaque default and transparent edit submitted explicitly; restricted controls and collapsed summary verified",
  );

  await page.getByRole("tab", { name: /^本地历史/ }).click();
  await page
    .getByRole("button", { name: "删除本地历史", exact: true })
    .first()
    .click();
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "确认", exact: true })
    .click();
  await page.getByRole("dialog").waitFor({ state: "hidden" });
  assert.equal(
    (await invoke(page, "bootstrap")).image_history.length,
    imagesBefore + 1,
  );
  assert.equal(
    fixture.requests.get("POST /v1/images/generations"),
    generationsBefore + 1,
  );
  assert.equal(fixture.requests.get("POST /v1/images/edits"), 1);
  assert.equal(generated.mime, "image/png");
  await page.screenshot({ path: path.join(evidence, "image-history.png") });
  console.log(
    "Node and model selection, image generation and editing, usage reconciliation, clipboard, file dialogs and history verified.",
  );
}

async function logout(page, navigation) {
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
  console.log("Local logout and server session revocation verified.");
}

let browser;
let page;
async function connect() {
  await waitForWebView();
  browser = await chromium.connectOverCDP(endpoint, {
    noDefaults: true,
    timeout: 15_000,
  });
  const context = browser.contexts()[0];
  if (!context) throw new Error("WebView2 did not expose a browser context.");
  page =
    context.pages()[0] ??
    (await context.waitForEvent("page", { timeout: 15_000 }));
  page.setDefaultTimeout(15_000);
  page.on("pageerror", (error) => console.error(error));
  await page
    .getByRole("navigation")
    .getByRole("link", { name: "首页", exact: true })
    .waitFor({ timeout: 90_000 });
}

try {
  if (process.env.ORBIT_BASELINE_EXECUTABLE) {
    launch(process.env.ORBIT_BASELINE_EXECUTABLE);
    await connect();
    await authorizeAccount(page);
    const navigation = page.getByRole("navigation");
    await navigation.getByRole("link", { name: "接入", exact: true }).click();
    await choose(page, "access-node", fixture.nodeLabel);
    await navigation
      .getByRole("link", { name: "图片工作台", exact: true })
      .click();
    await choose(page, "image-model", fixture.modelLabel);
    await page.getByRole("button", { name: "高级模式", exact: true }).click();
    await choose(page, "image-quality", "高");
    await choose(page, "image-size", "1024x1024");
    await choose(page, "image-background", "透明");
    await page
      .locator("#image-prompt")
      .fill("Orbit v1.0.1 compatibility history");
    await page.getByRole("button", { name: "生成图片", exact: true }).click();
    await waitForImages(page, 1);
    await invoke(page, "update_settings", {
      patch: { theme: "dark", close_behavior: "platform_default" },
    });
    baseline = await invoke(page, "bootstrap");
    assert.equal(baseline.tasks[0].parameters.quality, "high");
    assert.equal(baseline.tasks[0].parameters.size, "1024x1024");
    await page.screenshot({ path: path.join(evidence, "v1.0.1-baseline.png") });
    await native("close");
    await exited();
    await browser.close().catch(() => {});
    browser = undefined;
    page = undefined;
    receipt.checks.push(
      "published v1.0.1 executable created native credential, legacy settings and image history",
    );
    const { stdout } = await powershell(
      "pwsh.exe",
      [
        "-NoProfile",
        "-NonInteractive",
        "-File",
        "tests/windows/install-candidate.ps1",
      ],
      { timeout: 120_000 },
    );
    console.log(stdout.trim());
    receipt.checks.push(
      "candidate NSIS upgraded the v1.0.1 installation and installed bytes match the compiled program with the Tauri NSIS marker",
    );
  }
  launch(executable);
  await connect();

  const navigation = page.getByRole("navigation");
  await navigation.getByRole("link", { name: "首页", exact: true }).waitFor({
    timeout: 90_000,
  });
  const initial = await invoke(page, "bootstrap");
  assert.equal(initial.platform, "windows");
  if (baseline) {
    assert.equal(initial.session.status, "authenticated");
    assert.equal(initial.session.account.id, baseline.session.account.id);
    assert.equal(initial.active_connection_id, baseline.active_connection_id);
    assert.deepEqual(
      initial.image_history.map((image) => image.id),
      baseline.image_history.map((image) => image.id),
    );
    assert.equal(initial.settings.theme, "dark");
    assert.equal(initial.settings.close_behavior, "platform_default");
    receipt.checks.push(
      "candidate reopened v1.0.1 credential, account, connection, settings and image history",
    );
  } else {
    assert.deepEqual(initial.connections, []);
  }
  assert.equal(initial.clients.length, 2);
  assert(initial.clients.every((client) => client.platform_supported));
  assert(
    initial.clients.every(
      (client) => client.configuration_access !== "allowed",
    ),
  );
  if (!baseline) await authorizeAccount(page);
  assert.equal((await native("state")).tray_exists, true);
  await native("close");
  await visible(false);
  const secondary = spawn(executable, {
    stdio: "inherit",
    env: { ...appEnvironment, WEBVIEW2_USER_DATA_FOLDER: profile },
  });
  await expect.poll(() => secondary.exitCode, { timeout: 15_000 }).toBe(0);
  await visible(true);
  await native("menu", "settings", "tray-settings-menu.png");
  await expect(page.locator("#setting-close")).toContainText(
    "收起到菜单栏 / 系统托盘",
  );
  await choose(page, "setting-close", "退出应用");
  await expect
    .poll(async () => (await invoke(page, "bootstrap")).settings.close_behavior)
    .toBe("quit");
  await choose(page, "setting-close", "收起到菜单栏 / 系统托盘");
  await expect
    .poll(async () => (await invoke(page, "bootstrap")).settings.close_behavior)
    .toBe("keep_running");
  receipt.checks.push(
    "legacy close setting hides window, second launch activates primary, native tray Settings routes existing window",
  );
  await exerciseImages(page, navigation);
  await logout(page, navigation);

  const nextTheme = initial.settings.theme === "dark" ? "light" : "dark";
  await choose(page, "setting-theme", nextTheme === "dark" ? "深色" : "浅色");
  await expect
    .poll(async () => (await invoke(page, "bootstrap")).settings.theme, {
      timeout: 15_000,
    })
    .toBe(nextTheme);
  await choose(page, "setting-locale", "English");
  await navigation
    .getByRole("link", { name: "Settings", exact: true })
    .waitFor();
  assert.equal((await invoke(page, "bootstrap")).settings.locale, "en");
  const update = await invoke(page, "check_update");
  assert.equal(update.status, "disabled");
  await expect(page.locator(".settings-current-version strong")).toHaveText(
    update.current_version,
  );

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
  await page.keyboard.press("Escape");
  await page.getByRole("dialog").waitFor({ state: "hidden" });
  await choose(page, "setting-close", "Quit application");
  await expect
    .poll(async () => (await invoke(page, "bootstrap")).settings.close_behavior)
    .toBe("quit");
  await native("close");
  await exited();
  receipt.checks.push(
    "explicit quit-on-close terminates the primary process cleanly",
  );
  receipt.status = "passed";
  receipt.requests = Object.fromEntries(fixture.requests);
  await writeFile(
    path.join(evidence, "acceptance.json"),
    JSON.stringify(receipt, null, 2) + "\n",
  );
} catch (error) {
  receipt.status = "failed";
  receipt.error = String(error.message);
  await writeFile(
    path.join(evidence, "acceptance.json"),
    JSON.stringify(receipt, null, 2) + "\n",
  );
  console.error(
    "Panel fixture request counts:",
    Object.fromEntries(fixture.requests),
  );
  const notice = await page
    ?.locator(".toast")
    .textContent({ timeout: 1_000 })
    .catch(() => null);
  if (notice) console.error("Orbit UI notice:", notice);
  await page
    ?.screenshot({ path: path.join(evidence, "failure.png"), timeout: 5_000 })
    .catch(() => {});
  throw error;
} finally {
  await browser?.close().catch(() => {});
  if (app?.pid && app.exitCode === null && app.signalCode === null) app.kill();
  fixture.close();
}
