#!/usr/bin/env bash
# Rebuilds every committed artifact under public/nixbox/ from source:
#   1. nixbox/tier-a -> wasm32-unknown-unknown, release
#   2. wasm-bindgen (web target) -> the glue JS + .wasm the app loads
#   3. nixbox/nixpkgs-lib + nixbox/pkgs -> vfs.json.gz (see build-vfs.mjs)
#   4. lib/runtime/tier-a/worker.ts -> public/nixbox/worker.js (esbuild)
#
# Requires:
#   rustup target add wasm32-unknown-unknown
#   cargo install wasm-bindgen-cli --version 0.2.129 --locked
#     (must match nixbox/tier-a/Cargo.lock's wasm-bindgen version exactly —
#     wasm-bindgen refuses to process a .wasm built against a different
#     version of its own runtime support crate)
#   Node + esbuild (npm's, via npx — for build-vfs.mjs and worker.ts)
#
# Nothing here needs network access beyond what `cargo build` itself needs
# (crates.io, for nixbox/tier-a's own direct dependencies — vendor/tvix-eval
# and vendor/nix-compat are vendored source, not git/registry dependencies,
# so no access to git.snix.dev or github.com is needed to build).
set -euo pipefail
cd "$(dirname "$0")"

echo "==> cargo build --release --target wasm32-unknown-unknown (nixbox/tier-a)"
(cd tier-a && cargo build --release --target wasm32-unknown-unknown)

TARGET_DIR="${CARGO_TARGET_DIR:-tier-a/target}"
WASM="$TARGET_DIR/wasm32-unknown-unknown/release/nixbox_tier_a.wasm"
if [ ! -f "$WASM" ]; then
  WASM="tier-a/target/wasm32-unknown-unknown/release/nixbox_tier_a.wasm"
fi
echo "==> raw wasm: $(du -h "$WASM" | cut -f1)"

OUT="../public/nixbox"
mkdir -p "$OUT"

echo "==> wasm-bindgen (web target) -> $OUT"
wasm-bindgen "$WASM" --target web --out-dir "$OUT" --out-name nixbox_tier_a --no-typescript

# A second, nodejs-target build for the parity test suite (test/parity.test.ts
# runs in Node/vitest, not a browser, so it needs a CommonJS-loadable build
# rather than the web target's ESM-with-explicit-init one). Committed under
# nixbox/tier-a/pkg-node/ so `npm test` needs no Rust toolchain, matching
# the "runs in Node against the committed wasm" requirement.
echo "==> wasm-bindgen (nodejs target) -> tier-a/pkg-node (parity tests)"
rm -rf tier-a/pkg-node
wasm-bindgen "$WASM" --target nodejs --out-dir tier-a/pkg-node --out-name nixbox_tier_a --no-typescript
# package.json has "type": "module", so wasm-bindgen's CommonJS glue (it
# uses `require`/`exports`) needs the .cjs extension to be loadable at all.
mv tier-a/pkg-node/nixbox_tier_a.js tier-a/pkg-node/nixbox_tier_a.cjs

# wasm-opt -Oz was tried in the WP7 spike this productionizes and made the
# *compressed* size worse (code-layout changes hurt gzip/brotli more than
# the raw-size cut helped — see docs/spikes/A2-snix.md, "Bundle size"), so
# it's intentionally not run here.

echo "==> vfs.json.gz (nixpkgs lib + pkgs stub)"
node scripts/build-vfs.mjs

# The worker is built as a plain static asset (not a Vite-bundled
# `new Worker(new URL("./worker.ts", import.meta.url))`) because the app's
# framework (vinext, a Next.js-on-Vite layer targeting Cloudflare Workers)
# rewrites `import.meta.url` in client code to a synthetic, non-navigable
# `file:///ROOT/...` marker (see its import-meta-url Vite plugin), which
# `new Worker()` then refuses to load ("cannot be accessed from origin ...")
# — the browser's same-origin check happens before any network request, so
# fixing up the URL's origin client-side can't help. Serving a prebuilt,
# plain-path worker script from public/ sidesteps the whole transform: the
# runtime just does `new Worker("/nixbox/worker.js")`, identically in dev
# and production. worker.ts's own `import(glueUrl)` for the wasm glue is a
# genuinely dynamic (non-static) specifier, so esbuild leaves it as a real
# runtime `import()` — exactly what's wanted, since that URL is only known
# at request time (BASE + "nixbox_tier_a.js").
echo "==> esbuild (lib/runtime/tier-a/worker.ts -> $OUT/worker.js)"
npx --yes esbuild ../lib/runtime/tier-a/worker.ts \
  --bundle --format=esm --target=es2022 --platform=browser \
  --outfile="$OUT/worker.js"

echo "==> sizes (what the browser downloads)"
for f in "$OUT"/nixbox_tier_a_bg.wasm "$OUT"/nixbox_tier_a.js "$OUT"/vfs.json.gz "$OUT"/worker.js; do
  raw=$(stat -c%s "$f" 2>/dev/null || stat -f%z "$f")
  gz=$(gzip -9 -c "$f" | wc -c)
  br=""
  if command -v brotli >/dev/null 2>&1; then
    br=$(brotli -q 11 -c "$f" | wc -c)
  fi
  echo "  $(basename "$f"): raw=${raw}B gzip=${gz}B${br:+ brotli=${br}B}"
done

echo "==> done. public/nixbox/ is ready to serve; lib/runtime/tier-a/ loads it lazily."
