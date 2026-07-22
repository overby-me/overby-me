# Fe-C — session status (at the C1 boundary)

Written 2026-07-22. Branch: `fe-c/v0` off `main` (not pushed). This records
what is done, the one partial task, and how to resume at Phase C.

## TL;DR

Phase A (A1–A5) and Phase B's checking work (B1–B4) are **complete, tested,
and committed**. **Fe-C catches a real CVE**: instrumenting the vulnerable
`smallvec@=1.6.0` traps RUSTSEC-2021-0003 and names the right allocation
(the I10 canary), while the patched control and the hashbrown/regex-automata
suites run clean under instrumentation (no false positives). **B5 is
partial**: the I8 stack-scope mechanism is built and demonstrated
(use-after-scope-exit is caught), but the exact `corpus-rusqlite-0128`
(inner-lexical-scope UAF across real-C FFI) is not yet done — see below.

The session stops at **C1** (decide the mode order + implement the first
mode) as instructed. C1 is not started; the mode-order decision is still
open (`docs/both-modes.md` §Finding argues through-first).

All fe-c flake checks are green: `fmt`, `clippy`, `unit`, `miri`,
`interpose`, `census`, `provenance`, `instrument`, `corpus-smallvec`,
`false-positive`, `corpus-stackuaf`.

## Protocol discrepancy (unresolved)

The kickoff said to follow **§8 (autonomous session protocol)** of
`CLAUDE.md`. **There is no §8** — the file ends at §7. I worked §4's queue
and inferred a protocol: one commit per task, run the task's acceptance plus
fmt/clippy/unit/miri before committing, advance the `fe-c/v0` bookmark, never
push, never weaken a test. Flagging so a real §8 is written or the reference
dropped.

## Done (committed on `fe-c/v0`)

| Task | What ships | Evidence |
| ---- | ---------- | -------- |
| **A1** | 3-crate workspace, pinned nightly, nix wiring | `nix build .#cementite/.#fe-c-driver/.#cargo-fe-c`; devShell nightly; §6 API questions answered |
| **A2** | cementite core (48-bit `AllocId`, `Cap`/`PackedCap`, page-radix table, id-indexed liveness bitmap) | 24 unit tests, Miri-clean, criterion bench |
| **A3** | `FecAlloc` quarantining allocator | I7 verified; byte-budget FIFO; global-allocator integration test |
| **A4** | libc interposition (`dlsym(RTLD_NEXT)`) | strdup + cc-compiled C harness tracked; behind `interpose` feature |
| **A5** | `rustc_public` driver + visitation census | runs on the whole serde tree (13 crates, `skipped_bodies=0`) |
| **B1** | capability propagation dataflow (I10) | real `smallvec@=1.6.0` `insert_many rooted_writes=4 write_roots=["as_mut_ptr"]` |
| **B2** | MIR rewriting (`override_queries` on `optimized_mir`) | instrumented harness reports non-zero check counts, control clean |
| **B3** | raw-deref checking, **catches RUSTSEC-2021-0003** | real smallvec 1.6.0 aborts naming the SmallVec buffer (not the String); 1.6.1 control clean |
| **B4** | false-positive suite + **dependency-free runtime** | hashbrown (98 tests + 907k-check workload) and regex-automata (204 tests) instrumented, zero false traps |
| **B5** | I8 stack-scope mechanism (partial) | `corpus/stack-uaf` aborts `UseAfterScopeExit` naming the dead scope; gated behind `FEC_SCOPE_HOOKS` |

### The instrumentation, in brief

`fe-c-driver` has two modes (dispatched in `main.rs`):

- **Census/B1** (default): `rustc_public::run!` + a read-only census +
  provenance dataflow (`census.rs`, `provenance.rs`).
- **Instrument** (`FEC_INSTRUMENT`): a `rustc_driver::Callbacks` that wraps
  the `optimized_mir` query and rewrites MIR (`instrument.rs`): a rooted
  deref check (`__fec_check_deref_rooted(fault, root)`) before every raw
  access and pointer-write intrinsic, resolving the owning allocation from
  the *derivation root* (I10) and checking spatial bounds + temporal
  liveness. Stack scope hooks (I8) are a gated Pass 0.

`cementite` is the runtime (`check.rs` check/scope entry points, `table.rs`,
`alloc.rs`, `arena.rs`, `sys.rs`, `liveness.rs`, `interpose.rs`).

### Orchestration (the `cargo-fe-c` seam, built out during B3/B4)

- **Symbol-level injection (A4b, the ASan model).** Each instrumented crate
  only *declares* the `__fec_*` check symbols — the driver injects an
  `unsafe extern "C"` block into the AST (`inject_fec_decls`) and rewrites
  MIR to call it. There is **no cementite dependency edge** into third-party
  crates (`cargo tree` on smallvec/hashbrown is unchanged); cementite is
  linked *once* into the final binary. A leaf binary that installs `FecAlloc`
  (a Rust type, so it genuinely needs the crate) takes cementite as an
  ordinary Cargo path dependency — cargo links it and its object resolves
  every crate's `__fec_*`. Where there is no such edge (a future
  `-Zbuild-std` target), `FEC_CEMENTITE_RLIB` names a prebuilt
  `libcementite.rlib` the driver appends as a link-arg.
