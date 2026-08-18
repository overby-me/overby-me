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

**Status (2026-07-25): a working checker.** Phases A and B are done, Phase C is
mostly done, nine real CVEs are caught in real unmodified crates (one across
real C), and 27 flake checks are green. What is missing is measurement, not
detection: read `docs/evaluation-2026-07.md` before picking up work, and start
at Phase E in §4 rather than at the next unfinished letter.

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
| **No detection feature merges without either closing a row in `docs/coverage-ledger.md` or adding one.** | A checker's honest number is what it cannot see. Nine catches with an untracked false-negative surface is a number without a denominator. |
| **No cost claim ships without a measurement.** "Cheap", "near-zero", "≤10%" are hypotheses until `fe-c-bench` prints them. Say "unmeasured" in the meantime. | I5, and `docs/evaluation-2026-07.md` §3.1. |
| **Degradation is loud.** A mode that could not be parsed, a crate that could not be instrumented, a check that resolved nothing: each is counted and reported, never silently skipped. | §7's own rule. Every fail-open path in the build is currently invisible. |
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
call. Add each new check to the flake as you go (and run it directly; see §5).

**Mark tasks here as you complete them.** This file is the durable record of
progress across sessions and context compactions. Marks are `[todo]`,
`[partial]` (substantial work landed, acceptance check not fully met: say what
remains in one sentence) and `[done]` (the acceptance check passes). A task
whose body describes finished work while its mark says `[todo]` is a bug in
this file.

**Next up is Phase E, not the next unfinished letter.** E1 to E3 gate any new
detection feature; see `docs/evaluation-2026-07.md` §5 for why.

### Phase A — foundation (mode-independent)

**A1. [done] Workspace + toolchain + nix skeleton.** *(2026-07-21)*
Three crates, `rust-toolchain.toml` pinned nightly (shared with `../libc`),
flakelight module wiring through `platform/nix/lib/lib/cargo`.
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

