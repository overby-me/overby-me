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

**Open, and yours to decide when you reach it:** whether to build `through`
before `case`. `docs/both-modes.md` §Finding argues for through-first
(it needs less machinery and is the oracle for differential-testing `case`);
§3–§5 of PLAN assume the reverse. Tasks A1–B5 are mode-independent, so this
does not block you until Task C1.

---

## 4. Task queue

Work in order. Each task: acceptance is a check that passes, not a judgment
call. Add each new check to `nix flake check` as you go.

### Phase A — foundation (mode-independent)

**A1. Workspace + toolchain + nix skeleton.** *(done 2026-07-21)*
Three crates, `rust-toolchain.toml` pinned nightly (shared with `../libc`),
flakelight module wiring through `nix/lib/cargo`.
*Before writing any nix*, answer the six API questions in
`docs/nix-integration.md` §6 — especially #2 (is the per-crate derivation key
user-extensible? `harden` must enter the hash or mode flips silently reuse
stale artifacts).
✅ `nix build .#cementite` succeeds; `nix develop` provides nightly +
`rustc-dev` + `rust-src` + `miri`; `nix flake check` runs fmt/clippy.

**A2. `cementite` core data structures.** *(done 2026-07-22; measured
root-resolve: 4.6 ns spanning-alloc interior, 8.3 ns small-alloc page,
19.6 ns dense-page overflow chain, 8.2 ns miss)*
`AllocId` (48-bit), `CapFlags`, `Cap` (unpacked), `PackedCap`, page-radix
allocation table, id-indexed liveness bitmap. No allocator yet.
See `docs/cementite-api.md`.
✅ Unit tests; Miri-clean; a criterion bench reporting root-resolve cost
(reference point: ~2 ns for a dense-table lookup, warm cache, single thread).

**A3. Allocator.** *(done 2026-07-22)*
`FecAlloc` as `#[global_allocator]`; register/deregister; quarantine with a
byte budget and FIFO eviction.
✅ Liveness bit provably cleared before memory release (I7 — write the test
that fails if the order is swapped); quarantine stays inside its budget under
a churn stress test.

**A4. libc interposition, tier 1.** *(done 2026-07-22)*
`malloc`/`calloc`/`realloc`/`free`/`posix_memalign` via `dlsym(RTLD_NEXT)`.
✅ A small C harness's allocations appear in the table with correct bounds;
`strdup`-style libc-internal allocations too.

**A5. Driver skeleton + visitation census.** *(done 2026-07-22; runs on
the full serde tree — 13 crates, skipped_bodies=0 everywhere)*
`fe-c-driver` on `rustc_public`; enumerate MIR bodies; emit a report of every
pointer-typed local, every deref, every raw→safe cast, every FFI edge.
**No rewriting yet.**
✅ Runs on hello-world and on `serde`; the census is complete (I1) — spot-check
against a hand-audited small crate.

### Phase B — first checking

**B1. Capability propagation dataflow (I10).** *(done 2026-07-22)*
Resolve at derivation roots; propagate through offsets and projections; record
where propagation is lost.
✅ On `smallvec::insert_many`, the pass identifies the derivation root
(`as_mut_ptr()`) and propagates to the overflowing write. *Verified on real
`smallvec@=1.6.0`: `insert_many rooted_writes=4 write_roots=["as_mut_ptr"]`.*

**B2. MIR rewriting infrastructure.** *(done 2026-07-22)*
Insert calls to `cementite`; thread the returned pointer as a distinct SSA
value.
✅ Instrumented hello-world runs and reports non-zero check counts.
*Done via `rustc_driver::Callbacks` + `override_queries` wrapping
`optimized_mir`: clones each local body, splits blocks before raw derefs,
injects `cementite::__fec_check_deref(ptr)` (resolved by path). The harness
reports 3 checks fired, program output unchanged, control clean.*

**B3. Raw-deref checking (instrumentation point 0).**
✅ **`corpus-smallvec-0003` aborts**, and the report names the SmallVec
allocation — *not* the neighbouring `String`. That mis-attribution is the I10
regression canary; assert on it explicitly.

**B4. Cast checks (point 1) + `ensure`.**
✅ `false-positive` check green: `serde`, `regex`, `hashbrown` own test suites
pass instrumented.

**B5. Stack scope hooks (I8) + FFI boundary checks (point 3, both directions).**
✅ **`corpus-rusqlite-0128` aborts**, report names the dead stack scope, the
callback, *and* the registration site. This is also the first corpus entry
pulling real C — it doubles as the mixed-language build smoke test.

### Phase C — modes

**C1. Decide the mode order** (see §3 above). Record the decision and its
rationale in `PLAN.md` §2 under I4. Then implement the first mode end to end.

**C2. Dealloc-reachable re-checks (point 4, I6)** — `case` only.
✅ **`corpus-lru-0130` aborts** with both the free site and the `iter()`
reborrow site named.

**C3. The other mode**, with the `differential` check wired: any violation
`through` catches that `case` misses must map to a documented elision gap in
`docs/both-modes.md`, or it's a bug.

### Phase D — substrate

**D1.** `-Zbuild-std` sysroot derivations, keyed on (nightly × mode × target ×
cementite hash).
**D2.** `../libc` (Eyra lineage) built under instrumentation — whole-process
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
