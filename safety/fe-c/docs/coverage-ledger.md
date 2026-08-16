# Coverage ledger: what Fe-C does not check

Living document. Every known source of **false negatives**, in one place, with
the evidence for it and what would close it.

Why it exists: a checker's headline number is what it caught, and its honest
number is what it cannot see. `CLAUDE.md` §7 says "a false positive is a bug
report; a false negative is a CVE that shipped", so the false-negative surface
deserves a tracked artifact rather than being spread across STATUS prose and
commit messages.

**Rule: no new detection feature merges without either closing a row here or
adding one.** The both-modes rule (I4) governs behaviour; this governs
coverage. Rows are removed only when a check proves them closed.

Status marks: **open** (known gap, no work), **partial** (some cases covered),
**closed-by** (a check enforces it), **by-design** (a stated non-goal, listed
so the ledger is complete).

---

## 1. Memory the allocation table never learns about

A check resolves through `table::lookup`. Anything never registered resolves to
`None`, and `None` means pass (`check.rs:109`, `check.rs:244`). So every
unregistered region is a silent hole in both modes.

| Surface | Status | Effect | What closes it |
| ------- | ------ | ------ | -------------- |
| `static` / `const` data, including string literals and promoted temporaries | **open** | Any overflow rooted at a static resolves nothing and passes. Statics are never registered: the only `table::register` callers are the global allocator, the libc interposers, and `scope_enter`. | Register the image's data sections at startup (bounds are known from the link map), or resolve statics at the pass and pass a synthetic root. |
| `mmap` / `munmap` regions | **open** | Arena and mmap-backed allocators (the `bumpalo` class MEMORY records as deferred) are invisible. | Interceptor tier (d), already in the design (`cementite-api.md`). |
| Thread stacks, other than escape-analysed locals in instrumented bodies | **open** | A pointer into another thread's stack, or into a frame in an uninstrumented crate, resolves nothing. | `pthread_create` registration, tier (d). |
| Locals whose `layout_of` fails | **open** | No `scope_enter`, therefore no temporal check for that local (`instrument.rs:971`). | Count them (E1 in the evaluation); most are generic bodies where the size is not known until monomorphization. |
| Locals that escape by a route the analysis does not model | **partial** | `escaping_locals` models three sinks: pointer-to-integer cast, argument to a foreign call, capture into a closure aggregate (`instrument.rs:1241`+). A stack address stored into a struct field, a `static`, or a heap allocation and read back later is not modelled. | More sinks, or invert the analysis to "escapes unless provably frame-local". |
| Allocations from a non-`FecAlloc`, non-libc allocator (custom arenas, `jemalloc`, pool allocators) | **open** | Not registered, so not checked. | Documented limitation; arena support is a design question, not a patch. |
| C code's own interior allocations when `interpose` is off | **partial** | Interposition is behind a cargo feature. | Make the corpus assert which builds have it on. |

## 2. Accesses the pass never visits

I1 says the pass visits every access and elision is a policy on top. These are
the places where the visit itself does not happen, which the invariant is
specifically written to prevent.

| Surface | Status | Effect | What closes it |
| ------- | ------ | ------ | -------------- |
| Accesses inside uninstrumented crates | **partial** | With `FEC_INSTRUMENT_ONLY` scoping, only the listed crates are rewritten. Whole-graph is the default, but every corpus check uses the scoped form. `lru-0130`'s reborrow lives in the uninstrumented crate, which is why `minted_at` does not surface there. | Whole-graph corpus runs, plus the instrumentation manifest from E1. |
| Accesses in `core` / `alloc` / `std` that do not inline into an instrumented body | **open** | The `elf_rs` element is indexed through `core`'s `.get()`, which neither mode can see; the mint check is the only checkpoint. Same shape for `ptr::as_ref` (MEMORY: `caja`, RUSTSEC-2026-0130). | `-Zbuild-std` (D1), which STATUS records as blocked by `compiler_builtins` upstream monomorphization errors. |
| Accesses removed or merged by MIR optimization before the pass runs | **open** | The pass wraps `optimized_mir`, so "every access" means "every access that survived MIR opts". No signal distinguishes this from a policy elision. | Census comparing access counts at `mir_built` versus `optimized_mir`. |
| Accesses in build scripts, proc macros and `cementite` itself | **by-design** | Deliberately never instrumented (host-time code, or the definer of the symbols). | Nothing; correct as is. |
| Bodies in a crate where decl injection or symbol resolution failed | **open** | `inject_fec_decls` returns silently on a parse failure (`instrument.rs:86`); `find_fec_fns` returning `None` leaves every body untouched (`instrument.rs:103`). Silent, whole-crate. | E2 (fail closed). |
| `Vec::from_raw_parts`, `String::from_raw_parts`, `Box::from_raw` | **open** | Excluded from the slice-mint check by the two-argument filter (`instrument.rs:565`); `Box::from_raw` is in the design (point 1) but not implemented. These are owning raw-to-safe mints and a real CVE shape. | Extend the mint check: the three-argument form vets `cap * size_of::<T>()`. |
| Unsized accesses (`[T]`, `dyn Trait`) at a site where `layout_of` fails | **partial** | Falls back to the single-address check, so an overrun past the start is missed (`instrument.rs:307`). Inherent for a genuinely unknown extent; not inherent for fat pointers, whose length is in the pointer. | Fat-pointer length check, a separate feature. |
| Inline assembly, `volatile` accesses, union field reads, `transmute` round-trips | **open** | Not modelled anywhere. | v0.5's per-access validity fallback (`PLAN.md` §4). |

