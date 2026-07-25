# Evaluation: the Fe-C approach as built (2026-07-25)

Point-in-time review of the whole approach, written against the tree at
`main` (50 commits touching `rust/fe-c`, 2026-07-21 to 2026-07-23; 5.8k lines
of Rust in three crates, 27 flake checks, nine real CVEs caught).

Supersede this file with a new dated one rather than editing it, the way
`docs/traces/` works. Its job is to say what the evidence supports, what it
does not, and what to do next.

**Verdict in one paragraph.** The engineering approach is right and is working
faster than the plan assumed: MIR rewriting through `override_queries`, with no
compiler fork, symbol-level runtime injection and a freestanding runtime, got
from zero to nine real CVEs (including one across real C, in unmodified
`rusqlite` + bundled SQLite) in three days. The research thesis is intact. What
is missing is not more detections: it is *measurement*. Every claim that
distinguishes Fe-C from "a slower ASan with narrower coverage" is currently
unmeasured, and in two places the design record asserts properties the code does
not have. The next increment of effort should go almost entirely into making
degradation visible and overhead measurable, before any new checking feature
lands.

---

## 1. What the approach gets right (do not trade these away)

| Decision | Why it is holding up |
| -------- | -------------------- |
| **No compiler fork, no LLVM pass** | The whole tree is 5.8k lines and rides one pinned nightly. The equivalent CHERI/rustc forks cost years. This was the thesis and it is validated. |
| **`override_queries` on `optimized_mir`** | Reaches third-party crates without touching their source, and MIR inlining brings `core`'s slice indexing into view for free. It is also why `-Zbuild-std` turned out *not* to be the force multiplier the plan assumed (STATUS records the correction). |
| **Symbol-level injection (A4b, the ASan model)** | Instrumented crates gain no Cargo edge, `cargo tree` is unchanged, and the runtime links once. This is the only shape that could ever work for `core`/`alloc`, and it was found early. |
| **Freestanding runtime (I11)** | Forced empirically by dependency conflicts, then recognized as a requirement. Correct, and matches ASan/`compiler-builtins` precedent. |
| **The trace methodology** | `docs/traces/` found F9 (raw derefs were missing from the design entirely) and F10 (address-to-allocation lookup is not spatial safety) *before any code existed*. F10 in particular is the error most implementations of this idea actually ship. This practice is the project's biggest asset. |
| **`through` as the oracle for `case`** | A differential gate where the exhaustive mode is the reference implementation is a genuinely strong test structure, and it is wired and passing (`fe-c-differential`). |
| **Assert hygiene** | The false-positive assert requires >10k checks to have fired, so it cannot pass vacuously (`corpus/assert_false_positive.nu`). The nushell `(exit $x)` footgun was found and fixed. Both are the instincts a checker project needs. |
| **Everything offline and pure through nix** | Vendored yanked crates, pinned versions, reproducible corpus. Nothing here rots quietly. |

---

## 2. Where the design record and the implementation have diverged

None of these are bugs in the code. They are places where a doc asserts a
property the build does not have, which is the failure mode that matters most
for a safety tool, because the docs are what a user reads to decide what the
tool guarantees.

