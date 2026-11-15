import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";
import { once } from "node:events";
import { mkdir, mkdtemp, readFile, readdir, writeFile } from "node:fs/promises";
import { createServer } from "node:https";
import path from "node:path";
import { chromium, expect } from "@playwright/test";
import { releaseNames, updateEndpoint } from "../../scripts/github-release.mjs";

assert.equal(process.platform, "win32");
assert.equal(process.env.RUNNER_ENVIRONMENT, "github-hosted");
const directory = path.resolve("runtime/updater-acceptance");
const evidence = path.resolve("runtime/logs/windows-updater");
await mkdir(evidence, { recursive: true });
const build = JSON.parse(
  await readFile(path.join(directory, "build.json"), "utf8"),
);
const executable = process.env.ORBIT_UPDATER_EXECUTABLE;
const data = path.join(process.env.APPDATA, "io.orivelle.orbit");
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const target = await readFile(path.join(directory, "target-setup.exe"));
const invalid = await readFile(path.join(directory, "nonexecutable-setup.exe"));
const signature = (
  await readFile(path.join(directory, "target-setup.exe.sig"), "utf8")
).trim();
const invalidSignature = (
  await readFile(path.join(directory, "nonexecutable-setup.exe.sig"), "utf8")
).trim();
const wrongVersionSignature = (
  await readFile(path.join(directory, "wrong-version-setup.exe.sig"), "utf8")
).trim();
const manifestUrl = new URL(updateEndpoint(build.repository));
const artifactUrl = `https://github.com/${build.repository}/releases/download/v0.3.0/${releaseNames("0.3.0").windows}`;
let mode = "valid";
const requests = [];
const receipt = { status: "failed", ...build, checks: [] };
const server = createServer(
  {
    pfx: await readFile(process.env.ORBIT_UPDATER_PFX),
    passphrase: process.env.ORBIT_UPDATER_PASSPHRASE,
  },
  (request, response) => {
    requests.push({ path: request.url, mode });
    if (request.url === manifestUrl.pathname) {
      response.writeHead(200, { "Content-Type": "application/json" });
      response.end(
        JSON.stringify({
          version: "0.3.0",
          platforms: {
            "windows-x86_64": {
              url: artifactUrl,
              signature:
                mode === "nonexecutable"
                  ? invalidSignature
                  : mode === "wrong-version"
                    ? wrongVersionSignature
                    : signature,
            },
          },
        }),
      );
    } else if (request.url === new URL(artifactUrl).pathname) {
      const bytes =
        mode === "nonexecutable"
          ? invalid
          : mode === "corrupt"
            ? Buffer.from(target)
            : target;
      if (mode === "corrupt") bytes[bytes.length - 1] ^= 1;
      response.writeHead(200, {
        "Content-Type": "application/octet-stream",
        "Content-Length": bytes.length,
      });
      if (mode === "stall") response.write(bytes.subarray(0, 2048));
      else response.end(bytes);
    } else {
      response.writeHead(404).end();
    }
  },
);
const appEnvironment = { ...process.env };
for (const key of Object.keys(appEnvironment)) {
  if (
    /^(ORBIT_UPDATER_|TAURI_SIGNING_|ORBIT_RELEASE_|GH_TOKEN$|GITHUB_TOKEN$)/i.test(
      key,
    )
  )
    delete appEnvironment[key];
}
const profile = await mkdtemp(
  path.join(process.env.RUNNER_TEMP, "orbit-updater-webview-"),
);
let app;
let browser;
let page;
const invoke = (command, request = {}) =>
  page.evaluate(
    ({ command, request }) =>
      window.__TAURI_INTERNALS__.invoke(command, { request }),
    { command, request },
  );
