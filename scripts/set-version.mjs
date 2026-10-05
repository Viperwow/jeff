// Writes one version into every file that carries it: `node scripts/set-version.mjs 1.2.3`.
import { readFileSync, writeFileSync } from "node:fs";

const version = process.argv[2];
if (!/^\d+\.\d+\.\d+$/.test(version ?? "")) throw new Error(`not a version: ${version}`);

function edit(path, pattern, replacement) {
  const text = readFileSync(path, "utf8");
  if (!pattern.test(text)) throw new Error(`no version found in ${path}`);
  writeFileSync(path, text.replace(pattern, replacement));
}

edit("Cargo.toml", /(\[package\][^[]*?\nversion = )"[^"]*"/, `$1"${version}"`);
edit("Cargo.lock", /(name = "jengine"\r?\nversion = )"[^"]*"/, `$1"${version}"`);
