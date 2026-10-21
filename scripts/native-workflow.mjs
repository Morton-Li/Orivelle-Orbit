import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  access,
  lstat,
  mkdtemp,
  readFile,
  realpath,
  rm,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  digest as fileDigest,
  freezeSource,
  inspectMacApp,
  verifySource,
} from "./candidate-evidence.mjs";
import {
  prepareProjectDirectory,
  runSyntheticSignerProbe,
} from "./updater-signer-probe.mjs";

const root = await realpath(fileURLToPath(new URL("../", import.meta.url)));
const mode = process.argv[2] ?? "preflight";
if (
  !["preflight", "check", "signer-probe", "candidate", "bundle"].includes(mode)
) {
  throw new Error("Use preflight, check, signer-probe, candidate or bundle.");
}
const target =
  process.platform === "darwin" && process.arch === "arm64"
    ? "aarch64-apple-darwin"
    : process.platform === "win32" && process.arch === "x64"
      ? "x86_64-pc-windows-msvc"
      : null;
if (!target)
  throw new Error("A native macOS arm64 or Windows x64 host is required.");
const expectedNode = (
  await readFile(path.join(root, ".node-version"), "utf8")
).trim();
if (process.versions.node !== expectedNode) {
  throw new Error(
    `Prepare Node ${expectedNode}; this script does not install tools.`,
  );
}
const suffix = process.platform === "win32" ? ".exe" : "";
const rustBin = path.resolve(
  process.env.ORBIT_RUST_BIN ??
    path.join(os.homedir(), ".rustup", "toolchains", `1.97.1-${target}`, "bin"),
);
const cargo = path.join(rustBin, `cargo${suffix}`);
const rustc = path.join(rustBin, `rustc${suffix}`);
const rustfmt = path.join(rustBin, `rustfmt${suffix}`);
await Promise.all([access(cargo), access(rustc), access(rustfmt)]);
const inheritedEnv = {};
for (const [key, value] of Object.entries(process.env)) {
  const normalized = process.platform === "win32" ? key.toUpperCase() : key;
  if (normalized in inheritedEnv && inheritedEnv[normalized] !== value) {
    throw new Error("Ambiguous native build environment.");
  }
  inheritedEnv[normalized] = value;
}
// Compare case-insensitively before adding canonical values. Windows treats
// environment names as case-insensitive; duplicate npm variants can win in a child.
for (const key of Object.keys(inheritedEnv)) {
  const normalized = key.toUpperCase();
  if (
    normalized.startsWith("ORBIT_RELEASE_") ||
    normalized.startsWith("TAURI_SIGNING_") ||
    normalized.startsWith("TAURI_PRIVATE_") ||
    normalized.startsWith("APPLE_") ||
    normalized.startsWith("NPM_CONFIG_") ||
    normalized.startsWith("VITE") ||
    normalized.startsWith("CARGO_REGISTRIES_") ||
    normalized.startsWith("DITTO") ||
    [
      "RUSTC_WRAPPER",
      "RUSTC_WORKSPACE_WRAPPER",
      "CARGO_BUILD_TARGET",
      "TAURI_CONFIG",
      "NODE_ENV",
    ].includes(normalized)
  )
    delete inheritedEnv[key];
}
const npmGlobalConfig = path.join(root, ".cache/npm/disabled-global.npmrc");
const existingNpmGlobalConfig = await lstat(npmGlobalConfig).catch((error) => {
  if (error.code === "ENOENT") return null;
  throw error;
});
if (existingNpmGlobalConfig)
  throw new Error("The isolated npm global configuration path must be absent.");
const env = {
  ...inheritedEnv,
  PATH: `${rustBin}${path.delimiter}${process.env.PATH ?? ""}`,
  RUSTC: rustc,
  RUSTFMT: rustfmt,
  RUSTUP_TOOLCHAIN: "1.97.1",
  RUSTUP_AUTO_INSTALL: "0",
  CARGO_HOME: path.join(root, "runtime", "cargo-home"),
  CARGO_TARGET_DIR: path.join(root, "runtime", "target"),
  CARGO_BUILD_JOBS: "4",
  TMPDIR: path.join(root, "runtime", "tmp"),
  TEMP: path.join(root, "runtime", "tmp"),
  TMP: path.join(root, "runtime", "tmp"),
  NPM_CONFIG_CACHE: path.join(root, ".cache", "npm"),
  NPM_CONFIG_REGISTRY: "https://registry.npmmirror.com/",
  NPM_CONFIG_UPDATE_NOTIFIER: "false",
  NPM_CONFIG_USERCONFIG: os.devNull,
  NPM_CONFIG_GLOBALCONFIG: npmGlobalConfig,
};
function inspect(binary, args) {
  const result = spawnSync(binary, args, {
    cwd: root,
    env,
    encoding: "utf8",
    timeout: 15000,
  });
  if (result.error || result.status !== 0)
    throw new Error("Prepared toolchain preflight failed.");
  return result.stdout.trim();
}
const rustVersion = inspect(rustc, ["--version"]);
const cargoVersion = inspect(cargo, ["--version"]);
const npmVersion =
  process.platform === "win32"
    ? inspect("cmd.exe", ["/d", "/s", "/c", "npm --version"])
    : inspect("npm", ["--version"]);
