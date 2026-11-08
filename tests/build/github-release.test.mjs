import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import {
  copyFile,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  writeFile,
} from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import {
  assembleRelease,
  prepareReleaseSource,
  releaseNames,
  releaseVersion,
  updateEndpoint,
  verifyUpdaterSignature,
} from "../../scripts/github-release.mjs";

test("formal updates are tied to a stable tag and a GitHub repository", () => {
  assert.equal(releaseVersion("v0.2.0"), "0.2.0");
  for (const tag of ["0.2.0", "v01.2.0", "v0.2.0-rc.1", "v0.2.0/asset"]) {
    assert.throws(() => releaseVersion(tag), /release tag/);
  }
  assert.equal(
    updateEndpoint("Owner/Orbit"),
    "https://github.com/Owner/Orbit/releases/latest/download/latest.json",
  );
  assert.throws(() => updateEndpoint("Owner/Orbit/other"), /OWNER\/REPO/);
});

test("a release tag prepares a clean, versioned build checkout", async (t) => {
  const directory = await mkdtemp(path.resolve("runtime/tmp/release-source-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  await mkdir(path.join(directory, "src-tauri"));
  const files = [
    "Cargo.toml",
    "Cargo.lock",
    "package.json",
    "package-lock.json",
    "src-tauri/tauri.conf.json",
  ];
  for (const file of files) await copyFile(file, path.join(directory, file));
  const read = (file) => readFile(path.join(directory, file), "utf8");
  const git = (...args) =>
    execFileSync("git", args, { cwd: directory, encoding: "utf8" }).trim();
  git("init", "--quiet");
  git("add", "--", ...files);
  git(
    "-c",
    "user.name=Orbit Test",
    "-c",
    "user.email=test@example.invalid",
    "-c",
    "commit.gpgsign=false",
    "commit",
    "--quiet",
    "-m",
    "Fixture",
  );
  const source = git("rev-parse", "HEAD");
  const version =
    JSON.parse(await read("package.json")).version === "1.2.3"
      ? "1.2.4"
      : "1.2.3";
  const tag = `v${version}`;

  const prepared = await prepareReleaseSource(tag, directory);
  assert.equal(git("rev-parse", "HEAD^"), source);
  assert.equal(git("status", "--porcelain"), "");
  assert.equal(await prepareReleaseSource(tag, directory), prepared);
  for (const file of files.filter((item) => item.endsWith(".json"))) {
    const document = JSON.parse(await read(file));
    assert.equal(document.version, version);
    if (file === "package-lock.json")
      assert.equal(document.packages[""].version, version);
  }
  assert.ok((await read("Cargo.toml")).includes(`version = "${version}"`));
  const lock = (await read("Cargo.lock")).replaceAll("\r\n", "\n");
  for (const name of [
    "orbit-core",
    "orbit-credential-helper",
    "orivelle-orbit",
  ]) {
    assert.ok(lock.includes(`name = "${name}"\nversion = "${version}"`));
  }
});

test("the release requires both installers and advertises the verified macOS updater", async (t) => {
  const directory = await mkdtemp(path.resolve("runtime/tmp/github-release-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const wire = JSON.parse(
    await readFile(
      new URL(
        "../../crates/orbit-core/tests/fixtures/updater-wire.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  const names = releaseNames("0.2.0");
  const update = path.join(directory, names.update);
  await writeFile(update, Buffer.from(wire.artifact_base64, "base64"));
  await writeFile(path.join(directory, names.signature), wire.signature);
  await writeFile(path.join(directory, names.macos), "synthetic disk image");

  await assert.rejects(
    assembleRelease(directory, "v0.2.0", "Owner/Orbit", wire.public_key),
    /asset set is incomplete or unexpected/,
  );
  await writeFile(path.join(directory, names.windows), "synthetic installer");
  const assets = await assembleRelease(
    directory,
    "v0.2.0",
    "Owner/Orbit",
    wire.public_key,
  );
  const manifest = JSON.parse(
    await readFile(path.join(directory, "latest.json"), "utf8"),
  );
  assert.deepEqual(manifest, {
    version: "0.2.0",
    platforms: {
      "darwin-aarch64": {
        url:
          "https://github.com/Owner/Orbit/releases/download/v0.2.0/" +
          names.update,
        signature: wire.signature,
      },
    },
  });
  assert.equal(assets.length, 6);
  assert(assets.some(({ name }) => name === names.windows));
  await assert.rejects(
    verifyUpdaterSignature(
      update,
      wire.unversioned_signature,
      "0.2.0",
      wire.public_key,
    ),
    /release version/,
  );
  await assert.rejects(
    verifyUpdaterSignature(update, wire.signature, "0.3.0", wire.public_key),
    /release version/,
  );
  await writeFile(update, "modified update bytes");
  await assert.rejects(
    verifyUpdaterSignature(update, wire.signature, "0.2.0", wire.public_key),
    /package or trusted version signature/,
  );
});
