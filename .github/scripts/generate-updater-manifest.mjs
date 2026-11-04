import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const { GITHUB_REPOSITORY: repository, GITHUB_REF_NAME: tag } = process.env;
if (!repository || !/^v\d+\.\d+\.\d+(?:-[\w.-]+)?$/.test(tag ?? "")) {
  throw new Error("Expected a repository and a version tag");
}

const baseUrl = `https://github.com/${repository}/releases/download/${tag}`;
const prefix = `CC-Switch-X-${tag}`;
const platforms = {};

function addPlatform(targets, filename) {
  // Tauri expects the signature contents, not a URL or file name.
  const signature = readFileSync(join("dl", `${filename}.sig`), "utf8").trim();
  if (!signature) throw new Error(`Empty updater signature: ${filename}`);
  for (const target of targets) {
    platforms[target] = { signature, url: `${baseUrl}/${filename}` };
  }
}

addPlatform(["darwin-aarch64", "darwin-x86_64"], `${prefix}-macOS.tar.gz`);
addPlatform(["windows-x86_64"], `${prefix}-Windows.msi`);
addPlatform(["windows-aarch64"], `${prefix}-Windows-arm64.msi`);

// Tauri selects the bundle-specific key before the generic OS/architecture key.
for (const [target, filenameArch] of [
  ["x86_64", "x86_64"],
  ["aarch64", "arm64"],
]) {
  const linuxPrefix = `${prefix}-Linux-${filenameArch}`;
  addPlatform(
    [`linux-${target}`, `linux-${target}-appimage`],
    `${linuxPrefix}.AppImage`,
  );
  addPlatform([`linux-${target}-deb`], `${linuxPrefix}.deb`);
  addPlatform([`linux-${target}-rpm`], `${linuxPrefix}.rpm`);
}

writeFileSync(
  "latest.json",
  `${JSON.stringify({ version: tag.slice(1), notes: `Release ${tag}`, pub_date: new Date().toISOString(), platforms }, null, 2)}\n`,
);
