import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { appendFile, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { verifyUpdaterSignature } from "../../scripts/github-release.mjs";

assert.equal(process.env.RUNNER_ENVIRONMENT, "github-hosted");
const repository = process.env.GITHUB_REPOSITORY;
assert(/^[A-Za-z0-9-]+\/[A-Za-z0-9_.-]+$/.test(repository));
const execute = (binary, args) =>
  execFileSync(binary, args, {
    encoding: "utf8",
    windowsHide: true,
    timeout: 60_000,
    maxBuffer: 4 * 1024 * 1024,
  }).trim();
const git = (...args) => execute("git", args);
const api = (endpoint) =>
  JSON.parse(execute("gh", ["api", `repos/${repository}/${endpoint}`]));
const branch = encodeURIComponent(process.env.GITHUB_REF_NAME);
const runs = api(`actions/runs?branch=${branch}&per_page=20`).workflow_runs;
const head = git("rev-parse", "HEAD");
let reused = false;
for (const run of runs) {
  if (
    String(run.id) === process.env.GITHUB_RUN_ID ||
    run.status !== "completed" ||
    run.path !== ".github/workflows/windows-updater.yml" ||
    run.head_repository?.full_name !== repository ||
    !/^[a-f0-9]{40}$/.test(run.head_sha)
  )
    continue;
  try {
    git("merge-base", "--is-ancestor", run.head_sha, head);
  } catch {
    continue;
  }
  const changes = git("diff", "--name-only", run.head_sha, head)
    .split(/\r?\n/)
    .filter(Boolean);
  // A change to a production input, builder, lockfile or workflow requires fresh
  // packages. Only assertions, fixture plumbing and documentation may reuse them.
  if (
    !changes.every(
      (file) =>
        file === "tests/windows/updater.mjs" ||
        file === "tests/windows/with-updater-fixture.ps1" ||
        file === "tests/windows/reuse-updater.mjs" ||
        file.startsWith("docs/") ||
        file.startsWith("tests/acceptance/"),
    )
  )
    continue;
  const artifact = api(`actions/runs/${run.id}/artifacts`).artifacts.find(
    (item) =>
      item.name === "orbit-windows-updater-test-packages" && !item.expired,
  );
  if (!artifact) continue;
  const directory = path.resolve("runtime/updater-acceptance");
  execute("gh", [
    "run",
    "download",
    String(run.id),
    "--repo",
    repository,
    "--name",
    artifact.name,
    "--dir",
    directory,
  ]);
  const build = JSON.parse(
    await readFile(path.join(directory, "build.json"), "utf8"),
  );
  assert.equal(build.repository, repository);
  assert.equal(build.source, run.head_sha);
  const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
  assert.equal(hash(Buffer.from(build.public_key)), build.public_key_sha256);
  for (const [name, version] of [
    ["base", "0.2.0"],
    ["target", "0.3.0"],
  ]) {
    assert.equal(build.packages[name].version, version);
    const installer = path.join(directory, `${name}-setup.exe`);
    assert.equal(
      hash(await readFile(installer)),
      build.packages[name].installer_sha256,
    );
    await verifyUpdaterSignature(
      installer,
      (await readFile(installer + ".sig", "utf8")).trim(),
      version,
      build.public_key,
    );
  }
  const proof = {
    artifact_run: run.id,
    build_source: run.head_sha,
    acceptance_source: head,
    assertion_only_changes: changes,
  };
  await writeFile(
    path.join(directory, "reuse.json"),
    JSON.stringify(proof, null, 2) + "\n",
  );
  await appendFile(
    process.env.GITHUB_ENV,
    `ORBIT_UPDATER_REUSED_RUN=${run.id}\n`,
  );
  console.log(
    `Verified unchanged application source and signed test packages from run ${run.id}.`,
  );
  reused = true;
  break;
}
await appendFile(process.env.GITHUB_OUTPUT, `reused=${reused}\n`);
if (!reused)
  console.log(
    "Fresh signed test packages are required for this application source.",
  );
