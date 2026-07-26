//! Global variables, aliases and ifuncs, and the qualifiers they share with
//! functions.

use crate::attribute::AttributeSet;
use crate::constant::ConstId;
use crate::keyword::define_keyword_enum;
use crate::metadata::MdAttachment;
use crate::types::TypeId;
use crate::value::Name;
use llvm_support::Align;

define_keyword_enum! {
    /// How a symbol is linked.
    ///
    /// `External` is the default and prints as nothing, which is why it is
    /// last: everything before it is written out when present.
    Linkage {
        Private => "private",
        Internal => "internal",
        AvailableExternally => "available_externally",
        LinkOnce => "linkonce",
        Weak => "weak",
        Common => "common",
        Appending => "appending",
        ExternWeak => "extern_weak",
        LinkOnceOdr => "linkonce_odr",
        WeakOdr => "weak_odr",
        External => "external",
    }
}

define_keyword_enum! {
    /// ELF-style symbol visibility.
    Visibility {
        Default => "default",
        Hidden => "hidden",
        Protected => "protected",
    }
}

define_keyword_enum! {
    /// Windows storage class.
    DllStorageClass {
        Import => "dllimport",
        Export => "dllexport",
    }
}

define_keyword_enum! {
    /// Thread-local storage model.
    TlsModel {
        LocalDynamic => "localdynamic",
        InitialExec => "initialexec",
        LocalExec => "localexec",
        GeneralDynamic => "generaldynamic",
    }
}

define_keyword_enum! {
    /// Whether the address of a symbol is significant.
    UnnamedAddr {
        Local => "local_unnamed_addr",
        Global => "unnamed_addr",
    }
}

define_keyword_enum! {
    /// How a comdat's duplicate copies are resolved.
    ComdatKind {
        Any => "any",
        ExactMatch => "exactmatch",
        Largest => "largest",
        NoDeduplicate => "nodeduplicate",
        SameSize => "samesize",
    }
}

define_keyword_enum! {
    /// Whether a symbol is known to be defined in the linkage unit.
    RuntimePreemption {
        DsoLocal => "dso_local",
        DsoPreemptable => "dso_preemptable",
    }
}

/// The qualifiers every global-scope symbol can carry.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct GlobalQualifiers {
    pub linkage: Option<Linkage>,
    pub preemption: Option<RuntimePreemption>,
    pub visibility: Option<Visibility>,
    pub dll_storage: Option<DllStorageClass>,
    pub thread_local: Option<Option<TlsModel>>,
    pub unnamed_addr: Option<UnnamedAddr>,
    pub address_space: Option<u32>,
}

/// `$name = comdat any`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Comdat {
    pub name: String,
    pub kind: ComdatKind,
}

/// The `comdat` clause on a symbol: bare, or naming another comdat.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ComdatRef {
    pub name: Option<String>,
}

/// A global variable definition or declaration.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct GlobalVariable {
    pub name: Name,
    pub qualifiers: GlobalQualifiers,
    pub externally_initialized: bool,
    /// `constant` rather than `global`.
    pub is_constant: bool,
    /// The type of the value the symbol holds, not the pointer to it.
    pub value_type: TypeId,
    /// `None` for a declaration.
    pub initializer: Option<ConstId>,
    pub section: Option<String>,
    pub partition: Option<String>,
    pub comdat: Option<ComdatRef>,
    pub align: Option<Align>,
    /// `!dbg !0` and friends, and `#0` attribute groups, which globals accept
    /// as well as functions.
    pub metadata: Vec<MdAttachment>,
    pub attrs: AttributeSet,
    /// `!type !0` sanitizer metadata prints after the initialiser, but a
    /// `code_model` or similar sits before it; kept in source order.
    pub code_model: Option<String>,
    pub sanitizer: Option<String>,
}

/// `@a = alias i8, ptr @b`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Alias {
    pub name: Name,
    pub qualifiers: GlobalQualifiers,
    pub value_type: TypeId,
    pub aliasee: ConstId,
    pub partition: Option<String>,
    pub metadata: Vec<MdAttachment>,
}

/// `@a = ifunc i32 (i32), ptr @resolver`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct IFunc {
    pub name: Name,
    pub qualifiers: GlobalQualifiers,
    pub value_type: TypeId,
    pub resolver: ConstId,
    pub partition: Option<String>,
    pub metadata: Vec<MdAttachment>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qualifier_keywords_round_trip() {
        assert_eq!(Linkage::from_keyword("private"), Some(Linkage::Private));
        assert_eq!(Linkage::from_keyword("weak_odr"), Some(Linkage::WeakOdr));
        assert_eq!(Linkage::External.keyword(), "external");
        assert_eq!(Visibility::from_keyword("hidden"), Some(Visibility::Hidden));
        assert_eq!(
            DllStorageClass::from_keyword("dllexport"),
            Some(DllStorageClass::Export)
        );
        assert_eq!(
            TlsModel::from_keyword("localexec"),
            Some(TlsModel::LocalExec)
        );
        assert_eq!(
            UnnamedAddr::from_keyword("local_unnamed_addr"),
            Some(UnnamedAddr::Local)
        );
        assert_eq!(
            ComdatKind::from_keyword("largest"),
            Some(ComdatKind::Largest)
        );
        assert_eq!(
            RuntimePreemption::from_keyword("dso_local"),
            Some(RuntimePreemption::DsoLocal)
        );
        assert_eq!(Linkage::from_keyword("nonsense"), None);
    }
}