- The driver **never instruments** cementite itself, build scripts
  (`build_script_*`), or proc-macro crates: they are host-time or define the
  symbols, so injected calls would be undefined or recursive.
- **`FEC_INSTRUMENT_ONLY=<crate,…>`** scopes instrumentation to a crate list,
  leaving dependencies uninstrumented (a speed lever for the corpus checks);
  whole-graph is the default and now works because injection is symbol-level.
- **cementite is freestanding** (invariant I11): zero dependencies — not even
  build-time ones (raw `mmap` syscall in `sys.rs`; `build.rs` calls `cc`/`ar`
  directly). It links into any crate without version conflicts across
  separate cargo build graphs, and drags no build script into a dependent's
  graph for the whole-graph instrumenter to trip over.

## B5 — what's left for `corpus-rusqlite-0128`

The **I8 mechanism is done and demonstrated** (`corpus/stack-uaf` →
`UseAfterScopeExit`), now at **lexical granularity**: `scope_exit` fires at a
local's lexical death point — its `Drop { local }` terminator, else its
`StorageDead(local)` — not just at `Return`. So the inner-block-then-callback
shape (the borrow's target dies at a block's end while the read happens later
in the *same* frame) is caught, not only use-after-frame-return. The earlier
worry that "optimized MIR strips `StorageLive`/`Dead`" turned out not to
block this: optimized MIR keeps **drop terminators** (semantically required),
and the rusqlite local is a `String` (a `Drop` type), so its drop glue is a
reliable lexical death signal — no pre-optimization MIR hook was needed. The
exact rusqlite corpus entry needs three more pieces, each a real chunk:

1. **Escape analysis.** Scope hooks are still gated (`FEC_SCOPE_HOOKS`)
   because instrumenting *every* address-taken local is impractical
   (hashbrown times out registering/poisoning a stack region per call). Only
   locals whose address genuinely escapes the frame (cast to int, stored to a
   static/heap, or passed to an FFI call) need hooks. Reuse the
   `compute_roots` provenance to trace escape operands back to an
   address-of; then default-on becomes affordable.
2. **FFI inbound/outbound checks (point 3, I9).** `extern "C"` prologues
   validate safe-pointer params; outbound pointer args are marked escaped
   (`note_escape`). The `Violation` needs `escaped_at` so the report names
   the registration site (`create_scalar_function`), per trace F7.
3. **The real C build.** Vendor `rusqlite@0.25.3` + `libsqlite3-sys`
   (bundled), with provenance flowing through the trampoline. The A4
   cc-harness already proved the mixed-language build works; this is the
   first *corpus* entry to use it.

Also address-reuse soundness: a poisoned stack region that is not
re-registered by the next frame can produce a stale resolve. `scope_enter`
already replaces a dead region at the same base; a fuller fix ties poisoning
to the escape (only escaped regions stay findable-as-dead).

## Open items carried forward

- **Interposed frees don't quarantine.** `interpose::free` clears liveness
  (I7) then frees immediately; routing C frees through the shared quarantine
  needs a per-origin release dispatch on the node (System vs libc `free`).
- **`nix/lib/cargo` per-crate wrapper / hash-extension** (docs/nix-integration
  §6 Q1/Q2) — needed for the per-crate `harden` dial and for whole-tree
  instrumentation via a proper wrapper rather than `RUSTC=`.
- **`-Zbuild-std` sysroot (D1)** would make cementite a sysroot crate,
  removing the `FEC_INSTRUMENT_ONLY` scoping limitation and enabling
  whole-process coverage (D2 + `../libc`).
- **Toolchain pin** `nightly-2026-06-29`; `nix/miri-std.Cargo.lock` is a
  committed copy of the toolchain's `library/Cargo.lock` — refresh on bumps.
- **`FEC_DEBUG=1`** makes the driver print which bodies it instruments and
  how many checks — useful when validating a new corpus entry.

## How to resume (Phase C / C1)

1. Read this file, then `CLAUDE.md` §3 (the open mode-order decision) and
   §4.
2. **C1** is the recorded decision: through-first vs case-first
   (`docs/both-modes.md` argues through-first — it needs less machinery and
   is the oracle for differential-testing case). Record it under I4, then
   implement the first mode end to end.
3. The both-modes rule (I4) still binds: `docs/both-modes.md` must stay
   filled before any checking feature merges.
4. To finish B5 first, do the four pieces above in order; the escape
   analysis is the highest-leverage (unlocks default-on scope hooks).
