import assert from "node:assert/strict";
import {
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  rm,
  symlink,
  writeFile,
} from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import {
  captured,
  prepareProjectDirectory,
  runSyntheticSignerProbe,
} from "../../scripts/updater-signer-probe.mjs";

const project = fileURLToPath(new URL("../../", import.meta.url));
const env = Object.fromEntries(
  Object.entries(process.env).filter(
    ([key]) => !/^(ORBIT_RELEASE_|TAURI_SIGNING_|TAURI_PRIVATE_)/i.test(key),
  ),
);
const limits = { timeout: 500, grace: 100, settle: 1_000 };
async function workspace(action) {
  const root = await mkdtemp(path.join(project, "runtime/tmp/signer-test-"));
  try {
    for (const name of [
      "runtime/tmp",
      "runtime/logs",
      "node_modules/@tauri-apps/cli",
    ]) {
      await mkdir(path.join(root, name), { recursive: true, mode: 0o700 });
    }
    await writeFile(
      path.join(root, "node_modules/@tauri-apps/cli/package.json"),
      '{"version":"2.12.2"}',
    );
    return await action(root);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
}
async function gone(pid) {
  for (let attempt = 0; attempt < 30; attempt++) {
    try {
      process.kill(pid, 0);
    } catch (error) {
      if (error.code === "ESRCH") return true;
      throw error;
    }
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
  return false;
}
test(
  "dedicated Cargo cache rejects symlinks before starting any signer",
  { timeout: 5_000 },
  async () =>
    workspace(async (root) => {
      const sibling = path.join(root, "sibling");
      await mkdir(sibling);
      await writeFile(path.join(sibling, "keep"), "unchanged");
      await symlink(
        sibling,
        path.join(root, "runtime/target-signer-probe"),
        process.platform === "win32" ? "junction" : "dir",
      );
      await assert.rejects(
        prepareProjectDirectory(
          root,
          path.join(root, "runtime/target-signer-probe"),
        ),
        /project directory/,
      );
      const cli = path.join(root, "fake-cli.cjs");
      await writeFile(
        cli,
        'require("node:fs").writeFileSync("unexpected-start", "bad")',
      );
      await assert.rejects(
        runSyntheticSignerProbe({
          root,
          env,
          cargo: process.execPath,
          cli,
          source: { head: "a".repeat(40), tree: "b".repeat(40) },
          verifyFrozenSource() {},
          captureLimits: limits,
        }),
        /project directory/,
      );
      await assert.rejects(readFile(path.join(root, "unexpected-start")), {
        code: "ENOENT",
      });
      assert.equal(
        await readFile(path.join(sibling, "keep"), "utf8"),
        "unchanged",
      );
      assert.deepEqual(await readdir(path.join(root, "runtime/tmp")), []);
    }),
);

for (const scenario of ["ignore-term", "inherited-pipe"]) {
  test(
    `failed signer ${scenario} is bounded, clears owned keys and preserves sibling`,
    { timeout: 7_000 },
    async () =>
      workspace(async (root) => {
        const sibling = path.join(root, "runtime/tmp/existing-sibling");
        await mkdir(sibling);
        await writeFile(path.join(sibling, "keep"), "unchanged");
        const marker = path.join(root, "owned-child.pid");
        const manifest = path.join(
          root,
          "runtime/logs/synthetic-updater-signer-probe.json",
        );
        await writeFile(manifest, '{"old_success":true}');
        const cli = path.join(root, "fake-cli.cjs");
        const childCode =
          'process.on("SIGTERM",()=>{}); setInterval(()=>{},1000);';
        const start =
          scenario === "ignore-term"
            ? `fs.writeFileSync(${JSON.stringify(marker)}, String(process.pid)); ${childCode}`
            : `const child=require("node:child_process").spawn(process.execPath,["-e",${JSON.stringify(childCode)}],{stdio:["ignore",1,2]}); fs.writeFileSync(${JSON.stringify(marker)},String(child.pid)); child.unref(); process.exit(0);`;
        await writeFile(
          cli,
          `const fs=require("node:fs"); const key=process.argv[process.argv.indexOf("--write-keys")+1]; fs.writeFileSync(key,"FAKE_SYNTHETIC_PRIVATE_KEY"); process.stderr.write("DO_NOT_LOG_SYNTHETIC_SENTINEL"); ${start}`,
        );
        let pid;
        try {
          const before = performance.now();
          await assert.rejects(
            runSyntheticSignerProbe({
              root,
              env,
              cargo: process.execPath,
              cli,
              source: { head: "a".repeat(40), tree: "b".repeat(40) },
              verifyFrozenSource() {},
              captureLimits: limits,
            }),
            (error) => {
              assert.match(error.message, /resource limit/);
              assert.ok(!error.message.includes("SENTINEL"));
              return true;
            },
          );
          assert.ok(performance.now() - before < 4_500);
          pid = Number(await readFile(marker, "utf8"));
          assert.ok(Number.isInteger(pid) && pid > 0);
          assert.ok(await gone(pid), "owned signer/descendant must terminate");
          assert.deepEqual(await readdir(path.join(root, "runtime/tmp")), [
            "existing-sibling",
          ]);
          assert.equal(
            await readFile(path.join(sibling, "keep"), "utf8"),
            "unchanged",
          );
          await assert.rejects(readFile(manifest), { code: "ENOENT" });
        } finally {
          if (pid) {
            try {
              process.kill(pid, "SIGKILL");
            } catch (error) {
              if (error.code !== "ESRCH") throw error;
            }
          }
        }
      }),
  );
}

test(
  "output overflow stops retaining tool output and forcibly ends an owned writer",
  { timeout: 5_000 },
  async () =>
    workspace(async (root) => {
      const before = performance.now();
      await assert.rejects(
        captured(
          process.execPath,
          [
            "-e",
            'process.on("SIGTERM",()=>{}); setInterval(()=>process.stdout.write("X".repeat(4096)),10);',
          ],
          root,
          env,
          { ...limits, timeout: 2_000, outputLimit: 1_024 },
        ),
        /resource limit/,
      );
      assert.ok(performance.now() - before < 3_500);
    }),
);
