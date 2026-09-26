#!/usr/bin/env node
// Bundles the pinned Nixpkgs `lib/` (nixbox/nixpkgs-lib/lib) and the
// teaching `pkgs` stub (nixbox/pkgs/default.nix) into one JSON file,
// gzip-compressed, served at public/nixbox/vfs.json.gz. Loaded lazily by
// lib/runtime/tier-a and decompressed client-side with
// `DecompressionStream("gzip")` (see lib/runtime/tier-a/worker.ts) so the
// static file can be served as-is without relying on a server setting
// `Content-Encoding: gzip` on a `.gz` path.
//
// Every key is a virtual path exactly as tvix-eval's `EvalIO` (see
// nixbox/tier-a/src/lib.rs's `VfsIo`) will see it: "nixpkgs/lib/…" for the
// vendored lib, "nixpkgs/default.nix" for the pkgs stub — reachable at
// eval time as `import <nixpkgs/lib>` / `(import <nixpkgs> {}).lib` and
// `import <nixpkgs> {}` respectively, since lib.rs sets
// `NIX_PATH=nixpkgs=/nixpkgs`.
import { readFileSync, readdirSync, statSync, mkdirSync, writeFileSync } from "node:fs";
import { gzipSync } from "node:zlib";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "..", "..");
const nixboxDir = path.resolve(here, "..");

function walk(dir, base, out) {
  for (const entry of readdirSync(dir).sort()) {
    const full = path.join(dir, entry);
    const rel = base ? `${base}/${entry}` : entry;
    if (statSync(full).isDirectory()) {
      walk(full, rel, out);
    } else if (entry.endsWith(".nix")) {
      out[rel] = readFileSync(full, "utf8");
    }
  }
}

const files = {};
walk(path.join(nixboxDir, "nixpkgs-lib", "lib"), "nixpkgs/lib", files);
files["nixpkgs/default.nix"] = readFileSync(path.join(nixboxDir, "pkgs", "default.nix"), "utf8");

const json = JSON.stringify(files);
const rawBytes = Buffer.byteLength(json, "utf8");
const gz = gzipSync(Buffer.from(json, "utf8"), { level: 9 });

const outDir = path.join(root, "public", "nixbox");
mkdirSync(outDir, { recursive: true });
const outPath = path.join(outDir, "vfs.json.gz");
writeFileSync(outPath, gz);

console.log(`nixbox: vfs.json.gz — ${Object.keys(files).length} files`);
console.log(`  raw JSON: ${(rawBytes / 1024).toFixed(0)} KiB`);
console.log(`  gzip:     ${(gz.length / 1024).toFixed(0)} KiB -> ${outPath}`);
