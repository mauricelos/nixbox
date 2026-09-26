# NixQuest teaching `pkgs` stub — NOT Nixpkgs.
#
# Reachable in a lesson as `import <nixpkgs> {}` (this file, called with an
# attrset) and `(import <nixpkgs> {}).lib` (the real, vendored Nixpkgs
# `lib/`, ../nixpkgs-lib in this repo, bundled alongside this file at
# "nixpkgs/lib" — see nixbox/build.sh and nixbox/README.md).
#
# Every package below is a genuine `derivation` value — real `drvPath` and
# `outPath` store paths, computed by `derivationStrict` (nixbox/tier-a's
# native builtin, on `nix-compat`'s store-path hashing — see
# nixbox/tier-a/src/pkgs_builtins.rs) exactly as real Nix would for the
# same inputs. What's NOT real: there is no store and no builder — Tier A
# only evaluates, it never runs `builder`, so `nix-build`-ing any of these
# outside NixQuest would fail (no real source archives are fetched, no
# real build happens). Every `meta.description` below is written by hand
# for teaching purposes, not copied from real Nixpkgs, and versions are
# pinned to whatever was current when this stub was written — don't expect
# them to track upstream releases.
{ system ? "x86_64-linux" }:

let
  lib = import ./lib;

  # ---- stdenv / builder primitives ------------------------------------

  # A minimal `stdenv.mkDerivation`: merges its argument into a
  # `derivation {}` call with the handful of attributes real builds always
  # need (system, a /bin/sh builder, a `-c` args convention) filled in.
  # Real stdenv adds dozens of environment variables and a multi-phase
  # build script (see lesson 5.4); this stub only needs to produce correct
  # store paths, not build anything.
  mkDerivation = attrs:
    let
      name = if attrs ? pname && attrs ? version
        then "${attrs.pname}-${attrs.version}"
        else attrs.name;
      drvAttrs = (builtins.removeAttrs attrs [ "meta" "pname" "version" ]) // {
        inherit name system;
        builder = attrs.builder or "/bin/sh";
        args = attrs.args or [ "-c" "echo 'NixQuest teaching stub: not a real build' >&2; exit 1" ];
      };
    in
    derivation drvAttrs // {
      pname = attrs.pname or null;
      version = attrs.version or null;
      meta = attrs.meta or { };
    };

  # `fetchurl`/`fetchFromGitHub`: fixed-output derivations (their output
  # path is derived from the content hash, not the build inputs — the same
  # as real Nix), so they need a real sha256. NixQuest's own snippets that
  # use these always pass one; a lesson step that asks the learner to
  # supply a hash is exactly `lib.fakeHash` territory (module 5.6) and
  # produces a real, matchable "hash mismatch" *evaluation-time* error only
  # once Tier B can actually fetch — Tier A can't detect a wrong hash
  # without downloading, so it trusts whatever hash is given.
  fetchurl = { url, sha256, name ? baseNameOf url, ... }:
    derivation {
      inherit name system;
      builder = "builtin:fetchurl";
      args = [ ];
      urls = [ url ];
      outputHashMode = "flat";
      outputHashAlgo = "sha256";
      outputHash = sha256;
    };

  fetchFromGitHub = { owner, repo, rev, sha256, name ? "${repo}-${rev}-source", ... }:
    derivation {
      inherit name system;
      builder = "builtin:fetchFromGitHub";
      args = [ ];
      url = "https://github.com/${owner}/${repo}/archive/${rev}.tar.gz";
      outputHashMode = "recursive";
      outputHashAlgo = "sha256";
      outputHash = sha256;
    };

  writeText = name: text:
    derivation {
      inherit name system;
      builder = "/bin/sh";
      args = [ "-c" "echo -n \"$textPath\" > $out" ];
      textPath = text;
      outputHashMode = "flat";
    } // { inherit text; };

  writeShellApplication = { name, text, runtimeInputs ? [ ] }:
    mkDerivation {
      pname = name;
      version = "0";
      inherit name;
      meta.description = "A generated shell script named '${name}'";
      builder = "/bin/sh";
      args = [ "-c" "mkdir -p $out/bin; echo \"$scriptText\" > $out/bin/${name}; chmod +x $out/bin/${name}" ];
      scriptText = "#!/usr/bin/env bash\nset -euo pipefail\n" + text;
    };

  runCommand = name: env: buildCommand:
    derivation (env // {
      inherit name system;
      builder = "/bin/sh";
      args = [ "-c" buildCommand ];
    });

  mkShell = { packages ? [ ], shellHook ? "", ... }@args:
    mkDerivation ((builtins.removeAttrs args [ "packages" "shellHook" ]) // {
      name = args.name or "nix-shell";
      pname = "nix-shell";
      version = "0";
      meta.description = "A development shell (mkShell)";
      inherit packages shellHook;
      builder = "builtin:mkShell";
      args = [ ];
    });

  callPackage = fn: extraArgs:
    let
      f = if builtins.isFunction fn then fn else import fn;
      # Only pass the arguments `f` actually declares (real `callPackage`'s
      # auto-argument behaviour), falling back to `pkgsSelf` for anything
      # matching a name in this package set.
      wanted = builtins.functionArgs f;
      auto = builtins.intersectAttrs wanted pkgsSelf;
    in
    f (auto // extraArgs);

  # ---- a small, real-looking package set -------------------------------

  mkStub = { pname, version, description, sha256, homepage ? "", license ? lib.licenses.free or "free" }:
    mkDerivation {
      inherit pname version;
      src = fetchurl {
        url = "mirror://nixquest-teaching-stub/${pname}-${version}.tar.gz";
        inherit sha256;
      };
      meta = { inherit description homepage license; };
      builder = "/bin/sh";
      args = [ "-c" "mkdir -p $out/bin; cp -r $src/* $out/ 2>/dev/null || true" ];
    };

  pkgsSelf = {
    inherit lib fetchurl fetchFromGitHub writeText writeShellApplication
      runCommand mkShell callPackage system;

    stdenv = {
      inherit mkDerivation system;
      shell = "/bin/bash";
    };

    coreutils = mkStub {
      pname = "coreutils"; version = "9.5";
      description = "The basic file, shell and text manipulation utilities of the GNU operating system";
      sha256 = "1cwrf1rrfwl0rxryawy0rdfrywnl7pyxg5rgn9lvj80jw1jnh9k9";
    };
    bash = mkStub {
      pname = "bash"; version = "5.2";
      description = "The GNU Bourne-Again shell";
      sha256 = "1a5w26nhh05mvvhk80s7l3gsc47qjrb87qjvjqrmzxmkacvz32sm";
    };
    git = mkStub {
      pname = "git"; version = "2.47.1";
      description = "Distributed version control system";
      homepage = "https://git-scm.com/";
      sha256 = "0y3vhzn1kjfhwj7bkcx5b7yaqhqyc0n2b3z0m3wl81rqwj9wrgjk";
    };
    jq = mkStub {
      pname = "jq"; version = "1.7.1";
      description = "A lightweight and flexible command-line JSON processor";
      homepage = "https://jqlang.github.io/jq/";
      sha256 = "0d3nfa9rgbll1p85y6df4dj1qmjb1w9y8h9f2b1mvyz2syjq14x8";
    };
    ripgrep = mkStub {
      pname = "ripgrep"; version = "14.1.1";
      description = "A utility that combines the usability of ag with the raw speed of grep";
      homepage = "https://github.com/BurntSushi/ripgrep";
      sha256 = "1x1c47qslnw9d0y2n56sz44bnnv2yg9nhkjjm3j40j674ycn8b7a";
    };
    hello = mkStub {
      pname = "hello"; version = "2.12.1";
      description = "A program that produces a familiar, friendly greeting";
      homepage = "https://www.gnu.org/software/hello/";
      sha256 = "086vqwk2wl8zfs47sq2xpjc9k066ilmb8z6dn0q6ymwjzlm196cd";
    };
    cowsay = mkStub {
      pname = "cowsay"; version = "3.8.4";
      description = "A program which generates ASCII pictures of a cow with a message";
      sha256 = "0y1yhcvvv2zjjb47l6yf6k1s7v7nywmz78z2yq5v8v94j06yh6dd";
    };
    curl = mkStub {
      pname = "curl"; version = "8.11.1";
      description = "A command line tool for transferring files with URL syntax";
      homepage = "https://curl.se/";
      sha256 = "0lgy2vzwll9hy2cjdscn9y5v6ryw5q9r1j62l1kbf82yfmzcw3xd";
    };
    openssl = mkStub {
      pname = "openssl"; version = "3.4.0";
      description = "A cryptographic library that implements the SSL and TLS protocols";
      sha256 = "1n47iq7x40y6nsyz6r05gr2gk1jhaydxjm6b4nzha0k9nvxb7ai8";
    };
    zlib = mkStub {
      pname = "zlib"; version = "1.3.1";
      description = "Lossless data-compression library";
      sha256 = "0h9amjsq8jj6masmg4nnb2kjq9hivp1n8l6gd7q0k26v14aq2v9y";
    };
    python3 = mkStub {
      pname = "python3"; version = "3.12.7";
      description = "A high-level dynamically-typed programming language";
      homepage = "https://www.python.org/";
      sha256 = "1jm2vlvyibr2v3wch2lxvxvxqzn3yqhm6khsn4khzhbf0d2rccp7";
    };
    nodejs = mkStub {
      pname = "nodejs"; version = "22.11.0";
      description = "Event-driven I/O framework for the V8 JavaScript engine";
      homepage = "https://nodejs.org/";
      sha256 = "0jqbrjrqr2iizrp1fdqdvxk15wj0jw6dqwphbam9a8f0j5hxvfnv";
    };
    gcc = mkStub {
      pname = "gcc"; version = "14.2.0";
      description = "The GNU Compiler Collection, including C and C++ compilers";
      sha256 = "0kj7gdqgcxk2r8bqhg8r80z8flr9jm6mkbf8y4jbgcbxdz1nqfhq";
    };
    gnumake = mkStub {
      pname = "gnumake"; version = "4.4.1";
      description = "A tool to control the generation of non-source files from sources";
      sha256 = "1a4npj3yq6ivn1swzcxvw6f9ycm6ddjby0djh4hwn5nkbf5wk9hd";
    };
  };
in
pkgsSelf
