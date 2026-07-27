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
  # Runs one upstream suite through our `opt` and holds the agreement count
  # at or above a recorded number. The suite comes from `pkgs.llvm.src`, so
  # nothing upstream is vendored into the tree and the version is pinned by
  # the flake lock rather than by a copy that silently ages.
  #
  # The ratchet only moves up. A change that lowers it fails; a change that
  # raises it prints the new number to record here.
  # Compares what we print against what upstream prints, for every file in a
  # suite that both accept. Needs the real tools, not only the sources, and
  # they stay confined to this derivation.
  differentialCheck = pkgs: suite: ratchet:
    (cargoCheck pkgs "differential-${lib.toLower suite}" ''
      cargo build -p llvm-tools --offline --locked
      nu corpus/check-differential.nu "$CARGO_TARGET_DIR/debug/opt" \
        "${pkgs.llvm.src}/llvm/test/${suite}" ${toString ratchet}
    '')
    .overrideAttrs (previous: {
      nativeBuildInputs = previous.nativeBuildInputs ++ [pkgs.llvm];
    });

  upstreamCheck = pkgs: suite: ratchet: refusals:
    (cargoCheck pkgs "upstream-${lib.toLower suite}" ''
      cargo build -p llvm-tools --offline --locked
      nu corpus/check-upstream.nu "$CARGO_TARGET_DIR/debug/opt" \
        "${lib.getExe' pkgs.llvm "llvm-as"}" \
        "${pkgs.llvm.src}/llvm/test/${suite}" ${toString ratchet} ${toString refusals}
    '')
    .overrideAttrs (previous: {
      nativeBuildInputs = previous.nativeBuildInputs ++ [pkgs.llvm];
    });
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
        cargo build -p llvm-tools --offline --locked
        nu corpus/check-opt.nu "$CARGO_TARGET_DIR/debug/opt" corpus
      '';

    # IR built through the builder API rather than parsed: it has to verify,
    # and it has to print the text upstream prints for the same module.
    llvm-builder-smoke = pkgs:
      cargoCheck pkgs "builder-smoke" ''
        cargo test -p llvm-ir-parse --test builder --offline --locked -- --nocapture
      '';

    # The verifier accepts everything real llvm-as accepted, and rejects a
    # table of deliberately broken modules with the message each rule owns.
    llvm-verify-corpus = pkgs:
      cargoCheck pkgs "verify-corpus" ''
        cargo test -p llvm-ir-parse --test verify --offline --locked -- --nocapture
      '';

    # Conformance against upstream's own suites, measured rather than claimed:
    # every file in the suite, scored against what real llvm-as does with it.
    # See STATUS.md for what the remaining disagreements are.
    llvm-upstream-assembler = pkgs: upstreamCheck pkgs "Assembler" 407 14;
    llvm-upstream-verifier = pkgs: upstreamCheck pkgs "Verifier" 226 4;

    # Not whether we accept the same files, but whether we print the same
    # text. The corpus pins the printer against upstream's own output; this
    # pins it against inputs nobody wrote for us.
    llvm-opt-differential = pkgs: differentialCheck pkgs "Assembler" 120;
  };

  packages.rust-llvm = {lib, ...}:
    lib.buildCargoProject {
      pname = "rust-llvm";
      src = llvmSrc;
      index = ../../nix/lib/cargo/index;
      roots = ["llvm-tools"];

      rootAttrs.postInstall = ''
        # The upstream-compatible name is the point; the suffixed alias is
        # for a PATH that already has real LLVM on it, the way rust/gcc does
        # it for gcc and cc.
        ln -s $out/bin/opt $out/bin/opt-rs
      '';

      meta = {
        description = "LLVM-compatible compiler infrastructure written in Rust";
        homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/llvm";
        # Apache-2.0 WITH LLVM-exception, which nixpkgs spells as the pair.
        license = [lib.licenses.asl20 lib.licenses.llvm-exception];
        mainProgram = "opt";
        platforms = lib.platforms.linux ++ lib.platforms.darwin;
      };
    };
}
