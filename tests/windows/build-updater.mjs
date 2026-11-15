import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";
import { copyFile, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import {
  prepareReleaseSource,
  releaseNames,
} from "../../scripts/github-release.mjs";
import { captured } from "../../scripts/updater-signer-probe.mjs";
import { verifyNsisExecutable } from "../../scripts/windows-package-evidence.mjs";

assert.equal(process.platform, "win32");
assert.equal(process.env.RUNNER_ENVIRONMENT, "github-hosted");
assert(
  !Object.keys(process.env).some((key) =>
    /^(ORBIT_RELEASE_|TAURI_SIGNING_)/i.test(key),
  ),
);
const root = process.cwd();
const directory = path.resolve("runtime/updater-acceptance");
await mkdir(directory, { recursive: true });
const cli = path.resolve("node_modules/@tauri-apps/cli/tauri.js");
const key = path.join(directory, "synthetic.key");
const password = randomUUID();
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const manifest = {
  source: execFileSync("git", ["rev-parse", "HEAD"], {
    encoding: "utf8",
  }).trim(),
  repository: process.env.GITHUB_REPOSITORY,
  scope:
    "Ephemeral updater key; installed NSIS 0.2.0 to 0.3.0; no published release",
  packages: {},
};
async function sign(args) {
  const result = await captured(
    process.execPath,
    [cli, ...args],
    root,
    process.env,
  );
  assert.equal(
    result.code,
    0,
    "Synthetic signer failed; its output is deliberately withheld.",
  );
}
try {
  await sign([
    "signer",
    "generate",
    "--ci",
    "--write-keys",
    key,
    "--password",
    password,
  ]);
  const publicKey = (await readFile(key + ".pub", "utf8")).trim();
  const buildEnvironment = {
    ...process.env,
    ORBIT_RELEASE_UPDATER_PUBLIC_KEY: publicKey,
    TAURI_SIGNING_PRIVATE_KEY: (await readFile(key, "utf8")).trim(),
    TAURI_SIGNING_PRIVATE_KEY_PASSWORD: password,
  };
  manifest.public_key_sha256 = hash(Buffer.from(publicKey));
  for (const [name, version] of [
    ["base", "0.2.0"],
    ["target", "0.3.0"],
  ]) {
    const buildSource = await prepareReleaseSource("v" + version);
    execFileSync(
      process.execPath,
      ["scripts/native-workflow.mjs", "github-release"],
      {
        env: buildEnvironment,
        stdio: "inherit",
      },
    );
    const names = releaseNames(version);
    const installer = path.join(directory, name + "-setup.exe");
    await copyFile(
      path.resolve("runtime/github-release", names.windows),
      installer,
    );
    await copyFile(
      path.resolve("runtime/github-release", names.windowsSignature),
      installer + ".sig",
    );
    const payload = path.join(directory, name + "-payload");
    await mkdir(payload);
    execFileSync(
      path.join(process.env.ProgramFiles, "7-Zip/7z.exe"),
      [
        "e",
        "-tNsis",
        "-y",
        "-o" + payload,
        installer,
        "-ir!orivelle-orbit.exe",
      ],
      { stdio: "inherit" },
    );
    const compiled = await readFile(
      "runtime/target/x86_64-pc-windows-msvc/release/orivelle-orbit.exe",
    );
    const packaged = await readFile(path.join(payload, "orivelle-orbit.exe"));
    const proof = verifyNsisExecutable(compiled, packaged);
    manifest.packages[name] = {
      version,
      build_source: buildSource,
      installer_sha256: hash(await readFile(installer)),
      executable_sha256: proof.packaged_sha256,
      helper_sha256: hash(
        await readFile(
          "runtime/target/x86_64-pc-windows-msvc/release/orbit-credential-helper.exe",
        ),
      ),
      windows_image: proof.windows_image,
    };
  }
  // Valid signatures over deliberately unusable and wrongly versioned payloads
  // exercise the real download/installation gates, using no production key.
  const invalid = path.join(directory, "nonexecutable-setup.exe");
  await writeFile(
    invalid,
    "Synthetic signed non-executable. Never a release asset.\n",
  );
  await sign([
    "signer",
    "sign",
    "--private-key-path",
    key,
    "--password",
    password,
    "--app-version",
    "0.3.0",
    invalid,
  ]);
  const wrong = path.join(directory, "wrong-version-setup.exe");
  await copyFile(path.join(directory, "target-setup.exe"), wrong);
  await sign([
    "signer",
    "sign",
    "--private-key-path",
    key,
    "--password",
    password,
    "--app-version",
    "0.4.0",
    wrong,
  ]);
  await rm(wrong);
  await writeFile(
    path.join(directory, "build.json"),
    JSON.stringify(manifest, null, 2) + "\n",
  );
} finally {
  await rm(key, { force: true });
  await rm(key + ".pub", { force: true });
}
