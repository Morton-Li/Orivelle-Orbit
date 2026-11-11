import assert from "node:assert/strict";
import test from "node:test";
import { verifyNsisExecutable } from "../../scripts/windows-package-evidence.mjs";

const compiled = Buffer.from(
  "MZ synthetic executable\0__TAURI_BUNDLE_TYPE_VAR_UNK\0unchanged code and resources",
);
const packaged = Buffer.from(compiled.toString().replace("VAR_UNK", "VAR_NSS"));

test("NSIS proof accepts exactly the pinned Tauri package marker substitution", () => {
  const receipt = verifyNsisExecutable(compiled, packaged);
  assert.equal(receipt.packaged_sha256, receipt.expected_packaged_sha256);
  assert.notEqual(receipt.compiled_sha256, receipt.packaged_sha256);
  assert.equal(receipt.bytes, compiled.length);
});
test("NSIS proof rejects unrelated code or resource changes", () => {
  for (const offset of [0, packaged.length - 1]) {
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
  assert.throws(
    () =>
      verifyNsisExecutable(
        compiled,
        Buffer.from(packaged.toString().replace("VAR_NSS", "VAR_MSI")),
      ),
    /beyond Tauri/,
  );
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
  assert.throws(
    () => verifyNsisExecutable(Buffer.from("no marker"), packaged),
    /exactly one/,
  );
  assert.throws(
    () => verifyNsisExecutable(Buffer.concat([compiled, compiled]), packaged),
    /exactly one/,
  );
});
