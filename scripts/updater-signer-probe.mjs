import { spawn, spawnSync } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";
import {
  chmod,
  lstat,
  mkdir,
  mkdtemp,
  readFile,
  realpath,
  rm,
  writeFile,
} from "node:fs/promises";
import path from "node:path";

const version = "0.2.0";
const maxOutput = 1024 * 1024;
const digest = (bytes) => createHash("sha256").update(bytes).digest("hex");

// Signer stdout can contain a private key. Keep all CLI output bounded and in
// memory, including on failure; never forward it to the terminal or a log.
export async function captured(binary, args, root, env, limits = {}) {
  const timeout = limits.timeout ?? 30_000;
  const grace = limits.grace ?? 200;
  const settle = limits.settle ?? 2_000;
  const outputLimit = limits.outputLimit ?? maxOutput;
  for (const value of [timeout, grace, settle, outputLimit]) {
    if (!Number.isSafeInteger(value) || value <= 0)
      throw new Error("Invalid synthetic tool resource limit.");
  }
  if (
    timeout > 300_000 ||
    grace > 2_000 ||
    settle > 5_000 ||
    outputLimit > maxOutput
  )
    throw new Error("Synthetic tool resource limit is too large.");
  return new Promise((resolve, reject) => {
    const child = spawn(binary, args, {
      cwd: root,
      env,
      shell: false,
      detached: process.platform !== "win32",
      windowsHide: true,
      stdio: ["ignore", "pipe", "pipe"],
    });
    const parts = [];
    let length = 0;
    let stopping = false;
    let finished = false;
    let forceTimer;
    let settleTimer;
    const clearTimers = () => {
      clearTimeout(timer);
      clearTimeout(forceTimer);
      clearTimeout(settleTimer);
    };
    const fail = () => {
      if (finished) return;
      finished = true;
      clearTimers();
      child.stdout.destroy();
      child.stderr.destroy();
      child.unref();
      parts.length = 0;
      reject(new Error("Synthetic signer/verifier resource limit exceeded."));
    };
    const terminateOwnedTree = (signal) => {
      if (!Number.isInteger(child.pid) || child.pid <= 0) return;
      if (process.platform === "win32") {
        // Only the PID returned by our spawn; no process name, filter or remote.
        // Prepared Windows tests must separately prove the OS tree-kill behavior.
        if (env.SYSTEMROOT) {
          spawnSync(
            path.join(env.SYSTEMROOT, "System32/taskkill.exe"),
            ["/PID", String(child.pid), "/T", "/F"],
            { env, stdio: "ignore", windowsHide: true, timeout: 1_000 },
          );
        }
        try {
          child.kill("SIGKILL");
        } catch {
          /* bounded failure below */
        }
      } else {
        try {
          // detached gives this call its own process group. Descendants keeping
          // our pipes open inherit that group even if their parent has exited.
          process.kill(-child.pid, signal);
        } catch (error) {
          if (error.code !== "ESRCH") {
            try {
              child.kill(signal);
            } catch {
              /* bounded failure below */
            }
          }
        }
      }
    };
    const stop = () => {
      if (stopping || finished) return;
      stopping = true;
      parts.length = 0;
      terminateOwnedTree("SIGTERM");
      forceTimer = setTimeout(() => terminateOwnedTree("SIGKILL"), grace);
      // close may be withheld by inherited pipes. Always reach finally in time.
      settleTimer = setTimeout(fail, grace + settle);
    };
    const timer = setTimeout(stop, timeout);
    const collect = (chunk) => {
      if (stopping || finished) return;
      length += chunk.length;
      if (length > outputLimit) stop();
      else parts.push(chunk);
    };
    child.stdout.on("data", collect);
    child.stderr.on("data", collect);
    child.on("error", () => {
      if (finished) return;
      finished = true;
      clearTimers();
      parts.length = 0;
      reject(new Error("Prepared signer/verifier tool could not start."));
    });
    child.on("close", (code) => {
      if (finished) return;
      if (stopping) fail();
      else {
        finished = true;
        clearTimers();
        resolve({ code, output: Buffer.concat(parts).toString("utf8") });
      }
    });
  });
}

