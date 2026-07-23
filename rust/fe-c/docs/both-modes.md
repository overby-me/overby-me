# Both-modes table (I4)

I4 says: *no feature lands unless its behaviour is defined in both modes.*
This is that rule with teeth. Every row must be filled before the
corresponding code merges. An empty cell is a blocker, not a TODO.

## Instrumentation

| Feature | `case` | `through` |
| ------- | ------ | --------- |
| Raw-pointer deref (point 0) | checked against propagated cap | same, but via an **opaque** call the optimizer cannot fold or hoist |
| **Safe-pointer deref** | **elided** — vetted once at the cast | **checked** |
| Raw→safe cast (point 1) | `ensure` | `ensure`, and refreshes the carried cap |
| Safe pointer loaded from memory (point 2) | checked; default on for mixed-language, flag for pure-Rust | always checked |
| FFI inbound (point 3a) | prologue check on safe-pointer params/returns | same, plus cap materialization for the callee |
| FFI outbound (point 3b, I9) | `note_escape` | `note_escape` + shadow-slot flush |
| Dealloc-reachable re-check (point 4, I6) | **required**; needs the `nofree` callgraph analysis | **not needed** — every access is already checked |
| Stack scope hooks (point 5, I8) | required | required |
| Cap propagation (I10) | intraprocedural, in-flight only | interprocedural + at-rest (shadow slots, T1/T2) |

The single row that *is* the mode distinction is the bolded one. Everything
else differs in strength or in what machinery it needs, not in intent.

## Runtime

| Feature | `case` | `through` |
| ------- | ------ | --------- |
| Allocation table | authoritative | authoritative |
| Liveness bitmap (id-indexed) | same | same |
| Quarantine on free | on, budgeted | on, budgeted (GC candidate in v2) |
| Guard-page/canary sampling | on — defence for *opaque C* | on (same rationale; not load-bearing) |
| Unknown-provenance default | `strict-stack` | `strict` — it can afford it |
| Atomic pointer slots (T2) | not tracked; no at-rest caps exist | 128-bit coherent pair |
| Escaped allocations (I9) | elision-exempt, quarantine-eligible | already fully checked; escape only feeds reporting |

## Guarantees (state these differences, never blur them)

| Property | `case` | `through` |
| -------- | ------ | --------- |
| Spatial safety | deterministic where provenance is tracked; falls back per v0.5 policy where it is lost | deterministic |
| Temporal (free-before-scope) | deterministic | deterministic |
| Temporal (free-during-scope) | **single-thread-sound**; a concurrent free in the re-check window can be missed | no window exists |
| Aliasing/validity UB | assumed to hold in elided regions; the optimizer shares the assumption | assumptions inert — the optimizer never dereferences directly |
| Data races on pointer slots | n/a (no at-rest caps) | safe for race-free programs; T1 degrades a torn read to a table resolve |
| Trusted base | `cementite` + every unsafe block in elided regions | `cementite` + syscall stubs |

## Cross-mode calls (I3)

| Direction | Adapter behaviour |
| --------- | ----------------- |
| `case` → `through` | callee re-materializes caps at its prologue from the table; caller's elision assumptions do not propagate inward |
| `through` → `case` | pointers crossing in are re-validated at the seam; the `case` callee's elisions apply only to its own frame |
| either → foreign C | `note_escape`; escaped allocations lose elision eligibility in both modes |

The end-to-end property of a mixed process is *not* the max of its parts —
that is the composition theorem (PLAN §6), still open. Until it is written,
document mixed processes as "`through`-grade only in `through` crates."

## Finding: `through` is the *simpler* mode to build

Reading the tables together, `through` needs strictly **less** machinery than
`case`: no `nofree` callgraph analysis, no elision-soundness argument, no
free-during-scope reasoning, no `strict-stack` compromise. It is slower, not
harder. Three consequences worth weighing before v0 starts:

1. **`through` is the oracle for `case`.** Differential testing becomes
   possible: any violation `through` catches that `case` misses is either a
   documented elision gap or a bug. Without it there is no reference
   implementation to test the fast mode against.
