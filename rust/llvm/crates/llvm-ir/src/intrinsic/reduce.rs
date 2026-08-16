//! The name an instantiation reduces to.
//!
//! Every table in this module is keyed by the name LangRef documents, and a
//! module writes the name an instantiation of it carries, so a lookup asks
//! for `llvm.bswap.v4i32` and has to find `llvm.bswap`. This is that
//! reduction, kept beside the tables rather than inside one of them so that
//! regenerating a table cannot take it away.

use super::{declared, names, recognised, table};

/// Whether a component of a name is a mangled type rather than part of the
/// name itself.
///
/// This is the grammar [`super::mangle`] measured, read backwards: an
/// integer is `i` and its width, a pointer `p` and its address space, a
/// vector or an array a prefix, a count and the element's own spelling, a
/// literal struct `sl_` and its fields and `s`. The rest are the spellings
/// that carry no number, which is a closed set.
///
/// It was "has a digit in it" before, which is the same idea stated loosely,
/// and loosely is wrong in both directions. `interleave4` in
/// `llvm.vector.interleave4` has a digit and is not a type, and reducing
/// through it left `llvm.vector`; worse, `llvm.amdgcn.fdot2` reduced to
/// `llvm.amdgcn`, and a table keyed on that answers for every name the
/// target has. Nothing here strips a component that is not a type upstream
/// would have written.
fn mangled(part: &str) -> bool {
    // The spellings that carry no count of their own, including the two the
    // IR writes as words.
    // `fp128` beside `f128`: the second is what upstream mangles a `fp128`
    // to now, and the first is what the older names in its own tests carry.
    const SPELLED: &[&str] = &[
        "Metadata", "bf16", "bfloat", "double", "f128", "f16", "f32", "f64", "f80", "float",
        "fp128", "half", "isVoid", "label", "metadata", "ppcf128", "ptr", "token", "void",
        "x86amx",
    ];
    if SPELLED.contains(&part) {
        return true;
    }
    // A literal struct writes its fields between `sl_` and `s`.
    if let Some(fields) = part.strip_prefix("sl_") {
        return fields.ends_with('s');
    }
    // An integer or a pointer: the letter, then nothing but its number.
    for prefix in ["i", "p"] {
        if let Some(number) = part.strip_prefix(prefix)
            && !number.is_empty()
            && number.bytes().all(|byte| byte.is_ascii_digit())
        {
            return true;
        }
    }
    // A vector or an array: the prefix, the count, then whatever one element
    // spells. `nxv` before `v`, being the longer of the two.
    for prefix in ["nxv", "v", "a"] {
        let Some(rest) = part.strip_prefix(prefix) else {
            continue;
        };
        let count = rest
            .bytes()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        if count > 0 && count < rest.len() && mangled(&rest[count..]) {
            return true;
        }
    }
    false
}

/// The names a lookup tries: the whole name, then the whole name with its
/// trailing mangled types dropped one at a time, stopping at the first word,
/// and last of all the longest prefix that names an intrinsic at all.
///
/// Stopping at the first word keeps `llvm.vp.cttz.elts` from reaching
/// `llvm.vp.cttz`, and trying the whole name first keeps
/// `llvm.convert.to.fp16`, whose own last component is type-shaped.
///
/// The prefix at the end is how upstream finds an intrinsic: the longest
/// prefix of the name that it knows, with whatever follows ignored, which is
/// what makes `llvm.objectsize.i32.unnamed` an `llvm.objectsize`. It goes
/// last rather than first because the tables here are not the complete set
/// upstream has, and falling back to a shorter name that happens to be in
/// one would answer for the wrong intrinsic: `llvm.memcpy.element.unordered`
/// is not an `llvm.memcpy`. Asked last, it only ever fires where the strict
/// reading found nothing, and only when the prefix is a name something knows.
pub fn candidates(name: &str) -> impl Iterator<Item = &str> {
    let mut next = Some(name);
    let mut prefix = longest_known_prefix(name);
    std::iter::from_fn(move || {
        if let Some(candidate) = next {
            next = match candidate.rsplit_once('.') {
                Some((rest, last)) if rest.contains('.') && mangled(last) => Some(rest),
                _ => None,
            };
            if Some(candidate) == prefix {
                prefix = None;
            }
            return Some(candidate);
        }
        prefix.take()
    })
}

/// The longest prefix of the name that something here knows as an intrinsic,
/// or `None` when nothing does.
///
/// Prefixes only, on the dots, longest first: upstream reads a name by
/// finding the longest one it has and ignoring the rest, which is why
/// `llvm.objectsize.zzz` comes back `llvm.objectsize.i32.p0`.
fn longest_known_prefix(name: &str) -> Option<&str> {
    let mut candidate = name;
    loop {
        if names_an_intrinsic(candidate) {
            return Some(candidate);
        }
        let (rest, _) = candidate.rsplit_once('.')?;
        if !rest.contains('.') {
            return None;
        }
        candidate = rest;
    }
}

/// Whether this exact name is one LangRef documents, which is what a prefix
/// is measured against.
///
/// The documented names only, where [`is_known`] asks three tables. The other
/// two are stored reduced, so a name in them may be a reduction's own artefact
/// rather than an intrinsic: `llvm.dbg.label` reduces to `llvm.dbg` because
/// `label` is also a type spelling, and `llvm.dbg` is in the recognised table
/// for no other reason. Reading a prefix out of that would make every
/// `llvm.dbg.*` an `llvm.dbg`, which is two Linker files printed wrong. These
/// names are exact and harvested from the documentation rather than reduced.
fn names_an_intrinsic(name: &str) -> bool {
    names::is_documented(name) || table::signature_exact(name).is_some()
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

/// Whether upstream would recognise this name, which is a wider question
/// than whether LangRef documents it: the coroutine and exception-handling
/// intrinsics are documented in other files, `llvm.vector.interleave4` in
/// none, and every target's in the target backend.
///
/// This is what decides whether an undeclared call is an intrinsic upstream
/// builds a declaration for, and whether a name prints with upstream's
/// `; Unknown intrinsic` comment above it.
pub fn is_known(name: &str) -> bool {
    is_documented(name) || recognised::is_recognised(name) || declared::is_declared_intrinsic(name)
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

    /// Upstream knows names LangRef does not document, and the two questions
    /// are asked separately because only the second decides whether an
    /// undeclared call resolves.
    ///
    /// Each of these was measured. `llvm.vector.interleave4` and
    /// `llvm.vector.interleave9` are the pair that shows it is a fact about
    /// upstream and not about the spelling: the same call shape at four
    /// operands assembles and at nine does not, and LangRef documents
    /// neither, stopping at three.
    #[test]
    fn upstream_knows_names_langref_does_not_document() {
        use super::is_known;
        for name in [
            // Documented in Coroutines.rst rather than LangRef.
            "llvm.coro.id",
            "llvm.coro.save",
            // In ExceptionHandling.rst.
            "llvm.eh.typeid.for",
            // Documented nowhere at all.
            "llvm.vector.interleave4",
            // Documented only in the target backend, and reached through the
            // instantiation a module actually writes.
            "llvm.amdgcn.ds.append.p3",
            "llvm.aarch64.neon.vsli",
        ] {
            assert!(!is_documented(name), "{name} is not in LangRef");
            assert!(is_known(name), "{name} is one upstream recognises");
        }
        for name in ["llvm.vector.interleave9", "llvm.completely.invented.name"] {
            assert!(!is_known(name), "{name} is not an intrinsic upstream knows");
        }
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
