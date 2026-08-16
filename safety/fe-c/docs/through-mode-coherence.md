# Decision memo: `(ptr, cap)` coherence in `through` mode

Resolves PLAN §11 Q1. This is the InvisiCaps-hard part — the thing to decide
*before* v0, because the answer constrains `Cap`'s layout and the pass's
in-flight/at-rest distinction, both of which are expensive to retrofit.

## Problem

In `through` mode every pointer has an associated capability. Pointer and
capability must stay coherent. If they can be observed torn — pointer from
allocation A, capability from allocation B — then an out-of-bounds access
passes its check, and the mode's whole guarantee evaporates. Fil-C hit exactly
this with `_Atomic`/`volatile` pointers and solved it with indirection: an
auxiliary 128-bit allocation holding capability and pointer together, updated
with 128-bit atomics where the platform has them, or by allocating a fresh
pair and atomically swapping the pointer to it.

## Structural advantage Rust has over C

In C, *any* pointer may be concurrently accessed and the compiler cannot tell,
so Fil-C must be defensive everywhere. In Rust, concurrent pointer access is
**syntactically identifiable**: `AtomicPtr` and friends, and shared mutation
only through `UnsafeCell` under a `Sync` bound. Non-atomic data races are
already UB by Rust's own rules and are out of scope by construction.

That lets Fe-C put the expensive mechanism only where it is actually needed —
a tiny fraction of pointer slots — instead of on every store.

## Decision: three tiers

**T0 — in flight (registers, SSA values, arguments, returns).**
Carry `(ptr, cap)` as an unpacked register pair. Thread-local by construction;
no coherence problem exists. This is the overwhelming majority of accesses and
it is the fast path.

**T1 — at rest, non-atomic.**
Capability lives in a shadow slot keyed by the *address of the pointer slot*
(InvisiCaps' key idea: the capability is indexed by where the pointer lives,
not by its value, so in-memory layout, `repr(C)`, `ptr::copy` and transmutes
all keep working). Two stores, so a torn read is *possible* — but only under a
data race, which is already UB.

Mitigation so races cost performance rather than safety: **the table is
authoritative, the shadow slot is a verified cache.** On load, check the cap
against the pointer (does `cap.base..cap.base+cap.len` contain `ptr`, does
`cap.id` match?). On mismatch, fall back to resolving from the table. A torn
pair almost always fails this check and takes the slow-but-correct path.

**T2 — at rest, atomic (`AtomicPtr`, shared `UnsafeCell<*mut T>`).**
Full coherence required. Store `(ptr, packed_cap)` as one 128-bit value and
update with `cmpxchg16b` (x86-64) / `casp` (aarch64 LSE). Where 128-bit atomics
are unavailable, fall back to Fil-C's indirection trick (allocate the pair,
swap the pointer to it atomically).

## Layout consequence — decide now

For T2 the *stored* form must fit in 128 bits, so it cannot be the full
capability:

```text
stored:    ptr: u64  |  packed: u64   (= alloc_id : 48, flags : 16)
in-flight: Cap { base, len, id, flags }   // unpacked, in registers
```

`base`/`len` are recovered from the table by `id` on the T2 path. Atomics are
already expensive; one lookup there is acceptable.

### `epoch` collapses into a liveness bit

If allocation ids are never recycled (I2), each id is allocated exactly once
and freed at most once — so the "generation" is one bit, not a counter.
Temporal safety becomes: *does the carried `id` still have its liveness bit
set?* A bitmap indexed by id, one atomic load, no comparison against whatever
now occupies the address.

- 48-bit ids: ~9 years of headroom at 1M allocations/second. u32 would exhaust
  in about an hour — do not use it.
- I7 restated: `free` **clears the liveness bit** before releasing memory
  (same invariant, cheaper representation).
- This also removes the ABA concern structurally: a reused address gets a new
  id, and a stale pointer carries the old one.

## Honest caveat vs Fil-C

Fil-C makes data races *harmless to memory*. Fe-C `through` mode, as decided
here, guarantees memory safety for programs **free of data races on pointer
slots**, with racy programs degraded (T1) to a table re-resolve rather than a
hole — probabilistically safe, not unconditionally. Closing that gap entirely
means T2 treatment for every slot, which is not worth its cost.

State this in the README beside the other scoping caveats. It is a deliberate
position, not an oversight: Rust already declares those races UB, and Miri is
the tool that finds them.

## v0 commitments (cheap now, rewrites later)

1. `Cap` gets the split representation above — packed 64-bit stored form,
   unpacked in-flight form. Even though v0 never stores one.
2. `AllocId` is 48 bits with 16 flag bits; `Epoch` is deleted in favour of a
   liveness bitmap.
3. The MIR pass distinguishes **in-flight** from **at-rest** pointers from the
   first commit, even while only in-flight capabilities exist (I10 already
   requires the propagation machinery; this is the same dataflow).
4. `AtomicPtr` and `UnsafeCell` pointer slots are *identified* in v0 and
   recorded, even though nothing is done with them yet — so the T2 path can be
   added without re-analysing.

## Deferred

Sharding of the liveness bitmap; whether T1's consistency check is worth its
cost versus always resolving from the table; the non-128-bit-atomic fallback's
allocation churn; interaction with `through`-mode inlining once checks stop
being opaque calls.
