# Vendored Nixpkgs `lib`

`lib/` here is `NixOS/nixpkgs.git`'s `lib/` directory, branch `nixos-25.05`,
commit `ac62194c3917d5f474c1a844b6fd6da2db95077d` (2026-01-02) — the same
commit WP7's spike pinned (`docs/spikes/A2-snix.md`, "Pinned nixpkgs
`lib`"). 233 `.nix` files, ~2.0 MiB source. License: MIT (`COPYING`).

`nixbox/build.sh` bundles it (compressed, alongside `nixbox/pkgs/`) into
`public/nixbox/vfs.json.gz`, reachable at runtime as `import <nixpkgs/lib>`
and `(import <nixpkgs> {}).lib` (see `nixbox/pkgs/default.nix`).

To re-pin: `git clone --branch nixos-25.05 --depth 1
https://github.com/NixOS/nixpkgs.git`, copy `lib/` and `COPYING` here,
update this file's commit/date, and re-run `nixbox/build.sh`.
