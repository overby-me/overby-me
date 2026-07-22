# Fe-C — agent brief

**Point an agent at this file.** Read this, then `PLAN.md`. Everything else is
reference material, linked from the specific place it matters.

Rename to `AGENTS.md` if you prefer tool-neutral naming; if the monorepo root
already has a `CLAUDE.md`, this one is additive, not a replacement — follow
root conventions (nix-first, jj, flakelight, formatter choice) over anything
inferred here.

---

## 1. What this is

Fe-C adds runtime memory-safety checking to unsafe Rust and FFI boundaries,
with a **per-crate hardening dial**:

- `--harden=case` — check raw-pointer accesses and safe/unsafe boundaries;
  elide checks the type system already proves. Cheap.
- `--harden=through` — check every access via opaque runtime calls, so the
  optimizer cannot exploit assumptions the runtime doesn't enforce. The
  Fil-C-grade mode.

Three crates, one workspace, **no compiler fork, no LLVM linkage, no
submodules**: `cargo-fe-c` (subcommand + `RUSTC_WRAPPER`), `fe-c-driver`
(rustc-as-a-library, MIR analysis + rewriting), `cementite` (the runtime that
ships in the binary).

**Status: design complete, zero code written.** Your job starts at Task A1.

---

## 2. Hard rules

Violating these is a correctness bug, not a style disagreement. Each traces to
an invariant in `PLAN.md` §2.

| Rule | Why |
| ---- | --- |
| **Never resolve a check from the faulting address.** Capabilities are resolved at derivation roots, propagated through pointer arithmetic, compared at the deref. | I10. An overflow into an adjacent *live* allocation resolves valid and passes. This is a real missed CVE — `docs/traces/rustsec-2021-0003.md` §F10. |
| **`free` clears the liveness bit before releasing memory**, and freed addresses go through quarantine. | I7. Otherwise a reused address presents a valid capability (ABA). |
| **The pass visits every access from the first commit**; elision is a policy decision on top, never a missing visit. | I1. The additive framing already caused one false negative — §F9 of the same trace. |
| **No feature merges until both columns of `docs/both-modes.md` are filled.** | I4. |
| **Never fork rustc, vendor llvm-project, or add an LLVM pass.** MIR-level instrumentation needs none of it. | The whole project thesis. Forks rot in a year. |
| **Everything builds and tests through nix.** No second execution path. | Repo convention. |
| **Benchmark against ASan and Fil-C, never against our own fast mode.** | I5. |
| Sensitive strings in reports name *both* the free site and the site that minted the reference. | Debuggability is the pitch; see trace `-0130`. |

---

## 3. Settled — do not re-open

These were decided deliberately. If you think one is wrong, say so explicitly
and stop; do not quietly design around it.

- **MIR-level instrumentation**, not LLVM. Checks are ordinary calls into
  `cementite`.
- **`AllocId` is 48 bits; there is no epoch counter** — liveness is a bitmap
  indexed by id. (32-bit ids exhaust in ~1h at 1M allocs/sec.)
  `docs/through-mode-coherence.md`.
- **Three-tier coherence**: in-flight register pair / at-rest shadow slot /
  atomic 128-bit pair. Same doc.
- **Unknown-provenance default is `strict-stack`** — unknown heap/static
  provenance tolerated, dead-stack-region resolution always fatal.
  `docs/traces/rustsec-2021-0128.md` §F5.
- **Dealloc-reachable re-checks are required in v0**, not optional.
  `docs/traces/rustsec-2021-0130.md` §F1.
- **libc interposition, not libc instrumentation.** `#[no_mangle] extern "C"`
  Rust forwarding via `dlsym(RTLD_NEXT)`.
- **Naming**: `fil` is a reserved alias for `through`, unusable until the
  guarantee is earned. Don't use it in code or docs yet.
- **`cementite` is freestanding** — zero dependencies, `#![no_std]` where
  possible, direct syscalls. Not a preference: a runtime linked into every
  binary cannot depend on crates it may be instrumenting, and `-Zbuild-std`
  can't route through a Cargo dependency edge. PLAN I11.
- **Injection is by symbol, not by dependency edge.** Instrumented crates emit
  calls to `extern "C"` symbols (`__fec_*`); `cementite` is linked once into
  the final binary and resolves them. No Cargo dependency is added to any
  instrumented crate. This is the ASan model, and it is the only shape that
  works for `core`/`alloc` under `-Zbuild-std` (Task D1).

