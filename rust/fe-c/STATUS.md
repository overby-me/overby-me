# Fe-C — session status (at the C1 boundary)

Written 2026-07-22. Branch: `fe-c/v0` off `main` (not pushed). This records
what is done, the one partial task, and how to resume at Phase C.

## TL;DR

Phase A (A1–A5, plus A4b), and Phase B (B1–B4 and B5's mechanism) are
**complete, tested, and committed**. **Fe-C catches a real CVE**:
instrumenting the vulnerable `smallvec@=1.6.0` traps RUSTSEC-2021-0003 and
names the right allocation (the I10 canary), while the patched control and the
hashbrown/regex-automata suites run clean under instrumentation (no false
positives). **A4b** reworked the orchestration to symbol-level injection (the
ASan model): instrumented third-party crates gain no cementite dependency
edge, and cementite is freestanding (I11). **B5's I8/I9 mechanism is done**:
lexical-scope granularity, a default-on escape analysis with three sinks
(integer cast, foreign-call argument, closure capture), the outbound FFI
escape, and `escaped_at` naming the escape site — demonstrated by three
reproducers (`stack-uaf`, `ffi-escape`, `closure-escape`). The **exact**
`corpus-rusqlite-0128` remains blocked on a §3.2 pointer-loaded-from-memory
check (its closure reads the local through a *safe* reference) and the real
vendored `libsqlite3-sys` build — see the B5 section.

The session stops at **C1** (decide the mode order + implement the first
mode), the human's decision per §8. C1 is not started; the mode-order
decision is still open (`docs/both-modes.md` §Finding argues through-first).
The §3.2 check the exact rusqlite corpus needs is itself Phase-C-adjacent.

All fe-c flake checks are green: `fmt`, `clippy`, `unit`, `miri`, `interpose`,
`census`, `provenance`, `instrument`, `corpus-smallvec`, `false-positive`,
`corpus-stackuaf`, `ffi-escape`, `closure-escape`.

## Protocol

Following **§8** of `CLAUDE.md` (added mid-project): one commit per task, run
the fe-c flake checks (individually — never `nix flake check`, it OOMs; the
repo rule), mark tasks `[done]` in §4, advance the `fe-c/v0` bookmark, never
push to `main`, never weaken a test. Hard stop at C1 (human-only).

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
| **B5** | I8 stack scopes: lexical granularity + default-on escape analysis (partial) | `corpus/stack-uaf` (inner-block `String`, escaped, dereferenced later in the same frame) aborts `UseAfterScopeExit` naming the dead scope; hashbrown stays clean default-on |

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
reliable lexical death signal — no pre-optimization MIR hook was needed.

Scope hooks are now **default-on**: an escape analysis (`escaping_locals`, a
forward taint from address-of) keeps them to the locals whose address is
laundered out of the frame, so hashbrown gets few or no hooks and neither
times out nor false-aborts (`FEC_SCOPE_HOOKS` gate removed). Three escape
sinks are wired: a **pointer→integer cast** (`corpus/stack-uaf`), a **pointer
argument to a foreign `extern "C"` call** (`corpus/ffi-escape`), and a
**pointer captured into a heap-boxed closure** (`corpus/closure-escape` — the
rusqlite-0128 closure shape).

The **outbound FFI escape** (I9 / F6) and **`escaped_at`** (F7) are done. The
sink registers the escaping local; `scope_enter` records the escape site (the
source line, via the table's existing `site` field); a use-after-scope report
prints `escaped_at=<line>`, naming where the address was handed out. All three
reproducers abort `UseAfterScopeExit` naming the dead scope *and* the escape
site. The exact rusqlite corpus entry needs:

1. **The real C build.** Vendor a *non-yanked* vulnerable `rusqlite` (0.25.3
   is yanked — use 0.25.0–0.25.2 or 0.26.0–0.26.1) + `libsqlite3-sys`
   (bundled). The A4 cc-harness and `corpus/ffi-escape` already prove the
   mixed-language build; this is the first *corpus* entry to use a real
   third-party C dependency.
2. **A `§3.2` pointer-loaded-from-memory check.** *This is the real blocker.*
   The rusqlite closure reads `local` through a **safe reference** (`r.len()`),
   which v0's raw-deref instrumentation (point 0) does not touch — the raw
   dereference in the real trampoline is of the *live* closure box (`pApp`),
   which passes. Catching the dead-local read needs a check at the point a
   pointer/reference is loaded from memory and used (trace §3.2). The
   `closure-escape` reproducer sidesteps this by capturing a *raw* pointer, so
   its read is a checked raw dereference; the real rusqlite closure captures a
   safe `&T`.
3. *(nicety)* the **inbound `extern "C"` prologue check** (`ensure_foreign_arg`,
   trace step 4): validate safe-pointer params on the way in. Not needed for
   the abort (which fires at the callback's dereference), but completes the
   both-directions I9 story.

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
4. B5's I8/I9 **mechanism is done** (lexical scopes, escape analysis with
   three sinks, outbound FFI escape, `escaped_at`; three green reproducers).
   The **exact** `corpus-rusqlite-0128` remains blocked on a `§3.2`
   pointer-loaded-from-memory check (its closure reads the local through a
   *safe* reference, which point 0's raw-deref instrumentation does not touch)
   and the real vendored `libsqlite3-sys` build (0.25.3 is yanked; use a
   non-yanked vulnerable version). The `§3.2` check is Phase-C-adjacent, so
   this naturally lands with the modes rather than before C1.
