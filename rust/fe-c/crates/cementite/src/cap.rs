//! Capability types: the split packed/unpacked representation decided in
//! `docs/through-mode-coherence.md` (v0 commitment 1 and 2).
//!
//! `AllocId` is 48 bits and never recycled; liveness is a bitmap bit, not an
//! epoch counter. `Cap` is the in-flight (register) form produced at
//! derivation roots and compared at dereferences (I10). `PackedCap` is the
//! at-rest form: together with a pointer it fits in 128 bits so `through`
//! mode's atomic tier can update the pair with cmpxchg16b/casp.

/// Number of significant bits in an [`AllocId`].
pub const ALLOC_ID_BITS: u32 = 48;

/// Exclusive upper bound on raw [`AllocId`] values.
pub const ALLOC_ID_LIMIT: u64 = 1 << ALLOC_ID_BITS;

/// Allocation identity: 48 significant bits, allocated sequentially and
/// never recycled (about 9 years of headroom at 1M allocations/second).
///
/// Id 0 is reserved as the null/invalid id and is never handed out.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct AllocId(u64);

impl AllocId {
    /// Reserved invalid id.
    pub const NULL: AllocId = AllocId(0);

    /// Wraps a raw id value.
    ///
    /// # Panics
    ///
    /// Panics if `raw` does not fit in 48 bits. Ids come from the runtime's
    /// own sequential counter, so an out-of-range value is a logic bug, not
    /// an input condition.
    #[inline]
    pub const fn from_raw(raw: u64) -> AllocId {
        assert!(raw < ALLOC_ID_LIMIT, "AllocId out of 48-bit range");
        AllocId(raw)
    }

    /// The raw 48-bit value.
    #[inline]
    pub const fn raw(self) -> u64 {
        self.0
    }

    /// Whether this is the reserved null id.
    #[inline]
    pub const fn is_null(self) -> bool {
        self.0 == 0
    }
}

/// Capability flags: the low 16 bits of the packed form.
///
/// Kept as a plain bitset rather than an external bitflags dependency; the
/// runtime has no reason to pull a dependency for five constants.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct CapFlags(u16);

impl CapFlags {
    /// No flags set.
    pub const EMPTY: CapFlags = CapFlags(0);
    /// The allocation escaped through an outbound FFI edge (I9).
    pub const ESCAPED: CapFlags = CapFlags(1 << 0);
    /// The capability was narrowed to a subobject during propagation.
    pub const SUBOBJECT: CapFlags = CapFlags(1 << 1);
    /// Provenance is known but bounds are not (permissive-mode registration).
    pub const UNKNOWN_BOUNDS: CapFlags = CapFlags(1 << 2);
    /// The backing region is a stack scope registered via I8 hooks.
    pub const STACK: CapFlags = CapFlags(1 << 3);

    /// Builds flags from the raw low-16 bit pattern.
    #[inline]
    pub const fn from_bits(bits: u16) -> CapFlags {
        CapFlags(bits)
    }

    /// The raw bit pattern.
    #[inline]
    pub const fn bits(self) -> u16 {
        self.0
    }

    /// Set union.
    #[inline]
    #[must_use]
    pub const fn union(self, other: CapFlags) -> CapFlags {
        CapFlags(self.0 | other.0)
    }

    /// Whether every flag in `other` is set in `self`.
    #[inline]
    pub const fn contains(self, other: CapFlags) -> bool {
        self.0 & other.0 == other.0
    }
}

/// Resolved capability, in-flight (register) form.
///
/// Produced at derivation roots (allocation, cast, FFI entry, scope entry),
/// propagated through pointer arithmetic and projections, compared at the
/// dereference. Never resolved from a faulting address (I10).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cap {
    /// Base address of the allocation (or narrowed subobject).
    pub base: usize,
    /// Length in bytes of the allocation (or narrowed subobject).
    pub len: usize,
    /// Identity of the backing allocation.
    pub id: AllocId,
    /// Capability flags.
    pub flags: CapFlags,
}

