#!/usr/bin/env bash
# Rebuilds the NixBox artifact from source, into dist/ (or $NIXBOX_OUT_DIR):
#   1. tier-a -> wasm32-unknown-unknown, release
#   2. wasm-bindgen (web target) -> the glue JS + .wasm a host loads
#   3. nixpkgs-lib + pkgs -> vfs.json.gz (see scripts/build-vfs.mjs)
#   4. wasm-bindgen (nodejs target) -> tier-a/pkg-node, for the parity suite
#
# Requires:
#   rustup target add wasm32-unknown-unknown
#   cargo install wasm-bindgen-cli --version 0.2.129 --locked
#     (must match tier-a/Cargo.lock's wasm-bindgen version exactly —
#     wasm-bindgen refuses to process a .wasm built against a different
#     version of its own runtime support crate)
#   Node (for scripts/build-vfs.mjs)
#
# Nothing here needs network access beyond what `cargo build` itself needs
# (crates.io, for tier-a's own direct dependencies — vendor/tvix-eval and
# vendor/nix-compat are vendored source, not git/registry dependencies, so
# no access to git.snix.dev or github.com is needed to build).
#
# `flake.nix` does the same thing reproducibly, with a pinned toolchain and
# wasm-bindgen-cli build; prefer it when Nix is available.
set -euo pipefail
cd "$(dirname "$0")"

echo "==> cargo build --release --target wasm32-unknown-unknown (tier-a)"
(cd tier-a && cargo build --release --target wasm32-unknown-unknown)

TARGET_DIR="${CARGO_TARGET_DIR:-tier-a/target}"
WASM="$TARGET_DIR/wasm32-unknown-unknown/release/nixbox_tier_a.wasm"
if [ ! -f "$WASM" ]; then
  WASM="tier-a/target/wasm32-unknown-unknown/release/nixbox_tier_a.wasm"
fi
echo "==> raw wasm: $(du -h "$WASM" | cut -f1)"

OUT="${NIXBOX_OUT_DIR:-dist}"
mkdir -p "$OUT"

echo "==> wasm-bindgen (web target) -> $OUT"
wasm-bindgen "$WASM" --target web --out-dir "$OUT" --out-name nixbox_tier_a --no-typescript

# A second, nodejs-target build for the parity test suite (test/parity.test.ts
# runs in Node/vitest, not a browser, so it needs a CommonJS-loadable build
# rather than the web target's ESM-with-explicit-init one).
echo "==> wasm-bindgen (nodejs target) -> tier-a/pkg-node (parity tests)"
rm -rf tier-a/pkg-node
wasm-bindgen "$WASM" --target nodejs --out-dir tier-a/pkg-node --out-name nixbox_tier_a --no-typescript
# The nodejs target's glue uses `require`/`exports` (CommonJS); give it a
# .cjs extension so it loads correctly from an ESM ("type": "module") test
# runner too.
mv tier-a/pkg-node/nixbox_tier_a.js tier-a/pkg-node/nixbox_tier_a.cjs

# wasm-opt -Oz was tried and made the *compressed* size worse (code-layout
# changes hurt gzip/brotli more than the raw-size cut helped), so it's
# intentionally not run here.

echo "==> vfs.json.gz (nixpkgs lib + pkgs stub)"
NIXBOX_OUT_DIR="$OUT" node scripts/build-vfs.mjs

echo "==> sizes (what a browser downloads)"
for f in "$OUT"/nixbox_tier_a_bg.wasm "$OUT"/nixbox_tier_a.js "$OUT"/vfs.json.gz; do
  raw=$(stat -c%s "$f" 2>/dev/null || stat -f%z "$f")
  gz=$(gzip -9 -c "$f" | wc -c)
  br=""
  if command -v brotli >/dev/null 2>&1; then
    br=$(brotli -q 11 -c "$f" | wc -c)
  fi
  echo "  $(basename "$f"): raw=${raw}B gzip=${gz}B${br:+ brotli=${br}B}"
done

echo "==> done. $OUT/ is ready to serve (nixbox_tier_a.js, nixbox_tier_a_bg.wasm, vfs.json.gz)."