async function connect() {
  await expect
    .poll(
      async () => {
        const response = await fetch("http://127.0.0.1:9222/json/version", {
          signal: AbortSignal.timeout(1000),
        }).catch(() => null);
        return response?.ok ?? false;
      },
      { timeout: 60_000 },
    )
    .toBe(true);
  browser = await chromium.connectOverCDP("http://127.0.0.1:9222");
  await expect
    .poll(
      () =>
        browser
          .contexts()
          .flatMap((context) => context.pages())
          .filter((candidate) => !candidate.url().includes("menubar.html"))
          .length,
    )
    .toBeGreaterThan(0);
  page = browser
    .contexts()
    .flatMap((context) => context.pages())
    .find((candidate) => !candidate.url().includes("menubar.html"));
  await page.waitForFunction(() => Boolean(window.__TAURI_INTERNALS__));
  await invoke("bootstrap");
}
async function reject(command, request, code) {
  await assert.rejects(
    invoke(command, request),
    (error) => error.code === code,
  );
}
async function download() {
  assert.equal((await invoke("check_update")).status, "available");
  return invoke("download_update", {
    version: "0.3.0",
    confirmed: true,
    request_id: randomUUID(),
  });
}
try {
  server.listen(443, "127.0.0.1");
  await once(server, "listening");
  assert.equal(
    hash(await readFile(executable)),
    build.packages.base.executable_sha256,
  );
  app = spawn(executable, {
    env: { ...appEnvironment, WEBVIEW2_USER_DATA_FOLDER: profile },
    stdio: "inherit",
    windowsHide: true,
  });
  await connect();
  const initial = await invoke("bootstrap");
  assert.equal(initial.in_app_updates, true);
  await expect
    .poll(
      () =>
        requests.filter((request) => request.path === manifestUrl.pathname)
          .length,
    )
    .toBeGreaterThan(0);
  receipt.checks.push(
    "Signed Windows build exposes in-app updates and checks the pinned channel on startup",
  );
  await invoke("update_settings", {
    patch: { locale: "en", theme: "dark", check_updates_on_start: false },
  });
  const settings = (await invoke("bootstrap")).settings;
  const sentinel = path.join(data, "updater-acceptance-user-data.txt");
  await writeFile(
    sentinel,
    "Existing application data must survive the NSIS update.\n",
  );
  for (const [nextMode, code] of [
    ["corrupt", "invalid_update_signature"],
    ["wrong-version", "update_version_signature_mismatch"],
  ]) {
    mode = nextMode;
    await assert.rejects(download(), (error) => error.code === code);
  }
  receipt.checks.push(
    "Changed installer bytes and signatures bound to another version are rejected",
  );
  mode = "nonexecutable";
  assert.equal((await download()).status, "verified");
  await reject(
    "install_update",
    { version: "0.3.0", confirmed: true },
    "invalid_update_artifact",
  );
  assert.equal((await invoke("bootstrap")).maintenance_active, false);
  receipt.checks.push(
    "A signed non-executable cannot be installed; maintenance is released and the old app stays usable",
  );
  mode = "stall";
  assert.equal((await invoke("check_update")).status, "available");
  const requestId = randomUUID();
  await page.evaluate(
    ({ requestId }) => {
      window.updaterDownload = window.__TAURI_INTERNALS__
        .invoke("download_update", {
          request: { version: "0.3.0", confirmed: true, request_id: requestId },
        })
        .then(
          () => "unexpected success",
          (error) => error.code,
        );
    },
    { requestId },
  );
  await expect
    .poll(() =>
      requests.some(
        (request) =>
          request.mode === "stall" &&
          request.path === new URL(artifactUrl).pathname,
      ),
    )
    .toBe(true);
  await reject(
    "install_update",
    { version: "0.3.0", confirmed: true },
    "update_busy",
  );
  await invoke("cancel_update_download", { request_id: requestId });
  assert.equal(
    await page.evaluate(() => window.updaterDownload),
    "update_download_cancelled",
  );
  receipt.checks.push(
    "An active download blocks installation; cancellation completes without replacing the application",
  );
  mode = "valid";
  assert.equal((await download()).status, "verified");
  await reject(
    "install_update",
    { version: "0.3.0", confirmed: false },
    "confirmation_required",
  );
  for (const name of await readdir(path.join(data, "updates"))) {
    if (!name.endsWith(".artifact")) continue;
    const file = path.join(data, "updates", name);
    const bytes = await readFile(file);
    if (hash(bytes) === hash(target)) {
      bytes[0] ^= 1;
      await writeFile(file, bytes);
    }
  }
  await reject(
    "install_update",
    { version: "0.3.0", confirmed: true },
    "update_artifact_changed",
  );
  assert.equal(
    hash(await readFile(executable)),
    build.packages.base.executable_sha256,
  );
  assert.deepEqual((await invoke("bootstrap")).settings, settings);
  receipt.checks.push(
    "Confirmation is required; changes to the cached installer are caught by re-verification immediately before installation",
  );
  await page.evaluate(() => {
    window.location.hash = "/settings";
  });
  await page
    .getByRole("button", { name: "Check updates", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Download and update", exact: true })
    .click();
  const install = page.getByRole("button", {
    name: "Install and restart",
    exact: true,
  });
  await expect(install).toBeVisible({ timeout: 60_000 });
  await page.screenshot({ path: path.join(evidence, "verified-update.png") });
  await install.click();
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Confirm", exact: true })
    .click()
    .catch((error) => {
      if (!/closed|destroyed/i.test(String(error))) throw error;
    });
  await expect.poll(() => app.exitCode, { timeout: 30_000 }).toBe(0);
  await expect
    .poll(async () => hash(await readFile(executable)), { timeout: 90_000 })
    .toBe(build.packages.target.executable_sha256);
  await connect();
  await page.evaluate(() => {
    window.location.hash = "/settings";
  });
  await expect(page.locator(".settings-current-version strong")).toHaveText(
    "0.3.0",
  );
  assert.equal((await invoke("check_update")).status, "up_to_date");
  const updated = await invoke("bootstrap");
  assert.deepEqual(updated.settings, settings);
  assert.equal(updated.maintenance_active, false);
  assert.equal(
    await readFile(sentinel, "utf8"),
    "Existing application data must survive the NSIS update.\n",
  );
  assert.equal(
    hash(await readFile(path.join(data, "bin/orbit-credential-helper.exe"))),
    build.packages.target.helper_sha256,
  );
  await page.screenshot({
    path: path.join(evidence, "updated-and-restarted.png"),
  });
  receipt.checks.push(
    "The settings-page flow installs 0.3.0 over 0.2.0 in a path containing spaces and automatically restarts the packaged app",
  );
  receipt.checks.push(
    "Settings, the disabled startup-check choice and existing user data survive; the new embedded helper is deployed and the channel reports up to date",
  );
  receipt.status = "passed";
} catch (error) {
  receipt.error = String(error.stack ?? error);
  if (page && !page.isClosed())
    await page
      .screenshot({ path: path.join(evidence, "failure.png") })
      .catch(() => undefined);
  throw error;
} finally {
  receipt.requests = requests;
  await writeFile(
    path.join(evidence, "acceptance.json"),
    JSON.stringify(receipt, null, 2) + "\n",
  );
  await browser?.close().catch(() => undefined);
  server.closeAllConnections();
  server.close();
}
