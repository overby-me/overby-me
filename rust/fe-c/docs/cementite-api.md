# `cementite` — runtime API (design draft)

The hard phase. The only Fe-C crate that ships inside a hardened binary.
Everything here is shaped by I2 (capability-shaped API), I6/I7 (re-checks,
epoch-on-free) and the trace findings in
`docs/traces/rustsec-2021-0130.md`.

Draft status: **pre-implementation.** Signatures are the contract to argue
about now, cheaply.

## Core types

```rust
/// Resolved capability, in-flight (register) form. Produced at derivation
/// roots, propagated through pointer arithmetic (I10), compared at derefs.
#[derive(Clone, Copy)]
pub struct Cap {
    pub base:  usize,
    pub len:   usize,
    pub id:    AllocId,
    pub flags: CapFlags,   // escaped, subobject-narrowed, unknown-bounds, …
}

/// 48 bits. Never recycled — ~9 years of headroom at 1M allocations/sec.
/// (u32 would exhaust in about an hour; do not.)
pub struct AllocId(u64);   // 48 significant bits

/// At-rest form: fits with a pointer in 128 bits so `through` mode's atomic
/// tier can update the pair with cmpxchg16b / casp. `base`/`len` are recovered
/// from the table by `id`. See docs/through-mode-coherence.md.
#[repr(C)]
pub struct PackedCap { pub id_and_flags: u64 }
```

Liveness is a **bitmap indexed by `AllocId`**, not a per-allocation epoch
counter: with never-recycled ids each id is allocated once and freed at most
once, so the generation is one bit. `free` clears it before releasing memory
(I7). A reused address gets a fresh id, so stale pointers carrying the old id
fail structurally — no ABA window.

## Check surface (what the MIR pass emits)

```rust
/// Cast site: raw -> safe. Validates that `ptr` is live, has provenance in a
/// known allocation, and that `[ptr, ptr+size)` is inside it.
/// Returns the pointer so the pass can thread a distinct SSA value (letting
/// later analysis distinguish "raw" from "vetted safe").
pub fn ensure(ptr: *const u8, size: usize) -> *const u8;

/// Same, plus alignment requirement.
pub fn ensure_aligned(ptr: *const u8, size: usize, align: usize) -> *const u8;

/// Re-validation after a may-free call (I6). Split from `ensure` so the two
/// can be counted, sampled and reported separately.
pub fn recheck(ptr: *const u8, size: usize) -> *const u8;

/// FFI prologue: a safe-pointer parameter arrived from a foreign caller that
/// only guaranteed ABI compatibility.
pub fn ensure_foreign_arg(ptr: *const u8, size: usize) -> *const u8;

/// After a call returning a safe pointer (catches stack use-after-return).
pub fn ensure_returned(ptr: *const u8, size: usize) -> *const u8;

/// `through` mode: check the access itself. Opaque to the optimizer by
/// construction — this is what makes aliasing assumptions inert.
pub fn check_access(ptr: *const u8, size: usize, kind: AccessKind);

/// **Hot path (I10).** Raw-pointer dereference checked against a capability
/// already propagated from its derivation root — a register compare, no table
/// lookup. Never resolve the faulting address instead: an overflow into an
/// adjacent live allocation would resolve valid and pass.
pub fn check_deref(ptr: *const u8, size: usize, cap: Cap);

/// Fallback when propagation is lost (int->ptr round-trips, opaque calls,
/// unions): resolve from the table, then apply the v0.5 validity policy.
pub fn check_deref_unpropagated(ptr: *const u8, size: usize);
```

Failure path is always the same: `report_and_abort(Violation)` — never a
`Result`, never unwinding (unwinding through a corrupted-memory condition is
its own hazard).

## Table