**Open, and yours to decide when you reach it:** whether to build `through`
before `case`. `docs/both-modes.md` §Finding argues for through-first
(it needs less machinery and is the oracle for differential-testing `case`);
§3–§5 of PLAN assume the reverse. Tasks A1–B5 are mode-independent, so this
does not block you until Task C1.

---

## 4. Task queue

Work in order. Each task: acceptance is a check that passes, not a judgment
call. Add each new check to `nix flake check` as you go.

**Mark tasks `[done]` here as you complete them** — this file is the durable
record of progress across sessions and context compactions.

### Phase A — foundation (mode-independent)

**A1. [done] Workspace + toolchain + nix skeleton.** *(2026-07-21)*
Three crates, `rust-toolchain.toml` pinned nightly (shared with `../libc`),
flakelight module wiring through `nix/lib/cargo`.
*Before writing any nix*, answer the six API questions in
`docs/nix-integration.md` §6 — especially #2 (is the per-crate derivation key
user-extensible? `harden` must enter the hash or mode flips silently reuse
stale artifacts).
✅ `nix build .#cementite` succeeds; `nix develop` provides nightly +
`rustc-dev` + `rust-src` + `miri`; `nix flake check` runs fmt/clippy.

**A2. [done] `cementite` core data structures.** *(2026-07-22; measured
root-resolve: 4.6 ns spanning-alloc interior, 8.3 ns small-alloc page,
19.6 ns dense-page overflow chain, 8.2 ns miss)*
`AllocId` (48-bit), `CapFlags`, `Cap` (unpacked), `PackedCap`, page-radix
allocation table, id-indexed liveness bitmap. No allocator yet.
See `docs/cementite-api.md`.
✅ Unit tests; Miri-clean; a criterion bench reporting root-resolve cost
(reference point: ~2 ns for a dense-table lookup, warm cache, single thread).

**A3. [done] Allocator.** *(2026-07-22)*
`FecAlloc` as `#[global_allocator]`; register/deregister; quarantine with a
byte budget and FIFO eviction.
✅ Liveness bit provably cleared before memory release (I7 — write the test
that fails if the order is swapped); quarantine stays inside its budget under
a churn stress test.

**A4. [done] libc interposition, tier 1.** *(2026-07-22)*
`malloc`/`calloc`/`realloc`/`free`/`posix_memalign` via `dlsym(RTLD_NEXT)`.
✅ A small C harness's allocations appear in the table with correct bounds;
`strdup`-style libc-internal allocations too.

**A4b. [done] `cargo-fe-c` orchestration.** *(2026-07-22)*
`RUSTC_WRAPPER` that instruments the whole dependency graph and links
`cementite` once into the final binary. **Symbol-level injection only** — no
Cargo dependency edge into instrumented crates (see §3). Was missing from the
original queue; A1–A4 cannot reach third-party crates without it.
✅ A binary with third-party dependencies builds with every crate
instrumented; `cargo tree` on those crates is unchanged; `nm` shows
`__fec_*` resolved once in the final artifact.
*Verified on the `smallvec-0003` corpus binary: whole-graph instrumented,
`cargo tree` shows no cementite edge under `smallvec`, and `nm` shows
`__fec_check_deref_rooted` defined exactly once. The driver never instruments
cementite, build scripts (`build_script_*`) or proc-macros; cementite is
freestanding (I11), so its `build.rs` invokes `cc`/`ar` directly and no
build-toolchain crate enters a dependent's graph. A leaf binary that installs
`FecAlloc` takes cementite as an ordinary path dependency.*

**A5. [done] Driver skeleton + visitation census.** *(2026-07-22; runs on
the full serde tree — 13 crates, skipped_bodies=0 everywhere)*
`fe-c-driver` on `rustc_public`; enumerate MIR bodies; emit a report of every
pointer-typed local, every deref, every raw→safe cast, every FFI edge.
**No rewriting yet.**
✅ Runs on hello-world and on `serde`; the census is complete (I1) — spot-check
against a hand-audited small crate.

### Phase B — first checking

**B1. [done] Capability propagation dataflow (I10).** *(2026-07-22)*
Resolve at derivation roots; propagate through offsets and projections; record
where propagation is lost.
✅ On `smallvec::insert_many`, the pass identifies the derivation root
(`as_mut_ptr()`) and propagates to the overflowing write. *Verified on real
`smallvec@=1.6.0`: `insert_many rooted_writes=4 write_roots=["as_mut_ptr"]`.*

