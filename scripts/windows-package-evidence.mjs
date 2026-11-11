import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";

const compiledMarker = Buffer.from("__TAURI_BUNDLE_TYPE_VAR_UNK");
const nsisMarker = Buffer.from("__TAURI_BUNDLE_TYPE_VAR_NSS");
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");

export function verifyNsisExecutable(compiled, packaged) {
  // The pinned Tauri bundler stamps the package type, then restores the build
  // executable. Reproduce only that exact transformation; every other byte
  // must match. https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.12.2/crates/tauri-bundler/src/bundle.rs
  const offset = compiled.indexOf(compiledMarker);
  assert(
    offset >= 0 && compiled.lastIndexOf(compiledMarker) === offset,
    "The compiled executable must contain exactly one Tauri bundle-type marker.",
  );
  const expected = Buffer.from(compiled);
  nsisMarker.copy(expected, offset);
  assert(
    expected.equals(packaged),
    "The NSIS program differs from the compiled executable beyond Tauri's package marker.",
  );
  return {
    verification: "Exact bytes with the pinned Tauri UNK to NSS package marker",
    marker_offset: offset,
    bytes: packaged.length,
    compiled_sha256: hash(compiled),
    expected_packaged_sha256: hash(expected),
    packaged_sha256: hash(packaged),
  };
}

if (
  process.argv[1] &&
  pathToFileURL(path.resolve(process.argv[1])).href === import.meta.url
) {
  const [compiledPath, packagedPath, receiptPath] = process.argv.slice(2);
  assert(
    compiledPath && packagedPath,
    "Pass the compiled and packaged executable paths.",
  );
  const receipt = verifyNsisExecutable(
    await readFile(compiledPath),
    await readFile(packagedPath),
  );
  if (receiptPath)
    await writeFile(receiptPath, JSON.stringify(receipt, null, 2) + "\n");
  console.log(JSON.stringify(receipt));
}