export async function prepareProjectDirectory(root, absolute) {
  const relative = path.relative(root, absolute);
  if (
    !relative ||
    relative === ".." ||
    relative.startsWith(`..${path.sep}`) ||
    path.isAbsolute(relative)
  )
    throw new Error("Build cache must remain inside this project.");
  const existing = await lstat(absolute).catch((error) => {
    if (error.code === "ENOENT") return null;
    throw error;
  });
  if (existing && (!existing.isDirectory() || existing.isSymbolicLink()))
    throw new Error("Build cache must be a project directory.");
  await mkdir(absolute, { recursive: true, mode: 0o700 });
  const resolved = await realpath(absolute);
  if (
    !resolved.startsWith(`${root}${path.sep}`) ||
    (await lstat(absolute)).isSymbolicLink()
  )
    throw new Error("Build cache must remain inside this project.");
}

async function publicBox(file) {
  const info = await lstat(file);
  if (!info.isFile() || info.isSymbolicLink() || info.size > 16 * 1024)
    throw new Error("Invalid synthetic public signature box.");
  const encoded = (await readFile(file, "utf8")).trim();
  if (!encoded || Buffer.from(encoded, "base64").toString("base64") !== encoded)
    throw new Error("Invalid synthetic public signature encoding.");
  return encoded;
}

