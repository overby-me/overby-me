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

  # Offline vendor directory for cargo-driven checks (clippy, unit, miri).
  # Sources and checksums come straight from the lockfiles, mirroring
  # nix/lib/cargo's fetch scheme, so the checks stay pure. Two locks feed
  # it: the workspace's own, and a committed copy of the pinned
  # toolchain's library/Cargo.lock so `cargo miri setup` can build its
  # sysroot offline (refresh nix/miri-std.Cargo.lock on toolchain bumps).
  vendorFor = pkgs: let
    locks = [./Cargo.lock ./nix/miri-std.Cargo.lock];
    thirdParty = lib.unique (lib.concatMap (
        lockFile: let
          lock = builtins.fromTOML (builtins.readFile lockFile);
        in
          builtins.filter (p: p ? checksum) (lock.package or [])
      )
      locks);
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
  # nightly with vendored sources. nushell is available for assertion
  # scripts (repo convention).
  cargoCheck = pkgs: name: script:
    pkgs.runCommand "fe-c-${name}" {
      nativeBuildInputs = [(fecToolchain pkgs) pkgs.nushell];
    } ''
      cp -r ${fecSrc}/. build
      chmod -R u+w build
      cd build
      export HOME=$TMPDIR
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

    # --all-features so the interpose module and its C harness are linted
    # too (the C harness is compiled by build.rs via the sandbox cc).
    fe-c-clippy = pkgs:
      cargoCheck pkgs "clippy" ''
        cargo clippy --workspace --all-targets --all-features --offline --locked -- -D warnings
      '';

    fe-c-unit = pkgs:
      cargoCheck pkgs "unit" ''
        cargo test --workspace --offline --locked
      '';

    # libc interposition (A4): builds the cementite test binary with the
    # #[no_mangle] malloc overrides and a cc-compiled C harness, asserting
    # foreign/libc-internal allocations register with correct bounds.
    fe-c-interpose = pkgs:
      cargoCheck pkgs "interpose" ''
        cargo test -p cementite --features interpose --offline --locked
      '';

    # Driver visitation census (A5): build the rustc_public driver, run it
    # on the hand-audited fixture, assert it drives a real compilation and
    # the census meets the minimums with no skipped bodies (I1).
    fe-c-census = pkgs:
      cargoCheck pkgs "census" ''
        cargo build -p fe-c-driver --offline --locked
        export LD_LIBRARY_PATH="$(rustc --print sysroot)/lib''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
        drv="$CARGO_TARGET_DIR/debug/fe-c-driver"
        fixture=crates/fe-c-driver/tests/fixtures/census_fixture.rs
        FEC_CENSUS_OUT="$TMPDIR/census.json" "$drv" "$fixture" \
          -o "$TMPDIR/fixbin" --edition 2021
        # The driver drove a real compilation, not just analysis.
        test -x "$TMPDIR/fixbin"
        nu crates/fe-c-driver/tests/assert_census.nu "$TMPDIR/census.json"
      '';

    # Capability propagation dataflow (B1, I10): run the driver over the
    # insert_many-shaped fixture and assert each write is traced to its
    # as_mut_ptr derivation root (both the direct-deref and ptr::write
    # forms real unsafe code uses).
    fe-c-provenance = pkgs:
      cargoCheck pkgs "provenance" ''
        cargo build -p fe-c-driver --offline --locked
        export LD_LIBRARY_PATH="$(rustc --print sysroot)/lib''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
        drv="$CARGO_TARGET_DIR/debug/fe-c-driver"
        fixture=crates/fe-c-driver/tests/fixtures/provenance_fixture.rs
        FEC_PROV_FN=insert_many_like "$drv" "$fixture" \
          -o "$TMPDIR/pf" --edition 2021 >"$TMPDIR/prov.log" 2>/dev/null
        cat "$TMPDIR/prov.log"
        nu crates/fe-c-driver/tests/assert_provenance.nu "$TMPDIR/prov.log"
      '';

    # cementite's own unsafe under Miri (the miri-runtime tier from
    # docs/nix-integration.md section 3). Leak checking is off: table
    # metadata is forever-allocated by design.
    fe-c-miri = pkgs:
      cargoCheck pkgs "miri" ''
        export MIRIFLAGS=-Zmiri-ignore-leaks
        cargo miri test -p cementite --offline --locked
      '';
  };
}
