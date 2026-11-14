import assert from "node:assert/strict";
import test from "node:test";
import {
  verifyNsisExecutable,
  verifyWindowsGuiExecutable,
} from "../../scripts/windows-package-evidence.mjs";

const marker = Buffer.from("__TAURI_BUNDLE_TYPE_VAR_UNK");
const compiled = Buffer.alloc(1024);
compiled.write("MZ");
compiled.writeUInt32LE(0x80, 0x3c);
compiled.writeUInt32LE(0x4550, 0x80);
compiled.writeUInt16LE(0x8664, 0x84);
compiled.writeUInt16LE(0xf0, 0x94);
compiled.writeUInt16LE(0x20b, 0x98);
compiled.writeUInt16LE(2, 0xdc);
marker.copy(compiled, 512);
const packaged = Buffer.from(compiled);
Buffer.from("__TAURI_BUNDLE_TYPE_VAR_NSS").copy(packaged, 512);

test("NSIS proof accepts exactly the pinned Tauri package marker substitution", () => {
  const receipt = verifyNsisExecutable(compiled, packaged);
  assert.equal(receipt.packaged_sha256, receipt.expected_packaged_sha256);
  assert.notEqual(receipt.compiled_sha256, receipt.packaged_sha256);
  assert.equal(receipt.bytes, compiled.length);
  assert.equal(receipt.windows_image.subsystem, "windows_gui");
});
test("NSIS proof rejects unrelated code or resource changes", () => {
  for (const offset of [0x40, packaged.length - 1]) {
    const altered = Buffer.from(packaged);
    altered[offset] ^= 1;
    assert.throws(
      () => verifyNsisExecutable(compiled, altered),
      /beyond Tauri/,
    );
  }
});
test("NSIS proof rejects another package type or an unstamped program", () => {
  assert.throws(() => verifyNsisExecutable(compiled, compiled), /beyond Tauri/);
  const msi = Buffer.from(packaged);
  Buffer.from("__TAURI_BUNDLE_TYPE_VAR_MSI").copy(msi, 512);
  assert.throws(() => verifyNsisExecutable(compiled, msi), /beyond Tauri/);
});
test("NSIS proof rejects extra bytes", () => {
  assert.throws(
    () =>
      verifyNsisExecutable(
        compiled,
        Buffer.concat([packaged, Buffer.from([0])]),
      ),
    /beyond Tauri/,
  );
});
test("NSIS proof rejects a missing or ambiguous compiled marker", () => {
  const missing = Buffer.from(compiled);
  missing.fill(0, 512, 512 + marker.length);
  assert.throws(() => verifyNsisExecutable(missing, packaged), /exactly one/);
  assert.throws(
    () => verifyNsisExecutable(Buffer.concat([compiled, compiled]), packaged),
    /exactly one/,
  );
});

test("console executables fail packaging even when the NSIS payload matches", () => {
  const consoleBuild = Buffer.from(compiled);
  const consolePackage = Buffer.from(packaged);
  consoleBuild.writeUInt16LE(3, 0xdc);
  consolePackage.writeUInt16LE(3, 0xdc);
  assert.throws(
    () => verifyNsisExecutable(consoleBuild, consolePackage),
    /Windows GUI subsystem/,
  );
  assert.throws(
    () => verifyNsisExecutable(compiled, consolePackage),
    /Windows GUI subsystem/,
  );
});

test("GUI validation rejects truncated, non-PE and non-x64 images", () => {
  for (const length of [0, 63, 0x80 + 23, 0xdc + 1]) {
    assert.throws(() =>
      verifyWindowsGuiExecutable(compiled.subarray(0, length)),
    );
  }
  for (const [offset, value] of [
    [0, 0],
    [0x80, 0],
    [0x84, 0x14c],
    [0x94, 69],
    [0x98, 0x10b],
  ]) {
    const altered = Buffer.from(compiled);
    altered.writeUInt16LE(value, offset);
    assert.throws(() => verifyWindowsGuiExecutable(altered));
  }
  const invalidOffset = Buffer.from(compiled);
  invalidOffset.writeUInt32LE(0xffff_ffff, 0x3c);
  assert.throws(() => verifyWindowsGuiExecutable(invalidOffset), /PE header/);
});