export async function runSyntheticSignerProbe({
  root,
  env,
  cargo,
  cli,
  source,
  verifyFrozenSource,
  captureLimits = {},
}) {
  const packageJson = JSON.parse(
    await readFile(
      path.join(root, "node_modules/@tauri-apps/cli/package.json"),
    ),
  );
  if (packageJson.version !== "2.12.2")
    throw new Error("The synthetic probe requires locked Tauri CLI 2.12.2.");
  if (
    Object.keys(env).some((key) =>
      /^(ORBIT_RELEASE_|TAURI_SIGNING_|TAURI_PRIVATE_)/i.test(key),
    )
  )
    throw new Error(
      "Release/signing environment must be removed for the probe.",
    );

  const tempRoot = path.join(root, "runtime/tmp");
  if ((await realpath(tempRoot)) !== tempRoot)
    throw new Error("Synthetic signer files must remain in the project.");
  const manifestPath = path.join(
    root,
    "runtime/logs/synthetic-updater-signer-probe.json",
  );
  // A failed new attempt must never leave an older success as its current result.
  await rm(manifestPath, { force: true });
  const targetDirectory = path.join(root, "runtime/target-signer-probe");
  await prepareProjectDirectory(root, targetDirectory);
  const directory = await mkdtemp(path.join(tempRoot, "signer-probe-"));
  let report;
  try {
    await chmod(directory, 0o700);
    const privateMode = (await lstat(directory)).mode & 0o777;
    if (process.platform !== "win32" && privateMode !== 0o700)
      throw new Error("Synthetic signer directory is not private.");
    const artifact = path.join(directory, "synthetic-update.bin");
    const bytes = Buffer.from(
      JSON.stringify({
        kind: "orbit-synthetic-updater",
        version,
        id: randomUUID(),
      }),
    );
    await writeFile(artifact, bytes, { mode: 0o600, flag: "wx" });
    const key = path.join(directory, "synthetic.key");
    const otherKey = path.join(directory, "other-synthetic.key");
    const invoke = async (args) => {
      const result = await captured(
        process.execPath,
        [cli, ...args],
        root,
        env,
        captureLimits,
      );
      if (result.code !== 0)
        throw new Error("Locked CLI synthetic signing step failed.");
      // Deliberately discard even successful CLI output.
    };
    for (const keyPath of [key, otherKey]) {
      await invoke([
        "signer",
        "generate",
        "--ci",
        "--write-keys",
        keyPath,
        "--password",
        "",
      ]);
      const info = await lstat(keyPath);
      if (!info.isFile() || info.isSymbolicLink() || info.size > 64 * 1024)
        throw new Error("Invalid synthetic private key file.");
      await chmod(keyPath, 0o600);
    }
    const signArgs = [
      "signer",
      "sign",
      "--private-key-path",
      key,
      "--password",
      "",
    ];
    await invoke([...signArgs, "--app-version", version, artifact]);
    const signature = await publicBox(`${artifact}.sig`);
    await rm(`${artifact}.sig`);
    await invoke([...signArgs, artifact]);
    const unversioned = await publicBox(`${artifact}.sig`);
    await rm(`${artifact}.sig`);
    // The CLI must reject control characters before creating a signature file.
    for (const control of ["\t", "\r", "\n"]) {
      const rejected = await captured(
        process.execPath,
        [
          cli,
          ...signArgs,
          "--app-version",
          `${version}${control}injected`,
          artifact,
        ],
        root,
        env,
        captureLimits,
      );
      if (
        rejected.code === 0 ||
        (await lstat(`${artifact}.sig`).catch((error) => {
          if (error.code === "ENOENT") return null;
          throw error;
        }))
      )
        throw new Error("CLI accepted an invalid synthetic version.");
    }
    const publicKey = await publicBox(`${key}.pub`);
    const fixture = path.join(directory, "fixture.json");
    await writeFile(
      fixture,
      JSON.stringify({
        public_key: publicKey,
        other_public_key: await publicBox(`${otherKey}.pub`),
        signature,
        unversioned_signature: unversioned,
        version,
        artifact_base64: bytes.toString("base64"),
      }),
      { mode: 0o600, flag: "wx" },
    );
    const verified = await captured(
      cargo,
      [
        "test",
        "--locked",
        "-p",
        "orbit-core",
        "--test",
        "updater_cli",
        "synthetic_cli_signed_artifact",
        "--",
        "--ignored",
        "--exact",
      ],
      root,
      {
        ...env,
        ORBIT_SYNTHETIC_UPDATER_FIXTURE: fixture,
        CARGO_TARGET_DIR: targetDirectory,
      },
      { ...captureLimits, timeout: captureLimits.timeout ?? 300_000 },
    );
    if (
      verified.code !== 0 ||
      !/test synthetic_cli_signed_artifact \.\.\. ok/.test(verified.output) ||
      !/1 passed; 0 failed/.test(verified.output)
    )
      throw new Error(
        "Synthetic artifact failed the actual Rust verifier probe.",
      );
    report = {
      kind: "synthetic-updater-signer-probe",
      source_head: source.head,
      source_tree: source.tree,
      cli_version: packageJson.version,
      cli_source_commit: "5b23c82ea5ba6b9aec420dd71f60493edc486aa9",
      version,
      artifact_bytes: bytes.length,
      artifact_sha256: digest(bytes),
      synthetic_public_key_sha256: digest(Buffer.from(publicKey)),
      signed_box_sha256: digest(Buffer.from(signature)),
      same_artifact_bytes_used_by_rust_verifier: true,
      actual_cli_versioned_and_unversioned_signatures: true,
      cli_rejected_version_controls: ["TAB", "CR", "LF"],
      rust_verifier: "passed",
      verifier_exit_code: verified.code,
      separate_test_target: "runtime/target-signer-probe",
      release_trust_configured: false,
      formal_certificate_used: false,
      native_app_launched: false,
      actual_install_or_update_verified: false,
    };
  } finally {
    // This exact mkdtemp directory is owned by this call; never retain its keys.
    await rm(directory, { recursive: true, force: true, maxRetries: 3 });
  }
  verifyFrozenSource();
  report.source_verified_before_and_after = true;
  report.tracked_dirty = false;
  report.completed_at = new Date().toISOString();
  report.synthetic_private_files_removed = true;
  await writeFile(manifestPath, `${JSON.stringify(report, null, 2)}\n`, {
    mode: 0o600,
    flag: "wx",
  });
  return report;
}