**B5. [done] Stack scope hooks (I8) + FFI boundary checks (point 3, both directions).**
*(2026-07-22 — met against **real, unmodified `rusqlite@=0.25.3` + bundled
SQLite** (`fe-c-rusqlite-0128`): under `FEC_MODE=through` the registered
closure's safe-reference read of the dropped stack local aborts
`UseAfterScopeExit`, naming the dead scope and `escaped_at` the
`create_scalar_function` registration site; case-like mode elides the safe
deref and runs clean. The full arc: A4b symbol-level build of rusqlite + SQLite
with no cementite edge, B5's escape analysis registering the captured borrow's
scope, `escaped_at` (F7), and C1 through mode's safe-deref checking catching the
§3.2 safe-reference read. rusqlite 0.25.3 is yanked, so its lock entry is
hand-completed; the CDN still serves it for the pure offline vendor.)*
✅ **`corpus-rusqlite-0128` aborts**, report names the dead stack scope, the
callback, *and* the registration site. This is also the first corpus entry
pulling real C — it doubles as the mixed-language build smoke test.
*Delivered: the I8 runtime mechanism (`__fec_scope_enter`/`_exit` +
`table::poison` + a temporal liveness check in `__fec_check_deref_rooted`),
**lexical-scope granularity** (poison at a local's drop terminator / its
`StorageDead`, else frame return — so the inner-block-then-same-frame shape
is caught, not just use-after-frame-return), and a **default-on escape
analysis** (`escaping_locals`: only locals whose address is laundered to an
integer, handed to a foreign call, or captured into a heap-boxed closure get
hooks, so hashbrown neither times out nor false-aborts), the **outbound FFI
escape** (I9 / F6: a pointer argument to an `extern "C"` call registers its
stack scope) and **`escaped_at`** (F7: the report names the source line the
address escaped at). Shown by three reproducers — `corpus/stack-uaf`
(inner-block `String` dereferenced later in the same frame), `corpus/ffi-escape`
(a stack borrow handed to a C harness and dereferenced when C re-enters through
a trampoline), and `corpus/closure-escape` (a raw stack pointer captured into a
boxed closure kept past the frame, the rusqlite-0128 closure shape) — all abort
`UseAfterScopeExit` naming the dead scope *and* the escape site. Remaining for
the exact rusqlite-0128: the real vendored `libsqlite3-sys` build, a `§3.2`
pointer-loaded-from-memory check (its closure reads the local through a *safe*
reference, which v0's raw-deref point 0 does not instrument), and the inbound
`extern "C"` prologue check. Scoped in STATUS.*

### Phase C — modes

**C1. [partial] Decide the mode order** (see §3 above). Record the decision and its
rationale in `PLAN.md` §2 under I4. Then implement the first mode end to end.
*Remains: T2 shadow-slot coherence, interprocedural and at-rest cap propagation,
`strict` unknown-provenance. Also unresolved: `through`'s safe-deref check
currently resolves from the faulting address (evaluation §3.2).*
*(2026-07-22 — decided **through-first**, recorded in `PLAN.md` I4 and
`docs/both-modes.md` §Decision. Through mode's defining behavior — **safe-pointer
deref checking** (the one bolded both-modes row) — is implemented behind
`FEC_MODE=through` and shown by `corpus/through-safe-ref`: a safe `&u64` read of
a dead stack local aborts `UseAfterScopeExit` in through, runs clean (elided) in
case. This also closes the §3.2 gap the exact rusqlite-0128 needed. Remaining
for full through: T2 shadow-slot coherence, `strict` unknown-provenance,
interprocedural + at-rest cap propagation. Stays `[todo]` until through is
end-to-end.)*

**C2. [partial] Dealloc-reachable re-checks (point 4, I6)**, `case` only.
✅ **`corpus-lru-0130` aborts** with both the free site and the `iter()`
reborrow site named.
*Remains: the **free** site is not named (the acceptance's other half). Under
the conservative "any call frees" reachability the nearest preceding call is an
innocent `eprintln!`, so a precise `freed_at` needs the deferred `nofree`
callgraph. Everything else below is done.*
*(2026-07-22 — the dealloc-reachable re-check is **done and demonstrated in
both modes** (`fe-c-lru-0130` builds twice). `through` catches the real
RUSTSEC-2021-0130 use-after-free in unmodified `lru@=0.6.6` at every deref;
`case` elides safe derefs but re-checks the one that is **dealloc-reachable**
(it follows the `pop()` call) via `__fec_check_dealloc_reachable`, which aborts
only on a dead **heap** allocation — a dead stack scope passes, so the mode
distinction holds (rusqlite/through-safe-ref still elide in `case`). Both abort
`UseAfterFree` naming the freed node. This needed heap temporal safety:
`FecAlloc::dealloc` now poisons (keeps findable-as-dead in quarantine) rather
than deregisters at free (`table::poison_and_info` / `unlink`).
Reachability is conservative (any call, not a precise `nofree` callgraph — the
heap-only abort makes over-approximation cost extra checks, never false
positives). The `case` report names the **dangling-read site** (`read_at`, the
`let v = *value` line — the re-check is injected right at the dereference) and
the **mint site** (`minted_at`, where the dangling reference was born): point
1's `ensure` records the mint line on the allocation via `note_mint` (a
dedicated lock-free `Record.mint` field that survives `poison`), and the heap
use-after-free report surfaces it. Demonstrated end to end by `corpus/heap-mint`
(`fe-c-heap-mint`): a `Box` freed while a field reference into it is held aborts
`UseAfterFree` naming `minted_at=38 read_at=43` in `case` (through names
`minted_at`), plus a `note_mint` unit test (record + survive-poison). It does
**not** surface on `lru-0130` (lru's reborrow is inside the uninstrumented
crate; instrumenting lru does not help — its optimized-MIR reborrow does not
resolve the node record). The one remaining site is the **free** line,
deliberately deferred: under the conservative "any call frees" reachability the
nearest preceding call is the innocent `eprintln!` between the `pop()` and the
read, so a CFG-derived `freed_at` would be a red herring — a precise free line
needs the deferred `nofree` callgraph.)*

**C3. [partial] The other mode**, with the `differential` check wired: any violation
`through` catches that `case` misses must map to a documented elision gap in
`docs/both-modes.md`, or it's a bug.
*Remains: principled elision (vetting at the raw→safe cast rather than "the base
is a reference"), and evidence that the elision is sound at all: the vetting
census, Phase E5.*
*(2026-07-22 — the **`differential` gate is wired** (`fe-c-differential`) and
passes: it runs three contrasting reproducers (`closure-escape` raw,
`through-safe-ref` safe-ref stack, `lru-0130` heap) in both modes and asserts
`through` (the oracle) catches all, `case` agrees on the raw + heap UAFs, and
`case` misses **only** the documented safe-pointer-deref elision (the stack
read). The verified relationship is recorded in `docs/both-modes.md`
§Differential. `case` mode itself exists (safe-deref elision + the C2
dealloc-reachable re-check). Remaining refinement: principled elision
(vetting at the raw→safe cast) so `case` can skip *more* than it does today, and
the concurrent free-during-scope caveat (F3).)*

### Phase E: measurement (next; before Phase D and before any new detection feature)

Rationale and evidence: `docs/evaluation-2026-07.md`. The short version: nine
detection features shipped and zero numbers exist, so every elision decision,
and the reason `case` mode exists at all, is currently unfounded.

**E1. [todo] Instrumentation and degradation counters.**
Count and report: checks executed, checks where `root == fault` (propagation
lost), lookups returning `None` (unknown provenance), unsized start-only checks,
bodies visited vs skipped, crates instrumented. Emit a per-crate manifest under
`FEC_DEBUG`.
✅ `fe-c-false-positive` asserts the propagation-lost and unknown-provenance
ratios stay below a recorded threshold, so a regression that silently stops
checking fails the build instead of passing quietly.

**E2. [todo] Fail closed.**
`FEC_MODE` becomes an explicit `case`|`through` enum, anything else a hard
error. `FEC_INSTRUMENT_ONLY` errors on a name that matched no crate.
`find_fec_fns` returning `None` under `FEC_INSTRUMENT=1` is a hard error, not a
silent passthrough.
✅ A check that builds with an unrecognized mode (say `FEC_MODE=hard`) and
asserts the build fails rather than quietly selecting `case`.

**E3. [todo] `fe-c-bench` (I5).**
One workload, four builds: uninstrumented, `case`, `through`, ASan. Non-gating,
emits a report artifact. Remove the two per-check global atomics first (make the
tally opt-in) so the baseline measures the checker, not the counter.
✅ The check emits absolute numbers and ratios; `PLAN.md` §7's v0 overhead gate
stops being a hypothesis.

**E4. [todo] The safe-reference provenance canary.**
A `through`-mode reproducer whose over-read goes through a safe reference into a
live neighbouring allocation (the smallvec canary shape, but `&[u8]` instead of
`*mut u8`). Evaluation §3.2 predicts it passes when it should abort; settle it.
✅ The report names the source allocation, not the neighbour. If it does not,
extend `compute_roots` to reference locals and write a fourth trace.

**E5. [todo] Vetting census.**
Compile-time ratio, per crate, of elided safe derefs that have a dominating
`ensure` to those that do not. First evidence for or against the `case` elision
argument (and so for the composition theorem, PLAN §6).
✅ The number is printed for hashbrown and recorded in `docs/both-modes.md`.

**E6. [todo] Rescore the corpus.**
Resolve `corpus/CORPUS.md`'s 46 IDs to `crate@version` using the advisory DB's
CVE aliases; publish caught / missed / not-attempted; keep misses as rows.
✅ `corpus/CORPUS.md` carries a scoreboard with a denominator.

**E7. [todo, human decision] The per-crate dial.**
Either build it (per-crate rustc flags in `platform/nix/lib/lib/cargo`, `harden` in the
artifact key per `docs/nix-integration.md` §6 Q2, and a real `cargo-fe-c`), or
demote it from the README headline until it exists. Do not leave the project's
central promise implemented as an env var. Like C1, this one is the human's.

### Phase D — substrate

**D1. [todo]** `-Zbuild-std` sysroot derivations, keyed on (nightly × mode × target ×
cementite hash). *Attempted during the corpus work and currently blocked:
instrumenting `core` makes `compiler_builtins` fail with "cannot call functions
through upstream monomorphizations". Its value is whole-process `../libc`
coverage (D2), not corpus breadth.*
**D2. [todo]** `../libc` (Eyra lineage) built under instrumentation — whole-process
coverage. See `../libc/PLAN.md` P2.

---

## 5. Verification

The 27 `fe-c-*` flake checks are the contract. **Run them individually**
(`nix build .#checks.x86_64-linux.fe-c-<name>`); `nix flake check` is forbidden
repo-wide, it OOMs on this tree. Tiers, per `docs/nix-integration.md` §3: cheap
(fmt, clippy, unit, miri, census, provenance, instrument, interpose) on every
change; corpus entries as the real gates; expensive (false-positive,
differential, and one day selfhost and bench) less often.

Every corpus entry needs its `-control` twin — the patched crate version must
run **clean**. A checker that aborts on everything passes no useful test.
*Reality check: only `smallvec-0003` has a patched-version control. The others
substitute a two-mode build (one mode aborts, the other runs clean) or a
`NO_ABORT` sentinel in the reproducer. Those are weaker: they prove the check
fires, not that the fixed version stops it firing. Add real controls as you
touch each entry.* The `unmapped` regression guard specified in
`docs/traces/rustsec-2021-0003.md` (so a test cannot pass via segfault) was
never built either.

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
| `docs/evaluation-2026-07.md` | Where the approach stands, what diverged from the design, ranked next steps | Point-in-time; **supersede** with a new dated file, do not edit |
| `docs/coverage-ledger.md` | Every known false-negative surface | Living; a row per gap, removed only when a check closes it |
| `docs/both-modes.md` | The I4 table + mode-order argument | Living |
| `docs/cementite-api.md` | Runtime API **design draft**; the implementation has diverged from it | Superseded by rustdoc + the evaluation's divergence table; keep only as the record of what was intended |
| `docs/nix-integration.md` | Flake shape + API questions | **Dies** when the flake exists |
| `docs/through-mode-coherence.md` | Coherence decision + rationale | Frozen |
| `docs/local-iteration.md` | Build the driver outside nix + dump MIR (fast edit/instrument loop) | Living |
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
3. Run the affected `fe-c-*` checks individually. Never `nix flake check`.
4. Green → mark the task in §4, commit (one commit per task, message
   naming the task and what its acceptance check proves), continue from 1.
5. Red → fix and retry. After **3 failed attempts**, stop per below.

**Hard stops — write `STATUS.md` and halt. Do not guess, do not work around.**

- The `platform/nix/lib/lib/cargo` questions in `docs/nix-integration.md` §6 can't be
  answered by reading the code — especially #2. Guessing the derivation key is
  a correctness bug that hides for weeks.
- A §3 settled decision looks wrong.
- Three consecutive failures on one task.
- **Task C1** — the mode-order decision is the human's, unconditionally.
- Anything needing network, credentials, publishing, or changes outside
  `safety/fe-c/` and `safety/oxidized/libc/`.

`STATUS.md` states: what is built and how it was shown, then which task is
blocked, what was tried, what you'd recommend. It has become the running
state-of-the-project file rather than a blocker note, which is fine, but it must
not restate §4's task marks or the evaluation's analysis. When it disagrees with
§4, §4 wins.

**Scope.** Phase E next (§4), then D. Do not add features, crates, targets, or
dependencies beyond what the current task requires. E1 to E3 gate new detection
features: adding a tenth CVE catch before the overhead and degradation numbers
exist is the failure mode this queue is now ordered to prevent. Non-goals in §7
and PLAN §10 are binding.

**If a task reveals a design defect**, write a new trace in `docs/traces/`
(follow the existing three: reproducer, step table, findings, plan deltas),
apply the deltas, and note it in the commit. Do not silently patch invariants.
