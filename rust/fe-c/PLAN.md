# Fe-C — Plan

> Starting work? Read `CLAUDE.md` first — it carries the hard rules, the
> settled decisions, and the ordered task queue. This file is the design
> record behind them.

Thesis: **check what you can't prove, prove what checking can't reach, and have
a theorem — not a convention — for how the modes compose.** Fil-C is one
degenerate point (check everything, prove nothing); pure Rust is the other
(prove the types, trust the unsafe). Fe-C is the lattice between them, per
crate, with sound seams as the long-term research contribution.

## 1. Prior art and what we take from each

| Source | Taken |
| ------ | ----- |
| Fil-C (Pizlo) | The bar for `through` mode: unconditional "garbage in, memory safety out"; runtime-owned semantics so the optimizer can't exploit UB; ~1.5–4× cost as the number to beat |
| SafeFFI (arXiv:2510.20688, USENIX Sec '26) | Check placement at raw→safe casts; hoisting out of type-proven regions; dealloc-reachability re-checks; the RustSec eval corpus. Their numbers (HWASan 3.22× → 2.12×/2.51×) show elision works; we replace their probabilistic HWASan backend with a deterministic table |
| Miri | Semantics oracle and the aliasing-UB complement we do **not** replicate |
| rustc_public (ex Stable MIR) | The version-coupling seam for the driver |
| Kani | The maintenance model: pinned nightly, automated bumps, no fork |
| CHERI rustc / strict provenance | Frontend lessons: provenance-clean pointer handling; what a multi-year fork costs (the thing we refuse to pay) |
| Eyra / origin / c-ward | The all-Rust substrate (`../libc`) that makes whole-process coverage possible |
| GWP-ASan | Sampled guard pages + canaries: probabilistic detection *inside* uninstrumented C, free at the allocator seam |
| Zig #36237 | Naming precedent: `fil` as the label for the Fil-C-grade mode, not the project |

## 2. Design invariants (v0 commitments — cheap now, rewrites later)

- **I1 — Total visitation, sparse policy.** The MIR pass visits *every* access
  from the first commit; a policy object decides check-or-skip. `case` mode is
  a policy, never a separate pass.
- **I2 — Capability-shaped runtime API.** Every check conceptually resolves a
  capability `(base, len, alloc_id, epoch)` from the allocation table.
  Allocation IDs are never recycled; liveness is an epoch. Quarantine (v0) and
  GC (v2) are interchangeable behind this interface; pointer-carried
  capabilities (v1) cache what the table already defines.
- **I3 — Mode is ABI.** Each crate's mode is recorded in metadata; cross-mode
  call adapters are specified now, while only one mode exists. The per-crate
  dial is ultimately a linking problem; leaving it implicit in v0 is how v2
  becomes impossible.
- **I4 — Both-modes rule.** No feature lands unless its behavior is defined in
  both modes. The filled-in table is `docs/both-modes.md`; an empty cell is a
  blocker, not a TODO. (Anti-gravity: prevents `case` users' needs from quietly
  becoming the whole roadmap.) **Open staging question raised there**:
  `through` needs strictly less machinery than `case` and is the natural oracle
  for differential-testing it, which argues for building it first — §3–§5 below
  currently assume the opposite order. Decide deliberately and record it.
- **I5 — Benchmark against Fil-C and ASan, never against our own fast mode.**
- **I6 — Re-checks are required, not optional.** Dealloc-reachable re-checking
  (§3.4) ships in v0. A cast-site-only build misses the entire class of
  "reference minted while live, invalidated later through an alias" — which is
  most real Rust UAF CVEs. See `docs/traces/rustsec-2021-0130.md` §F1.
- **I7 — `free` bumps the epoch before releasing; freed addresses are
  quarantined.** Without both, a reissued allocation presents a valid
  capability at a stale address (ABA) and temporal safety silently fails.
  See trace §F2.
- **I8 — Stack scope regions are table-registered.** The MIR pass emits scope
  enter/exit hooks; frame teardown poisons the region (epoch bump). Without
  this, stack use-after-free/return resolves to *no capability* and passes.
  See `docs/traces/rustsec-2021-0128.md` §F5.
- **I9 — Outbound FFI records escape.** Pointers passed to `extern "C"` are
  marked escaped; escaped allocations are exempt from elision that assumes
  Rust-exclusive access, and are quarantine-eligible regardless of crate mode.
  See docs/traces/rustsec-2021-0128.md §F6.
- **I10 — Provenance travels; checks never re-resolve from the faulting
  address.** Capabilities are resolved at derivation roots (allocation, cast,
  FFI entry, scope entry), propagated through pointer arithmetic and
  projections in the MIR dataflow, and compared at the dereference. Looking the
  *faulting address* up in the table is not spatial safety: an overflow into an
  adjacent live allocation resolves to a valid capability and passes. Side
  benefit: the hot path becomes a register compare, not a lookup, and the
  machinery is exactly what `through` mode needs (I2). See
  `docs/traces/rustsec-2021-0003.md` §F10.

## 3. v0 — `case` mode (case hardening)

Instrumentation points (MIR rewriting into plain calls to `cementite`).
The design is **subtractive**: start from checking every access, elide only
what the type system proves (I1). Point 0 is the default; 1–5 are what make
the elisions sound.

0. **Every raw-pointer dereference** (`*const T`/`*mut T` reads and writes),
   checked against a *propagated* capability per I10. Safe-pointer accesses are
   the ones elided — vetted once at their cast site. Omitting this was a real
   false-negative: see `docs/traces/rustsec-2021-0003.md` §F9.
1. Raw→safe casts: reborrows `&*p` / `&mut *p` (including through field/index
   projections) and `Box::from_raw` → `cementite::ensure(ptr, size_of::<T>())`.
2. Safe-pointer loads from memory (fields, globals) — unsafe code may have
   corrupted stored pointers. Default **on** for mixed-language builds,
   flag-controlled for pure-Rust (cost/coverage knob).
3. FFI boundaries, both directions:
   - *inbound*: `extern "C"`-visible fn prologues validate safe-pointer params
     (foreign callers only guarantee ABI, not types); validate safe-pointer
     returns after calls that cross the FFI line;
   - *outbound* **(I9)**: pointer args at `extern "C"` call sites are recorded
     as escaped — this is what makes the inbound check on a later callback
     meaningful. See `docs/traces/rustsec-2021-0128.md`.
4. Deallocation-reachable re-checks **(required, per I6)**: after any call that
   may transitively free (bottom-up `nofree` callgraph analysis, serialized
   cross-crate), re-validate live safe pointers before their next dereference
   (SafeFFI Alg. 1; single-thread-sound, documented as such). Worked example:
   `docs/traces/rustsec-2021-0130.md`.
5. Stack scope enter/exit hooks **(I8)**, sharing an implementation with the
   return-value re-check.

Runtime (`cementite`):

- Allocation table: page-indexed radix → `(base, len, alloc_id, epoch)`;
  O(1) lookup; epochs make use-after-free deterministic, quarantine delays
  address reuse.
- `#[global_allocator]` + libc interposition tiers: (a) malloc family,
  (b) `mem*`/`str*`, (c) ptr+len syscall wrappers (`read`, `write`, …),
  (d) `mmap`/`munmap`, `pthread_create` (stack registration). Interceptors are
  `#[no_mangle] extern "C"` Rust forwarding via `dlsym(RTLD_NEXT)` — the
  project stays pure Rust.
- Opaque-C interior: sampled guard-page allocations + canary-checked `free`.
- Failure = immediate abort with a report naming the cast/boundary site.

## 4. v0.5 — per-access validity fallback

With I1 in place this is a policy change: wherever provenance analysis punts
(dynamic GEP-equivalents, unions, int↔ptr traffic), check the access itself
against the table instead of giving up. Deterministic, slower, still no
optimizer entanglement.

## 5. v1 — `through` mode (through hardening), first stage

- Every access in a `through` crate lowered to opaque runtime calls; `noalias`
  and validity attributes become inert because the optimizer never directly
  dereferences.
- Pointer-carried capabilities (flight `(ptr, cap)` pairs) within `through`
  crates; shadow capability slots for pointers at rest.
- Cross-mode adapters: safe pointers entering a `case` crate from `through`
  (and vice versa) are re-validated at the seam per I3.
- Pointer/capability coherence is tiered (in-flight register pair / at-rest
  shadow slot / atomic 128-bit pair), decided in
  `docs/through-mode-coherence.md`. Rust's advantage over C: `AtomicPtr` and
  `UnsafeCell` make concurrently-accessed slots syntactically identifiable, so
  the expensive tier stays rare. v0 must already split `Cap` into packed/
  unpacked forms, use 48-bit ids with a liveness bitmap (no epoch counter), and
  distinguish in-flight from at-rest pointers.

## 6. v2 — temporal safety and composition

- GC or epoch-based deferred reclamation behind the I2 interface for
  raw-escaped allocations; eager free stays wherever ownership proves
  uniqueness (Rust subsidizes the common case Fil-C's collector pays for).
- The composition theorem: precise conditions under which `case`, `through`,
  and (externally) verified crates link into one process with a stated
  end-to-end property. SafeFFI's cast checks are an informal special case;
  the general statement is the publishable core of this project.

## 7. Acceptance gates

| Stage | Gate |
| ----- | ---- |
| v0 | Builds and runs a tokio-class app and the `../libc` substrate under `case`; reproducible RustSec CVEs from the SafeFFI corpus trap at the boundary; false-positive suite (serde/regex/hashbrown test suites) green; overhead *measured and published* (hypothesis: ≤10% on mostly-safe crates — a hypothesis until the numbers exist) |
| v0.5 | Fallback engaged automatically; corpus coverage strictly grows |
| v1 | Leaf crates run `through`; adapters proven by mixed-mode tests; overhead vs Fil-C published on shared C-via-FFI benchmarks |
| v2 | UAF corpus deterministic under raw-escape; composition write-up |

## 8. Toolchain coupling policy

- `rust-toolchain.toml` is the single source of truth (shared with `../libc`).
- `rustc_public` first; `rustc_internal` bridge only as documented escape
  hatches, each with an issue link tracking the missing public API.
- Weekly spindle pipeline bumps the nightly, runs build + corpus, opens a PR.
  Breakage budget: one sitting per bump, or the coupling surface gets reduced.

## 9. Nix integration ❄️

Detail in `docs/nix-integration.md` (including API questions to verify against
`nix/lib/cargo` before wiring anything).

- Flake wiring via the repo's flakelight modules; workspace built with the
  in-house `nix/lib/cargo` (per-crate derivations, gradual builds). That model
  fits Fe-C unusually well: `harden` becomes a per-crate derivation attribute,
  so flipping one crate's mode rebuilds only that crate and its dependents,
  and the instrumented sysroots are just more nodes in the same graph.
- **Packages**: `cargo-fe-c`, `cementite`,
  `fe-c-sysroot-{case,through}-{target}` — instrumented `core`/`alloc`/`std`
  as derivations keyed on (nightly, mode, target). This is the expensive
  artifact; building it once per nightly bump and serving it from harmonia is
  the single biggest DX win nix buys us.
- **devShell**: pinned nightly + `rustc-dev` + `rust-src` + miri + just
  recipes that shell out to nix.
- **Checks** (`nix flake check` = CI):
  - `fmt` / `statix` / `deadnix` / `clippy`
  - `unit`, `ui` (trybuild-style driver diagnostics)
  - `corpus-rustsec`: pinned vulnerable crate versions, vendored through
    `nix/lib/cargo`'s lockfile handling so the check is pure/offline, asserted
    to abort with an Fe-C report
  - `false-positive`: clean crates' own test suites under `case`
  - `selfhost`: `cementite` and `fe-c-driver` built under `case` mode
  - `miri-runtime`: the runtime's own unsafe under Miri
  - `bench`: criterion, non-gating, emits a report artifact (I5: baselines are
    ASan and Fil-C where a C-via-FFI benchmark can be shared)
- **Pipelines** (.tangled/spindle): check-on-PR; weekly nightly-bump;
  bench-report.

## 10. Non-goals (v0)

macOS/Windows targets · dynamic linking (the substrate lacks it anyway) ·
aliasing-model checking (Miri's job; run it in CI beside Fe-C) ·
production containment claims · LLVM passes of any kind.

## 11. Open questions

1. ~~Atomic `(ptr, cap)` coherence for `through`~~ — **resolved**, see
   `docs/through-mode-coherence.md`. Remaining sub-questions (bitmap sharding,
   whether T1's consistency check earns its cost, non-128-bit-atomic fallback
   churn) are deferred there.
2. Async-signal-safety requirements for table/bitmap lookup → dictates the
   locking model (carried from the cementite draft).
3. `case`-mode interaction with `noalias`-driven hoisting: is
   `-Zmutable-noalias=off` a supported knob, and what does it cost?
4. Fact transport if an LLVM-level backend is ever wanted (sidecar file vs
   attributes) — deliberately deferred; MIR rewriting needs neither.
5. Memory bound of quarantine under adversarial churn (and when GC wins).
6. Upstreaming path: which rustc_public gaps to file; whether `case` mode
   could eventually inform rustc's own sanitizer support.

## 12. Relationship to ../libc

`../libc` (vendored Eyra lineage) is the substrate that turns Fe-C from a
boundary checker into a whole-process checker: `-Zbuild-std` + the substrate
under `case` mode means "libc" is just more instrumentable Rust. Its plan
gates on Fe-C at phase P2; Fe-C's v0 gate builds it in return.