2. **It resolves the naming risk structurally.** Earlier concern: a Fil-C-adjacent
   name promising a guarantee the default mode doesn't deliver. If `through`
   is what exists first and `case` arrives as the opt-in optimization, the
   promise is honest from commit one.
3. **It front-loads the hard runtime.** T2 coherence and shadow slots move
   earlier — which is where the genuine engineer-years live. That is a cost,
   though the coherence memo already fixed the layout decisions that would
   otherwise force a rewrite.

**Recommendation**: build the runtime and the pass for `through` first, ship
`case` as the second milestone with a differential-test gate against
`through`. PLAN §3–§5 currently assume the reverse order; reorder only after
deciding deliberately, and record the decision here either way.

Counter-argument for the current order, for the record: `case` reaches useful
CI overhead sooner, validates all the rustc_public/build-std/Eyra plumbing on
a cheaper path, and produces publishable numbers earlier.

## Decision (2026-07-22, Task C1)

**`through` first.** Recorded in PLAN §2 under I4. The runtime and pass are
built for `through` as the first milestone; `case` follows as the opt-in
optimization with a `differential` gate against `through` (Task C3). Chosen
for the three reasons above: less machinery, an oracle for `case`, and an
honest safety promise from commit one. The bolded mode-distinction row — safe
pointer derefs are **checked** in `through`, **elided** in `case` — is what
the first `through` milestone implements; the `case` elision arrives later.

## Differential gate result (2026-07-22, Task C3)

`through` (the oracle, checks every deref) and `case` (elides safe derefs,
re-checks dealloc-reachable ones) are run against the corpus in both modes
(`fe-c-differential`, plus each corpus entry's own two-mode check). The verified
relationship, exactly as the tables predict:

| Reproducer | bug | `through` | `case` | why |
| ---------- | --- | --------- | ------ | --- |
| `closure-escape` | raw-pointer stack UAF | abort | abort | raw deref, checked in both (point 0) |
| `smallvec-0003` | heap OOB | abort | abort | write-intrinsic / raw deref, both |
| `cast-oob` (whole-object) | raw→safe cast OOB | abort | abort | the cast ensure (point 1), both modes |
| `cast-oob field` | field reborrow `&(*p).b` OOB | abort | abort | field-granular cast ensure — closes the `case` elision gap |
| `cast-oob direct` | direct field read `(*p).b` OOB | abort | abort | projected deref fault (point 0 faults on `p + offset`, not the base) |
| `cast-oob extent` | field read overrunning the end | abort | abort | projected extent check (`__fec_check_extent` over `[p+offset, +size)`) |
| `cast-oob whole-extent` | `*p` read wider than the alloc | abort | abort | whole-object extent check (`[p, p+size_of::<T>())`) |
| `slice-oob` | `from_raw_parts` slice lies about its length | abort | abort | slice-extent check at the mint (point 1) — `case` elides the slice's derefs, so the `from_raw_parts` is the only checkpoint |
| `elf-rs-0079` | real CVE: `elf_rs` `from_raw_parts` with an attacker section count | abort | abort | slice-extent check at the mint over a *generic* element (injected `size_of::<T>()`); the element is indexed via core's `.get()`, so the mint is the only checkpoint in **either** mode |
| `lru-0130` | heap use-after-free | abort | abort | `case` via the dealloc-reachable re-check (point 4) |
| `heap-mint` | heap UAF, mint named | abort | abort | both name `minted_at`; `case` also `read_at` (both-sites debuggability) |
| `through-safe-ref` | safe-ref stack UAF | abort | **elide** | the bolded row: `case` elides safe-pointer derefs |
| `rusqlite-0128` | safe-ref stack UAF (real CVE) | abort | **elide** | same elision |

The **only** violations `through` catches that `case` misses are the
safe-pointer-deref stack use-after-scope reads — precisely the elision the
bolded row documents. No undocumented gap: the gate passes. (Concurrent
free-during-scope, F3, is the other stated `case` limitation; it is not
exercised by this single-threaded corpus.)
