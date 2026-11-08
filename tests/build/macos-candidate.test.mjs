import assert from "node:assert/strict";
import {
  chmod,
  copyFile,
  mkdir,
  mkdtemp,
  readFile,
  rename,
  rm,
  symlink,
  writeFile,
} from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import {
  CODEX_SOURCE_MATERIALS,
  digest,
  freezeSource,
  inspectMacApp,
  verifySource,
} from "../../scripts/candidate-evidence.mjs";

const testAppBundle = process.platform === "win32" ? test.skip : test;

const plist = {
  CFBundleIdentifier: "io.orivelle.orbit",
  CFBundleShortVersionString: "0.1.0",
  CFBundleVersion: "0.1.0",
  CFBundlePackageType: "APPL",
  CFBundleExecutable: "orivelle-orbit",
  LSMinimumSystemVersion: "15.0",
};
const expected = {
  identifier: "io.orivelle.orbit",
  version: "0.1.0",
  minimum: "15.0",
};
async function fixture(t) {
  const project = await mkdtemp(
    path.join(process.cwd(), "runtime/tmp/app-evidence-"),
  );
  t.after(() => rm(project, { recursive: true, force: true }));
  const app = path.join(project, "Orbit.app");
  await mkdir(path.join(app, "Contents/MacOS"), { recursive: true });
  await mkdir(path.join(app, "Contents/Resources"));
  await writeFile(path.join(app, "Contents/Info.plist"), "synthetic plist");
  const binary = path.join(app, "Contents/MacOS/orivelle-orbit");
  await writeFile(binary, "synthetic executable");
  await chmod(binary, 0o700);
  const icon = path.join(app, "Contents/Resources/icon.icns");
  await writeFile(icon, "synthetic icon");
  for (const material of CODEX_SOURCE_MATERIALS.files) {
    const target = path.join(app, material.path);
    await mkdir(path.dirname(target), { recursive: true });
    await copyFile(
      new URL(
        `../../crates/orbit-core/resources/codex/${CODEX_SOURCE_MATERIALS.cli_version}/${material.name}`,
        import.meta.url,
      ),
      target,
    );
  }
  return {
    project,
    app,
    binary,
    icon,
    executable: await digest(binary),
    expected,
    metadata: async () => plist,
  };
}

testAppBundle(
  "bundle proof includes resources, metadata and executable permissions",
  async (t) => {
    const options = await fixture(t);
    const before = await inspectMacApp(options);
    assert.deepEqual(before.codex_source_materials, {
      project: "OpenAI Codex",
      cli_version: "0.163.0-alpha.10.2",
      source_commit: "4ebc485ca321243727539187169425fcf972f2c4",
      catalog_sha256:
        "943ca7fe1d19ed019054158303f0dbbd80b3bef43b3aa425a71aa7cb3fb52c2b",
      files: CODEX_SOURCE_MATERIALS.files.map(({ path, bytes, sha256 }) => ({
        path,
        bytes,
        sha256,
      })),
    });
    await writeFile(options.icon, "changed icon");
    const changedResource = await inspectMacApp(options);
    assert.notEqual(before.package_sha256, changedResource.package_sha256);
    await chmod(options.binary, 0o600);
    await assert.rejects(inspectMacApp(options), /executable differs/);
  },
);

testAppBundle(
  "required Codex materials cannot be omitted or disabled by empty expectations",
  async (t) => {
    const options = await fixture(t);
    for (const material of CODEX_SOURCE_MATERIALS.files) {
      const target = path.join(options.app, material.path);
      const original = await readFile(target);
      await rm(target);
      await assert.rejects(inspectMacApp(options), /Codex source material/);
      await assert.rejects(
        inspectMacApp({ ...options, expected: { ...expected, resources: [] } }),
        /Codex source material/,
      );
      await writeFile(target, original);
    }
  },
);

testAppBundle(
  "same-length altered Codex materials fail their exact source hashes",
  async (t) => {
    const options = await fixture(t);
    for (const material of CODEX_SOURCE_MATERIALS.files) {
      const target = path.join(options.app, material.path);
      const original = await readFile(target);
      const altered = Buffer.from(original);
      altered[0] ^= 1;
      await writeFile(target, altered);
      await assert.rejects(inspectMacApp(options), /Codex source material/);
      await writeFile(target, original);
    }
  },
);

testAppBundle(
  "correct Codex bytes at another resource destination do not satisfy the pin",
  async (t) => {
    const options = await fixture(t);
    const wrong = path.join(
      options.app,
      "Contents/Resources/codex-wrong-target",
    );
    await mkdir(wrong);
    for (const material of CODEX_SOURCE_MATERIALS.files) {
      const target = path.join(options.app, material.path);
      const misplaced = path.join(wrong, material.name);
      await rename(target, misplaced);
      await assert.rejects(inspectMacApp(options), /Codex source material/);
      await rename(misplaced, target);
    }
  },
);

testAppBundle(
  "a directory cannot replace a required Codex regular file",
  async (t) => {
    const options = await fixture(t);
    const target = path.join(options.app, CODEX_SOURCE_MATERIALS.files[0].path);
    await rm(target);
    await mkdir(target);
    await assert.rejects(inspectMacApp(options), /Codex source material/);
  },
);

testAppBundle(
  "a replaced app executable cannot inherit the compiled candidate proof",
  async (t) => {
    const options = await fixture(t);
    await writeFile(options.binary, "replacement executable");
    await assert.rejects(inspectMacApp(options), /executable differs/);
  },
);

testAppBundle(
  "wrong identity, version, minimum system or executable traversal is rejected",
  async (t) => {
    const options = await fixture(t);
    for (const patch of [
      { CFBundleIdentifier: "another.app" },
      { CFBundleShortVersionString: "0.2.0" },
      { LSMinimumSystemVersion: "14.0" },
      { CFBundleExecutable: "../private" },
    ]) {
      await assert.rejects(
        inspectMacApp({
          ...options,
          metadata: async () => ({ ...plist, ...patch }),
        }),
        /metadata differs/,
      );
    }
  },
);

testAppBundle(
  "symlink resources and a bundle outside the project cannot be included",
  async (t) => {
    const options = await fixture(t);
    const other = path.join(options.project, "external-fixture");
    await writeFile(other, "synthetic private material");
    await symlink(other, path.join(options.app, "Contents/Resources/link"));
    await assert.rejects(inspectMacApp(options), /unsupported entry/);
    await assert.rejects(
      inspectMacApp({
        ...options,
        project: path.join(options.app, "Contents"),
      }),
      /project directory/,
    );
  },
);

function inspect({
  head = "a".repeat(40),
  tree = "b".repeat(40),
  dirty = "",
} = {}) {
  return (_binary, args) =>
    args.includes("HEAD") ? head : args.includes("HEAD^{tree}") ? tree : dirty;
}
test("a tracked edit before compilation refuses a promotable candidate", () => {
  assert.throws(
    () => freezeSource(inspect({ dirty: " M src/App.vue" })),
    /clean tracked tree/,
  );
});
test("a clean commit change or tracked edit during compilation cannot be relabelled", () => {
  const frozen = freezeSource(inspect());
  verifySource(frozen, inspect());
  assert.throws(
    () => verifySource(frozen, inspect({ head: "c".repeat(40) })),
    /Source changed/,
  );
  assert.throws(
    () => verifySource(frozen, inspect({ dirty: " M src/App.vue" })),
    /clean tracked tree/,
  );
});
