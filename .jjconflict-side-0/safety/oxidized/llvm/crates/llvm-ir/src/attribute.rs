//! Attributes on functions, parameters, returns and call sites.
//!
//! An attribute is one of six shapes: a bare keyword, a keyword with integer
//! arguments, a keyword with a type argument, the `range` form, one of a
//! handful of keywords with their own bespoke argument grammar, and an
//! arbitrary quoted key with an optional quoted value.
//!
//! The bespoke ones (`memory(...)`, `captures(...)`, `nofpclass(...)`,
//! `uwtable(...)`, `allockind(...)`, `initializes(...)`) keep their argument
//! text verbatim. Each has a grammar of its own that no pass reads yet, and a
//! faithful syntactic form beats either a wrong model or a dropped attribute.
//! The keyword itself is still checked, so an unknown attribute is an error.

pub mod order;

use crate::keyword::define_keyword_enum;
use crate::types::TypeId;
use llvm_support::ApInt;

define_keyword_enum! {
    /// Attributes spelled as a bare keyword.
    ///
    /// The list is LangRef's, and the parser rejects anything outside it, so
    /// an attribute upstream adds shows up as a loud error naming the keyword
    /// rather than as a silently dropped property.
    EnumAttr {
        AllocAlign => "allocalign",
        AllocPtr => "allocptr",
        AlwaysInline => "alwaysinline",
        Builtin => "builtin",
        Cold => "cold",
        Convergent => "convergent",
        CoroDestroyOnlyWhenComplete => "coro_only_destroy_when_complete",
        CoroElideSafe => "coro_elide_safe",
        DeadOnReturn => "dead_on_return",
        DeadOnUnwind => "dead_on_unwind",
        DisableSanitizerInstrumentation => "disable_sanitizer_instrumentation",
        FnRetThunkExtern => "fn_ret_thunk_extern",
        Hot => "hot",
        ImmArg => "immarg",
        InReg => "inreg",
        InlineHint => "inlinehint",
        JumpTable => "jumptable",
        MinSize => "minsize",
        MustProgress => "mustprogress",
        Naked => "naked",
        Nest => "nest",
        NoAlias => "noalias",
        NoBuiltin => "nobuiltin",
        NoCallback => "nocallback",
        NoCapture => "nocapture",
        NoCfCheck => "nocf_check",
        NoDivergenceSource => "nodivergencesource",
        NoDuplicate => "noduplicate",
        NoExt => "noext",
        NoFree => "nofree",
        NoImplicitFloat => "noimplicitfloat",
        NoInline => "noinline",
        NoMerge => "nomerge",
        NoProfile => "noprofile",
        NoRecurse => "norecurse",
        NoRedZone => "noredzone",
        NoReturn => "noreturn",
        NoSanitizeBounds => "nosanitize_bounds",
        NoSanitizeCoverage => "nosanitize_coverage",
        NoSync => "nosync",
        NoUndef => "noundef",
        NoUnwind => "nounwind",
        NonLazyBind => "nonlazybind",
        NonNull => "nonnull",
        NullPointerIsValid => "null_pointer_is_valid",
        OptDebug => "optdebug",
        OptForFuzzing => "optforfuzzing",
        OptNone => "optnone",
        OptSize => "optsize",
        PreSplitCoroutine => "presplitcoroutine",
        ReadNone => "readnone",
        ReadOnly => "readonly",
        Returned => "returned",
        ReturnsTwice => "returns_twice",
        SafeStack => "safestack",
        SanitizeAddress => "sanitize_address",
        HybridPatchable => "hybrid_patchable",
        SanitizeHwAddress => "sanitize_hwaddress",
        SanitizeMemTag => "sanitize_memtag",
        SanitizeMemory => "sanitize_memory",
        SanitizeNumericalStability => "sanitize_numerical_stability",
        SanitizeRealtime => "sanitize_realtime",
        SanitizeRealtimeBlocking => "sanitize_realtime_blocking",
        SanitizeThread => "sanitize_thread",
        SanitizeType => "sanitize_type",
        ShadowCallStack => "shadowcallstack",
        SignExt => "signext",
        SkipProfile => "skipprofile",
        Speculatable => "speculatable",
        SpeculativeLoadHardening => "speculative_load_hardening",
        Ssp => "ssp",
        SspReq => "sspreq",
        SspStrong => "sspstrong",
        StrictFp => "strictfp",
        SwiftAsync => "swiftasync",
        SwiftError => "swifterror",
        SwiftSelf => "swiftself",
        UwTable => "uwtable",
        Writable => "writable",
        WriteOnly => "writeonly",
        WillReturn => "willreturn",
        ZeroExt => "zeroext",
    }
}

define_keyword_enum! {
    /// Attributes taking one or two integer arguments.
    IntAttr {
        Align => "align",
        AllocSize => "allocsize",
        AlignStack => "alignstack",
        Dereferenceable => "dereferenceable",
        DereferenceableOrNull => "dereferenceable_or_null",
        VScaleRange => "vscale_range",
    }
}