| Designed | Built | Consequence |
| -------- | ----- | ----------- |
| **Hot path is a register compare against a propagated capability** (`docs/traces/rustsec-2021-0003.md` F10 delta 4; `docs/cementite-api.md` `check_deref(ptr, size, cap)`) | `__fec_check_deref_rooted(fault, root)` calls `table::lookup(root)` on **every** check (`crates/cementite/src/check.rs:109`). No capability is ever carried in a register. | The performance argument for `case` mode rests on machinery that does not exist. The one measurement that does exist (A2's root-resolve bench: 4.6 to 19.6 ns) is 2 to 10 times the design's own "~2 ns dense-table lookup" reference point, and in the built system that cost is per-access rather than per-root. |
| **Provenance travels; never resolve from the faulting address** (Hard Rule 1, I10) | True for raw pointers. For **safe-reference** bases, `compute_roots` computes nothing (`instrument.rs:1547` filters to `is_raw_ptr_local`), so `root` falls back to the base local itself (`instrument.rs:283`) and the runtime resolves from the address being accessed. | `through` mode's defining check (safe-pointer derefs) is structurally in the F10 shape. See §3.2: this is the single most important thing to test next. |
| **Mode is ABI, per-crate, recorded in metadata, with cross-mode adapters (I3)** | Mode is a process-global env var read at rustc invocation time (`instrument.rs:182`), matched as `Ok("through")` with everything else falling through to `case`. | The per-crate hardening dial, which is the headline feature in `README.md`, has no implementation, no test, and no user-facing surface. A typo in `FEC_MODE` silently selects the weaker mode. |
| **`cargo-fe-c`: cargo subcommand + `RUSTC_WRAPPER` that instruments the graph** | 13 lines that print "not yet implemented" and exit 2 (`crates/cargo-fe-c/src/main.rs`). All orchestration is `RUSTC=$drv` plus env vars inside `default.nix` check scripts. | There is no way to use Fe-C outside this repo's nix checks. That is fine for now, but the README describes it in the present tense. |
| **`harden` enters the per-crate derivation key** (`docs/nix-integration.md` §6 Q2, flagged as a correctness bug if guessed wrong) | Corpus checks invoke `cargo build` with `RUSTC=` and env vars in a temp `CARGO_TARGET_DIR`. `nix/lib/cargo`'s per-crate key is untouched. | Mode is not in any derivation hash. Today the checks sidestep this by using separate target dirs per mode; the moment the dial is real, this is the stale-artifact bug the plan warned about. |
| **Unknown-provenance policy is a knob (`strict` / `permissive` / `strict-stack`) with counters exposing how often each path fires** (`docs/cementite-api.md`) | Hard-coded behaviour: `table::lookup` returning `None` means "pass". No knob, no counters. | The dead-stack-is-fatal half of `strict-stack` is implemented and correct. The measurement half, which is what would tell you how often checks silently do nothing, does not exist. |
| **`ensure` returns the vetted pointer so the pass can thread a distinct SSA value** | `__fec_ensure` returns `()`. `case`'s elision is "the base local is a reference, so skip", not "this value was vetted". | The soundness argument for `case` is asserted, not tracked. See §3.4. |
| **`cementite` is `#![no_std]` where possible** | No `#![no_std]`; uses `std::sync::Mutex`, `std::io`, `format!` (`crates/cementite/src/lib.rs`). Zero *external* dependencies, which is the load-bearing half of I11. | Fine, but say "zero dependencies, std-linked" rather than implying freestanding-in-the-no_std-sense. |
| **Guard-page / canary sampling (GWP-ASan style) for opaque C** | Not implemented. | README lists it under "What it does". |
| **`ensure_foreign_arg` / `ensure_returned` (FFI inbound, point 3a)** | Not implemented (correctly tracked as open in STATUS). | README lists `extern "C"` prologue checks under "What it does". |

---

## 3. The five structural risks, ranked

### 3.1 Nothing about performance has been measured, and the architecture as built points the wrong way

There is no `fe-c-bench` check. `bench` appears in `PLAN.md` §9, in
`docs/nix-integration.md` §3's expensive tier, and in I5 ("benchmark against
ASan and Fil-C, never against our own fast mode"), and it exists nowhere in
`default.nix`. The only number in the tree is A2's microbench of
`table::lookup` in isolation.

That number is not encouraging, and no document records it as such:

- 4.6 ns spanning-alloc interior, 8.3 ns small-alloc page, 19.6 ns dense-page
  overflow chain, 8.2 ns miss.
- The design's own reference point was "~2 ns for a dense-table lookup", and
  the design assumed lookups happen **at derivation roots only**.
- As built, a lookup happens **per access**, plus two global atomic
  read-modify-writes per check: `REPORTER_REGISTERED.swap(true, Relaxed)` and
  `DEREF_CHECKS.fetch_add(1, Relaxed)` (`check.rs:60`, `check.rs:68`). The
  `swap` is a one-time-effect guard that pays an atomic RMW forever; the
  counter is a single global cache line every thread writes to on every check.
- Table mutation (`register`, `deregister`, `poison`, and therefore **every
  stack scope enter and exit**) takes one global `Mutex` (`table.rs:104`, taken
  at six sites). Quarantine takes a second one (`alloc.rs:96`).

A tool whose pitch is "near-zero cost for safe code, cheaper than ASan because
Rust already proved most of it" cannot leave this unmeasured while shipping
nine detection features. The risk is not that the numbers are bad; it is that
nobody knows, so every elision decision (which is to say, the entire reason
`case` mode exists) is currently unfounded.

Two of those costs are also a correctness signal, not just a performance one:
the global mutex in the malloc interposer means `PLAN.md` §11 Q2
(async-signal-safety) is no longer an open question but a live defect. A
`malloc` from a signal handler can deadlock against an interrupted
`register`.

### 3.2 `through` mode's defining check may resolve from the faulting address

`compute_roots` only ever populates entries for locals whose type is
`ty::RawPtr` (`instrument.rs:1547`). `DerefFinder` collects safe-reference
places when `check_safe` is set (`instrument.rs:1492`). At the injection site,
the root is `roots.get(&base_local).unwrap_or(base_local)`
(`instrument.rs:283`), so for a safe reference the root **is** the reference,
and `extent_verify` resolves `table::lookup(root)` on the address being
accessed (`check.rs:244`).

That is precisely the shape F10 says is not spatial safety. The prediction that
follows: a `through`-mode over-read through a safe reference that lands inside
an **adjacent live allocation** resolves to the neighbour's capability and
passes. The existing I10 canary (`corpus/smallvec-0003`) only exercises the raw
path, so it would not catch this.

This is a prediction from reading the code, not a measured result. It is also
cheap to falsify, and it is exactly the kind of thing this project's own trace
methodology exists to settle. Write the experiment before believing either
answer: a `through`-only reproducer shaped like the smallvec canary, where the
over-read goes through `&[u8]` rather than `*mut u8` and the victim is a live
neighbouring `String`, asserting that the report names the source allocation
and not the neighbour.

If it confirms, the fix is to extend `compute_roots` to reference locals
(a reborrow `&*p` roots at `p`'s root; a reference from a call or an argument
roots at itself, and *that* case should be counted, not silently accepted).

### 3.3 Every degradation path is silent, in a project whose stated rule is "louder failures"

`CLAUDE.md` §7 says: more checks, fewer elisions, louder failures. The build
does the opposite at every seam, and each one is invisible:

| Path | What happens | Where |
| ---- | ------------ | ----- |
| `FEC_MODE` unset or misspelled | Silently `case`, the weaker mode | `instrument.rs:182` |
| `FEC_INSTRUMENT_ONLY` names a crate that never matches | That crate is silently uninstrumented | `main.rs` scoping |
| The injected `extern "C"` decls fail to parse | `inject_fec_decls` returns, crate is silently uninstrumented | `instrument.rs:86` |
| `find_fec_fns` finds no decls | Every body returns unmodified, no diagnostic | `instrument.rs:103` |
| Provenance lost on a raw pointer | Root defaults to the pointer itself, check degrades to faulting-address resolution | `instrument.rs:283` |
| `table::lookup` returns `None` | Check passes | `check.rs:109`, `check.rs:244` |
| An unsized access | Start-only check, no extent | `instrument.rs:307` |
| `layout_of` fails for a scope local | No `scope_enter`, so no temporal check for that local | `instrument.rs:971` |
| A panic unwinds past a scope | `drop_sites` skips cleanup blocks, so the region stays registered live | `instrument.rs:1140` |

Each is individually defensible (fail-open beats false positives while
bootstrapping). Together they mean the headline results have no denominator:
"zero false positives on hashbrown at 5.1M checks" is a strong claim only if
you also know what fraction of accesses were checked at all, how many checks
resolved nothing, and how many resolved from the fault. None of those numbers
exist.

The fix is small and pays for itself immediately: a counter per degradation
path, printed beside the existing check tally at exit, plus a compile-time
manifest of which crates and bodies were actually instrumented. That turns
several of the claims in `STATUS.md` from assertions into measurements.

### 3.4 `case`-mode elision is a heuristic, not a tracked property

`case` elides a dereference because its base local has reference type
(`is_ref_local`), and justifies that with "the reference was vetted at its
raw-to-safe cast". Nothing checks that the vetting actually happened. A
reference can reach a `case` body without ever passing an instrumented `ensure`:
from an uninstrumented crate, from a function argument, from a `static`, from
`Vec::from_raw_parts` (deliberately excluded from the slice-mint check,
`instrument.rs:565`), or from `Box::from_raw`.

The designed mechanism, `ensure` returning a vetted pointer so the pass threads
a distinct SSA value, would have made this checkable. It was dropped, which is
a reasonable simplification, but the property it protected is now unmonitored.

This matters more than it looks, because `PLAN.md` §6 calls the composition
theorem "the publishable core of this project", and the theorem is a statement
about exactly this: under what conditions elision is sound. Right now there is
no evidence about it at all.

Concrete first step, entirely at compile time: a **vetting census**. For every
elided safe deref, does a dominating `ensure` on that reference exist in the
same body? Report the ratio per crate. That single number is the first
empirical evidence for or against the elision argument, and it costs one
dataflow pass over machinery the driver already has.

### 3.5 The corpus drifted from a declared gate to a self-selected one, and the docs report the numerator only

`corpus/CORPUS.md` declares a 46-entry acceptance corpus from SafeFFI Table 1,
with per-row "required / stretch / known-hard" marks, and a "to do at the
computer" step (resolve each ID to `crate@version`) that was never done.

What actually happened, and it was good work: nine CVEs were caught, chosen by
what the checker could reach. But the two sets barely overlap. Of the nine,
exactly one (RUSTSEC-2021-0003) is confirmably a declared row. One more
(CVE-2019-15551, which is very probably the alias of RUSTSEC-2019-0009,
`smallvec 0.6.9 grow`) is likely a second. The remaining 44 rows have never
been resolved, attempted, or scored, and the other seven catches
(`rusqlite-0128`, `lru-0130`, `simple-slab-0039`, `toodee-0028`,
`elf-rs-0079`, `binary-vec-io-0109`, `partial-sort-0016`) are not in the
declared corpus at all.

That is survivorship bias with a paper trail: the corpus that gates the project
was replaced, without a decision being recorded, by the corpus the tool
happens to catch. `STATUS.md` reports "Nine real CVEs" prominently and the
denominator nowhere. MEMORY already records the honest read ("the vein is mined
out; the next yield needs an alignment check or interprocedural analysis, not
more harvesting"), which is the right conclusion and belongs in the repo rather
than in an agent's memory.

The fix is not to abandon the opportunistic corpus, which is genuinely valuable.
It is to publish both: resolve the 46 IDs (the advisory DB carries the CVE
aliases), score them honestly as caught / missed / not attempted, and keep the
misses as first-class rows. For a checker, the misses are the informative data.

---

## 4. Two smaller things worth fixing while you are in there

**Detection depends on build profile, and only one profile is tested.** Every
corpus check builds with the default dev profile except `partial-sort-0016`,
whose fixture sets `opt-level = 3, debug-assertions = false` because that is
"the only shape the CVE exhibits". MEMORY records the mirror case: at debug
opt-level the slice `Index` does not inline, so `case` saw nothing until the
`from_raw_parts` mint check was added. So detection is a function of the
optimization level, and the matrix has one cell filled. At minimum, run one
representative entry per class in both debug and release, and state the
profile-dependence in the README beside the other caveats.

**`through` mode does not have the property its name promises.** The claim in
`README.md` and `docs/both-modes.md` is that accesses go through opaque runtime
calls "so the optimizer cannot exploit assumptions the runtime doesn't
enforce", and that aliasing assumptions become "inert". As built, the pass
injects a call *before* an otherwise ordinary access, at `optimized_mir`, after
rustc's MIR pipeline; LLVM still emits the load or store with `noalias`,
`dereferenceable` and `align` intact and may still exploit them. What `through`
delivers is "check before every access", which is a real and useful property,
and it is not Fil-C's. Two honest options: restate the guarantee, or move
injection earlier and lower the access itself through the runtime. Restating is
the cheap one and should happen now; the other is a v1 research question.

Related and mechanical: injecting at `optimized_mir` means I1 ("the pass visits
every access") is enforced over *post-optimization* MIR. An access that MIR
optimization merged or removed is never visited, and the invariant cannot tell
the difference between "elided by policy" and "never seen". Worth a sentence in
PLAN, and worth a census that compares access counts at `mir_built` against
`optimized_mir` if you ever want to close it.

---

## 5. Recommended order of work

The through-line: **stop adding detection features until degradation is visible
and overhead is measured.** Each new detection feature currently enlarges an
untracked false-negative surface, and the marginal CVE is no longer teaching
you anything (MEMORY: the vein is mined out).

Each task below follows the repo's convention: acceptance is a check that
passes, not a judgement call.

**E1. Instrumentation and degradation counters.**
Counters for: checks executed, checks where `root == fault` (propagation lost),
lookups returning `None` (unknown provenance), unsized start-only checks,
bodies visited, bodies skipped, crates instrumented. Print beside the existing
tally at exit; make the driver emit a per-crate manifest under `FEC_DEBUG`.
Acceptance: `fe-c-false-positive` asserts the propagation-lost and
unknown-provenance ratios are below a recorded threshold, so a regression that
silently stops checking fails the build.

**E2. Fail closed.**
`FEC_MODE` becomes an explicit enum: `case` or `through`, and anything else is
a hard error. `FEC_INSTRUMENT_ONLY` errors on a name that never matched any
crate. `find_fec_fns` returning `None` under `FEC_INSTRUMENT=1` is a hard
error, not a silent passthrough. Acceptance: a check that sets an unrecognized
mode (say `FEC_MODE=hard`) and asserts the build fails rather than quietly
selecting `case`.

**E3. The overhead bench (`fe-c-bench`, I5).**
One workload, four builds: uninstrumented, `case`, `through`, ASan. Publish
absolute numbers and ratios as an artifact; non-gating. Remove the two
per-check atomics first (make the tally opt-in under an env var) so the
baseline measures the checker rather than the counter. Acceptance: the check
emits a report; `PLAN.md` §7's v0 gate stops being a hypothesis.

**E4. The safe-reference provenance canary (§3.2).**
A `through`-mode reproducer whose over-read goes through a safe reference into
a live neighbour, asserting the report names the source allocation. If it
fails, extend `compute_roots` to reference locals and count the cases that root
at themselves. Acceptance: a new corpus entry, green, plus a fourth trace in
`docs/traces/` if it turns out to be a design defect rather than an oversight.

**E5. The vetting census (§3.4).**
Compile-time ratio of elided safe derefs with a dominating `ensure` to those
without, per crate. Acceptance: the number is printed for hashbrown and
recorded in `docs/both-modes.md` as the first evidence for the elision
argument.

**E6. Rescore the corpus (§3.5).**
Resolve the 46 IDs to `crate@version` with advisory aliases; publish
caught / missed / not-attempted; keep misses as rows. Acceptance:
`corpus/CORPUS.md` carries a scoreboard with a denominator.

**E7. Decide the per-crate dial's fate.**
Either build it (the `nix/lib/cargo` per-crate flags patch, `harden` in the
artifact key, and a real `cargo-fe-c`), or demote it from the README headline
to a v2 goal until it exists. Do not leave the central promise implemented as
an env var. This is a human decision, like C1 was.

After those, the feature frontier MEMORY already identified is the right one:
alignment checking, interprocedural provenance, and the T1/T2 at-rest
capability layer. Those are where new CVE classes actually live.

---

## 6. What would falsify the thesis

Worth writing down while the project is still young enough to change course.
Fe-C's thesis is that Rust's proofs let a checker be *both* cheaper than ASan
and more precise, with a documented lattice between the modes. The thesis is in
trouble if any of these turn out true:

1. **`through` overhead lands in ASan's range or worse** with no clear path
   down. Then `through` is a slower Miri-adjacent tool and the interesting mode
   is `case` alone.
2. **`case` overhead is not meaningfully below `through`.** Then elision buys
   nothing and the mode distinction, plus the composition theorem it implies,
   is not worth its complexity.
3. **The vetting census shows most elided derefs have no dominating `ensure`.**
   Then `case` is not "sound modulo the type system" but "checks less and hopes",
   and the honest framing changes.
4. **Propagation-lost rates are high in real crates.** Then I10 holds in the
   canary and not in practice, and the spatial-safety claim needs qualifying.

Each of these is answered by E1, E3 and E5, which is the strongest argument for
doing them before anything else.
