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
      # Corpus reproducer crates (separate workspaces) for the corpus
      # checks. Ignored by the workspace build; used by fe-c-corpus-*.
      ./corpus
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
    locks = [
      ./Cargo.lock
      ./nix/miri-std.Cargo.lock
      # The B2 instrumentation harness is its own workspace with its own
      # lock (cementite path dep + rustix tree).
      ./crates/fe-c-driver/tests/fixtures/harness/Cargo.lock
      # The B3 corpus reproducer + its patched control.
      ./corpus/smallvec-0003/Cargo.lock
      ./corpus/smallvec-0003-control/Cargo.lock
      # The B4 false-positive workload (hashbrown tree).
      ./corpus/false-positive/Cargo.lock
      # The B5 stack-UAF reproducer (no third-party deps).
      ./corpus/stack-uaf/Cargo.lock
    ];
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

    # MIR instrumentation (B2): build the harness with FEC_INSTRUMENT, run
    # it, and assert the injected cementite checks fire at runtime with the
    # program's behaviour unchanged; a control build fires zero checks.
    fe-c-instrument = pkgs:
      cargoCheck pkgs "instrument" ''
        cargo build -p fe-c-driver -p cementite --offline --locked
        export LD_LIBRARY_PATH="$(rustc --print sysroot)/lib''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
        drv="$CARGO_TARGET_DIR/debug/fe-c-driver"
        h=crates/fe-c-driver/tests/fixtures/harness

        # Instrumented build (separate target dir: env changes are not
        # cargo-fingerprinted, so instrumented and control must not share).
        ( cd "$h" && FEC_INSTRUMENT=1 RUSTC="$drv" CARGO_TARGET_DIR="$TMPDIR/ti" \
            cargo build --offline --locked )
        "$TMPDIR/ti/debug/fec-harness" >"$TMPDIR/ins.log" 2>&1

        # Uninstrumented control.
        ( cd "$h" && RUSTC="$drv" CARGO_TARGET_DIR="$TMPDIR/tc" \
            cargo build --offline --locked )
        "$TMPDIR/tc/debug/fec-harness" >"$TMPDIR/ctl.log" 2>&1

        echo "--- instrumented ---"; cat "$TMPDIR/ins.log"
        echo "--- control ---"; cat "$TMPDIR/ctl.log"
        nu crates/fe-c-driver/tests/assert_instrument.nu "$TMPDIR/ins.log" "$TMPDIR/ctl.log"
      '';

    # Corpus RUSTSEC-2021-0003 (B3, I10 canary): build the smallvec 1.6.0
    # reproducer instrumented and assert it aborts naming the SmallVec
    # allocation (not the neighbouring String), while the patched 1.6.1
    # control runs clean. cementite is force-injected into every compile so
    # smallvec itself is instrumented.
    fe-c-corpus-smallvec = pkgs:
      cargoCheck pkgs "corpus-smallvec" ''
        cargo build -p fe-c-driver -p cementite --offline --locked
        export LD_LIBRARY_PATH="$(rustc --print sysroot)/lib''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
        drv="$CARGO_TARGET_DIR/debug/fe-c-driver"
        export FEC_INSTRUMENT=1
        export FEC_CEMENTITE_RLIB="$CARGO_TARGET_DIR/debug/libcementite.rlib"
        export FEC_CEMENTITE_DEPS="$CARGO_TARGET_DIR/debug/deps"

        # Reproducer: vulnerable smallvec 1.6.0, must abort naming SmallVec.
        ( cd corpus/smallvec-0003 \
            && RUSTC="$drv" CARGO_TARGET_DIR="$TMPDIR/tr" cargo build --offline --locked )
        set +e
        "$TMPDIR/tr/debug/smallvec-0003" >"$TMPDIR/repro.log" 2>&1
        repro_exit=$?
        set -e

        # Control: patched smallvec 1.6.1, must run clean.
        ( cd corpus/smallvec-0003-control \
            && RUSTC="$drv" CARGO_TARGET_DIR="$TMPDIR/tc" cargo build --offline --locked )
        set +e
        "$TMPDIR/tc/debug/smallvec-0003-control" >"$TMPDIR/control.log" 2>&1
        control_exit=$?
        set -e

        echo "--- reproducer (exit $repro_exit) ---"; cat "$TMPDIR/repro.log"
        echo "--- control (exit $control_exit) ---"; cat "$TMPDIR/control.log"
        nu corpus/assert_smallvec_0003.nu \
          "$TMPDIR/repro.log" "$repro_exit" "$TMPDIR/control.log" "$control_exit"
      '';

    # False-positive suite (B4): instrument hashbrown's SwissTable unsafe
    # and hammer it under FecAlloc — legitimate in-bounds unsafe must never
    # trap. (serde/regex/hashbrown full test suites also pass instrumented;
    # see STATUS. hashbrown was chosen for the offline check as the most
    # raw-pointer-heavy with a tractable dependency tree.)
    fe-c-false-positive = pkgs:
      cargoCheck pkgs "false-positive" ''
        cargo build -p fe-c-driver -p cementite --offline --locked
        export LD_LIBRARY_PATH="$(rustc --print sysroot)/lib''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
        drv="$CARGO_TARGET_DIR/debug/fe-c-driver"
        export FEC_INSTRUMENT=1
        export FEC_CEMENTITE_RLIB="$CARGO_TARGET_DIR/debug/libcementite.rlib"
        export FEC_CEMENTITE_DEPS="$CARGO_TARGET_DIR/debug/deps"
        ( cd corpus/false-positive \
            && FEC_INSTRUMENT_ONLY=hashbrown,fec_fp RUSTC="$drv" \
               CARGO_TARGET_DIR="$TMPDIR/t" cargo build --offline --locked )
        set +e
        "$TMPDIR/t/debug/fec-fp" >"$TMPDIR/fp.log" 2>&1
        fp_exit=$?
        set -e
        cat "$TMPDIR/fp.log"
        nu corpus/assert_false_positive.nu "$TMPDIR/fp.log" "$fp_exit"
      '';

    # Stack scope hooks (B5, I8): with FEC_SCOPE_HOOKS on, build the
    # stack-UAF reproducer (a stack pointer laundered past its frame, as the
    # rusqlite-0128 closure does across FFI) and assert the stale deref
    # aborts UseAfterScopeExit naming the dead stack scope.
    fe-c-corpus-stackuaf = pkgs:
      cargoCheck pkgs "corpus-stackuaf" ''
        cargo build -p fe-c-driver -p cementite --offline --locked
        export LD_LIBRARY_PATH="$(rustc --print sysroot)/lib''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
        drv="$CARGO_TARGET_DIR/debug/fe-c-driver"
        export FEC_INSTRUMENT=1 FEC_SCOPE_HOOKS=1
        export FEC_CEMENTITE_RLIB="$CARGO_TARGET_DIR/debug/libcementite.rlib"
        export FEC_CEMENTITE_DEPS="$CARGO_TARGET_DIR/debug/deps"
        ( cd corpus/stack-uaf \
            && FEC_INSTRUMENT_ONLY=stack_uaf RUSTC="$drv" \
               CARGO_TARGET_DIR="$TMPDIR/t" cargo build --offline --locked )
        set +e
        "$TMPDIR/t/debug/stack-uaf" >"$TMPDIR/su.log" 2>&1
        su_exit=$?
        set -e
        cat "$TMPDIR/su.log"
        nu corpus/assert_stack_uaf.nu "$TMPDIR/su.log" "$su_exit"
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
