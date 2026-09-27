# Builds the NixBox artifact: a directory with exactly the three files a
# host application needs to serve:
#   nixbox_tier_a.js       — wasm-bindgen `--target web` glue (ESM)
#   nixbox_tier_a_bg.wasm  — the compiled evaluator
#   vfs.json.gz            — the nixpkgs-lib + pkgs VFS bundle
#
# `tier-a/pkg-node/` (the nodejs-target build the parity suite runs
# against) is a separate output, `passthru.pkg-node`, since a host serving
# this to a browser has no use for it.
{
  lib,
  stdenv,
  rustPlatform,
  wasm-bindgen-cli-0_2_129,
  nodejs,
  lld,
}:

let
  # The whole repo: tier-a's Cargo.toml pulls in vendor/tvix-eval and
  # vendor/nix-compat as path dependencies, and build-vfs.mjs reads
  # ../nixpkgs-lib and ../pkgs relative to itself, so all of it needs to be
  # in the sandbox, not just tier-a/.
  src = lib.cleanSourceWith {
    src = ../.;
    filter =
      name: type:
      let
        base = baseNameOf name;
      in
      !(base == ".git" || base == "target" || base == "pkg-node" || base == "dist");
  };

  tierA = rustPlatform.buildRustPackage {
    pname = "nixbox-tier-a";
    version = "0.1.0";

    inherit src;
    cargoRoot = "tier-a";

    cargoLock.lockFile = ../tier-a/Cargo.lock;

    # rustc's wasm32-unknown-unknown target links with lld (as `rust-lld`
    # normally, bundled with rustc itself — but the plain nixpkgs `rustc`
    # derivation doesn't put it on PATH under the bare name `lld` its
    # driver invokes), so it needs to be available explicitly.
    nativeBuildInputs = [ lld ];

    # Nothing here can meaningfully run `cargo test` for a wasm32 cdylib on
    # the build host; the parity suite (which actually exercises the
    # compiled wasm, via wasm-bindgen's nodejs target) is the `parity`
    # flake check instead.
    doCheck = false;

    # This crate is a cdylib (no `[[bin]]`) built for a foreign target
    # (wasm32-unknown-unknown), so neither buildRustPackage's default
    # `cargo build`-then-`cargo install` phases apply directly — drive
    # cargo ourselves. Cross-compiling to wasm32-unknown-unknown from any
    # host stdenv nixpkgs ships works out of the box: its std doesn't need
    # a C toolchain or a nixpkgs cross stdenv, only the target's std
    # library, which nixpkgs' plain `rustc` already includes (verified:
    # `cargo build --target wasm32-unknown-unknown` succeeds against
    # plain nixpkgs rustc/cargo with no extra setup).
    buildPhase = ''
      runHook preBuild
      (cd tier-a && cargo build -j "$NIX_BUILD_CORES" --target wasm32-unknown-unknown --release --offline)
      runHook postBuild
    '';

    installPhase = ''
      runHook preInstall
      mkdir -p "$out"
      cp tier-a/target/wasm32-unknown-unknown/release/nixbox_tier_a.wasm "$out/"
      runHook postInstall
    '';

    meta.license = lib.licenses.gpl3Only;
  };

  # wasm-bindgen's two targets (web / nodejs) from the one compiled wasm.
  mkBindgenOut =
    target:
    stdenv.mkDerivation {
      pname = "nixbox-tier-a-bindgen-${target}";
      version = "0.1.0";
      dontUnpack = true;
      nativeBuildInputs = [ wasm-bindgen-cli-0_2_129 ];
      buildPhase = ''
        runHook preBuild
        wasm-bindgen "${tierA}/nixbox_tier_a.wasm" \
          --target ${target} --out-dir "$out" --out-name nixbox_tier_a --no-typescript
        runHook postBuild
      '';
      installPhase = "true";
    };

  web = mkBindgenOut "web";
  pkgNode = stdenv.mkDerivation {
    pname = "nixbox-tier-a-pkg-node";
    version = "0.1.0";
    dontUnpack = true;
    src = mkBindgenOut "nodejs";
    installPhase = ''
      runHook preInstall
      mkdir -p "$out"
      cp "$src"/* "$out/"
      # package.json ("type": "module") needs the CommonJS glue under .cjs.
      mv "$out/nixbox_tier_a.js" "$out/nixbox_tier_a.cjs"
      runHook postInstall
    '';
  };

  vfs = stdenv.mkDerivation {
    pname = "nixbox-vfs";
    version = "0.1.0";
    inherit src;
    nativeBuildInputs = [ nodejs ];
    buildPhase = ''
      runHook preBuild
      NIXBOX_OUT_DIR="$out" node scripts/build-vfs.mjs
      runHook postBuild
    '';
    installPhase = "true";
  };
in
stdenv.mkDerivation {
  pname = "nixbox";
  version = "0.1.0";
  dontUnpack = true;

  installPhase = ''
    runHook preInstall
    mkdir -p "$out"
    cp "${web}/nixbox_tier_a.js" "$out/"
    cp "${web}/nixbox_tier_a_bg.wasm" "$out/"
    cp "${vfs}/vfs.json.gz" "$out/"
    runHook postInstall
  '';

  passthru = {
    inherit tierA pkgNode;
  };

  meta = {
    description = "NixBox — a real Nix language evaluator compiled to wasm32, for the browser";
    homepage = "https://github.com/mauricelos/nixbox";
    license = lib.licenses.gpl3Only;
    platforms = lib.platforms.all;
  };
}
