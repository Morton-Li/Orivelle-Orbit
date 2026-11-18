import assert from "node:assert/strict";
import { execFile, spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdir, mkdtemp, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { promisify } from "node:util";
import { setTimeout as delay } from "node:timers/promises";
import { chromium, expect } from "@playwright/test";
import { startPanelFixture } from "./panel-fixture.mjs";
import { verifyWindowsGuiExecutable } from "../../scripts/windows-package-evidence.mjs";
const run = promisify(execFile);
async function digest(file) {
  const bytes = await readFile(file).catch((error) => {
    if (error.code === "ENOENT") return null;
    throw error;
  });
  return bytes && createHash("sha256").update(bytes).digest("hex");
}

export async function exerciseClientConfiguration(
  page,
  { invoke, evidence, receipt },
) {
  assert.equal(process.platform, "win32");
  assert.equal(process.env.ORBIT_WINDOWS_TEST_SCOPE, "disposable-vm");
  const helper = path.join(
    process.env.APPDATA,
    "io.orivelle.orbit",
    "bin",
    "orbit-credential-helper.exe",
  );
  verifyWindowsGuiExecutable(await readFile(helper));
  receipt.helper_sha256 = await digest(helper);
  let state = await invoke(page, "bootstrap");
  const connection = state.connections.find(
    (item) => item.id === state.active_connection_id,
  );
  assert(connection?.node_id, "Select the controlled fixture node first.");
  const results = [];
  const selected = process.env.ORBIT_WINDOWS_CLIENT_KIND;
  assert(selected === undefined || ["codex", "claude"].includes(selected));
  for (const kind of selected ? [selected] : ["codex", "claude"]) {
    const target = state.clients.find((client) => client.client === kind);
    assert(
      target.installed && target.configuration_access === "allowed",
      `${kind} native preflight failed`,
    );
    console.log(new Date().toISOString(), kind, "preview");
    const originalCodex =
      kind === "codex"
        ? await Promise.all(target.config_paths.map(digest))
        : null;
    const preview = await invoke(page, "preview_client_config", {
      connection_id: connection.id,
      expected_revision: connection.revision,
      client: kind,
      ...(kind === "claude"
        ? {
            claude_mapping: {
              sonnet: "synthetic-responses",
              opus: "synthetic-responses",
              haiku: "synthetic-responses",
            },
          }
        : {}),
    });
    assert(
      preview.can_apply,
      preview.blocking_reason ?? "Preview is unavailable.",
    );
    assert.equal(preview.paths.length, kind === "codex" ? 2 : 3);
    const before = await Promise.all(preview.paths.map(digest));
    if (kind === "codex") {
      assert.equal(preview.expires_at, null);
      assert.deepEqual(preview.paths, target.config_paths);
      assert.deepEqual(
        before,
        originalCodex,
        "Reviewing must preserve both complete files.",
      );
    }
    const originalMain =
      kind === "claude"
        ? JSON.parse(
            await readFile(preview.paths[2], "utf8").catch((error) => {
              if (error.code === "ENOENT") return "{}";
              throw error;
            }),
          )
        : undefined;
    console.log(new Date().toISOString(), kind, "apply");
    const applied = await invoke(page, "apply_client_config", {
      preview_id: preview.id,
      confirmed: true,
    });
    assert.equal(applied.state, "saved");
    assert(applied.snapshot_id);
    const progress = {
      client: kind,
      snapshot_id: applied.snapshot_id,
      before_sha256: before,
      client_request_completed: false,
      helper_pipe_verified: false,
      restored: false,
      revoked_helper_verified: false,
    };
    receipt.client_progress ??= [];
    receipt.client_progress.push(progress);
    const checkpoint = () =>
      writeFile(
        path.join(evidence, "client-progress.json"),
        JSON.stringify(receipt.client_progress, null, 2) + "\n",
      );
    await checkpoint();
    let args;
    let profile;
    let openedClient = false;
    try {
      if (kind === "claude") {
        profile = JSON.parse(await readFile(preview.paths[0], "utf8"));
        assert.equal(profile.inferenceCredentialHelperWindows, helper);
        args = profile.inferenceCredentialHelperArgs;
      } else {
        const text = await readFile(
          preview.paths.find((file) => file.endsWith("config.toml")),
          "utf8",
        );
        assert.match(text, /model_provider\s*=\s*["']orivelle_orbit["']/);
        const match = text.match(
          /args\s*=\s*\[\s*["']codex["']\s*,\s*["']([0-9a-f-]{36})["']\s*\]/,
        );
        assert(match, "The generated helper binding must be present.");
        args = ["codex", match[1]];
        const cli = path.join(target.app_path, "resources", "codex.exe");
        const loaded = await run(cli, ["debug", "models"], {
          timeout: 60_000,
          maxBuffer: 8 * 1024 * 1024,
          windowsHide: true,
        });
        assert(
          loaded.stdout.includes("synthetic-responses"),
          "Installed Codex CLI must read the generated model catalog.",
        );
        const directory = await mkdtemp(
          path.join(process.env.TEMP, "orbit-codex-message-"),
        );
        const environment = { ...process.env };
        delete environment.ORBIT_FIXTURE_PFX;
        delete environment.ORBIT_FIXTURE_PASSPHRASE;
        const answering = run(
          cli,
          [
            "exec",
            "--ephemeral",
            "--json",
            "--skip-git-repo-check",
            "--sandbox",
            "read-only",
            "Windows native configuration check; do not run tools or edit files.",
          ],
          {
            cwd: directory,
            env: environment,
            timeout: 90_000,
            maxBuffer: 1024 * 1024,
            windowsHide: true,
          },
        );
        // exec also accepts piped prompt content. Close the unused pipe so the
        // real CLI starts the supplied prompt instead of waiting for more input.
        answering.child.stdin.end();
        const answered = await answering;
        assert(
          answered.stdout.includes("Windows native configuration verified."),
          "Installed Codex CLI must authenticate through its configured helper and complete a controlled Responses request.",
        );
        progress.client_request_completed = true;
      }
      const output = await run(helper, args, {
        timeout: 30_000,
        windowsHide: true,
      });
      assert.match(
        output.stdout.trim(),
        kind === "claude" ? /^orbt_map_[A-Za-z0-9_-]+$/ : /^orv_sk_/,
      );
      progress.helper_pipe_verified = true;
      if (kind === "claude") {
        console.log(
          new Date().toISOString(),
          "observing installed Claude loading",
        );
        openedClient = true;
        const observed = await run(
          "powershell.exe",
          [
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            "tests/windows/observe-claude.ps1",
            "-AppPath",
            target.app_path,
            "-EvidenceDirectory",
            path.join(evidence, "claude"),
          ],
          { timeout: 150_000, windowsHide: true },
        );
        const loaded = JSON.parse(observed.stdout.trim());
        assert(
          loaded.helper_started_by_installed_client,
          "Installed Claude did not invoke its configured helper; inspect claude/claude-loaded.png and JSON.",
        );
        assert(
          loaded.driver?.response_rendered,
          "Installed Claude must render the controlled native Messages response.",
        );
        progress.client_request_completed = true;
      }
    } finally {
      await checkpoint();
      console.log(new Date().toISOString(), kind, "restore");
      const currentMain = openedClient
        ? JSON.parse(await readFile(preview.paths[2], "utf8"))
        : undefined;
      const restore = await invoke(page, "preview_restore", {
        snapshot_id: applied.snapshot_id,
        ...(openedClient ? { coordination: "claude_main_settings_v2" } : {}),
      });
      assert(
        restore.can_apply,
        restore.blocking_reason ?? "Restore preview is unavailable.",
      );
      const restored = await invoke(page, "restore_client_config", {
        preview_id: restore.id,
        confirmed: true,
      });
      assert.equal(restored.state, "restored");
      assert.deepEqual(
        await Promise.all(
          (openedClient ? preview.paths.slice(0, 2) : preview.paths).map(
            digest,
          ),
        ),
        openedClient ? before.slice(0, 2) : before,
        "Restore must preserve the exact original bytes/absence.",
      );
      if (openedClient) {
        const retained = JSON.parse(await readFile(preview.paths[2], "utf8"));
        assert.equal(retained.deploymentMode, originalMain.deploymentMode);
        delete retained.deploymentMode;
        delete currentMain.deploymentMode;
        assert.deepEqual(
          retained,
          currentMain,
          "Client-created settings must survive explicit coordinated restore.",
        );
      }
      if (args)
        await assert.rejects(
          run(helper, args, { timeout: 30_000, windowsHide: true }),
          "The restored binding must no longer return a credential.",
        );
      progress.restored = true;
      progress.revoked_helper_verified = Boolean(args);
      await checkpoint();
    }
    results.push({
      client: kind,
      installed_version: target.version,
      app_path: target.app_path,
      checks: [
        "native preflight",
        "guarded preview and save",
        "GUI helper with piped stdout",
        kind === "claude"
          ? "installed Claude GUI invoked helper; Credential Manager mapping and controlled HTTPS forward"
          : "installed CLI loaded catalog and authenticated helper-backed Responses request",
        kind === "claude"
          ? "exact library restore and current client settings preserved"
          : "exact restore",
        "restored helper binding disabled",
      ],
    });
    receipt.client_configuration = results;
    await writeFile(
      path.join(evidence, "client-configuration.json"),
      JSON.stringify(results, null, 2) + "\n",
    );
    state = await invoke(page, "bootstrap");
  }
  receipt.checks.push(
    "actual installed clients: configuration, Windows Credential Manager helper, native mapping, exact restore and revocation",
  );
  receipt.client_configuration = results;
  await writeFile(
    path.join(evidence, "client-configuration.json"),
    JSON.stringify(results, null, 2) + "\n",
  );
}

if (
  process.argv[1] &&
  pathToFileURL(path.resolve(process.argv[1])).href === import.meta.url
) {
  assert.equal(process.env.ORBIT_WINDOWS_TEST_SCOPE, "disposable-vm");
  const evidence = path.resolve(
    "runtime/logs/windows-client-" +
      (process.env.ORBIT_WINDOWS_CLIENT_FORMAT ?? "msix") +
      (process.env.ORBIT_WINDOWS_CLIENT_KIND
        ? "-" + process.env.ORBIT_WINDOWS_CLIENT_KIND
        : ""),
  );
  await mkdir(evidence, { recursive: true });
  const executable = path.resolve(process.env.ORBIT_WINDOWS_EXECUTABLE);
  verifyWindowsGuiExecutable(await readFile(executable));
  const profile = await mkdtemp(
    path.join(process.env.TEMP, "orbit-client-webview-"),
  );
  const environment = { ...process.env, WEBVIEW2_USER_DATA_FOLDER: profile };
  delete environment.ORBIT_FIXTURE_PFX;
  delete environment.ORBIT_FIXTURE_PASSPHRASE;
  const recoveryId = process.env.ORBIT_WINDOWS_RESTORE_SNAPSHOT;
  const readRecovery = async () => {
    const { DatabaseSync } = await import("node:sqlite");
    const db = new DatabaseSync(
      path.join(process.env.APPDATA, "io.orivelle.orbit", "orbit.sqlite3"),
      { readOnly: true },
    );
    try {
      return db
        .prepare(
          "SELECT json_extract(payload,'$.client') client, json_extract(payload,'$.phase') phase, json_extract(payload,'$.paths') paths FROM records WHERE kind='journal' AND id=?",
        )
        .get(recoveryId);
    } finally {
      db.close();
    }
  };
  const recoveryBefore = recoveryId ? await readRecovery() : undefined;
  const recoveryBinding =
    recoveryBefore?.client === "codex"
      ? (
          await readFile(
            path.join(process.env.USERPROFILE, ".codex", "config.toml"),
            "utf8",
          )
        ).match(
          /args\s*=\s*\[\s*["']codex["']\s*,\s*["']([0-9a-f-]{36})["']\s*\]/,
        )?.[1]
      : undefined;
  const fixture = await startPanelFixture();
  const app = spawn(executable, {
    env: environment,
    stdio: "inherit",
    windowsHide: true,
  });
  let browser;
  const receipt = {
    scope: "explicit disposable Windows VM; controlled HTTPS peer",
    executable_sha256: await digest(executable),
    source_manifest_sha256: process.env.ORBIT_WINDOWS_SOURCE_MANIFEST,
    checks: [],
  };
  const invoke = (page, command, request = {}) =>
    page.evaluate(
      async ({ command, request }) => {
        try {
          return await window.__TAURI_INTERNALS__.invoke(command, { request });
        } catch (error) {
          throw new Error(JSON.stringify(error));
        }
      },
      { command, request },
    );
  try {
    await expect
      .poll(
        async () =>
          fetch("http://127.0.0.1:9222/json/version", {
            signal: AbortSignal.timeout(1000),
          }).then(
            (r) => r.ok,
            () => false,
          ),
        { timeout: 120_000 },
      )
      .toBe(true);
    browser = await chromium.connectOverCDP("http://127.0.0.1:9222", {
      noDefaults: true,
    });
    let page;
    await expect
      .poll(
        () => {
          page = browser
            .contexts()
            .flatMap((context) => context.pages())
            .find(
              (candidate) =>
                candidate.url().includes("tauri.localhost") &&
                !candidate.url().includes("#/menubar"),
            );
          return Boolean(page);
        },
        { timeout: 30_000 },
      )
      .toBe(true);
    await page.waitForFunction(() => !!window.__TAURI_INTERNALS__);
    let state = await invoke(page, "bootstrap");
    if (recoveryId) {
      assert(recoveryBefore, "Recovery source must exist.");
      const current = await readRecovery();
      if (current.phase !== "restored") {
        const preview = await invoke(page, "preview_restore", {
          snapshot_id: recoveryId,
          ...(recoveryBefore.client === "claude"
            ? { coordination: "claude_main_settings_v2" }
            : {}),
        });
        console.log(
          "Recovery preview",
          JSON.stringify({
            can_apply: preview.can_apply,
            blocking_reason: preview.blocking_reason,
          }),
        );
        assert(
          preview.can_apply,
          preview.blocking_reason ?? "Recovery is unavailable.",
        );
        const restored = await invoke(page, "restore_client_config", {
          preview_id: preview.id,
          confirmed: true,
        });
        assert.equal(restored.state, "restored");
      }
      assert.equal((await readRecovery()).phase, "restored");
      if (recoveryBinding)
        await assert.rejects(
          run(
            path.join(
              process.env.APPDATA,
              "io.orivelle.orbit",
              "bin",
              "orbit-credential-helper.exe",
            ),
            ["codex", recoveryBinding],
            { timeout: 30_000, windowsHide: true },
          ),
        );
      receipt.recovery = {
        client: recoveryBefore.client,
        before_phase: recoveryBefore.phase,
        after_phase: "restored",
        method: current.phase === "restored" ? "startup" : "confirmed preview",
        revoked_helper_checked: Boolean(recoveryBinding),
        files: await Promise.all(
          JSON.parse(recoveryBefore.paths).map(async (file) => ({
            path: file,
            sha256: await digest(file),
          })),
        ),
      };
      receipt.checks.push(
        "unfinished switch restored through native recovery; source marked restored and prior helper denied",
      );
    } else {
      if (state.session.status === "authenticated") {
        await invoke(page, "logout", {
          connection_id: state.active_connection_id,
        });
      }
      console.log(new Date().toISOString(), "native login start");
      await invoke(page, "start_login");
      console.log(new Date().toISOString(), "native login request ready");
      const external = await chromium.launch({
        channel: "msedge",
        args: ["--no-proxy-server", "--disable-background-networking"],
      });
      try {
        const approval = await external.newPage();
        await approval.goto(fixture.authorizationUrl);
        await approval
          .getByRole("link", { name: "Approve test account" })
          .click({ timeout: 120_000 });
        await approval
          .getByText("Orbit authorization completed.", { exact: false })
          .waitFor();
      } finally {
        await external.close();
      }
      await expect
        .poll(async () => (await invoke(page, "bootstrap")).session.status, {
          timeout: 120_000,
        })
        .toBe("authenticated");
      state = await invoke(page, "bootstrap");
      const connection = state.connections.find(
        (c) => c.id === state.active_connection_id,
      );
      await invoke(page, "select_node", {
        connection_id: connection.id,
        expected_revision: connection.revision,
        node_id: state.nodes[0].id,
      });
      await exerciseClientConfiguration(page, { invoke, evidence, receipt });
    }
    assert.deepEqual(fixture.failures, []);
    receipt.status = "passed";
    await page.screenshot({ path: path.join(evidence, "native-access.png") });
  } catch (error) {
    receipt.status = "failed";
    receipt.error = String(error.message);
    throw error;
  } finally {
    await writeFile(
      path.join(evidence, "acceptance.json"),
      JSON.stringify(receipt, null, 2) + "\n",
    );
    await browser?.close().catch(() => {});
    if (app.exitCode === null) app.kill();
    fixture.close();
    await delay(100);
  }
}
