import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  copyFile,
  mkdir,
  readFile,
  readdir,
  rm,
  writeFile,
} from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
const output = path.join(root, ".cache/orbit-brand-export");
const work = path.join(root, ".cache/orbit-brand-generation");
const icons = path.join(root, "src-tauri/icons");
const cli = path.join(root, "node_modules/@tauri-apps/cli/tauri.js");
const sizes = [16, 24, 32, 48, 64, 96, 128, 256, 512, 1024, 2048];
const source = await readFile(
  path.join(root, "src/assets/orbit-mark.svg"),
  "utf8",
);
const geometry = source.match(/<(?:path|circle)\b[^>]*\/>/g)?.join("\n  ");
if (!geometry || !source.includes('viewBox="0 0 256 256"')) {
  throw new Error("Expected the approved 256-unit Orbit vector mark.");
}

function mark(color) {
  return geometry.replaceAll("#326b58", color);
}
function logo(color, size = 256) {
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${size}" height="${size}" viewBox="0 0 256 256" aria-label="Orbit">\n  ${mark(color)}\n</svg>\n`;
}
function applicationIcon(background, foreground) {
  return `<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 112 112" aria-label="Orbit">\n  <rect x="2" y="2" width="108" height="108" rx="25" fill="${background}"/>\n  <g transform="translate(12 12) scale(0.34375)">\n    ${mark(foreground)}\n  </g>\n</svg>\n`;
}
function generate(input, directory, pngSizes = []) {
  const args = [cli, "icon", input, "--output", directory];
  for (const size of pngSizes) args.push("--png", String(size));
  const result = spawnSync(process.execPath, args, {
    cwd: root,
    env: { ...process.env, TMPDIR: work },
    encoding: "utf8",
    timeout: 60000,
  });
  if (result.error || result.status !== 0) {
    throw new Error(
      result.stderr || result.error?.message || "Icon generation failed.",
    );
  }
}

await rm(output, { recursive: true, force: true });
await rm(work, { recursive: true, force: true });
await mkdir(path.join(output, "SVG"), { recursive: true });
await mkdir(path.join(output, "platform"), { recursive: true });
await mkdir(work, { recursive: true });
await mkdir(icons, { recursive: true });

try {
  const variants = {
    "logo-green": logo("#326b58"),
    "logo-white": logo("#eef4ef"),
    "logo-black": logo("#000000"),
    "app-icon-light": applicationIcon("#eef4ef", "#326b58"),
    "app-icon-dark": applicationIcon("#326b58", "#eef4ef"),
  };
  for (const [name, svg] of Object.entries(variants)) {
    const vector = path.join(output, "SVG", `${name}.svg`);
    await writeFile(vector, svg);
    generate(vector, path.join(output, "PNG", name), sizes);
  }
  await copyFile(
    path.join(root, "src/assets/orbit-mark.svg"),
    path.join(output, "SVG/orbit-mark-master.svg"),
  );
  await copyFile(
    path.join(output, "SVG/app-icon-light.svg"),
    path.join(root, "src/assets/orbit-app-icon.svg"),
  );

  const native = path.join(work, "native");
  generate(path.join(output, "SVG/app-icon-light.svg"), native);
  for (const name of ["icon.icns", "icon.ico"]) {
    await copyFile(path.join(native, name), path.join(icons, name));
    await copyFile(
      path.join(native, name),
      path.join(output, "platform", name),
    );
  }
  const light = path.join(output, "PNG/app-icon-light");
  for (const size of [16, 32, 64, 128, 256, 512, 1024]) {
    await copyFile(
      path.join(light, `${size}x${size}.png`),
      path.join(icons, `${size}x${size}.png`),
    );
  }
  await copyFile(
    path.join(light, "256x256.png"),
    path.join(icons, "128x128@2x.png"),
  );
  await copyFile(path.join(light, "512x512.png"), path.join(icons, "icon.png"));
  await writeFile(path.join(icons, "tray-template.svg"), logo("#000000", 24));
  await copyFile(
    path.join(output, "PNG/logo-black/48x48.png"),
    path.join(icons, "tray-template@2x.png"),
  );
  await copyFile(
    path.join(icons, "tray-template.svg"),
    path.join(output, "platform/tray-template.svg"),
  );
  await copyFile(
    path.join(icons, "tray-template@2x.png"),
    path.join(output, "platform/tray-template@2x.png"),
  );

  await writeFile(
    path.join(output, "README.txt"),
    [
      "Orbit 标志定稿 · 2026-11-12",
      "",
      "SVG/orbit-mark-master.svg 是可任意缩放的矢量母版。",
      "SVG/ 包含森林绿、雾白、黑色透明标志及浅色、深色应用图标。",
      `PNG/ 为五种版本分别提供 ${sizes.join("、")} 像素的正方形无损 PNG。`,
      "所有尺寸均从 SVG 直接渲染，不以小图放大，不使用有损压缩。",
      "logo-white 使用品牌雾白 #EEF4EF。浅色应用图标为当前应用默认版本。",
      "platform/ 包含 macOS ICNS、Windows ICO 和菜单栏模板。",
      "",
      "定稿参数：圆环半径 80、线宽 24、缺口 60°；圆点半径 18。",
      "画布为 256 × 256，圆点中心为 (191.640, 64.360)。",
      "品牌色：森林绿 #326B58，雾白 #EEF4EF。",
      "manifest.json 记录文件尺寸、色深和 SHA-256，便于无损归档核对。",
      "",
    ].join("\n"),
  );
  const files = [];
  async function inventory(directory) {
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      const absolute = path.join(directory, entry.name);
      if (entry.isDirectory()) {
        await inventory(absolute);
        continue;
      }
      const data = await readFile(absolute);
      const item = {
        path: path.relative(output, absolute).split(path.sep).join("/"),
        bytes: data.length,
        sha256: createHash("sha256").update(data).digest("hex"),
      };
      if (entry.name.endsWith(".png")) {
        if (
          !data
            .subarray(0, 8)
            .equals(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]))
        ) {
          throw new Error(`Invalid PNG: ${item.path}`);
        }
        Object.assign(item, {
          width: data.readUInt32BE(16),
          height: data.readUInt32BE(20),
          bitDepth: data[24],
          colorType: data[25],
        });
        const expected = entry.name.match(/^(\d+)x(\d+)\.png$/);
        if (
          expected &&
          (item.width !== Number(expected[1]) ||
            item.height !== Number(expected[2]))
        ) {
          throw new Error(`Incorrect PNG dimensions: ${item.path}`);
        }
        if (item.bitDepth !== 8 || item.colorType !== 6) {
          throw new Error(`Expected lossless RGBA PNG: ${item.path}`);
        }
      }
      files.push(item);
    }
  }
  await inventory(output);
  files.sort((a, b) => a.path.localeCompare(b.path, "en"));
  await writeFile(
    path.join(output, "manifest.json"),
    `${JSON.stringify(
      {
        brand: "Orivelle Orbit",
        master: "SVG/orbit-mark-master.svg",
        sizes,
        files,
      },
      null,
      2,
    )}\n`,
  );
  console.log(
    `Generated ${sizes.length} resolutions for ${Object.keys(variants).length} variants: ${output}`,
  );
} finally {
  await rm(work, { recursive: true, force: true });
}
