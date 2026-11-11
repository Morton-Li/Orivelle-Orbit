import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import {
  copyFile,
  mkdir,
  readFile,
  readdir,
  writeFile,
} from "node:fs/promises";
import path from "node:path";

const [builtPath, packagedPath, directory] = process.argv.slice(2);
assert(
  builtPath && packagedPath && directory,
  "Pass the built executable, packaged executable and evidence directory.",
);
await mkdir(directory, { recursive: true });
const built = await readFile(builtPath);
const packaged = await readFile(packagedPath);
function pe(bytes) {
  assert.equal(bytes.toString("ascii", 0, 2), "MZ");
  const offset = bytes.readUInt32LE(0x3c);
  assert.equal(bytes.readUInt32LE(offset), 0x4550);
  const count = bytes.readUInt16LE(offset + 6);
  const table = offset + 24 + bytes.readUInt16LE(offset + 20);
  return {
    pe_offset: offset,
    timestamp: bytes.readUInt32LE(offset + 8),
    checksum: bytes.readUInt32LE(offset + 24 + 64),
    sections: Array.from({ length: count }, (_, index) => {
      const section = table + index * 40;
      return {
        name: bytes.toString("ascii", section, section + 8).replace(/\0/g, ""),
        bytes: bytes.readUInt32LE(section + 16),
        offset: bytes.readUInt32LE(section + 20),
      };
    }),
  };
}
let count = Math.abs(built.length - packaged.length);
const first = [];
for (let index = 0; index < Math.min(built.length, packaged.length); index++) {
  if (built[index] === packaged[index]) continue;
  count++;
  if (first.length < 128)
    first.push({
      offset: index,
      built: built[index],
      packaged: packaged[index],
    });
}
const digest = (bytes) => createHash("sha256").update(bytes).digest("hex");
const receipt = {
  built: { bytes: built.length, sha256: digest(built), ...pe(built) },
  packaged: {
    bytes: packaged.length,
    sha256: digest(packaged),
    ...pe(packaged),
  },
  different_bytes: count,
  first_differences: first,
};
await writeFile(
  path.join(directory, "binary-diff.json"),
  JSON.stringify(receipt, null, 2) + "\n",
);
await copyFile(builtPath, path.join(directory, "built.exe"));
await copyFile(packagedPath, path.join(directory, "packaged.exe"));
const nsis = path.resolve(
  "runtime/target/x86_64-pc-windows-msvc/release/nsis/x64",
);
for (const file of await readdir(nsis)) {
  if (file.endsWith(".nsi"))
    await copyFile(path.join(nsis, file), path.join(directory, file));
}
console.log(
  JSON.stringify({
    different_bytes: count,
    built_bytes: built.length,
    packaged_bytes: packaged.length,
    first_differences: first.slice(0, 16),
  }),
);
