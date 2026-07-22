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
      # lock (a cementite path dep; cementite is dependency-free, I11).
      ./crates/fe-c-driver/tests/fixtures/harness/Cargo.lock
      # The B3 corpus reproducer + its patched control.
      ./corpus/smallvec-0003/Cargo.lock
      ./corpus/smallvec-0003-control/Cargo.lock
      # The B4 false-positive workload (hashbrown tree).
      ./corpus/false-positive/Cargo.lock
      # The B5 stack-UAF reproducer (no third-party deps).
      ./corpus/stack-uaf/Cargo.lock
      # The B5 / I9 cross-FFI escape reproducer (no third-party deps).
      ./corpus/ffi-escape/Cargo.lock
      # The B5 / I9 closure heap-escape reproducer (no third-party deps).
      ./corpus/closure-escape/Cargo.lock
      # The C1 through-mode safe-reference reproducer (no third-party deps).
      ./corpus/through-safe-ref/Cargo.lock
      # The real RUSTSEC-2021-0128 corpus: rusqlite 0.25.3 + bundled SQLite.
      # rusqlite 0.25.3 is yanked (its lock entry is hand-added), but yanked
      # crates still serve from the CDN, so fetchurl vendors it like any other.
      ./corpus/rusqlite-0128/Cargo.lock
      # The real RUSTSEC-2021-0130 corpus: lru 0.6.6 (use-after-free).
      ./corpus/lru-0130/Cargo.lock
      # The point-1 raw->safe cast OOB reproducer (no third-party deps).
      ./corpus/cast-oob/Cargo.lock
      # Heap UAF with mint-site naming (no third-party deps).
      ./corpus/heap-mint/Cargo.lock
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
    # control runs clean. Whole-graph instrumentation (no FEC_INSTRUMENT_ONLY)
    # covers smallvec too; the injected checks are symbol-level (A4b) and
    # resolve against cementite, which the fixture links as a path dependency.
    fe-c-corpus-smallvec = pkgs:
      cargoCheck pkgs "corpus-smallvec" ''
        cargo build -p fe-c-driver --offline --locked
        export LD_LIBRARY_PATH="$(rustc --print sysroot)/lib''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
        drv="$CARGO_TARGET_DIR/debug/fe-c-driver"
        export FEC_INSTRUMENT=1

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
        cargo build -p fe-c-driver --offline --locked
        export LD_LIBRARY_PATH="$(rustc --print sysroot)/lib''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
        drv="$CARGO_TARGET_DIR/debug/fe-c-driver"
        export FEC_INSTRUMENT=1
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

    # Stack scope hooks (B5, I8): build the stack-UAF reproducer (an
    # inner-block stack local laundered past its scope, as the rusqlite-0128
    # closure does across FFI) and assert the stale deref, later in the same
    # frame, aborts UseAfterScopeExit naming the dead stack scope. Scope hooks
    # are default-on (an escape analysis keeps them to laundered locals), so no
    # FEC_SCOPE_HOOKS is needed.
    fe-c-corpus-stackuaf = pkgs:
      cargoCheck pkgs "corpus-stackuaf" ''
        cargo build -p fe-c-driver --offline --locked
        export LD_LIBRARY_PATH="$(rustc --print sysroot)/lib''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
        drv="$CARGO_TARGET_DIR/debug/fe-c-driver"
        export FEC_INSTRUMENT=1
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

    # Cross-FFI escape (B5, I9 / trace F6): build the ffi-escape reproducer
    # (a stack borrow handed out to a genuinely C-compiled harness, the frame
    # returned, then C re-enters Rust through a trampoline and dereferences
    # the dead local) and assert it aborts UseAfterScopeExit naming the dead
    # stack scope. The escape analysis recognises the outbound extern-C
    # pointer argument; the C harness itself is not instrumented (F8).
    fe-c-ffi-escape = pkgs:
      cargoCheck pkgs "ffi-escape" ''
        cargo build -p fe-c-driver --offline --locked
        export LD_LIBRARY_PATH="$(rustc --print sysroot)/lib''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
        drv="$CARGO_TARGET_DIR/debug/fe-c-driver"
        export FEC_INSTRUMENT=1
        ( cd corpus/ffi-escape \
            && FEC_INSTRUMENT_ONLY=ffi_escape RUSTC="$drv" \
               CARGO_TARGET_DIR="$TMPDIR/t" cargo build --offline --locked )
        set +e
        "$TMPDIR/t/debug/ffi-escape" >"$TMPDIR/fe.log" 2>&1
        fe_exit=$?
        set -e
        cat "$TMPDIR/fe.log"
        nu corpus/assert_ffi_escape.nu "$TMPDIR/fe.log" "$fe_exit"
      '';

    # Closure heap-escape (B5, I9): build the closure-escape reproducer (a raw
    # pointer to a stack local captured by-move into a boxed closure kept past
    # the frame, then dereferenced when the closure runs) and assert it aborts
    # UseAfterScopeExit naming the dead scope and the capture site. This is the
    # rusqlite-0128 closure shape; the escape analysis recognises the pointer
    # captured into a heap-boxed closure.
    fe-c-closure-escape = pkgs:
      cargoCheck pkgs "closure-escape" ''
        cargo build -p fe-c-driver --offline --locked
        export LD_LIBRARY_PATH="$(rustc --print sysroot)/lib''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
        drv="$CARGO_TARGET_DIR/debug/fe-c-driver"
        export FEC_INSTRUMENT=1
        ( cd corpus/closure-escape \
            && FEC_INSTRUMENT_ONLY=closure_escape RUSTC="$drv" \
               CARGO_TARGET_DIR="$TMPDIR/t" cargo build --offline --locked )
        set +e
        "$TMPDIR/t/debug/closure-escape" >"$TMPDIR/ce.log" 2>&1
        ce_exit=$?
        set -e
        cat "$TMPDIR/ce.log"
        nu corpus/assert_closure_escape.nu "$TMPDIR/ce.log" "$ce_exit"
      '';

    # Through-mode safe-deref checking (C1): the one bolded row of the
    # both-modes table. A closure captures a safe &u64 to a stack local and
    # reads it back after the frame dies. Built twice: with FEC_MODE=through
    # the safe dereference is checked and it aborts UseAfterScopeExit; with
    # FEC_MODE unset (case-like) the safe deref is elided and it runs clean.
    fe-c-through-safe-ref = pkgs:
      cargoCheck pkgs "through-safe-ref" ''
        cargo build -p fe-c-driver --offline --locked
        export LD_LIBRARY_PATH="$(rustc --print sysroot)/lib''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
        drv="$CARGO_TARGET_DIR/debug/fe-c-driver"
        export FEC_INSTRUMENT=1 FEC_INSTRUMENT_ONLY=through_safe_ref

        # through mode: the safe deref is checked -> abort.
        ( cd corpus/through-safe-ref \
            && FEC_MODE=through RUSTC="$drv" CARGO_TARGET_DIR="$TMPDIR/tt" cargo build --offline --locked )
        set +e
        "$TMPDIR/tt/debug/through-safe-ref" >"$TMPDIR/th.log" 2>&1
        th_exit=$?
        set -e

        # case-like mode (FEC_MODE unset): the safe deref is elided -> clean.
        ( cd corpus/through-safe-ref \
            && RUSTC="$drv" CARGO_TARGET_DIR="$TMPDIR/tc" cargo build --offline --locked )
        set +e
        "$TMPDIR/tc/debug/through-safe-ref" >"$TMPDIR/ca.log" 2>&1
        ca_exit=$?
        set -e

        echo "--- through (exit $th_exit) ---"; cat "$TMPDIR/th.log"
        echo "--- case-like (exit $ca_exit) ---"; cat "$TMPDIR/ca.log"
        nu corpus/assert_through_safe_ref.nu "$TMPDIR/th.log" "$th_exit" "$TMPDIR/ca.log" "$ca_exit"
      '';

    # The real CVE (B5): RUSTSEC-2021-0128 against unmodified rusqlite 0.25.3 +
    # bundled SQLite. A closure captures a stack borrow, is registered with
    # SQLite, outlives the frame, and is invoked by SQLite (C) — reading the
    # dropped local through a safe reference. Built twice: FEC_MODE=through
    # aborts UseAfterScopeExit naming the dead scope and the registration site;
    # FEC_MODE unset (case-like) elides the safe deref and runs clean. The
    # first corpus entry pulling real third-party C; only the Rust boundary is
    # instrumented (rusqlite_0128), not SQLite.
    fe-c-rusqlite-0128 = pkgs:
      cargoCheck pkgs "rusqlite-0128" ''
        cargo build -p fe-c-driver --offline --locked
        export LD_LIBRARY_PATH="$(rustc --print sysroot)/lib''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
        drv="$CARGO_TARGET_DIR/debug/fe-c-driver"
        export FEC_INSTRUMENT=1 FEC_INSTRUMENT_ONLY=rusqlite_0128

        # through mode: the closure's safe-reference read is checked -> abort.
        ( cd corpus/rusqlite-0128 \
            && FEC_MODE=through RUSTC="$drv" CARGO_TARGET_DIR="$TMPDIR/tt" cargo build --offline --locked )
        set +e
        "$TMPDIR/tt/debug/rusqlite-0128" >"$TMPDIR/th.log" 2>&1
        th_exit=$?
        set -e

        # case-like mode (FEC_MODE unset): the safe deref is elided -> clean.
        ( cd corpus/rusqlite-0128 \
            && RUSTC="$drv" CARGO_TARGET_DIR="$TMPDIR/tc" cargo build --offline --locked )
        set +e
        "$TMPDIR/tc/debug/rusqlite-0128" >"$TMPDIR/ca.log" 2>&1
        ca_exit=$?
        set -e

        echo "--- through (exit $th_exit) ---"; cat "$TMPDIR/th.log"
        echo "--- case-like (exit $ca_exit) ---"; cat "$TMPDIR/ca.log"
        nu corpus/assert_rusqlite_0128.nu "$TMPDIR/th.log" "$th_exit" "$TMPDIR/ca.log" "$ca_exit"
      '';

    # Heap use-after-free (RUSTSEC-2021-0130): real lru 0.6.6. iter() yields a
    # reference into a node; the loop pop()s (frees) the node and reads the
    # value through the dangling reference. Built in BOTH modes: through checks
    # every dereference; case elides safe derefs but re-checks this one because
    # it is dealloc-reachable (follows the pop() call) — the point-4 / I6
    # re-check (C2). Both resolve the freed heap allocation, kept findable in
    # quarantine, and abort UseAfterFree. Heap temporal safety, the complement
    # of the stack-scope UAF corpora.
    fe-c-lru-0130 = pkgs:
      cargoCheck pkgs "lru-0130" ''
        cargo build -p fe-c-driver --offline --locked
        export LD_LIBRARY_PATH="$(rustc --print sysroot)/lib''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
        drv="$CARGO_TARGET_DIR/debug/fe-c-driver"
        export FEC_INSTRUMENT=1 FEC_INSTRUMENT_ONLY=lru_0130

        ( cd corpus/lru-0130 \
            && FEC_MODE=through RUSTC="$drv" CARGO_TARGET_DIR="$TMPDIR/tt" cargo build --offline --locked )
        set +e
        "$TMPDIR/tt/debug/lru-0130" >"$TMPDIR/th.log" 2>&1
        th_exit=$?
        set -e

        ( cd corpus/lru-0130 \
            && RUSTC="$drv" CARGO_TARGET_DIR="$TMPDIR/tc" cargo build --offline --locked )
        set +e
        "$TMPDIR/tc/debug/lru-0130" >"$TMPDIR/ca.log" 2>&1
        ca_exit=$?
        set -e

        echo "--- through (exit $th_exit) ---"; cat "$TMPDIR/th.log"
        echo "--- case (exit $ca_exit) ---"; cat "$TMPDIR/ca.log"
        nu corpus/assert_lru_0130.nu "$TMPDIR/th.log" "$th_exit" "$TMPDIR/ca.log" "$ca_exit"
      '';

    # Raw->safe cast ensure (point 1, §3.1): a raw pointer past the end of a
    # Vec buffer is cast to a safe `&u64`. The cast ensure resolves the
    # derivation root and validates the referent's extent, aborting OutOfBounds
    # in BOTH modes (case elides the later derefs; through's deref check
    # resolves the off-the-end faulting address, so it too relies on the cast
    # ensure). The spatial-at-mint check that makes case-mode elision sound.
    fe-c-cast-oob = pkgs:
      cargoCheck pkgs "cast-oob" ''
        cargo build -p fe-c-driver --offline --locked
        export LD_LIBRARY_PATH="$(rustc --print sysroot)/lib''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
        drv="$CARGO_TARGET_DIR/debug/fe-c-driver"
        export FEC_INSTRUMENT=1 FEC_INSTRUMENT_ONLY=cast_oob

        ( cd corpus/cast-oob \
            && FEC_MODE=through RUSTC="$drv" CARGO_TARGET_DIR="$TMPDIR/tt" cargo build --offline --locked )
        ( cd corpus/cast-oob \
            && RUSTC="$drv" CARGO_TARGET_DIR="$TMPDIR/tc" cargo build --offline --locked )

        # Four scenarios, each in both modes: a whole-object `&*bad` reborrow
        # (no arg), a field reborrow `&(*p).b` past the end (`field`), a *direct*
        # field read `(*p).b` whose start is past the end (`direct`, caught by
        # the projected deref fault), and a direct read whose start is in bounds
        # but whose extent overruns (`extent`, caught by the extent check). All
        # abort OutOfBounds.
        for scenario in whole:"" field:field direct:direct extent:extent; do
          name="''${scenario%%:*}"; arg="''${scenario#*:}"
          set +e
          "$TMPDIR/tt/debug/cast-oob" $arg >"$TMPDIR/t-$name.log" 2>&1; t_exit=$?
          "$TMPDIR/tc/debug/cast-oob" $arg >"$TMPDIR/c-$name.log" 2>&1; c_exit=$?
          set -e
          echo "--- through/$name (exit $t_exit) ---"; cat "$TMPDIR/t-$name.log"
          echo "--- case/$name (exit $c_exit) ---"; cat "$TMPDIR/c-$name.log"
          nu corpus/assert_cast_oob.nu "through/$name" "$TMPDIR/t-$name.log" "$t_exit"
          nu corpus/assert_cast_oob.nu "case/$name" "$TMPDIR/c-$name.log" "$c_exit"
        done
      '';

    # Heap UAF with mint-site naming (trace -0130 debuggability): a Box is freed
    # while a field reference minted into it is held, then read. Both modes abort
    # UseAfterFree; because the mint (`&(*p).b`) is in this instrumented binary,
    # the report names `minted_at` (where the reference was born) — and case also
    # names `read_at`. Demonstrates the both-sites naming end to end.
    fe-c-heap-mint = pkgs:
      cargoCheck pkgs "heap-mint" ''
        cargo build -p fe-c-driver --offline --locked
        export LD_LIBRARY_PATH="$(rustc --print sysroot)/lib''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
        drv="$CARGO_TARGET_DIR/debug/fe-c-driver"
        export FEC_INSTRUMENT=1 FEC_INSTRUMENT_ONLY=heap_mint

        ( cd corpus/heap-mint \
            && FEC_MODE=through RUSTC="$drv" CARGO_TARGET_DIR="$TMPDIR/tt" cargo build --offline --locked )
        set +e
        "$TMPDIR/tt/debug/heap-mint" >"$TMPDIR/th.log" 2>&1
        th_exit=$?
        set -e

        ( cd corpus/heap-mint \
            && RUSTC="$drv" CARGO_TARGET_DIR="$TMPDIR/tc" cargo build --offline --locked )
        set +e
        "$TMPDIR/tc/debug/heap-mint" >"$TMPDIR/ca.log" 2>&1
        ca_exit=$?
        set -e

        echo "--- through (exit $th_exit) ---"; cat "$TMPDIR/th.log"
        echo "--- case (exit $ca_exit) ---"; cat "$TMPDIR/ca.log"
        nu corpus/assert_heap_mint.nu "$TMPDIR/th.log" "$th_exit" "$TMPDIR/ca.log" "$ca_exit"
      '';

    # Differential gate (C3, I4): `through` is the oracle. Build three
    # contrasting reproducers in both modes and assert `through` catches all,
    # `case` agrees on the raw + heap UAFs, and `case` misses only the
    # documented safe-pointer-deref elision (the stack-scope read). Any other
    # through-catch that case missed would be an undocumented gap — a bug.
    fe-c-differential = pkgs:
      cargoCheck pkgs "differential" ''
        cargo build -p fe-c-driver --offline --locked
        export LD_LIBRARY_PATH="$(rustc --print sysroot)/lib''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
        drv="$CARGO_TARGET_DIR/debug/fe-c-driver"
        export FEC_INSTRUMENT=1

        build() {
          ( cd "corpus/$1" \
              && FEC_MODE="$3" FEC_INSTRUMENT_ONLY="$2" RUSTC="$drv" \
                 CARGO_TARGET_DIR="$TMPDIR/$1-$3" cargo build --offline --locked )
        }
        run() {
          set +e
          "$TMPDIR/$1-$2/debug/$1" >"$TMPDIR/$1-$2.log" 2>&1
          local e=$?
          set -e
          echo "$e"
        }

        build closure-escape closure_escape through
        build closure-escape closure_escape case
        build through-safe-ref through_safe_ref through
        build through-safe-ref through_safe_ref case
        build lru-0130 lru_0130 through
        build lru-0130 lru_0130 case

        ce_th=$(run closure-escape through);      ce_ca=$(run closure-escape case)
        tsr_th=$(run through-safe-ref through);   tsr_ca=$(run through-safe-ref case)
        lru_th=$(run lru-0130 through);           lru_ca=$(run lru-0130 case)

        echo "closure-escape (raw UAF):    through=$ce_th  case=$ce_ca"
        echo "through-safe-ref (safe UAF): through=$tsr_th case=$tsr_ca"
        echo "lru-0130 (heap UAF):         through=$lru_th case=$lru_ca"
        nu corpus/assert_differential.nu \
          "$ce_th" "$ce_ca" "$tsr_th" "$tsr_ca" "$lru_th" "$lru_ca"
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
