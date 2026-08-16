//! Value and target primitives shared by every other crate in LLVM-rs.
//!
//! Four things live here, in the order a module needs them: arbitrary-width
//! integers, floating-point constants, the data layout that decides how types
//! are laid out in memory, and the target triple that names the machine.
//!
//! Nothing in this crate knows about the IR. That is deliberate: these types
//! are also what a target description, an object writer and a constant folder
//! will each need, and none of them should have to depend on the whole IR to
//! ask how wide a pointer is.

pub mod apfloat;
pub mod apint;
pub mod data_layout;
pub mod triple;

pub use apfloat::{ApFloat, FloatParseError, FloatSemantics};
pub use apint::{ApInt, ParseIntError};
pub use data_layout::{
    AlignSpec, DataLayout, DataLayoutParseError, Endianness, FunctionPointerAlign, Mangling,
    PointerSpec,
};
pub use triple::{Arch, Env, Os, Triple, Vendor};

/// A power-of-two alignment in bytes.
///
/// The IR spells alignments in bytes and requires them to be powers of two, so
/// carrying that invariant in the type keeps every `align 3` out of the rest
/// of the codebase.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Align(u32);

impl Align {
    pub const ONE: Align = Align(0);

    /// `None` when `bytes` is zero or not a power of two.
    pub fn from_bytes(bytes: u64) -> Option<Align> {
        if bytes == 0 || !bytes.is_power_of_two() {
            return None;
        }
        Some(Align(bytes.trailing_zeros()))
    }

    /// Rounds up to the next power of two.
    pub fn from_bytes_rounded_up(bytes: u64) -> Align {
        if bytes <= 1 {
            return Align::ONE;
        }
        Align(bytes.next_power_of_two().trailing_zeros())
    }

    pub fn from_bits(bits: u32) -> Option<Align> {
        Align::from_bytes(u64::from(bits).div_ceil(8))
    }

    pub fn bytes(self) -> u64 {
        1u64 << self.0
    }

    pub fn bits(self) -> u64 {
        self.bytes() * 8
    }

    pub fn log2(self) -> u32 {
        self.0
    }

    pub fn max(self, other: Align) -> Align {
        Align(self.0.max(other.0))
    }

    /// Rounds `offset` up to the next multiple of this alignment.
    pub fn align_up(self, offset: u64) -> u64 {
        let mask = self.bytes() - 1;
        (offset + mask) & !mask
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alignments_are_powers_of_two() {
        assert_eq!(Align::from_bytes(1).unwrap().bytes(), 1);
        assert_eq!(Align::from_bytes(8).unwrap().bytes(), 8);
        assert_eq!(Align::from_bytes(4096).unwrap().bytes(), 4096);
        assert!(Align::from_bytes(0).is_none());
        assert!(Align::from_bytes(3).is_none());
        assert!(Align::from_bytes(24).is_none());
    }

    #[test]
    fn rounding_and_bits() {
        assert_eq!(Align::from_bytes_rounded_up(0).bytes(), 1);
        assert_eq!(Align::from_bytes_rounded_up(3).bytes(), 4);
        assert_eq!(Align::from_bytes_rounded_up(8).bytes(), 8);
        assert_eq!(Align::from_bits(128).unwrap().bytes(), 16);
        assert_eq!(Align::from_bytes(16).unwrap().bits(), 128);
        assert_eq!(Align::from_bytes(16).unwrap().log2(), 4);
    }

    #[test]
    fn aligning_offsets_up() {
        let eight = Align::from_bytes(8).unwrap();
        assert_eq!(eight.align_up(0), 0);
        assert_eq!(eight.align_up(1), 8);
        assert_eq!(eight.align_up(8), 8);
        assert_eq!(eight.align_up(9), 16);
        assert_eq!(Align::ONE.align_up(7), 7);
        assert_eq!(
            Align::from_bytes(4)
                .unwrap()
                .max(Align::from_bytes(8).unwrap()),
            Align::from_bytes(8).unwrap()
        );
    }
}
