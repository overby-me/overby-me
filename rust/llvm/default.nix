# LLVM-rs: flakelight module.
#
# One derivation per check, built individually
# (`nix build .#checks.x86_64-linux.llvm-fmt`). `nix flake check` stays
# forbidden repo-wide, so nothing here assumes it.
{lib, ...}: let
  llvmSrc = lib.fileset.toSource {
    root = ./.;
    fileset = lib.fileset.unions [
      ./Cargo.toml
      ./Cargo.lock
      ./crates
      # The round-trip check reads the corpus from the source tree, so it has
      # to be part of the source the checks build from.
      ./corpus
    ];
  };

  # Shared harness for the cargo-driven checks. Unlike rust/fe-c's, this one
  # needs no vendor directory: the workspace has no third-party dependencies,
  # so `--offline` costs nothing and `--locked` has nothing to resolve.
  cargoCheck = pkgs: name: script:
    pkgs.runCommand "llvm-${name}" {
      nativeBuildInputs = [
        pkgs.cargo
        pkgs.rustc
        pkgs.clippy
        pkgs.rustfmt
        pkgs.nushell
        # runCommand builds on stdenvNoCC, and rustc still shells out to cc
        # to link a test binary.
        pkgs.stdenv.cc
      ];
    } ''
      cp -r ${llvmSrc}/. build
      chmod -R u+w build
      cd build
      export HOME=$TMPDIR
      export CARGO_HOME=$TMPDIR/cargo-home
      export CARGO_TARGET_DIR=$TMPDIR/target
      mkdir -p $CARGO_HOME
      ${script}
      touch $out
    '';
in {
  checks = {
    llvm-fmt = pkgs:
      cargoCheck pkgs "fmt" ''
        cargo fmt --check
      '';

    llvm-clippy = pkgs:
      cargoCheck pkgs "clippy" ''
        cargo clippy --workspace --all-targets --offline --locked -- -D warnings
      '';

    llvm-unit = pkgs:
      cargoCheck pkgs "unit" ''
        cargo test --workspace --offline --locked
      '';

    # The T0 headline: every corpus file is canonical llvm-dis output, and
    # parsing one and printing it back reproduces it byte for byte.
    llvm-roundtrip = pkgs:
      cargoCheck pkgs "roundtrip" ''
        cargo test -p llvm-ir-parse --test roundtrip --offline --locked -- --nocapture
      '';
  };

  # No `packages.rust-llvm` yet: the workspace is libraries only, so there is
  # nothing to install. It arrives with the llvm-tools crate, along with the
  # upstream-compatible `opt` name and its `opt-rs` alias (PLAN.md section 9.1).
}