**B2. [done] MIR rewriting infrastructure.** *(2026-07-22)*
Insert calls to `cementite`; thread the returned pointer as a distinct SSA
value.
✅ Instrumented hello-world runs and reports non-zero check counts.
*Done via `rustc_driver::Callbacks` + `override_queries` wrapping
`optimized_mir`: clones each local body, splits blocks before raw derefs,
injects `cementite::__fec_check_deref(ptr)` (resolved by path). The harness
reports 3 checks fired, program output unchanged, control clean.*

**B3. [done] Raw-deref checking (instrumentation point 0).** *(2026-07-22)*
✅ **`corpus-smallvec-0003` aborts**, and the report names the SmallVec
allocation — *not* the neighbouring `String`. That mis-attribution is the I10
regression canary; assert on it explicitly.
*Verified on real `smallvec@=1.6.0`: the check resolves the capability from
the `as_mut_ptr()` root (a 1-byte spilled buffer), the overflowing
`ptr::write`/`ptr::copy` traps, and the report names that buffer, not the
neighbouring String. Patched `1.6.1` runs clean (259 checks, no false
positive). The `fe-c-corpus-smallvec` check builds both offline; smallvec is
instrumented via symbol-level `__fec_*` injection (no cementite dependency
edge, A4b), and cementite is linked once into the final binary.*

**B4. [done] Cast checks (point 1) + `ensure`.** *(2026-07-22)*
✅ `false-positive` check green: `serde`, `regex`, `hashbrown` own test suites
pass instrumented.
*Instrumented and run clean: `hashbrown@0.14.5` own suite (98 tests) and a
907k-check SwissTable workload; `regex-automata@0.4.16` own suite (204
tests) — the two most raw-pointer-heavy crates, zero false traps. Cast
sites (`&*p`/`&mut *p` reborrows) are indirect places already covered by
the rooted deref check at every access; elision (the design's
`ensure`-returns-vetted-pointer) is deferred, and checking every access is
sound (I1). Two orchestration changes made this possible: cementite is now
dependency-free (raw mmap syscall, no rustix/libc/bitflags to reconcile
across build graphs) and `FEC_INSTRUMENT_ONLY` scopes instrumentation to a
crate list so deep dep trees don't need cementite as a sysroot crate (D1).
The `fe-c-false-positive` check runs the hashbrown workload offline.*

**B5. [todo] Stack scope hooks (I8) + FFI boundary checks (point 3, both directions).**
*(partial 2026-07-22 — I8 mechanism done + demonstrated; the exact
`corpus-rusqlite-0128` acceptance is not yet met, so this stays `[todo]`.
See `STATUS.md`.)*
✅ **`corpus-rusqlite-0128` aborts**, report names the dead stack scope, the
callback, *and* the registration site. This is also the first corpus entry
pulling real C — it doubles as the mixed-language build smoke test.
*Delivered: the I8 runtime mechanism (`__fec_scope_enter`/`_exit` +
`table::poison` + a temporal liveness check in `__fec_check_deref_rooted`)
and frame-granularity scope emission, demonstrated by `corpus/stack-uaf`:
an escaped stack pointer dereferenced after its frame returns aborts
`UseAfterScopeExit` naming the dead scope (new `fe-c-corpus-stackuaf`
check). Gated behind `FEC_SCOPE_HOOKS` — instrumenting every address-taken
local is impractical without an escape analysis (hashbrown times out).
Remaining for the exact rusqlite-0128: an escape analysis (so it can be
default-on), lexical-scope granularity (optimized MIR strips
`StorageLive`/`Dead`, so the inner-block scope needs a pre-optimization MIR
hook), the FFI inbound/outbound checks (I9), and the real `libsqlite3-sys`
build. Scoped in STATUS.*

### Phase C — modes

**C1. [todo] Decide the mode order** (see §3 above). Record the decision and its
rationale in `PLAN.md` §2 under I4. Then implement the first mode end to end.

**C2. [todo] Dealloc-reachable re-checks (point 4, I6)** — `case` only.
✅ **`corpus-lru-0130` aborts** with both the free site and the `iter()`
reborrow site named.

**C3. [todo] The other mode**, with the `differential` check wired: any violation
`through` catches that `case` misses must map to a documented elision gap in
`docs/both-modes.md`, or it's a bug.

### Phase D — substrate