define_keyword_enum! {
    /// Attributes taking a type argument.
    TypeAttr {
        ByRef => "byref",
        ByVal => "byval",
        ElementType => "elementtype",
        InAlloca => "inalloca",
        Preallocated => "preallocated",
        StructRet => "sret",
    }
}

define_keyword_enum! {
    /// Attributes whose arguments have a grammar of their own, carried as
    /// text until something needs to read them.
    StructuredAttr {
        AllocKind => "allockind",
        Captures => "captures",
        Initializes => "initializes",
        Memory => "memory",
        NoFpClass => "nofpclass",
        UwTable => "uwtable",
    }
}

/// Which assembler syntax an inline assembly string is written in.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum AsmDialect {
    #[default]
    Att,
    Intel,
}

/// One attribute.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Attribute {
    Enum(EnumAttr),
    Int {
        kind: IntAttr,
        first: u64,
        second: Option<u64>,
    },
    Type {
        kind: TypeAttr,
        ty: TypeId,
    },
    /// `range(i8 0, 8)`: a half-open interval the value stays inside.
    Range {
        ty: TypeId,
        lower: ApInt,
        upper: ApInt,
    },
    Structured {
        kind: StructuredAttr,
        arguments: String,
    },
    /// `"target-cpu"="x86-64"`, or a bare `"probe-stack"`.
    String {
        key: String,
        value: Option<String>,
    },
}

impl Attribute {
    /// True for attributes that go before the type in a parameter list, which
    /// is all of them; kept as a named predicate so the printer reads clearly.
    pub fn is_parameter_attribute(&self) -> bool {
        !matches!(self, Attribute::String { .. })
    }
}

/// The attributes attached to one thing, plus any attribute groups it refers
/// to by number.
///
/// Upstream hoists function attributes into numbered groups and leaves
/// parameter attributes inline, so both halves have to survive a round trip
/// separately. Group contents live in the module, not here.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct AttributeSet {
    pub attributes: Vec<Attribute>,
    pub groups: Vec<u32>,
}

impl AttributeSet {
    pub fn is_empty(&self) -> bool {
        self.attributes.is_empty() && self.groups.is_empty()
    }

    pub fn has(&self, attribute: EnumAttr) -> bool {
        self.attributes
            .iter()
            .any(|a| matches!(a, Attribute::Enum(e) if *e == attribute))
    }

    pub fn get_int(&self, kind: IntAttr) -> Option<u64> {
        self.attributes.iter().find_map(|a| match a {
            Attribute::Int { kind: k, first, .. } if *k == kind => Some(*first),
            _ => None,
        })
    }

    pub fn push(&mut self, attribute: Attribute) {
        self.attributes.push(attribute);
    }
}

/// Whether a word is the name of an attribute at all, in any of the four
/// shapes one comes in. A quoted key such as `frame-pointer` is not: it is
/// carried rather than named.
pub fn names_an_attribute(word: &str) -> bool {
    EnumAttr::from_keyword(word).is_some()
        || IntAttr::from_keyword(word).is_some()
        || TypeAttr::from_keyword(word).is_some()
        || StructuredAttr::from_keyword(word).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keywords_round_trip() {
        for keyword in ["nounwind", "noreturn", "willreturn", "zeroext", "uwtable"] {
            let attribute = EnumAttr::from_keyword(keyword).unwrap();
            assert_eq!(attribute.keyword(), keyword);
        }
        assert_eq!(EnumAttr::from_keyword("not-an-attribute"), None);
        assert_eq!(IntAttr::from_keyword("align"), Some(IntAttr::Align));
        assert_eq!(TypeAttr::from_keyword("sret"), Some(TypeAttr::StructRet));
        assert_eq!(
            StructuredAttr::from_keyword("memory"),
            Some(StructuredAttr::Memory)
        );
    }

    #[test]
    fn uwtable_is_both_bare_and_parenthesised() {
        // `uwtable` alone is a keyword; `uwtable(async)` has an argument. Both
        // spellings appear in real output, so both have to exist.
        assert!(EnumAttr::from_keyword("uwtable").is_some());
        assert!(StructuredAttr::from_keyword("uwtable").is_some());
    }

    #[test]
    fn sets_answer_questions_about_their_contents() {
        let mut set = AttributeSet::default();
        assert!(set.is_empty());
        set.push(Attribute::Enum(EnumAttr::NoUnwind));
        set.push(Attribute::Int {
            kind: IntAttr::Align,
            first: 8,
            second: None,
        });
        assert!(!set.is_empty());
        assert!(set.has(EnumAttr::NoUnwind));
        assert!(!set.has(EnumAttr::NoReturn));
        assert_eq!(set.get_int(IntAttr::Align), Some(8));
        assert_eq!(set.get_int(IntAttr::Dereferenceable), None);
    }
}
