# Fe-C — autonomous session status

Written 2026-07-22 at the end of an autonomous build session. Branch:
`fe-c/v0` (7 commits off `main`, none pushed). This file records what is
done, why the session halted where it did, and exactly how to resume.

## TL;DR

Phase A (foundation, A1–A5) is **complete, tested, and committed**, plus
the first Phase-B analysis (**B1**, capability propagation). The session
**halted before B2** (MIR rewriting): rewriting requires internal
`rustc_middle::mir` mutation plus linking `cementite` into instrumented
crates, which cannot be completed *and verified* in one session without
faking the acceptance — and the hard rules (and the "if blocked, write
STATUS.md and halt" instruction) forbid that. B2's approach is now
fact-checked and scoped below.

All 7 flake checks are green:
`fe-c-fmt`, `fe-c-clippy` (--all-features), `fe-c-unit`, `fe-c-miri`,
`fe-c-interpose`, `fe-c-census`, `fe-c-provenance`.

## Protocol discrepancy (please resolve)

The kickoff instruction said to follow **§8 (autonomous session protocol)**
of `CLAUDE.md` "exactly". **`CLAUDE.md` has no §8** — it ends at §7 ("When
unsure"). The task queue is §4. I proceeded on §4's queue and inferred a
reasonable protocol: one commit per task, run the task's acceptance check
plus fmt/clippy/unit/miri before committing, advance the `fe-c/v0` bookmark
after each, never push, never weaken a test. If §8 was meant to exist,
it was never written; nothing downstream depended on it, but flagging so a
real §8 can be added or the reference dropped.

## Done (committed on `fe-c/v0`)

| Task | Commit | Acceptance evidence |
| ---- | ------ | ------------------- |
| docs import + layout | `doc(rust): Import Fe-C and Libc-rs design docs` | files moved to `docs/`, `docs/traces/`, `corpus/`; libc docs to `../libc` |
| **A1** workspace + nix | `feat: workspace, pinned nightly, nix wiring` | `nix build .#cementite/.#fe-c-driver/.#cargo-fe-c`; `nix develop .#fe-c` gives nightly+rustc-dev+rust-src+miri; default shell rustc unchanged (stable 1.95). `docs/nix-integration.md` §6 answered against the real `nix/lib/cargo`. |
| **A2** cementite core | `feat: cementite core data structures` | 48-bit `AllocId`, `Cap`/`PackedCap`, page-radix table, id-indexed liveness bitmap. 18 unit tests, Miri-clean, criterion root-resolve bench (4.6–19.6 ns). |
| **A3** allocator + quarantine | `feat: quarantining global allocator` | `FecAlloc`; I7 (liveness cleared before release, verified by a test that fails on reorder); byte-budgeted FIFO quarantine holds under 10k churn; real `#[global_allocator]` integration test. |
| **A4** libc interposition | `feat: libc allocator interposition` | `malloc/calloc/realloc/free/posix_memalign` via `dlsym(RTLD_NEXT)`; bootstrap buffer; `#[thread_local]` reentrancy guard also coordinates with `FecAlloc` (no double-registration). `strdup` + a cc-compiled C harness tracked. Behind off-by-default `interpose` feature. |
| **A5** driver + census | `feat: rustc_public driver + visitation census` | `fe-c-driver` on `rustc_public`; census of pointer locals/derefs/casts/FFI. Runs on hello-world and the **whole serde tree** (13 crates, `skipped_bodies=0`). Hand-audited fixture check. |
| **B1** propagation dataflow | `feat: capability propagation dataflow (I10)` | Resolves at derivation roots, propagates through offsets/`ptr::add`, handles writes via `ptr::write`/`ptr::copy` intrinsics. **On real `smallvec@=1.6.0`: `insert_many rooted_writes=4 write_roots=["as_mut_ptr"]`.** |
| infra | `chore(nix/devshell): Exclude rust/fe-c from the stable clippy hook` | the pre-commit clippy hook runs stable cargo, which can't build `#![feature(rustc_private)]`; fe-c's clippy runs on nightly via the `fe-c-clippy` flake check. |

### Where the code lives

- `crates/cementite/src/` — `cap.rs` (capabilities), `liveness.rs`
  (bitmap), `table.rs` (allocation table), `arena.rs` (forever-mmap
  arenas), `alloc.rs` (`FecAlloc` + quarantine), `interpose.rs` (A4,
  feature-gated), `sys.rs` (private mmap backend).
- `crates/fe-c-driver/src/` — `main.rs` (RUSTC/RUSTC_WRAPPER drop-in),
  `census.rs` (A5), `provenance.rs` (B1).
- Checks + fixtures under `crates/fe-c-driver/tests/` and `default.nix`.

## Halt point: B2 (MIR rewriting) — scoped, not started

**Why here.** `rustc_public` (stable MIR) is read-only — confirmed by
reading the toolchain source: no `&mut Body`, transform, or rewrite API.
Rewriting must therefore drop to `rustc_driver::Callbacks` +
`config.override_queries` and mutate internal `rustc_middle::mir::Body`.
That is a different driver entry point than the `rustc_public::run!` the
census/B1 use, plus a link-integration problem. It is real work with real
uncertainty; a half-done version that "compiles but doesn't instrument"
would be a fake pass, so it was not started rather than faked.

**Fact-checked approach (verified against the pinned nightly's source):**

1. New driver mode built on `rustc_driver::run_compiler(&args, &mut cb)`
   with a `Callbacks::config` that sets
   `config.override_queries = Some(|_sess, providers| { … })`
   (the field is `Option<fn(&Session, &mut Providers)>`).
2. Inside, wrap `providers.optimized_mir`: call
   `(rustc_interface::DEFAULT_QUERY_PROVIDERS.optimized_mir)(tcx, def_id)`
   for the original `&Body`, **clone**, insert instrumentation, and return
   it via the arena (existing MIR passes use `tcx.alloc_steal_mir`; the
   override returns `&'tcx Body`). The B1 analysis already identifies
   exactly which statements/terminators to instrument, on the *stable* MIR
   — reuse its classification, mapping stable↔internal via
   `rustc_public::rustc_internal::{internal, run}`.
3. **The remaining hard piece** is the injected call's target and linking:
   to insert `check_deref(ptr, size, cap)` the pass needs the callee's
   `DefId` and `cementite` linked into the instrumented crate. Two options:
   - resolve `cementite::check_*` by path via `tcx` (requires the crate to
     `--extern cementite=…`, injected by `cargo-fe-c`), or
   - inject an FFI call to a `#[no_mangle] extern "C"` cementite export
     (needs an extern-decl `DefId` synthesised in the crate).
   Either way, `cargo-fe-c` must add `cementite` to the link (and, for
   whole-program coverage, `-Zbuild-std` — Task D1). This is the piece to
   build first; the MIR mutation itself is mechanical once the target
   resolves.

**Minimal B2 acceptance path.** Add `#[no_mangle] extern "C" fn
__fec_check_deref(ptr, size)` to `cementite` that bumps a runtime counter
and prints it on exit; have `cargo-fe-c` link `cementite` (staticlib);
inject a call before each raw deref the B1 pass flags; confirm an
instrumented hello-world runs and the counter is non-zero.

## B3–B5 depend on B2

- **B3** (raw-deref checking, `corpus-smallvec-0003` aborts, I10 canary):
  needs B2 + `check_deref` comparing the *propagated* cap (B1 already
  yields the right provenance so the report names the SmallVec allocation,
  not the neighbouring `String`). Fixture: vendored `smallvec@1.6.0` (its
  source is already resolved and the reproducer is in
  `docs/traces/rustsec-2021-0003.md`), plus a `-control` (`1.6.1`) and a
  `-unmapped` variant.
- **B4** (cast checks + `ensure`, false-positive suite): serde/regex/
  hashbrown test suites pass instrumented — needs B2 + the `nix/lib/cargo`
  per-crate wrapper (see below).
- **B5** (stack scope hooks I8 + FFI checks, `corpus-rusqlite-0128`
  aborts): needs B2 + `libsqlite3-sys` (real C — the A4 cc-harness already
  proved the mixed-language build works).

## Open items noted along the way

- **Interposed frees don't quarantine yet.** `interpose::free` clears
  liveness (I7) then frees immediately. Routing C frees through the shared
  quarantine needs a per-origin release dispatch on the quarantine node
  (System vs libc `free`); the node already carries the origin pointer, so
  this is a small, well-scoped addition. (`crates/cementite/src/alloc.rs`,
  `interpose.rs`.)
- **`nix/lib/cargo` has no per-crate wrapper / hash-extension** (verified,
  `docs/nix-integration.md` §6 Q1/Q2). The per-crate `harden` dial and
  per-crate instrumentation (phase C / B4) need this patch: thread a
  per-crate flags/driver attribute from `crateOverrides` through
  `build/buildCrate.nix`, and add `harden` to the artifact hash string in
  `build/buildCargoProject.nix` (~line 220) or mode flips reuse stale
  artifacts.
- **Toolchain pin.** `rust-toolchain.toml` = `nightly-2026-06-29` (newest
  date in the locked `rust-overlay` input with rustc-dev/rust-src/miri).
  `nix/miri-std.Cargo.lock` is a committed copy of the toolchain's
  `library/Cargo.lock` so `cargo miri setup` builds offline; **refresh it
  on every nightly bump**.
- **Driver runtime linkage.** `nix build .#fe-c-driver` bakes the
  `librustc_driver` rpath, so the built binary is self-contained. Run it
  outside nix with `LD_LIBRARY_PATH=$(rustc --print sysroot)/lib`.

## How to resume

1. Read this file, then `CLAUDE.md` §4 (queue) and §2 (hard rules).
2. Start B2 with the minimal acceptance path above. Build the
   `cargo-fe-c` link-injection of `cementite` first; the MIR mutation is
   mechanical once the check-fn `DefId` resolves.
3. Keep the both-modes rule (I4) in view: `docs/both-modes.md` must stay
   filled before any checking feature merges. The open mode-order decision
   (Task C1, through-first vs case-first) is still open and does not block
   B1–B5.
