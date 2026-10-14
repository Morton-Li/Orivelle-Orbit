import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import { lstat, readdir, realpath } from "node:fs/promises";
import path from "node:path";

export function freezeSource(inspect) {
  const head = inspect("git", ["rev-parse", "HEAD"]);
  const tree = inspect("git", ["rev-parse", "HEAD^{tree}"]);
  if (
    !/^[a-f0-9]{40}$/.test(head) ||
    !/^[a-f0-9]{40}$/.test(tree) ||
    inspect("git", ["status", "--porcelain", "--untracked-files=no"])
  ) {
    throw new Error(
      "Candidate build requires a committed, clean tracked tree.",
    );
  }
  return { head, tree };
}

export function verifySource(frozen, inspect) {
  const current = freezeSource(inspect);
  if (current.head !== frozen.head || current.tree !== frozen.tree)
    throw new Error(
      "Source changed during candidate build; rebuild the candidate.",
    );
}

export async function digest(file) {
  const metadata = await lstat(file);
  if (
    !metadata.isFile() ||
    metadata.isSymbolicLink() ||
    metadata.size > 512 * 1024 ** 2
  )
    throw new Error("Candidate artifact is not a bounded regular file.");
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(file)) hash.update(chunk);
  return { bytes: metadata.size, sha256: hash.digest("hex") };
}

export async function inspectMacApp({
  project,
  app,
  executable,
  expected,
  metadata,
}) {
  const projectRoot = await realpath(project);
  const rootMetadata = await lstat(app);
  const appRoot = await realpath(app);
  if (
    !rootMetadata.isDirectory() ||
    rootMetadata.isSymbolicLink() ||
    !appRoot.startsWith(`${projectRoot}${path.sep}`)
  )
    throw new Error("App bundle must be a project directory.");

  const entries = [];
  let totalBytes = 0;
  async function visit(file, depth = 0) {
    if (depth > 32 || entries.length >= 2048)
      throw new Error("App bundle has too many entries.");
    const stat = await lstat(file);
    const relative =
      path.relative(appRoot, file).split(path.sep).join("/") || ".";
    if (stat.isSymbolicLink() || (!stat.isFile() && !stat.isDirectory()))
      throw new Error("App bundle contains an unsupported entry.");
    const entry = {
      path: relative,
      kind: stat.isDirectory() ? "directory" : "file",
      mode: stat.mode & 0o777,
    };
    if (stat.isFile()) {
      totalBytes += stat.size;
      if (totalBytes > 1024 ** 3)
        throw new Error("App bundle exceeds the candidate byte limit.");
      Object.assign(entry, await digest(file));
    }
    entries.push(entry);
    if (stat.isDirectory()) {
      for (const name of (await readdir(file)).sort())
        await visit(path.join(file, name), depth + 1);
    }
  }
  await visit(appRoot);
  entries.sort((left, right) =>
    left.path < right.path ? -1 : left.path > right.path ? 1 : 0,
  );
  const info = entries.find((item) => item.path === "Contents/Info.plist");
  if (!info || info.kind !== "file") throw new Error("App has no Info.plist.");
  const plist = await metadata(path.join(appRoot, "Contents/Info.plist"));
  if (
    !plist ||
    typeof plist !== "object" ||
    Array.isArray(plist) ||
    plist.CFBundleIdentifier !== expected.identifier ||
    plist.CFBundleShortVersionString !== expected.version ||
    plist.LSMinimumSystemVersion !== expected.minimum ||
    plist.CFBundlePackageType !== "APPL" ||
    !/^[a-zA-Z0-9_-][a-zA-Z0-9._-]{0,127}$/.test(plist.CFBundleExecutable ?? "")
  )
    throw new Error(
      "App metadata differs from the locked candidate configuration.",
    );
  const binary = entries.find(
    (item) => item.path === `Contents/MacOS/${plist.CFBundleExecutable}`,
  );
  if (
    !binary ||
    binary.kind !== "file" ||
    !(binary.mode & 0o111) ||
    binary.sha256 !== executable.sha256 ||
    binary.bytes !== executable.bytes
  )
    throw new Error("App executable differs from the compiled candidate.");
  return {
    path: path.relative(projectRoot, appRoot),
    identifier: plist.CFBundleIdentifier,
    version: plist.CFBundleShortVersionString,
    build_version: plist.CFBundleVersion ?? null,
    minimum_system: plist.LSMinimumSystemVersion,
    executable: plist.CFBundleExecutable,
    bytes: totalBytes,
    package_sha256: createHash("sha256")
      .update(JSON.stringify(entries))
      .digest("hex"),
    digest_format: "sha256-of-sorted-path-kind-mode-size-file-sha256-v1",
    entries,
  };
}
