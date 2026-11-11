import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  access,
  copyFile,
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
  releaseNames,
  updateEndpoint,
  updaterPublicKey,
  verifyUpdaterSignature,
} from "./github-release.mjs";
import {
  prepareProjectDirectory,
  runSyntheticSignerProbe,
} from "./updater-signer-probe.mjs";

const root = await realpath(fileURLToPath(new URL("../", import.meta.url)));
const mode = process.argv[2] ?? "preflight";
if (
  ![
    "preflight",
    "check",
    "signer-probe",
    "candidate",
    "bundle",
    "github-bundle",
    "github-dmg",
    "github-release",
  ].includes(mode)
) {
  throw new Error(
    "Use preflight, check, signer-probe, candidate, bundle, github-bundle, github-dmg or github-release.",
  );
}
const githubRelease = mode === "github-release";
const githubBundle = mode === "github-bundle";
const githubDmg = mode === "github-dmg";
const bundling =
  mode === "bundle" || githubBundle || githubDmg || githubRelease;
const target =
  process.platform === "darwin" && process.arch === "arm64"
    ? "aarch64-apple-darwin"
    : process.platform === "win32" && process.arch === "x64"
      ? "x86_64-pc-windows-msvc"
      : null;
if (!target)
  throw new Error("A native macOS arm64 or Windows x64 host is required.");
if (githubRelease && process.platform !== "darwin")
  throw new Error("The signed updater release build requires macOS arm64.");
