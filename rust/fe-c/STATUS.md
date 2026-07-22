# Fe-C — session status (Phase C underway, through-first)

Written 2026-07-22. Branch: `fe-c/v0` off `main` (pushed to origin). This
records what is done and how to continue building out `through` mode.

## TL;DR

Phase A (A1–A5, plus A4b) and Phase B (B1–B4 and B5's mechanism) are
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
reproducers (`stack-uaf`, `ffi-escape`, `closure-escape`).

**Phase C is underway.** C1's mode-order decision is **through-first**
(recorded in `PLAN.md` I4 and `docs/both-modes.md` §Decision). Through mode's
defining behavior — **safe-pointer deref checking** (the one bolded both-modes
row; `FEC_MODE=through`) — is implemented and shown by `corpus/through-safe-ref`
(through aborts on a safe-reference read of a dead local; case-like mode elides
it).

**B5's exact `corpus-rusqlite-0128` is now MET** (`fe-c-rusqlite-0128`). Fe-C
catches the **real CVE in real, unmodified `rusqlite@=0.25.3` + bundled
SQLite**: the closure captures a stack borrow via `create_scalar_function`,
SQLite (C) invokes it after the frame returns, and its safe-reference read of
the dropped local aborts `UseAfterScopeExit` under `FEC_MODE=through`, naming
the dead scope and `escaped_at` the registration site; case-like mode elides
the safe deref and runs clean. rusqlite 0.25.3 is yanked, so its lock entry is
hand-completed (the CDN still serves it, so the offline fetchurl vendor works).
That safe-reference read was the §3.2 gap, now closed by through's safe-deref
checking. So B5 is fully done, on real third-party C.

**Heap use-after-free** works too, in **both modes** (`fe-c-lru-0130`): real,
unmodified `lru@=0.6.6` — its `iter()` yields a reference into a node, the loop
`pop()`s (frees) the node, and reads the value through the dangling reference.
`through` checks every deref; **`case`** (C2) elides safe derefs but re-checks
this one because it is **dealloc-reachable** (follows the `pop()` call) via
`__fec_check_dealloc_reachable`, which aborts only on a dead **heap**
allocation — a dead stack scope passes, so the mode distinction holds. Both
abort `UseAfterFree`. This needed `FecAlloc::dealloc` to **poison** the freed
allocation (keep it findable-as-dead in quarantine) rather than deregister at
free. A **third real CVE** (after RUSTSEC-2021-0003 and -0128). The `case`
report also names the **dangling-read site** (`read_at=34`): the re-check is
injected right at the dereference, so it carries that line as a third argument.

**Remaining for full `through` mode** (all substantial, interdependent, and all
*performance/precision*, not correctness — through is already sound and
exhaustive): T2 shadow-slot coherence (the at-rest cap layout that replaces the
table lookup), `strict` unknown-provenance (needs full cap propagation first,
or it false-positives on foreign statics), and interprocedural + at-rest
capability propagation. For `case`: the C2 report now names the freed node, the
**mint** site (`minted_at`, where the dangling reference was born), and the
dangling-read site (`read_at`). The mint line is recorded on the allocation by
`note_mint` at `ensure` time (a dedicated lock-free `Record.mint` field that
survives `poison`) and surfaced end to end by `corpus/heap-mint`
(`fe-c-heap-mint`): a `Box` freed while a field reference into it is held aborts
`UseAfterFree` naming `minted_at=35 read_at=40` in `case` (through names
`minted_at`). It does **not** surface on `lru-0130` because lru's reborrow is
inside the uninstrumented crate; instrumenting lru does not help (its
optimized-MIR reborrow does not resolve the node record), so `heap-mint` is the
demonstrator. The one remaining site is the **free** line, deferred by
correctness: a CFG-derived `freed_at` would name the innocent `eprintln!`
sitting between the `pop()` and the read, so a precise free line needs the
deferred `nofree` callgraph.

**Instrumentation points landed.** point 0 (raw deref), point 1 (raw→safe cast
`ensure` — `corpus/cast-oob` aborts OutOfBounds in both modes for **both** a
whole-object `&*bad` reborrow and a **field reborrow** `&(*p).b` off the end,
the spatial check that makes `case` elision sound; field reborrows were added
to close a `case` gap — an unvetted `&(*p).f` off a mis-cast base would have had
its later derefs elided, a missing visit under I1, and the instrumented
hashbrown suite stays clean at 5.1M checks), point 4 (dealloc-reachable
re-check, `case`), point 5 (stack scope hooks, I8), and I9 outbound escape are
all in.
The `differential` gate (C3) is wired and passing (`fe-c-differential`). Not
yet: point 2 (`through` covers loaded pointers via safe-deref checking;
`case`'s "load from memory" variant is subsumed by point 1 for now), point 3a
(FFI inbound prologue), and the `through` performance layer (T2 shadow slots).

All 19 fe-c flake checks are green: `fmt`, `clippy`, `unit`, `miri`,
`interpose`, `census`, `provenance`, `instrument`, `corpus-smallvec`,
`false-positive`, `corpus-stackuaf`, `ffi-escape`, `closure-escape`,
`through-safe-ref`, `rusqlite-0128`, `lru-0130`, `cast-oob`, `heap-mint`,
`differential`.

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
| **B5** | I8 stack scopes + I9 FFI escape; **catches the real RUSTSEC-2021-0128** | `fe-c-rusqlite-0128`: real unmodified `rusqlite@=0.25.3` + bundled SQLite aborts `UseAfterScopeExit` under `FEC_MODE=through`, naming the dead scope + `create_scalar_function` registration site; case-like elides. Plus `stack-uaf`/`ffi-escape`/`closure-escape`/`through-safe-ref` |

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

## B5 — the `corpus-rusqlite-0128` arc (done)

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
site.

**The exact `corpus-rusqlite-0128` is done** (`fe-c-rusqlite-0128`): real,
unmodified `rusqlite@=0.25.3` + bundled SQLite, built under `FEC_MODE=through`,
aborts `UseAfterScopeExit` on the closure's safe-reference read of the dropped
local, naming the dead scope and `escaped_at` the `create_scalar_function`
registration site; case-like mode elides the safe deref and runs clean. The
`§3.2` pointer-through-a-safe-reference gap that used to block it is closed by
through's safe-deref checking (C1). rusqlite 0.25.3 is yanked, so its lock
entry is hand-completed; the CDN still serves the `.crate`, so the pure offline
fetchurl vendor works. The only optional follow-on is:

1. *(nicety)* the **inbound `extern "C"` prologue check** (`ensure_foreign_arg`,
   trace step 4): validate safe-pointer params on the way in. Not needed for
   the abort (which fires at the callback's dereference), but completes the
   both-directions I9 story.

Also address-reuse soundness: a poisoned stack region that is not
re-registered by the next frame can produce a stale resolve. `scope_enter`
already replaces a dead region at the same base; a fuller fix ties poisoning
to the escape (only escaped regions stay findable-as-dead).

## Open items carried forward

- **point 0 checks the projected element's *start*, not its full extent.**
  point 0 now faults on the **accessed** address for a simple projected place
  (`&raw const (*p).f` = `p + offset(f)`, via `is_simple_projected_place` +
  `raw_const_place_as_u8`), so a **direct** projected access through a mis-cast
  base (`base.add(k) as *const Struct` then `(*p).f`, no reborrow, no
  `ptr::write`) is caught — the `cast-oob direct` scenario aborts `OutOfBounds`
  in both modes, and the false-positive suite stays clean (5.1M hashbrown
  checks). The **residual** is narrow: a field that *starts* in bounds but
  *extends* past the end (a subobject wider than the remaining allocation).
  Closing it needs the full extent (`fault + size`), which the single-address
  `deref_rooted` does not compare; the fix is a `__fec_check_extent` (the
  `ensure` extent logic without the mint recording) routed for projected
  derefs. Deferred as low-value (only bites a mis-cast base whose first field is
  in bounds) and to keep `deref_rooted`'s arity stable across the smallvec /
  rusqlite / stack asserts.
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

## How to resume (finishing `through` mode, then `case`)

1. Read this file, then `CLAUDE.md` §3–§4 and `docs/both-modes.md` (the `through`
   column is the spec for what's left).
2. **C1 is decided (through-first) and its defining behavior is in** (safe-deref
   checking under `FEC_MODE=through`, `corpus/through-safe-ref`). To finish
   `through` end to end, work the remaining `through` column rows: at-rest cap
   propagation (T2 shadow slots — the 128-bit coherent pair, layout already
   fixed in `docs/through-mode-coherence.md`), interprocedural propagation, then
   `strict` unknown-provenance (do this **last** — it is only sound once caps
   exist everywhere, else it false-positives on foreign statics). The both-modes
   rule (I4) binds: keep `docs/both-modes.md` filled as each lands.
3. The exact `corpus-rusqlite-0128` is **done** (`fe-c-rusqlite-0128`) — Fe-C
   catches the real CVE in real `rusqlite@=0.25.3` + bundled SQLite under
   `FEC_MODE=through`. (Vendoring a yanked crate: hand-complete its `Cargo.lock`
   entry — cargo builds a yanked version when it is already fully locked, and
   the CDN still serves the `.crate` for fetchurl. See `corpus/rusqlite-0128`.)
4. **C2** (dealloc-reachable re-checks, `case`-only) and **C3** (`case` + the
   `differential` gate against `through`) are the `case` milestone — they need
   `case` mode, which is the second milestone after `through` is complete.
