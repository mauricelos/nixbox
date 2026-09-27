#!/usr/bin/env node
/**
 * Standalone parity-suite runner, with no test-framework dependency: used
 * by `nix flake check` (which has no npm/vitest available) and by
 * `./build.sh`-based workflows that just want a pass/fail exit code. Same
 * comparison rules as test/parity.test.ts (the vitest suite kept for
 * editor integration / a single `npm test`-style workflow when this repo
 * is used from a Node project directly) — see that file for the fuller
 * rationale in comments.
 *
 * Usage: node run-parity-tests.mjs [path/to/pkg-node/nixbox_tier_a.cjs]
 * Defaults to ./pkg-node/nixbox_tier_a.cjs relative to this file.
 */
import { readdirSync, readFileSync, statSync } from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const HERE = path.dirname(new URL(import.meta.url).pathname);
const SNIPPETS_DIR = path.join(HERE, "test", "snippets");
const PKG_NODE = path.resolve(process.argv[2] ?? path.join(HERE, "pkg-node", "nixbox_tier_a.cjs"));

const KNOWN_GAPS = new Set(["36-select-on-function", "37-elemat-oob"]);

function loadSnippets() {
  const entries = readdirSync(SNIPPETS_DIR).sort();
  const out = [];
  for (const entry of entries) {
    const full = path.join(SNIPPETS_DIR, entry);
    if (statSync(full).isDirectory()) {
      const files = {};
      let expected = null;
      for (const f of readdirSync(full)) {
        if (f.endsWith(".nix")) files[f] = readFileSync(path.join(full, f), "utf8");
        else if (f.endsWith(".expected")) expected = readFileSync(path.join(full, f), "utf8");
      }
      if (expected !== null) out.push({ name: entry, files, entry: "default.nix", expected });
    } else if (entry.endsWith(".nix")) {
      const name = entry.slice(0, -4);
      const expectedPath = path.join(SNIPPETS_DIR, `${name}.expected`);
      out.push({
        name,
        files: { [entry]: readFileSync(full, "utf8") },
        entry,
        expected: readFileSync(expectedPath, "utf8"),
      });
    }
  }
  return out;
}

function primaryErrorMessage(expectedText) {
  let last = null;
  for (const line of expectedText.split("\n")) {
    const m = /^error: (.*)$/.exec(line.trim());
    if (m) last = m[1];
  }
  return last;
}

function compare(expectedText, result) {
  const trimmedExpected = expectedText.replace(/\n$/, "");
  const isErrorExpected =
    trimmedExpected.startsWith("error:") || /\nerror: /.test(expectedText) || expectedText.startsWith("error");
  if (isErrorExpected) {
    if (result.ok) {
      return { pass: false, detail: `expected error, got ok printed=${JSON.stringify(result.printed)}` };
    }
    const want = primaryErrorMessage(expectedText);
    const got = result.message;
    const pass = want !== null && got === want;
    return { pass, detail: pass ? "" : `expected primary message ${JSON.stringify(want)}, got ${JSON.stringify(got)}` };
  }
  if (!result.ok) {
    return { pass: false, detail: `expected ok, got error: ${result.message}` };
  }
  const pass = result.printed === trimmedExpected;
  return { pass, detail: pass ? "" : `expected ${JSON.stringify(trimmedExpected)}, got ${JSON.stringify(result.printed)}` };
}

const { eval_json } = require(PKG_NODE);
const snippets = loadSnippets();
if (snippets.length === 0) {
  console.error("no snippets found under", SNIPPETS_DIR);
  process.exit(1);
}

let failures = 0;
let unexpectedPasses = 0;
for (const s of snippets) {
  const req = { files: s.files, entry: s.entry, timeoutMs: 5000, strict: true };
  const raw = eval_json(JSON.stringify(req));
  const result = JSON.parse(raw);
  const { pass, detail } = compare(s.expected, result);
  const isKnownGap = KNOWN_GAPS.has(s.name);
  if (isKnownGap) {
    if (pass) {
      unexpectedPasses++;
      console.error(`UNEXPECTED PASS ${s.name} (remove from KNOWN_GAPS)`);
    } else {
      console.log(`ok (known gap) ${s.name}`);
    }
  } else if (pass) {
    console.log(`ok ${s.name}`);
  } else {
    failures++;
    console.error(`FAIL ${s.name}: ${detail}`);
  }
}

console.log(`\n${snippets.length} snippets, ${failures} failures, ${unexpectedPasses} unexpected known-gap passes`);
if (failures > 0 || unexpectedPasses > 0) process.exit(1);
