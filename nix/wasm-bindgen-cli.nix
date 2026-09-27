# wasm-bindgen-cli, pinned to exactly the version resolved in
# ../tier-a/Cargo.lock (currently 0.2.129). wasm-bindgen refuses to process
# a .wasm built against a different version of its own runtime-support
# crate, so this must always track that lockfile's `wasm-bindgen` entry.
#
# nixpkgs (as of the pin in flake.nix) doesn't package this exact version
# (it has 0.2.108 in the 25.11 channel and a handful of newer
# `wasm-bindgen-cli_0_2_*` aliases in unstable, but not 0.2.129), so it's
# built here directly from crates.io with buildRustPackage. The two hashes
# below are content hashes (source tarball, then vendored Cargo
# dependencies) — bump both together when bumping the version:
#   1. set `hash`/`cargoHash` to `lib.fakeHash`;
#   2. `nix build .#wasm-bindgen-cli` (or build this file directly);
#   3. copy the two "got:" hashes nix prints back in, in order (source
#      hash first, then vendor hash).
{
  lib,
  rustPlatform,
  fetchCrate,
}:

rustPlatform.buildRustPackage rec {
  pname = "wasm-bindgen-cli";
  version = "0.2.129";

  src = fetchCrate {
    inherit pname version;
    hash = "sha256-pcecKQd7E8Opw6bkFoE569epUi7gh5qpQF1e5PJY6V8=";
  };

  cargoHash = "sha256-vmUrWVU7kPJJxO5qIVeAkwQyWDELO1Z4Z5gitz2kco8=";

  # wasm-bindgen-cli's own test suite spins up a browser/wasmtime and needs
  # network access for some fixtures — irrelevant here, we only need the
  # `wasm-bindgen` binary.
  doCheck = false;

  meta = {
    description = "Pinned wasm-bindgen CLI (matches tier-a/Cargo.lock)";
    license = with lib.licenses; [ asl20 mit ];
    mainProgram = "wasm-bindgen";
  };
}
