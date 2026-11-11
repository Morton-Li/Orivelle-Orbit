import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  lstat,
  mkdir,
  mkdtemp,
  readFile,
  readlink,
  realpath,
  rm,
  writeFile,
} from "node:fs/promises";
import path from "node:path";

if (
  process.platform !== "darwin" ||
  process.env.RUNNER_ENVIRONMENT !== "github-hosted"
)
  throw new Error("Verify the Finder window on a GitHub-hosted macOS runner.");

const root = process.cwd();
const config = JSON.parse(await readFile("src-tauri/tauri.conf.json", "utf8"));
const settings = config.bundle.macOS.dmg;
const diskImage = path.resolve(
  process.argv[2] ??
    `runtime/target/aarch64-apple-darwin/release/bundle/dmg/${config.productName}_${config.version}_aarch64.dmg`,
);
const evidence = path.join(root, "runtime/logs/dmg");
await mkdir(evidence, { recursive: true });
const work = await mkdtemp(path.join(root, "runtime/tmp/dmg-layout-"));
const mount = path.join(work, "mounted");
await mkdir(mount);
let mounted = false;
const receipt = {
  source: process.env.GITHUB_SHA,
  build_source: process.env.ORBIT_DMG_BUILD_SOURCE ?? process.env.GITHUB_SHA,
  artifact_run: process.env.ORBIT_DMG_ARTIFACT_RUN ?? process.env.GITHUB_RUN_ID,
  version: config.version,
};
const run = (command, args) =>
  execFileSync(command, args, { encoding: "utf8", timeout: 60_000 });
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const screenshot = path.join(evidence, "finder-installation.png");

try {
  run("/usr/bin/hdiutil", ["verify", diskImage]);
  run("/usr/bin/hdiutil", [
    "attach",
    "-readonly",
    "-noautoopen",
    "-mountpoint",
    mount,
    diskImage,
  ]);
  mounted = true;
  assert(
    (await lstat(path.join(mount, `${config.productName}.app`))).isDirectory(),
  );
  assert.equal(
    await readlink(path.join(mount, "Applications")),
    "/Applications",
  );
  assert(
    (await lstat(path.join(mount, ".DS_Store"))).size > 0,
    "Finder layout metadata is missing.",
  );
  const background = path.join(
    mount,
    ".background",
    path.basename(settings.background),
  );
  const expected = await readFile(
    path.join(root, "src-tauri", settings.background),
  );
  assert.equal(hash(await readFile(background)), hash(expected));

  const script = `
on run argv
  set mountedFolder to POSIX file (item 1 of argv) as alias
  set appName to item 2 of argv
  tell application "Finder"
    activate
    open mountedFolder
    delay 2
    set installWindow to front Finder window
    set installFolder to target of installWindow
    set appPoint to position of item appName of installFolder
    set folderPoint to position of item "Applications" of installFolder
    set windowBounds to bounds of installWindow
    set folderPath to POSIX path of (installFolder as alias)
    set resultValues to {((current view of installWindow) is icon view) as text, item 1 of appPoint, item 2 of appPoint, item 1 of folderPoint, item 2 of folderPoint, item 1 of windowBounds, item 2 of windowBounds, item 3 of windowBounds, item 4 of windowBounds, folderPath}
  end tell
  set AppleScript's text item delimiters to "|"
  return resultValues as text
end run`;
  const values = run("/usr/bin/osascript", [
    "-e",
    script,
    mount,
    `${config.productName}.app`,
  ])
    .trim()
    .split("|");
  assert.equal(values[0], "true");
  assert.deepEqual(values.slice(1, 5).map(Number), [
    settings.appPosition.x,
    settings.appPosition.y,
    settings.applicationFolderPosition.x,
    settings.applicationFolderPosition.y,
  ]);
  const bounds = values.slice(5, 9).map(Number);
  assert.equal(bounds[2] - bounds[0], settings.windowSize.width);
  assert.equal(bounds[3] - bounds[1], settings.windowSize.height);
  assert.equal(await realpath(values[9]), await realpath(mount));
  run("/usr/sbin/screencapture", [
    "-x",
    "-R",
    `${bounds[0]},${bounds[1]},${bounds[2] - bounds[0]},${bounds[3] - bounds[1]}`,
    screenshot,
  ]);
  Object.assign(receipt, {
    status: "passed",
    background_sha256: hash(expected),
    background_evidence: "Exact mounted image bytes and native Finder screenshot",
    app_position: values.slice(1, 3).map(Number),
    applications_position: values.slice(3, 5).map(Number),
    window_bounds: bounds,
    screenshot: path.basename(screenshot),
    applications_target: "/Applications",
  });
  console.log(
    "Verified the mounted DMG's Finder background, icon positions, window size and Applications link.",
  );
} catch (error) {
  Object.assign(receipt, { status: "failed", error: String(error.message) });
  try {
    run("/usr/sbin/screencapture", ["-x", screenshot]);
  } catch {}
  throw error;
} finally {
  await writeFile(
    path.join(evidence, "layout.json"),
    JSON.stringify(receipt, null, 2) + "\n",
  );
  if (mounted) run("/usr/bin/hdiutil", ["detach", mount, "-force"]);
  await rm(work, { recursive: true, force: true });
}