```rust
/// Registration is driven by the allocator and the libc interceptors,
/// never by instrumented code directly.
pub(crate) fn register(base: *mut u8, len: usize) -> AllocId;
pub(crate) fn deregister(base: *mut u8);   // bumps epoch BEFORE release (I7)
pub(crate) fn lookup(addr: usize) -> Option<Cap>;

/// Stack scope regions (I8). Emitted by the MIR pass at scope enter/exit;
/// exit poisons the region so escaped references into it resolve dead.
/// Cost is per-scope, not per-access.
pub fn scope_enter(base: *const u8, len: usize) -> AllocId;
pub fn scope_exit(id: AllocId);

/// Outbound FFI escape (I9). Marks an allocation as visible to foreign code:
/// exempt from Rust-exclusivity elision, quarantine-eligible, and the source
/// of `Violation::escaped_at`.
pub fn note_escape(ptr: *const u8, site: SiteId);
```

Shape: page-number radix → leaf record. Consulted at **derivation roots and
fallbacks only** (I10), not per access. Measured ~2 ns/check in a dense
micro-probe (warm cache, single thread — a floor, not a promise); the
per-access hot path is cheaper still, being a bounds compare against a
capability already in registers.

Open: sharding strategy and whether lookups must be async-signal-safe
(they must, if interceptors can run in handlers — likely yes; that constrains
locking to atomics/seqlock).

## Allocator & interceptors

```rust
pub struct FecAlloc;              // #[global_allocator]
```

- Quarantine on free: address withheld from reuse for a bounded window
  (bytes-based cap + FIFO), defeating ABA (I7 / trace §F2).
- Sampled guard-page + canary allocations (GWP-ASan style) so *opaque C*
  interior overflows still trap without instrumenting C.
- Interceptor tiers, all `#[no_mangle] extern "C"` Rust forwarding via
  `dlsym(RTLD_NEXT)`:
  1. `malloc`/`calloc`/`realloc`/`free`/`posix_memalign`
  2. `mem*` / `str*`
  3. ptr+len syscall wrappers (`read`, `write`, `recv`, …) — the curated
     semantic list; a few hundred entries covers real traffic
  4. `mmap`/`munmap`, `pthread_create` (stack registration)

Unknown-provenance policy (`getenv`, `localtime` statics, foreign stacks) is a
knob: `strict` rejects at the cast site, `permissive` registers with unknown
bounds and degrades to null/alignment checks, **`strict-stack` (v0 default)**
tolerates unknown *heap/static* provenance but treats a pointer resolving into
a known-but-dead **stack** region as always fatal. `strict-stack` is strictly
more precise than `permissive` without inheriting `strict`'s false-positive
risk on legitimate foreign statics (see `docs/traces/rustsec-2021-0128.md` §F5).
Counters expose how often each path fires.

## Reporting

```rust
pub struct Violation {
    pub kind: ViolationKind,   // OutOfBounds | UseAfterFree | UseAfterReturn | BadProvenance | Misaligned | Null
    pub ptr: usize,
    pub cap: Option<Cap>,      // freed allocation's identity, if known
    pub site: SiteId,          // cast/recheck/FFI site, resolved to source
    pub freed_at: Option<SiteId>,
    pub escaped_at: Option<SiteId>,  // outbound FFI site, if the pointer left Rust (F7)
}
```

The trace's acceptance criterion: a UAF report must name **both** the free site
and the reborrow site that minted the reference. That is the debuggability win
over vanilla sanitizers (fail at root cause, not at a distant deref).

## Config surface

Env-var driven (`FEC_*`) so it can be tuned without rebuilds: quarantine
budget, guard-page sampling rate, unknown-provenance policy, report verbosity,
abort-vs-continue for triage runs (never default).

## Open questions carried into implementation

1. Async-signal-safety requirements for `lookup` → dictates the locking model.
2. `Cap` size/layout when `through` mode starts carrying it in registers.
3. Whether `ensure` should return `*const u8` or an opaque newtype the pass
   unwraps (affects how well LLVM can still optimize `case`-mode code).
4. Interaction of quarantine budget with long-running services under churn.