impl Cap {
    /// Whether the access `[addr, addr + size)` is inside this capability.
    ///
    /// `size == 0` accesses are in bounds anywhere in `[base, base + len]`,
    /// matching Rust's zero-sized-access rules.
    #[inline]
    pub fn covers(&self, addr: usize, size: usize) -> bool {
        let Some(end) = addr.checked_add(size) else {
            return false;
        };
        addr >= self.base && end <= self.base + self.len
    }

    /// Packs the identity half for at-rest storage. `base`/`len` are
    /// recovered from the allocation table by id on that path.
    #[inline]
    pub const fn packed(&self) -> PackedCap {
        PackedCap::pack(self.id, self.flags)
    }
}

/// At-rest capability form: `id:48 | flags:16` in one word, so a
/// `(pointer, PackedCap)` pair fits in 128 bits (through-mode tier T2).
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PackedCap {
    /// `id` in the high 48 bits, `flags` in the low 16.
    pub id_and_flags: u64,
}

impl PackedCap {
    /// Packs an id and flags into the at-rest form.
    #[inline]
    pub const fn pack(id: AllocId, flags: CapFlags) -> PackedCap {
        PackedCap {
            id_and_flags: (id.raw() << 16) | flags.bits() as u64,
        }
    }

    /// The allocation id half.
    #[inline]
    pub const fn id(self) -> AllocId {
        AllocId::from_raw(self.id_and_flags >> 16)
    }

    /// The flags half.
    #[inline]
    pub const fn flags(self) -> CapFlags {
        CapFlags::from_bits(self.id_and_flags as u16)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alloc_id_roundtrip_and_limits() {
        assert!(AllocId::NULL.is_null());
        let max = AllocId::from_raw(ALLOC_ID_LIMIT - 1);
        assert_eq!(max.raw(), ALLOC_ID_LIMIT - 1);
        assert!(!max.is_null());
    }

    #[test]
    #[should_panic(expected = "48-bit range")]
    fn alloc_id_rejects_49_bits() {
        let _ = AllocId::from_raw(ALLOC_ID_LIMIT);
    }

    #[test]
    fn packed_cap_roundtrip() {
        let id = AllocId::from_raw(0x0000_dead_beef_cafe);
        let flags = CapFlags::ESCAPED.union(CapFlags::STACK);
        let packed = PackedCap::pack(id, flags);
        assert_eq!(packed.id(), id);
        assert_eq!(packed.flags(), flags);
        // The pair (ptr, PackedCap) must stay 128-bit updatable.
        assert_eq!(core::mem::size_of::<PackedCap>(), 8);
    }

    #[test]
    fn packed_cap_max_id_does_not_overflow() {
        let id = AllocId::from_raw(ALLOC_ID_LIMIT - 1);
        let packed = PackedCap::pack(id, CapFlags::from_bits(u16::MAX));
        assert_eq!(packed.id(), id);
        assert_eq!(packed.flags().bits(), u16::MAX);
    }

    #[test]
    fn covers_bounds() {
        let cap = Cap {
            base: 0x1000,
            len: 0x100,
            id: AllocId::from_raw(7),
            flags: CapFlags::EMPTY,
        };
        assert!(cap.covers(0x1000, 1));
        assert!(cap.covers(0x10ff, 1));
        assert!(cap.covers(0x1000, 0x100));
        assert!(!cap.covers(0x1100, 1));
        assert!(!cap.covers(0xfff, 1));
        assert!(!cap.covers(0x10ff, 2));
        // Zero-sized accesses: valid anywhere in [base, base + len].
        assert!(cap.covers(0x1100, 0));
        assert!(!cap.covers(0x1101, 0));
        // Address arithmetic overflow is out of bounds, not a panic.
        assert!(!cap.covers(usize::MAX, 2));
    }
}
