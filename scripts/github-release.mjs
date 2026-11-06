import { execFileSync } from "node:child_process";
import { createHash, createPublicKey, verify } from "node:crypto";
import { createReadStream } from "node:fs";
import { lstat, readFile, readdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { digest } from "./candidate-evidence.mjs";

const versionPattern =
  /^(0|[1-9][0-9]{0,9})\.(0|[1-9][0-9]{0,9})\.(0|[1-9][0-9]{0,9})$/;
const repositoryPattern = /^[A-Za-z0-9-]+\/[A-Za-z0-9_.-]+$/;

export function releaseVersion(tag) {
  const version = tag?.startsWith("v") && tag.slice(1);
  if (!version || !versionPattern.test(version)) {
    throw new Error("Expected a release tag of the form vMAJOR.MINOR.PATCH.");
  }
  return version;
}

function repositoryUrl(repository) {
  if (
    !repositoryPattern.test(repository) ||
    [".", ".."].includes(repository.split("/")[1])
  ) {
    throw new Error(
      "Expected the public GitHub repository in OWNER/REPO form.",
    );
  }
  return "https://github.com/" + repository;
}

export function updateEndpoint(repository) {
  return repositoryUrl(repository) + "/releases/latest/download/latest.json";
}

export function releaseNames(version) {
  const stem = "Orivelle-Orbit-v" + version;
  const update = stem + "-macos-arm64.app.tar.gz";
  return {
    macos: stem + "-macos-arm64.dmg",
    windows: stem + "-windows-x64-setup.exe",
    update,
    signature: update + ".sig",
  };
}

async function validateSourceVersion(tag) {
  const version = releaseVersion(tag);
  const [npm, npmLock, tauri, cargo] = await Promise.all(
    [
      "package.json",
      "package-lock.json",
      "src-tauri/tauri.conf.json",
      "Cargo.toml",
    ].map((file) => readFile(file, "utf8")),
  );
  const workspace = cargo
    .split(/^\[workspace\.package\]\s*$/m)[1]
    ?.split(/^\[/m)[0];
  const versions = [
    JSON.parse(npm).version,
    JSON.parse(npmLock).version,
    JSON.parse(npmLock).packages[""].version,
    JSON.parse(tauri).version,
    workspace?.match(/^version\s*=\s*"([^"]+)"\s*$/m)?.[1],
  ];
  if (versions.some((item) => item !== version)) {
    throw new Error("The release tag must match the committed Orbit versions.");
  }
}

function decodeBase64(value) {
  const bytes = Buffer.from(value, "base64");
  if (!bytes.length || bytes.toString("base64") !== value) {
    throw new Error("Invalid release signing format.");
  }
  return bytes;
}

export function updaterPublicKey(value) {
  const lines = decodeBase64(value ?? "")
    .toString("utf8")
    .trimEnd()
    .split("\n");
  if (lines.length !== 2 || !lines[0].startsWith("untrusted comment: ")) {
    throw new Error(
      "ORBIT_RELEASE_UPDATER_PUBLIC_KEY must contain the Tauri .key.pub text.",
    );
  }
  const bytes = decodeBase64(lines[1]);
  if (bytes.length !== 42 || bytes.toString("ascii", 0, 2) !== "Ed") {
    throw new Error("Invalid Minisign public key.");
  }
  return bytes;
}

export async function verifyUpdaterSignature(
  filename,
  signature,
  version,
  publicKey,
) {
  const key = updaterPublicKey(publicKey);
  const lines = decodeBase64(signature).toString("utf8").trimEnd().split("\n");
  const prefix = "trusted comment: ";
  if (lines.length !== 4 || !lines[2].startsWith(prefix)) {
    throw new Error("Invalid Tauri updater signature.");
  }
  const comment = lines[2].slice(prefix.length);
  const packet = decodeBase64(lines[1]);
  const global = decodeBase64(lines[3]);
  const versionFields = comment
    .split("\t")
    .filter((field) => field.startsWith("version:"));
  if (
    packet.length !== 74 ||
    packet.toString("ascii", 0, 2) !== "ED" ||
    !packet.subarray(2, 10).equals(key.subarray(2, 10)) ||
    global.length !== 64 ||
    versionFields.length !== 1 ||
    versionFields[0] !== "version:" + version
  ) {
    throw new Error(
      "The updater signature must match the configured key and release version.",
    );
  }
  const file = await lstat(filename);
  if (!file.isFile() || file.size === 0 || file.size > 512 * 1024 * 1024) {
    throw new Error("Invalid updater package size.");
  }
  const hash = createHash("blake2b512");
  for await (const chunk of createReadStream(filename)) hash.update(chunk);
  const verifyKey = createPublicKey({
    key: Buffer.concat([
      Buffer.from("302a300506032b6570032100", "hex"),
      key.subarray(10),
    ]),
    format: "der",
    type: "spki",
  });
  const payload = packet.subarray(10);
  if (
    !verify(null, hash.digest(), verifyKey, payload) ||
    !verify(
      null,
      Buffer.concat([payload, Buffer.from(comment)]),
      verifyKey,
      global,
    )
  ) {
    throw new Error(
      "The updater package or trusted version signature is invalid.",
    );
  }
}

export async function assembleRelease(directory, tag, repository, publicKey) {
  const version = releaseVersion(tag);
  const files = releaseNames(version);
  const required = Object.values(files);
  const permitted = new Set([...required, "latest.json", "SHA256SUMS"]);
  const entries = await readdir(directory);
  if (
    entries.some((entry) => !permitted.has(entry)) ||
    required.some((name) => !entries.includes(name))
  ) {
    throw new Error(
      "The macOS and Windows release asset set is incomplete or unexpected.",
    );
  }
  const signatureFile = path.join(directory, files.signature);
  const signatureInfo = await lstat(signatureFile);
  if (!signatureInfo.isFile() || signatureInfo.size > 16 * 1024) {
    throw new Error("Invalid Tauri updater signature file.");
  }
  const signature = (await readFile(signatureFile, "ascii")).trim();
  await verifyUpdaterSignature(
    path.join(directory, files.update),
    signature,
    version,
    publicKey,
  );
  const manifest = {
    version,
    platforms: {
      "darwin-aarch64": {
        url:
          repositoryUrl(repository) +
          "/releases/download/" +
          tag +
          "/" +
          files.update,
        signature,
      },
    },
  };
  await writeFile(
    path.join(directory, "latest.json"),
    JSON.stringify(manifest, null, 2) + "\n",
  );

  const assets = await Promise.all(
    [...required, "latest.json"].sort().map(async (name) => ({
      name,
      ...(await digest(path.join(directory, name))),
    })),
  );
  if (assets.some((asset) => asset.bytes === 0)) {
    throw new Error("A release asset is empty.");
  }
  const checksums = "SHA256SUMS";
  await writeFile(
    path.join(directory, checksums),
    assets.map(({ name, sha256 }) => sha256 + "  " + name + "\n").join(""),
  );
  return [
    ...assets,
    { name: checksums, ...(await digest(path.join(directory, checksums))) },
  ];
}

function gh(args) {
  return execFileSync("gh", args, {
    encoding: "utf8",
    timeout: 180_000,
    maxBuffer: 4 * 1024 * 1024,
  });
}

function api(endpoint, optional = false) {
  try {
    return JSON.parse(gh(["api", endpoint]));
  } catch (error) {
    if (optional && /HTTP 404\b/.test(String(error.stderr))) return undefined;
    throw error;
  }
}

function olderVersion(left, right) {
  const a = releaseVersion(left).split(".").map(Number);
  const b = releaseVersion(right).split(".").map(Number);
  const changed = a.findIndex((component, index) => component !== b[index]);
  return changed >= 0 && a[changed] < b[changed];
}

function isDraft(release, tag) {
  return (
    release.tag_name === tag &&
    release.draft === true &&
    release.prerelease === false
  );
}

function assetMatches(remote, local) {
  return (
    remote.state === "uploaded" &&
    remote.size === local.bytes &&
    remote.digest === "sha256:" + local.sha256
  );
}

async function publishRelease(directory, tag, repository, commit, publicKey) {
  await validateSourceVersion(tag);
  const assets = await assembleRelease(directory, tag, repository, publicKey);
  const repo = "repos/" + repository;
  if (api(repo + "/commits/" + tag).sha !== commit) {
    throw new Error("The release tag changed after the build.");
  }
  const latest = api(repo + "/releases/latest", true);
  if (latest && !olderVersion(latest.tag_name, tag)) {
    throw new Error(
      "The release tag must be newer than the current stable release.",
    );
  }

  const releases = gh([
    "api",
    "--paginate",
    repo + "/releases?per_page=100",
    "--jq",
    ".[] | {id, tag_name, draft, prerelease}",
  ])
    .trim()
    .split("\n")
    .filter(Boolean)
    .map((line) => JSON.parse(line));
  const existing = releases.filter((release) => release.tag_name === tag);
  if (
    existing.length > 1 ||
    (existing.length === 1 && !isDraft(existing[0], tag))
  ) {
    throw new Error(
      "A published release or conflicting draft already exists for this tag.",
    );
  }
  const draft =
    existing.length === 1
      ? api(repo + "/releases/" + existing[0].id)
      : JSON.parse(
          gh([
            "api",
            "--method",
            "POST",
            repo + "/releases",
            "-f",
            "tag_name=" + tag,
            "-f",
            "target_commitish=" + commit,
            "-f",
            "name=Orivelle Orbit " + tag,
            "-F",
            "draft=true",
            "-F",
            "prerelease=false",
            "-F",
            "generate_release_notes=true",
          ]),
        );
  if (
    !isDraft(draft, tag) ||
    draft.assets.some(
      (remote) => !assets.some((local) => local.name === remote.name),
    )
  ) {
    throw new Error(
      "The release draft contains unexpected assets or has already been published.",
    );
  }
  for (const asset of assets) {
    const remote = draft.assets.find((item) => item.name === asset.name);
    if (remote && assetMatches(remote, asset)) continue;
    const args = [
      "release",
      "upload",
      tag,
      path.join(directory, asset.name),
      "--repo",
      repository,
    ];
    if (remote) args.push("--clobber");
    gh(args);
  }

  const uploaded = api(repo + "/releases/" + draft.id);
  if (
    !isDraft(uploaded, tag) ||
    uploaded.assets.length !== assets.length ||
    !assets.every((local) =>
      uploaded.assets.some(
        (remote) => remote.name === local.name && assetMatches(remote, local),
      ),
    )
  ) {
    throw new Error(
      "GitHub release assets do not match the verified local files.",
    );
  }
  if (api(repo + "/commits/" + tag).sha !== commit) {
    throw new Error("The release tag changed before publication.");
  }
  gh([
    "api",
    "--method",
    "PATCH",
    repo + "/releases/" + draft.id,
    "-F",
    "draft=false",
    "-f",
    "make_latest=true",
  ]);
  console.log("Published verified release " + tag);
}

if (
  process.argv[1] &&
  pathToFileURL(path.resolve(process.argv[1])).href === import.meta.url
) {
  const [command, tag, directory] = process.argv.slice(2);
  if (command === "validate-tag") {
    await validateSourceVersion(tag);
  } else if (command === "publish" && directory) {
    await publishRelease(
      path.resolve(directory),
      tag,
      process.env.GITHUB_REPOSITORY,
      process.env.ORBIT_RELEASE_COMMIT,
      process.env.ORBIT_RELEASE_UPDATER_PUBLIC_KEY,
    );
  } else {
    throw new Error("Use validate-tag TAG or publish TAG DIRECTORY.");
  }
}