if (npmVersion !== "11.16.0") throw new Error("Prepare npm 11.16.0.");
if (
  !rustVersion.startsWith("rustc 1.97.1 ") ||
  !cargoVersion.startsWith("cargo 1.97.1 ")
) {
  throw new Error("Prepare the locked Rust 1.97.1 toolchain.");
}
if (!inspect(rustc, ["-vV"]).includes(`host: ${target}`)) {
  throw new Error("Cross compilation is not a platform verification gate.");
}
if (process.platform === "darwin") {
  const developer =
    process.env.ORBIT_DEVELOPER_DIR ?? "/Library/Developer/CommandLineTools";
  const sdk =
    process.env.ORBIT_MAC_SDK ?? path.join(developer, "SDKs", "MacOSX15.4.sdk");
  const clang = path.join(developer, "usr", "bin", "clang");
  await Promise.all([access(clang), access(sdk)]);
  Object.assign(env, {
    DEVELOPER_DIR: developer,
    SDKROOT: sdk,
    CC: clang,
    CXX: path.join(developer, "usr", "bin", "clang++"),
    CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER: clang,
    MACOSX_DEPLOYMENT_TARGET: "15.0",
  });
}
for (const directory of [
  "runtime",
  "runtime/cargo-home",
  "runtime/target",
  "runtime/target-signer-probe",
  "runtime/tmp",
  "runtime/logs",
  "runtime/candidates",
  ".cache",
  ".cache/npm",
]) {
  const absolute = path.join(root, directory);
  await prepareProjectDirectory(root, absolute);
}
async function run(binary, args) {
  await new Promise((resolve, reject) => {
    const child = spawn(binary, args, {
      cwd: root,
      env,
      stdio: "inherit",
      shell: false,
    });
    child.on("error", () =>
      reject(new Error("Required prepared build tool is unavailable.")),
    );
    child.on("exit", (code) =>
      code === 0 ? resolve() : reject(new Error(`Build step exited ${code}.`)),
    );
  });
}
async function digest(file) {
  return {
    path: path.relative(root, file),
    ...(await fileDigest(file)),
  };
}
const preflight = {
  target,
  node: process.versions.node,
  npm: npmVersion,
  rust: rustVersion,
  cargo: cargoVersion,
  cache_scope: "project",
  native_app_launched: false,
};
await writeFile(
  path.join(root, "runtime/logs/native-preflight.json"),
  `${JSON.stringify(preflight, null, 2)}\n`,
);
if (mode === "preflight") {
  console.log(JSON.stringify(preflight, null, 2));
} else if (mode === "check") {
  await run(cargo, ["fmt", "--all", "--", "--check"]);
  await run(cargo, [
    "test",
    "--locked",
    "-p",
    "orbit-core",
    "-p",
    "orbit-credential-helper",
  ]);
  await run(cargo, ["check", "--locked", "-p", "orivelle-orbit"]);
} else if (mode === "signer-probe") {
  const cli = path.join(root, "node_modules/@tauri-apps/cli/tauri.js");
  await access(cli);
  const source = freezeSource(inspect);
  console.log(
    JSON.stringify(
      await runSyntheticSignerProbe({
        root,
        env,
        cargo,
        cli,
        source,
        verifyFrozenSource: () => verifySource(source, inspect),
      }),
      null,
      2,
    ),
  );
} else {
  const source = freezeSource(inspect);
  const cli = path.join(root, "node_modules/@tauri-apps/cli/tauri.js");
  await access(cli);
  const toolPackage = JSON.parse(
    await readFile(
      path.join(root, "node_modules/@tauri-apps/cli/package.json"),
      "utf8",
    ),
  );
  if (toolPackage.version !== "2.12.2")
    throw new Error("Prepare the locked Tauri CLI 2.12.2.");
  const bundleConfig = { bundle: { active: true, useLocalToolsDir: true } };
  if (process.platform === "win32" && mode === "bundle") {
    // Tauri would otherwise download missing NSIS components from GitHub.
    // Prepare its complete local tool cache from an approved mirror first.
    const nsis = path.join(env.CARGO_TARGET_DIR, ".tauri", "NSIS");
    for (const file of [
      "makensis.exe",
      "Bin/makensis.exe",
      "Stubs/lzma-x86-unicode",
      "Stubs/lzma_solid-x86-unicode",
      "Plugins/x86-unicode/additional/nsis_tauri_utils.dll",
      "Include/MUI2.nsh",
      "Include/FileFunc.nsh",
      "Include/x64.nsh",
      "Include/nsDialogs.nsh",
      "Include/WinMessages.nsh",
      "Include/Win/COM.nsh",
      "Include/Win/Propkey.nsh",
      "Include/Win/RestartManager.nsh",
    ]) {
      await access(path.join(nsis, file));
    }
    const plugin = await readFile(
      path.join(nsis, "Plugins/x86-unicode/additional/nsis_tauri_utils.dll"),
    );
    if (
      createHash("sha1").update(plugin).digest("hex").toUpperCase() !==
      "75197FEE3C6A814FE035788D1C34EAD39349B860"
    ) {
      throw new Error(
        "Prepared NSIS plugin differs from the locked Tauri tool checksum.",
      );
    }
    bundleConfig.bundle.windows = { webviewInstallMode: { type: "skip" } };
  }
  await run(cargo, [
    "build",
    "--locked",
    "--release",
    "--target",
    target,
    "-p",
    "orbit-credential-helper",
    "--features",
    "system-integration",
  ]);
  const helper = path.join(
    env.CARGO_TARGET_DIR,
    target,
    "release",
    `orbit-credential-helper${suffix}`,
  );
  env.ORBIT_BUNDLED_HELPER_PATH = helper;
  const args = [
    cli,
    "build",
    "--target",
    target,
    "--features",
    "system-integration",
    "--ci",
    "--no-sign",
  ];
  if (mode === "bundle")
    args.push(
      "--bundles",
      process.platform === "darwin" ? "app" : "nsis",
      "--config",
      JSON.stringify(bundleConfig),
    );
  else args.push("--no-bundle");
  args.push("--", "--locked");
  await run(process.execPath, args);
  const executable = path.join(
    env.CARGO_TARGET_DIR,
    target,
    "release",
    `orivelle-orbit${suffix}`,
  );
  const helperEvidence = await digest(helper);
  const executableEvidence = await digest(executable);
  let appEvidence = null;
  let archiveEvidence = null;
  if (mode === "bundle" && process.platform === "darwin") {
    const config = JSON.parse(
      await readFile(path.join(root, "src-tauri/tauri.conf.json"), "utf8"),
    );
    const app = path.join(
      env.CARGO_TARGET_DIR,
      target,
      "release/bundle/macos",
      `${config.productName}.app`,
    );
    const expected = {
      identifier: config.identifier,
      version: config.version,
      minimum: config.bundle.macOS.minimumSystemVersion,
    };
    const metadata = async (file) =>
      JSON.parse(
        inspect("/usr/bin/plutil", ["-convert", "json", "-o", "-", file]),
      );
    appEvidence = await inspectMacApp({
      project: root,
      app,
      executable: executableEvidence,
      expected,
      metadata,
    });
    const archive = path.join(
      root,
      "runtime/candidates",
      `Orivelle-Orbit-${config.version}-macos-arm64-candidate.zip`,
    );
    const oldArchive = await lstat(archive).catch((error) => {
      if (error.code === "ENOENT") return null;
      throw error;
    });
    if (oldArchive && (!oldArchive.isFile() || oldArchive.isSymbolicLink()))
      throw new Error("Candidate archive must be a project regular file.");
    await run("/usr/bin/ditto", [
      "-c",
      "-k",
      "--keepParent",
      "--norsrc",
      "--noextattr",
      "--noacl",
      "--noqtn",
      app,
      archive,
    ]);
    const names = inspect("/usr/bin/unzip", ["-Z1", archive]).split("\n");
    const base = path.basename(app);
    if (
      names.length > 2048 ||
      names.some(
        (name) =>
          !name.startsWith(`${base}/`) ||
          name.includes("\\") ||
          name
            .replace(/\/$/, "")
            .split("/")
            .some((part) => !part || part === "." || part === ".."),
      )
    )
      throw new Error("Candidate archive has unexpected paths.");
    const extracted = await mkdtemp(
      path.join(env.TMPDIR, "candidate-roundtrip-"),
    );
    try {
      await run("/usr/bin/ditto", [
        "-x",
        "-k",
        "--norsrc",
        "--noextattr",
        "--noacl",
        "--noqtn",
        archive,
        extracted,
      ]);
      const restored = await inspectMacApp({
        project: root,
        app: path.join(extracted, base),
        executable: executableEvidence,
        expected,
        metadata,
      });
      const unchanged = await inspectMacApp({
        project: root,
        app,
        executable: executableEvidence,
        expected,
        metadata,
      });
      if (
        restored.package_sha256 !== appEvidence.package_sha256 ||
        unchanged.package_sha256 !== appEvidence.package_sha256
      )
        throw new Error(
          "Candidate archive roundtrip differs from the app bundle.",
        );
      archiveEvidence = {
        ...(await digest(archive)),
        app_roundtrip_verified: true,
        codex_source_materials: restored.codex_source_materials,
      };
    } finally {
      await rm(extracted, { recursive: true, force: true });
    }
  }
  verifySource(source, inspect);
  const manifest = {
    ...preflight,
    source_head: source.head,
    source_tree: source.tree,
    source_verified_before_and_after: true,
    tracked_dirty: false,
    mode,
    release_channel: "disabled",
    formal_signature: false,
    helper: helperEvidence,
    executable: executableEvidence,
    app: appEvidence,
    archive: archiveEvidence,
    actual_native_integration_verified: false,
  };
  await writeFile(
    path.join(root, "runtime/logs/candidate-manifest.json"),
    `${JSON.stringify(manifest, null, 2)}\n`,
  );
  console.log(JSON.stringify(manifest, null, 2));
}
