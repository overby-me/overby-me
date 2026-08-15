//! The name an instantiation reduces to.
//!
//! Every table in this module is keyed by the name LangRef documents, and a
//! module writes the name an instantiation of it carries, so a lookup asks
//! for `llvm.bswap.v4i32` and has to find `llvm.bswap`. This is that
//! reduction, kept beside the tables rather than inside one of them so that
//! regenerating a table cannot take it away.

use super::{names, table};

/// Whether a component of a name is a mangled type rather than part of the
/// name itself.
///
/// A mangled type usually carries the width or the count of what it
/// describes, so it has a digit in it: `i32`, `v4i32`, `nxv2i64`, `p0`,
/// `bf16`, `fp128`, `ppcf128`, `a4i32`. The ones that do not are the type
/// keywords the IR spells out, and those are a closed set rather than a
/// guess: every digit-free type spelling that appears after a documented
/// name across `llvm/test` is here, `half` on twenty-three names and the
/// rest on fewer.
///
/// Anything else with no digit in it is a word, and a word belongs to the
/// name: `elts` in `llvm.vp.cttz.elts` is not a type.
///
/// This was a list of prefixes before, which is the same idea stated in a
/// form that has to be kept up to date and was not: it knew `f128` and not
/// `fp128`, and no spelling of `bfloat` at all, which cost fifty-four
/// modules across three trees before the tree ratchets said so.
fn mangled(part: &str) -> bool {
    const SPELLED: &[&str] = &[
        "bfloat", "double", "float", "half", "isVoid", "metadata", "ptr", "token", "void",
    ];
    part.bytes().any(|byte| byte.is_ascii_digit()) || SPELLED.contains(&part)
}

/// The names a lookup tries: the whole name, then the whole name with its
/// trailing mangled types dropped one at a time, stopping at the first word.
///
/// Stopping there keeps `llvm.vp.cttz.elts` from reaching `llvm.vp.cttz`,
/// and trying the whole name first keeps `llvm.convert.to.fp16`, whose own
/// last component is type-shaped.
pub fn candidates(name: &str) -> impl Iterator<Item = &str> {
    let mut next = Some(name);
    std::iter::from_fn(move || {
        let candidate = next?;
        next = match candidate.rsplit_once('.') {
            Some((rest, last)) if rest.contains('.') && mangled(last) => Some(rest),
            _ => None,
        };
        Some(candidate)
    })
}

/// The name with every mangled type dropped: `llvm.bswap.v4i32` is
/// `llvm.bswap`.
///
/// This is the shortest thing [`candidates`] offers, so it can be shorter
/// than the name a table answers for. Prefer asking a table, which tries the
/// whole name first.
pub fn base_name(name: &str) -> &str {
    candidates(name).last().unwrap_or(name)
}

/// Whether this names an intrinsic LangRef documents, under its own name or
/// under the one it instantiates. Both tables are asked, neither containing
/// the other.
pub fn is_documented(name: &str) -> bool {
    candidates(name).any(|candidate| {
        names::is_documented(candidate) || table::signature_exact(candidate).is_some()
    })
}

#[cfg(test)]
mod tests {
    use super::{base_name, is_documented};
    use crate::intrinsic::{attributes, overloads, table};

    /// A mangled type comes off a name and a word does not.
    #[test]
    fn only_a_mangled_type_comes_off_a_name() {
        assert_eq!(base_name("llvm.bswap.v4i32"), "llvm.bswap");
        assert_eq!(base_name("llvm.umax.i8"), "llvm.umax");
        assert_eq!(base_name("llvm.memcpy.p0.p0.i64"), "llvm.memcpy");
        assert_eq!(base_name("llvm.fabs.bf16"), "llvm.fabs");
        assert_eq!(base_name("llvm.sqrt.fp128"), "llvm.sqrt");
        // A type the IR spells out rather than measuring.
        assert_eq!(base_name("llvm.is.fpclass.half"), "llvm.is.fpclass");
        assert_eq!(base_name("llvm.assume"), "llvm.assume");
        assert_eq!(
            base_name("llvm.vp.cttz.elts.i32.nxv16i1"),
            "llvm.vp.cttz.elts"
        );
    }

    /// A name answers from its own entry, or from the one it instantiates,
    /// and never from a shorter name that is merely a prefix of it.
    ///
    /// `llvm.vp.cttz` and `llvm.vp.cttz.elts` are both documented, and the
    /// second counts into an `i32` where the first returns its operand's
    /// type. The tied-position table holds only the first, so the right
    /// answer for the second is nothing rather than the first's.
    #[test]
    fn a_name_does_not_reach_a_shorter_one_that_is_its_prefix() {
        assert!(overloads::tied("llvm.vp.cttz").is_some());
        assert!(overloads::tied("llvm.vp.cttz.elts.i32.nxv16i1").is_none());
        assert!(table::signature("llvm.vp.cttz.elts.i32.nxv16i1").is_some());
        assert!(attributes::attributes("llvm.vp.cttz.elts.i32.nxv16i1").is_some());
        assert!(table::signature("llvm.aarch64.made.up.p0").is_none());
        assert!(overloads::tied("llvm.aarch64.made.up.p0").is_none());
    }

    /// A whole name wins over the one it reduces to, which is what keeps an
    /// intrinsic whose own last component is a mangled type readable.
    #[test]
    fn a_name_ending_in_a_type_is_still_a_name() {
        assert!(table::signature("llvm.convert.to.fp16").is_some());
        assert!(is_documented("llvm.convert.to.fp16"));
        // And the instantiations of one that is not.
        assert!(is_documented("llvm.fabs.bf16"));
        assert!(is_documented("llvm.sqrt.fp128"));
        assert!(!is_documented("llvm.aarch64.made.up.p0"));
    }
}
