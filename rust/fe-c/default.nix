{lib, ...}: let
  # rust-toolchain.toml is the single source of truth for the pin (PLAN.md
  # §8); rust-overlay reads channel + components + profile from it.
  fecToolchain = pkgs: pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;

  fecSrc = lib.fileset.toSource {
    root = ./.;
    fileset = lib.fileset.unions [
      ./Cargo.toml
      ./Cargo.lock
      ./crates
    ];
  };

  homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/fe-c";

  # Offline vendor directory for cargo-driven checks (clippy). Sources and
  # checksums come straight from Cargo.lock, mirroring nix/lib/cargo's
  # fetch scheme, so the checks stay pure once third-party deps appear.
  vendorFor = pkgs: let
    lock = builtins.fromTOML (builtins.readFile ./Cargo.lock);
    thirdParty =
      builtins.filter (p: p ? checksum) (lock.package or []);
    crateTar = p:
      pkgs.fetchurl {
        name = "${p.name}-${p.version}.crate";
        url = "https://static.crates.io/crates/${p.name}/${p.name}-${p.version}.crate";
        sha256 = p.checksum;
      };
  in
    pkgs.runCommand "fe-c-vendor" {} ''
      mkdir -p $out
      ${lib.concatMapStrings (p: ''
          tar -xzf ${crateTar p} -C $out
          printf '{"files":{},"package":"%s"}' "${p.checksum}" \
            > $out/${p.name}-${p.version}/.cargo-checksum.json
        '')
        thirdParty}
    '';

  # Shared harness for checks that drive real cargo against the pinned
  # nightly with vendored sources.
  cargoCheck = pkgs: name: script:
    pkgs.runCommand "fe-c-${name}" {
      nativeBuildInputs = [(fecToolchain pkgs)];
    } ''
      cp -r ${fecSrc}/. build
      chmod -R u+w build
      cd build
      export CARGO_HOME=$TMPDIR/cargo-home
      export CARGO_TARGET_DIR=$TMPDIR/target
      mkdir -p $CARGO_HOME
      cat > $CARGO_HOME/config.toml <<EOF
      [source.crates-io]
      replace-with = "vendored-sources"
      [source.vendored-sources]
      directory = "${vendorFor pkgs}"
      EOF
      ${script}
      touch $out
    '';

  mkFecPackage = {
    pname,
    root,
    mainProgram ? null,
    description,
  }: {
    lib,
    rust-bin,
    ...
  }:
    lib.buildCargoProject {
      inherit pname;

      src = fecSrc;
      index = ../../nix/lib/cargo/index;
      roots = [root];

      toolchain = rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;

      meta =
        {
          inherit description homepage;
          license = lib.licenses.mit;
          platforms = lib.platforms.linux;
        }
        // lib.optionalAttrs (mainProgram != null) {inherit mainProgram;};
    };
in {
  devShells.fe-c = pkgs: {
    packages = [
      (fecToolchain pkgs)
      pkgs.just
    ];
  };

  packages = {
    cementite = mkFecPackage {
      pname = "cementite";
      root = "cementite";
      description = "Fe-C runtime: allocation table, capability checks, quarantining allocator";
    };

    fe-c-driver = mkFecPackage {
      pname = "fe-c-driver";
      root = "fe-c-driver";
      mainProgram = "fe-c-driver";
      description = "Fe-C rustc-as-a-library driver: MIR analysis and rewriting";
    };

    cargo-fe-c = mkFecPackage {
      pname = "cargo-fe-c";
      root = "cargo-fe-c";
      mainProgram = "cargo-fe-c";
      description = "Fe-C cargo subcommand: drives instrumented builds";
    };
  };

  checks = {
    fe-c-fmt = pkgs:
      cargoCheck pkgs "fmt" ''
        cargo fmt --check
      '';

    fe-c-clippy = pkgs:
      cargoCheck pkgs "clippy" ''
        cargo clippy --workspace --all-targets --offline --locked -- -D warnings
      '';
  };
}
