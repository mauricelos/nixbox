{
  description = "NixBox — a real Nix language evaluator compiled to wasm32, for the browser";

  # A channel-tarball URL instead of `github:NixOS/nixpkgs/...`: it resolves
  # on any machine (including ones where github.com is blocked but
  # channels.nixos.org / releases.nixos.org are reachable) and is exactly
  # the same content the corresponding GitHub ref would give.
  inputs.nixpkgs.url = "https://channels.nixos.org/nixos-25.11/nixexprs.tar.xz";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
      pkgsFor = system: import nixpkgs { inherit system; };
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
          wasm-bindgen-cli-0_2_129 = pkgs.callPackage ./nix/wasm-bindgen-cli.nix { };
          nixbox = pkgs.callPackage ./nix/package.nix { inherit wasm-bindgen-cli-0_2_129; };
        in
        {
          inherit nixbox wasm-bindgen-cli-0_2_129;
          default = nixbox;
        }
      );

      devShells = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
        in
        {
          default = pkgs.mkShell {
            packages = [
              pkgs.rustc
              pkgs.cargo
              pkgs.nodejs_22
              self.packages.${system}.wasm-bindgen-cli-0_2_129
            ];
            shellHook = ''
              echo "NixBox development shell: $(rustc --version), $(node --version), $(wasm-bindgen --version)"
              echo "Add the wasm32 target once per machine: rustup target add wasm32-unknown-unknown"
              echo "(not needed under this shell's plain nixpkgs rustc, which already ships it)"
            '';
          };
        }
      );

      checks = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
          nixbox = self.packages.${system}.nixbox;
        in
        {
          inherit nixbox;
          parity = pkgs.runCommand "nixbox-parity" { nativeBuildInputs = [ pkgs.nodejs_22 ]; } ''
            cp -r ${./tier-a/test} ./test
            cp ${./tier-a/run-parity-tests.mjs} ./run-parity-tests.mjs
            node ./run-parity-tests.mjs ${nixbox.passthru.pkgNode}/nixbox_tier_a.cjs | tee $out
          '';
        }
      );
    };
}