if (githubDmg && process.platform !== "darwin")
  throw new Error("The styled DMG candidate requires macOS arm64.");
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
    normalized.startsWith("CODESIGN_CERTIFICATE_") ||
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
if (bundling && process.platform === "darwin") {
  // Tauri otherwise skips Finder's background and icon layout on CI.
  env.TAURI_BUNDLER_DMG_IGNORE_CI = "true";
}
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
  Object.assign(env, { DEVELOPER_DIR: developer, SDKROOT: sdk });
  const clang = inspect("/usr/bin/xcrun", ["--find", "clang"]);
  const clangxx = inspect("/usr/bin/xcrun", ["--find", "clang++"]);
  await Promise.all([access(clang), access(clangxx), access(sdk)]);
  Object.assign(env, {
    CC: clang,
    CXX: clangxx,
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
async function run(binary, args, stepEnv = env) {
  await new Promise((resolve, reject) => {
    const child = spawn(binary, args, {
      cwd: root,
      env: stepEnv,
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
  const config = JSON.parse(
    await readFile(path.join(root, "src-tauri/tauri.conf.json"), "utf8"),
  );
  const releaseInputs = {};
  if (githubRelease) {
    for (const name of [
      "ORBIT_RELEASE_UPDATER_PUBLIC_KEY",
      "TAURI_SIGNING_PRIVATE_KEY",
      "TAURI_SIGNING_PRIVATE_KEY_PASSWORD",
      "CODESIGN_CERTIFICATE_P12_BASE64",
      "CODESIGN_CERTIFICATE_PASSWORD",
    ]) {
      if (!process.env[name]?.trim())
        throw new Error("Missing release input " + name);
      releaseInputs[name] = process.env[name];
    }
    updaterPublicKey(releaseInputs.ORBIT_RELEASE_UPDATER_PUBLIC_KEY);
    releaseInputs.ORBIT_RELEASE_MAC_CERTIFICATE_SHA256 = (
      await readFile(
        path.join(root, "scripts/macos-signing-certificate.sha256"),
        "utf8",
      )
    ).trim();
    if (
      !/^[A-F0-9]{64}$/.test(releaseInputs.ORBIT_RELEASE_MAC_CERTIFICATE_SHA256)
    )
      throw new Error("Invalid pinned macOS signing certificate fingerprint.");
    releaseInputs.ORBIT_RELEASE_UPDATE_ENDPOINT = updateEndpoint(
      process.env.GITHUB_REPOSITORY,
    );
    bundleConfig.bundle.createUpdaterArtifacts = true;
    bundleConfig.plugins = {
      updater: {
        pubkey: releaseInputs.ORBIT_RELEASE_UPDATER_PUBLIC_KEY,
        endpoints: [releaseInputs.ORBIT_RELEASE_UPDATE_ENDPOINT],
        requireSignedVersion: true,
      },
    };
  }
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
  if (process.platform === "win32" && githubBundle) {
    // The hosted runner can fetch the tools verified by the pinned Tauri bundler.
    bundleConfig.bundle.windows = {
      webviewInstallMode: { type: "downloadBootstrapper", silent: true },
    };
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
  const packageFlags = [
    "--target",
    target,
    "--features",
    "system-integration",
    "--ci",
  ];
  const bundleOverride = ["--config", JSON.stringify(bundleConfig)];
  const build = [cli, "build", ...packageFlags, "--no-sign"];
  if (githubRelease) {
    const {
      ORBIT_RELEASE_UPDATER_PUBLIC_KEY,
      ORBIT_RELEASE_UPDATE_ENDPOINT,
      ORBIT_RELEASE_MAC_CERTIFICATE_SHA256,
    } = releaseInputs;
    // Compile with public trust inputs. Signing credentials are only needed by
    // the bundler after npm and Cargo have finished building the binaries.
    await run(
      process.execPath,
      [...build, "--no-bundle", ...bundleOverride, "--", "--locked"],
      {
        ...env,
        ORBIT_RELEASE_UPDATER_PUBLIC_KEY,
        ORBIT_RELEASE_UPDATE_ENDPOINT,
        ORBIT_RELEASE_MAC_CERTIFICATE_SHA256,
      },
    );
    await run(
      "/bin/bash",
      [
        "scripts/package-self-signed-macos.sh",
        process.execPath,
        cli,
        "bundle",
        ...packageFlags,
        "--bundles",
        "app,dmg",
        ...bundleOverride,
      ],
      { ...env, ...releaseInputs },
    );
  } else {
    if (bundling)
      build.push(
        "--bundles",
        process.platform === "darwin"
          ? githubDmg
            ? "app,dmg"
            : "app"
          : "nsis",
        ...bundleOverride,
      );
    else build.push("--no-bundle");
    await run(process.execPath, [...build, "--", "--locked"]);
  }
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
  if (bundling && !githubRelease && process.platform === "darwin") {
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
  if (githubRelease || (githubBundle && process.platform === "win32")) {
    const directory = path.join(root, "runtime/github-release");
    await prepareProjectDirectory(root, directory);
    const bundle = path.join(path.dirname(executable), "bundle");
    const names = releaseNames(config.version);
    if (githubRelease) {
      const app = path.join(bundle, "macos", config.productName + ".app");
      await run("/usr/bin/codesign", [
        "--verify",
        "--deep",
        "--strict",
        "-R",
        `=identifier "${config.identifier}"`,
        app,
      ]);
      const certificateDir = await mkdtemp(
        path.join(env.TMPDIR, "release-certificate-"),
      );
      try {
        const prefix = path.join(certificateDir, "certificate");
        await run("/usr/bin/codesign", [
          "--display",
          `--extract-certificates=${prefix}`,
          app,
        ]);
        const certificate = await fileDigest(prefix + "0");
        if (
          certificate.sha256 !==
          releaseInputs.ORBIT_RELEASE_MAC_CERTIFICATE_SHA256.toLowerCase()
        )
          throw new Error(
            "The signed application uses a different update certificate.",
          );
      } finally {
        await rm(certificateDir, { recursive: true, force: true });
      }
      const dmg = path.join(
        bundle,
        "dmg",
        config.productName + "_" + config.version + "_aarch64.dmg",
      );
      const update = app + ".tar.gz";
      const signature = update + ".sig";
      await run("/usr/bin/codesign", ["--verify", "--strict", dmg]);
      await run("/usr/bin/hdiutil", ["verify", dmg]);
      await verifyUpdaterSignature(
        update,
        (await readFile(signature, "ascii")).trim(),
        config.version,
        releaseInputs.ORBIT_RELEASE_UPDATER_PUBLIC_KEY,
      );
      await copyFile(dmg, path.join(directory, names.macos));
      await copyFile(update, path.join(directory, names.update));
      await copyFile(signature, path.join(directory, names.signature));
    } else {
      const installer = path.join(
        bundle,
        "nsis",
        config.productName + "_" + config.version + "_x64-setup.exe",
      );
      await copyFile(installer, path.join(directory, names.windows));
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
    release_channel: releaseInputs.ORBIT_RELEASE_UPDATE_ENDPOINT ?? "disabled",
    formal_signature: githubRelease,
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