**D1. [todo]** `-Zbuild-std` sysroot derivations, keyed on (nightly × mode × target ×
cementite hash).
**D2. [todo]** `../libc` (Eyra lineage) built under instrumentation — whole-process
coverage. See `../libc/PLAN.md` P2.

---

## 5. Verification

`nix flake check` is the contract. Tiers, per `docs/nix-integration.md` §3:
cheap (fmt, clippy, unit, ui, miri-runtime) on every change; corpus
(`lru-0130`, `rusqlite-0128`, `smallvec-0003`, each with a patched-version
control) as the real gates; expensive (false-positive, selfhost, differential,
bench) nightly.

Every corpus entry needs its `-control` twin — the patched crate version must
run **clean**. A checker that aborts on everything passes no useful test.

The three traced entries above are the *worked* ones; `corpus/CORPUS.md` holds
the full 46-entry acceptance table with per-row gate marks (required /
stretch / known-hard). Resolve each ID to `crate@version` + reproducer as you
build out `checks.corpus-rustsec`. Its note 5 is useful early: much of the
corpus is reachable through allocator and `mem*` interposition alone, so
Task A4 can start catching real CVEs before the driver exists.

---

## 6. Map

| File | What it is | Lifecycle |
| ---- | ---------- | --------- |
| `CLAUDE.md` (this) | Rules + task queue | Update as tasks complete |
| `PLAN.md` | Invariants, phases, gates, open questions | Living |
| `README.md` | Human-facing; threat model | Living |
| `docs/both-modes.md` | The I4 table + mode-order argument | Living |
| `docs/cementite-api.md` | Runtime API draft | **Dies** when rustdoc exists |
| `docs/nix-integration.md` | Flake shape + API questions | **Dies** when the flake exists |
| `docs/through-mode-coherence.md` | Coherence decision + rationale | Frozen |
| `docs/traces/*.md` | CVE traces: design evidence + test specs | Frozen, dated |
| `corpus/CORPUS.md` | The 46-entry acceptance corpus (SafeFFI Table 1), with per-row v0 gate marks | Living — resolve IDs to pinned versions as you go |

The traces are the reason invariants I6–I10 exist — each found a real design
defect before any code was written. Cite them; don't rewrite them. If you find
a new defect, **write a fourth trace** rather than silently patching PLAN.

---

## 7. When unsure

State the ambiguity and pick the conservative option: **more checks, fewer
elisions, louder failures**. A false positive is a bug report; a false
negative is a CVE that shipped.

Do not add scope. Explicit non-goals (PLAN §10): macOS/Windows, dynamic
linking, aliasing-model checking (that's Miri — run it alongside, don't
reimplement it), production containment claims, LLVM passes.

---

## 8. Autonomous session protocol

When running unattended, follow this exactly.

**Assume you will lose context.** Compaction will happen mid-run. The repo is
the only durable state: task marks in §4, blockers in `STATUS.md`, reasoning in
commit messages. Never rely on remembering anything from earlier in the run.

**Branch.** Work on `fe-c/v0`, created off main. Never commit to main, never
rewrite published history, never force-push. Push the feature branch only.

**Loop.**

1. Read §4. Find the first task not marked `[done]`.
2. Implement it — that task only. Do not start the next one.
3. Run `nix flake check`.
4. Green → mark the task `[done]` in §4, commit (one commit per task, message
   naming the task and what its acceptance check proves), continue from 1.
5. Red → fix and retry. After **3 failed attempts**, stop per below.

**Hard stops — write `STATUS.md` and halt. Do not guess, do not work around.**

- The `nix/lib/cargo` questions in `docs/nix-integration.md` §6 can't be
  answered by reading the code — especially #2. Guessing the derivation key is
  a correctness bug that hides for weeks.
- A §3 settled decision looks wrong.
- Three consecutive failures on one task.
- **Task C1** — the mode-order decision is the human's, unconditionally.
- Anything needing network, credentials, publishing, or changes outside
  `rust/fe-c/` and `rust/libc/`.

`STATUS.md` states: which task, what was tried, what's blocking, what you'd
recommend. It is deleted when the blocker is resolved.

**Scope.** Tasks A1 → B5, then stop at C1. Do not add features, crates,
targets, or dependencies beyond what the current task requires. Non-goals in
§7 and PLAN §10 are binding.

**If a task reveals a design defect**, write a new trace in `docs/traces/`
(follow the existing three: reproducer, step table, findings, plan deltas),
apply the deltas, and note it in the commit. Do not silently patch invariants.
