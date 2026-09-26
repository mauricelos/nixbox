/**
 * Runs every snippet in nixbox/tier-a/test/snippets/ through the committed
 * nodejs-target wasm build (nixbox/tier-a/pkg-node/, produced by
 * nixbox/build.sh) and checks it against its `.expected` file: value-exact
 * for a printing snippet, primary-message-exact (the last `error: ...`
 * line) for an error snippet. Needs no Rust toolchain — the wasm is
 * committed, same as public/nixbox/'s web-target build (see
 * nixbox/README.md).
 */
import { readdirSync, readFileSync, statSync } from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";
import { describe, expect, it } from "vitest";

const require = createRequire(import.meta.url);

const TIER_A_DIR = path.resolve(import.meta.dirname, "..");
const SNIPPETS_DIR = path.resolve(import.meta.dirname, "snippets");
const PKG_NODE = path.join(TIER_A_DIR, "pkg-node", "nixbox_tier_a.cjs");

type EvalResult = {
  ok: boolean;
  printed?: string | null;
  json?: unknown;
  message?: string | null;
  errorClass?: string | null;
  line?: number | null;
  col?: number | null;
};

type Snippet = { name: string; files: Record<string, string>; entry: string; expected: string };

function loadSnippets(): Snippet[] {
  const entries = readdirSync(SNIPPETS_DIR).sort();
  const out: Snippet[] = [];
  for (const entry of entries) {
    const full = path.join(SNIPPETS_DIR, entry);
    if (statSync(full).isDirectory()) {
      // e.g. 35-import/{default.nix,pkg.nix,default.expected}
      const files: Record<string, string> = {};
      let expected: string | null = null;
      for (const f of readdirSync(full)) {
        if (f.endsWith(".nix")) {
          files[f] = readFileSync(path.join(full, f), "utf8");
        } else if (f.endsWith(".expected")) {
          expected = readFileSync(path.join(full, f), "utf8");
        }
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

/** The primary error message: the last `error: ...` line, per nixbox/tier-a/test/snippets/README.md. */
function primaryErrorMessage(expectedText: string): string | null {
  let last: string | null = null;
  for (const line of expectedText.split("\n")) {
    const m = /^error: (.*)$/.exec(line.trim());
    if (m) last = m[1]!;
  }
  return last;
}

function compare(expectedText: string, result: EvalResult): { pass: boolean; detail: string } {
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
  } else {
    if (!result.ok) {
      return { pass: false, detail: `expected ok, got error: ${result.message}` };
    }
    const pass = result.printed === trimmedExpected;
    return { pass, detail: pass ? "" : `expected ${JSON.stringify(trimmedExpected)}, got ${JSON.stringify(result.printed)}` };
  }
}

// Two bonus snippets (beyond the 35 required by WP10) document known,
// accepted gaps rather than required parity — see their entries in
// docs/spikes/A2-snix.md's parity table:
//   36-select-on-function: a function value in CppNix's error-context text
//     prints as `«lambda @ file:line:col»`; tvix-eval prints `<LAMBDA>`
//     (which IS the correct rendering for actual *value* printing, just not
//     for this one error-message context) — would need an upstream
//     tvix-eval patch to thread span info into that specific message.
//   37-elemat-oob: CppNix's `elemAt` out-of-bounds message names the
//     builtin and the list's size; tvix-eval's `IndexOutOfBounds` ErrorKind
//     carries neither — same story, needs an upstream patch, deferred.
const KNOWN_GAPS = new Set(["36-select-on-function", "37-elemat-oob"]);

describe("NixBox tier A parity (nixbox/tier-a/test/snippets)", () => {
  const { eval_json } = require(PKG_NODE) as { eval_json: (input: string) => string };
  const snippets = loadSnippets();

  it("has snippets to check", () => {
    expect(snippets.length).toBeGreaterThan(0);
  });

  for (const s of snippets) {
    const run = () => {
      const req = { files: s.files, entry: s.entry, timeoutMs: 5000, strict: true };
      const raw = eval_json(JSON.stringify(req));
      const result = JSON.parse(raw) as EvalResult;
      const { pass, detail } = compare(s.expected, result);
      expect(pass, detail).toBe(true);
    };
    if (KNOWN_GAPS.has(s.name)) {
      // vitest's `.fails`: passes the suite as long as this stays failing;
      // it starts failing the *suite* the day it starts passing, which is
      // exactly the nudge to promote it out of KNOWN_GAPS.
      it.fails(`${s.name} (known gap, see KNOWN_GAPS above)`, run);
    } else {
      it(`${s.name}`, run);
    }
  }
});
