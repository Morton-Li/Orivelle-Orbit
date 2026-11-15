import assert from "node:assert/strict";
import { chromium, expect } from "@playwright/test";

assert.equal(process.platform, "win32");
assert.equal(process.env.ORBIT_WINDOWS_TEST_SCOPE, "disposable-vm");
const expectedVersion = process.argv[2];
assert(expectedVersion);
const endpoint = "http://127.0.0.1:9223";
await expect
  .poll(
    () =>
      fetch(endpoint + "/json/version", {
        signal: AbortSignal.timeout(1000),
      }).then(
        (response) => response.ok,
        () => false,
      ),
    { timeout: 30_000 },
  )
  .toBe(true);
const browser = await chromium.connectOverCDP(endpoint, { noDefaults: true });
try {
  const page = browser
    .contexts()
    .flatMap((context) => context.pages())
    .find(
      (candidate) =>
        candidate.url().includes("tauri.localhost") &&
        !candidate.url().includes("#/menubar"),
    );
  assert(page, "The real main WebView must be present.");
  const state = await page.evaluate(() =>
    Promise.race([
      window.__TAURI_INTERNALS__.invoke("bootstrap", { request: {} }),
      new Promise((_, reject) =>
        setTimeout(
          () => reject(new Error("Native initialization timed out.")),
          90_000,
        ),
      ),
    ]),
  );
  assert.equal(state.platform, "windows");
  const client = state.clients.find((item) => item.client === "codex");
  assert.equal(client.installed, true);
  assert.equal(client.version, expectedVersion);
  console.log(
    JSON.stringify({
      platform: state.platform,
      locale: state.settings.locale,
      installed_codex_version: client.version,
      initialized: true,
    }),
  );
} finally {
  await browser.close();
}