## 3. Checks that resolve from the faulting address

Hard Rule 1 forbids resolving a check from the faulting address, because an
overflow into an adjacent live allocation resolves valid and passes (F10).
These are the paths where the built system does it anyway, as a fallback.

| Surface | Status | Effect | What closes it |
| ------- | ------ | ------ | -------------- |
| Safe-reference derefs in `through` mode | **open** | `compute_roots` only populates raw-pointer locals (`instrument.rs:1547`), so a reference's root is itself (`instrument.rs:283`) and the lookup uses the accessed address. This is `through`'s defining check. **Predicted, not yet measured**: see `docs/evaluation-2026-07.md` §3.2 for the canary that settles it. | Root computation over reference locals, plus a `through` twin of the smallvec I10 canary. |
| Raw pointers whose provenance the dataflow lost | **partial** | Same fallback, by design ("never a false positive, at worst a false negative"). Frequency unknown: nothing counts it. | E1 counters, then a policy decision on what to do when the root is the fault. |
| Pointers loaded from memory (design point 2) | **open** | `through` covers them only insofar as the load's own deref is checked; there is no at-rest capability, so the loaded value's provenance starts fresh at the faulting address. | The T1 shadow-slot layer, already specified in `docs/through-mode-coherence.md`. |
| Opaque-origin pointers (foreign returns, int-to-pointer casts) | **partial** | Rooted at themselves deliberately, which is what makes `simple-slab`'s `libc::malloc` buffer resolvable. Correct for the base, wrong for anything derived before the checker saw it. | Nothing better available without interprocedural analysis. |

## 4. Properties never checked

| Property | Status | Notes |
| -------- | ------ | ----- |
| Alignment | **open** | `ensure_aligned` and `ViolationKind::Misaligned` are in the design; neither exists. MEMORY records alignment as one of the two remaining sources of new CVE classes. |
| Initialization / validity of read values | **by-design** | Miri's job (PLAN §10). |
| Aliasing model (Stacked/Tree Borrows) | **by-design** | Miri's job. Stated in the README. |
| Type confusion beyond extent | **open** | A mis-cast that stays inside the allocation and respects the extent passes. Inherent to a bounds-and-liveness table. |
| Free-during-scope under concurrency | **by-design, `case` only** | Stated in the README and `both-modes.md` (F3). `through` has no window. |
| Double free / invalid free | **open** | Not checked at the free path; `FecAlloc::dealloc` poisons but does not validate the argument. |
| Leaks | **by-design** | Not a memory-safety property. |

## 5. Environment assumptions

| Assumption | Status | Effect |
| ---------- | ------ | ------ |
| Single-threaded-ish table mutation | **open** | `register` / `deregister` / `poison` / scope hooks all take one global `Mutex` (`table.rs:104`). Correct, but every allocation and every stack scope in every thread serialises on it. |
| No allocation from signal handlers | **open** | `std::sync::Mutex` in the interposed `malloc` path is not async-signal-safe. `PLAN.md` §11 Q2 is filed as an open question; it is now a live defect. |
| No unwinding past a scope exit | **open** | `drop_sites` skips cleanup blocks (`instrument.rs:1140`), so a panic unwinding past a registered scope leaves the region registered **live**. The next frame reusing that address registers over it. Effect is a stale live region: missed temporal checks, and possibly wrong bounds. |
| Injected checks never unwind | **partial** | Every injected call uses `UnwindAction::Unreachable`. Sound while checks abort; a trap that ever wanted to unwind would be UB. |
| Process exits normally enough to run `atexit` | **partial** | The check tally is printed from an `atexit` handler; an aborting run prints it only if the abort path flushes first. Assert scripts depend on the tally. |
| 48-bit virtual addresses | **open** | Registration above the tracked VA bits aborts loudly (`table.rs:39`). Documented, and loud, which is the right shape. |

## 6. Build-configuration dependence

Detection is a function of how the target was compiled, and only one
configuration is tested. This deserves its own section because it means the
guarantee is not a property of the program.

| Dependence | Evidence |
| ---------- | -------- |
| Optimization level changes what is caught, in both directions | `partial-sort-0016` needs `opt-level = 3` with `debug-assertions = false` to exhibit at all. Conversely, at the corpus's default dev profile the slice `Index` does **not** inline, which is why the `case` slice gap existed until the `from_raw_parts` mint check was added (STATUS). |
| All corpus checks build with the dev profile, except that one | `default.nix`; no release-profile twin exists for any entry. |
| Inlining decides whether a `core` access is visible at all | The `elf_rs` case: `.get()` stays a call, so neither mode sees the element access. |
| The corpus asserts on abort plus report contents, not on a mapped-versus-unmapped distinction | `docs/traces/rustsec-2021-0003.md` specifies an "unmapped" regression guard so a test cannot pass via segfault; it was never built. |

## 7. Deliberate non-goals, listed for completeness

macOS and Windows targets; dynamic linking; instrumenting C dependencies (no
LLVM pass); aliasing-model checking; production containment claims for `case`.
All stated in `PLAN.md` §10 and the README. They are not gaps; they are the
boundary.
