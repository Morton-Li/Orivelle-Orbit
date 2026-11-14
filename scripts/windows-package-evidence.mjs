import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";

const compiledMarker = Buffer.from("__TAURI_BUNDLE_TYPE_VAR_UNK");
const nsisMarker = Buffer.from("__TAURI_BUNDLE_TYPE_VAR_NSS");
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");

export function verifyWindowsGuiExecutable(bytes) {
  // PE32+ Optional Header: Subsystem is the WORD at offset 68.
  // https://learn.microsoft.com/windows/win32/debug/pe-format
  assert(
    bytes.length >= 0x40 && bytes.toString("ascii", 0, 2) === "MZ",
    "Orbit must be a Windows PE executable.",
  );
  const pe = bytes.readUInt32LE(0x3c);
  assert(
    pe >= 0x40 && pe + 24 <= bytes.length && bytes.readUInt32LE(pe) === 0x4550,
    "Orbit has an invalid PE header.",
  );
  const optional = pe + 24;
  const optionalBytes = bytes.readUInt16LE(pe + 20);
  assert(
    bytes.readUInt16LE(pe + 4) === 0x8664 &&
      optionalBytes >= 70 &&
      optional + optionalBytes <= bytes.length &&
      bytes.readUInt16LE(optional) === 0x20b,
    "Orbit must be a Windows x64 PE32+ executable.",
  );
  assert.equal(
    bytes.readUInt16LE(optional + 68),
    2,
    "Orbit must use the Windows GUI subsystem; console builds open a terminal.",
  );
  return { machine: "amd64", format: "PE32+", subsystem: "windows_gui" };
}

export function verifyNsisExecutable(compiled, packaged) {
  const windowsImage = verifyWindowsGuiExecutable(compiled);
  verifyWindowsGuiExecutable(packaged);
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
    windows_image: windowsImage,
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
