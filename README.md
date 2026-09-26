# NixBox tier A

The real Nix language evaluator, running client-side. This is Tier A of the
"NixBox" design in `docs/PLAN.md`: `tvix-eval` (the evaluator lineage that
became `snix-eval`) compiled to `wasm32-unknown-unknown`, running in a
dedicated Web Worker behind the app's `NixRuntime` interface
(`lib/runtime/types.ts`). It productionizes the spike at
`docs/spikes/A2-snix.md` (the spike crate that lived at `spikes/snix-wasm/`
has been moved here and superseded).

No real Nix store, no builds, no network access at eval time — everything
runs in an in-memory virtual filesystem per request.

## Layout

- `tier-a/` — the Rust crate (`nixbox-tier-a`), compiled to wasm.
  - `src/lib.rs` — `EvalRequest`/`EvalResponse`, the in-memory `EvalIO` VFS
    (`VfsIo`, with two documented wasm-specific workarounds), and
    `run_eval`, the eval entry point.
  - `src/cppnix.rs` — maps `tvix_eval::ErrorKind` (and rnix parser errors)
    to CppNix 2.28's exact primary-message wording, using spans/source text
    to recover details CppNix's message needs but tvix's `ErrorKind` alone
    doesn't carry (e.g. the actual operand values in an `assert` failure).
    This is the highest-value, most novel part of this crate.
  - `src/pkgs_builtins.rs` — a native `derivationStrict` builtin producing
    real `.drv`/output paths via `nix-compat`, backed by an in-memory
    "known paths" store (no real store or build).
  - `vendor/tvix-eval/`, `vendor/nix-compat/` — vendored (not git/registry)
    copies of the pinned tvix commit (`9fbca101331eecbf5c1cd44861f3d24aff101b82`,
    the public `tvlfyi/tvix` mirror — `snix-eval`'s upstream home,
    `git.snix.dev`, is blocked by network policy), trimmed to what's needed
    for wasm32 and patched:
    - a few `ErrorKind` variants (`TypeError`, `NotCallable`,
      `NotCoercibleToString`) gained a `value: String` field, so
      `cppnix.rs` can put the actual value in the message the way CppNix
      does.
    - `Compiler::new` no longer calls `std::env::current_dir()` (unavailable
      on wasm32) when compiling with no source location — needed for
      `add_src_builtin` (the well-known `derivation` function) to work at
      all on wasm32. See the "NixBox wasm32 patch" comment in
      `vendor/tvix-eval/src/compiler/mod.rs`.
    Both patches are called out at the top of `tier-a/Cargo.toml`.
  - `examples/parity.rs`, `examples/pkgs_check.rs` — native (non-wasm) dev
    tools for fast iteration; `cargo run --example parity` runs the same
    corpus as the vitest suite, natively, in ~1s instead of a full wasm
    rebuild. (`35-import` fails there only — see "Known gaps" below.)
  - `test/parity.test.ts` — the vitest parity suite; see "Testing".
  - `pkg-node/` — a committed `wasm-bindgen --target nodejs` build, used
    only by the parity test suite (Node/vitest can't load the `--target
    web` build the browser uses).
- `nixpkgs-lib/` — a vendored, pinned copy of nixpkgs' `lib/` (commit
  `ac62194c3917d5f474c1a844b6fd6da2db95077d`, `nixos-25.05`; see its own
  `README.md` for the exact pin and re-pin instructions), reachable in the
  VFS as `import <nixpkgs/lib>` and `(import <nixpkgs> {}).lib`.
- `pkgs/default.nix` — a small **teaching** `pkgs`, reachable as `import
  <nixpkgs> {}`: `stdenv.mkDerivation`, `mkShell`, `writeShellApplication`,
  `writeText`, `runCommand`, `callPackage`, `fetchurl`/`fetchFromGitHub`,
  and real (`derivation`-backed) packages for git, jq, ripgrep, hello,
  cowsay, curl, openssl, zlib, python3, nodejs, gcc, gnumake, coreutils,
  bash. Not real nixpkgs — a stub with plausible `pname`/`version`/`meta`,
  clearly marked as such in its own header comment.
- `scripts/build-vfs.mjs` — bundles `nixpkgs-lib/lib` + `pkgs/default.nix`
  into `public/nixbox/vfs.json.gz` (gzip, one file the worker fetches once
  and decompresses client-side with `DecompressionStream`).
- `build.sh` — rebuilds every committed artifact from source (see below).

The Web Worker + host-side runtime that load and drive this crate live at
`lib/runtime/tier-a/` (worker.ts, runtime.ts) — see that directory's own
doc comments. `lib/runtime/select.ts` returns tier A for any lesson with
`runtime: "eval"`, lazily (nothing loads until the first `eval()` call).

## Building

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.129 --locked   # must match
                                                              # tier-a/Cargo.lock's
                                                              # resolved wasm-bindgen
                                                              # version exactly
./nixbox/build.sh
```

This needs no network access beyond crates.io for `nixbox-tier-a`'s own
direct dependencies (the vendored crates need no `git.snix.dev` or GitHub
access). It regenerates, all under version control:

- `public/nixbox/nixbox_tier_a.js` + `nixbox_tier_a_bg.wasm` — the
  `--target web` build the browser loads (glue JS: 8.8 KB / gzip 2.6 KB;
  wasm: raw 2.0 MB / gzip 627 KB).
- `public/nixbox/vfs.json.gz` — the lib+pkgs VFS bundle (234 files, raw JSON
  961 KiB, gzip 221 KiB).
- `nixbox/tier-a/pkg-node/` — the `--target nodejs` build, for the parity
  test suite only (not served to the browser).

Cold-load total for the browser: wasm + glue + VFS ≈ 2.24 MB raw / ≈ 850 KB
gzip. It's fetched lazily (only when a lesson needs `runtime: "eval"`),
under a dedicated module Worker, with `lib/runtime/tier-a`'s
`onLoadProgress` available for a progress bar.

## Testing

```sh
npm test    # runs everything under vitest.config.ts's `include`, which
            # now covers nixbox/**/*.test.ts — no Rust toolchain needed,
            # it runs against the committed nixbox/tier-a/pkg-node/ build.
```

`nixbox/tier-a/test/parity.test.ts` runs every snippet in
`spikes/parity/snippets/` through the evaluator and checks it against its
`.expected` file: value-exact (the `printed` field) for a value snippet,
primary-message-exact (the last `error: ...` line) for an error snippet.
35/35 required snippets pass. Two bonus snippets beyond the required
corpus (`36-select-on-function`, `37-elemat-oob`) are marked `it.fails` —
known, documented, accepted gaps (see the comment above `KNOWN_GAPS` in
that file); the suite goes red the day either one starts passing, as a
nudge to promote it out of that list.

## Known gaps vs. real Nix

- **Function values in *error-context* text**: CppNix prints
  `«lambda @ file:line:col»`; tvix-eval prints `<LAMBDA>` in that one
  context. (Note: `<LAMBDA>` *is* the correct rendering for ordinary value
  **printing** — `printed`/`json` — which is what the brief actually
  requires; this gap is specifically about a function value appearing
  inside an error message.)
- **`elemAt` out-of-bounds wording**: CppNix names the builtin and the
  list's size (`'builtins.elemAt' called with index 5 on a list of size
  2`); tvix's `IndexOutOfBounds` `ErrorKind` carries neither, so the
  message is generic (`list index '5' is out of bounds`).
- **Derivation-attrset printing under `--strict`**: a full
  `stdenv.mkDerivation`/`mkShell` result prints as `«derivation
  "path.drv"»` here vs. real Nix's full attrset with `«repeated»` cycle
  markers. This is a **deliberate upstream tvix-eval design choice**
  (`vendor/tvix-eval/src/value/attrs.rs`, checking `s == "derivation"`) to
  avoid infinite recursion on the self-referential derivation attrset,
  using a more conservative cycle-avoidance strategy than real Nix's
  per-node tracking. Not patched further here: removing the special case
  without implementing real cycle-tracking risks a stack overflow.
- **`input_derivations`** is never populated in `derivationStrict` — an
  accepted simplification, since the teaching `pkgs` stub's packages don't
  chain derivations as build inputs (documented in `pkgs_builtins.rs`).
- **Generic arithmetic type errors** (e.g. `1 + "a"`) use a generic
  `TypeError`-style message rather than CppNix's distinct "cannot add a
  string to an integer" wording — not in the required 35-snippet corpus,
  not pursued.

Everything else in the required 35-snippet parity corpus — including
`derivation`'s `drvPath`/`outPath` (byte-for-byte matching real
`nix-instantiate`), `<nixpkgs>`/`<nixpkgs/lib>` resolution, `assert`
operand values, syntax-error line/col, and `builtins.trace` — matches real
Nix 2.28.3 exactly.
