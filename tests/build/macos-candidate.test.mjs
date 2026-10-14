import assert from "node:assert/strict";
import {
  chmod,
  mkdir,
  mkdtemp,
  rm,
  symlink,
  writeFile,
} from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import {
  digest,
  freezeSource,
  inspectMacApp,
  verifySource,
} from "../../scripts/candidate-evidence.mjs";

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

test("bundle proof includes resources, metadata and executable permissions", async (t) => {
  const options = await fixture(t);
  const before = await inspectMacApp(options);
  await writeFile(options.icon, "changed icon");
  const changedResource = await inspectMacApp(options);
  assert.notEqual(before.package_sha256, changedResource.package_sha256);
  await chmod(options.binary, 0o600);
  await assert.rejects(inspectMacApp(options), /executable differs/);
});

test("a replaced app executable cannot inherit the compiled candidate proof", async (t) => {
  const options = await fixture(t);
  await writeFile(options.binary, "replacement executable");
  await assert.rejects(inspectMacApp(options), /executable differs/);
});

test("wrong identity, version, minimum system or executable traversal is rejected", async (t) => {
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
});

test("symlink resources and a bundle outside the project cannot be included", async (t) => {
  const options = await fixture(t);
  const other = path.join(options.project, "external-fixture");
  await writeFile(other, "synthetic private material");
  await symlink(other, path.join(options.app, "Contents/Resources/link"));
  await assert.rejects(inspectMacApp(options), /unsupported entry/);
  await assert.rejects(
    inspectMacApp({ ...options, project: path.join(options.app, "Contents") }),
    /project directory/,
  );
});

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
