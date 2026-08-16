//! Metadata nodes.
//!
//! Metadata is modelled syntactically. A `!DISubprogram(...)` keeps its tag
//! and its fields as written instead of becoming a typed debug-info object,
//! because DWARF modelling belongs in the debug-info crate at T1 and until
//! then a faithful syntactic node round-trips while a half-modelled one loses
//! whatever field it did not know about.
//!
//! Node numbering is the number in the text: `!7` parses to `MdId(7)` and
//! prints as `!7`. Upstream renumbers metadata on output, so a hand-written
//! module with sparse numbers prints differently through us than through
//! `llvm-dis`; canonical input, which is what the corpus holds, is unaffected.

/// The DWARF vocabulary a specialized node's word-valued fields take.
pub mod dwarf;
pub mod expression;

use crate::ByteString;
use crate::types::TypeId;
use crate::value::{MdId, Value};

/// An operand of a metadata tuple.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum MdOperand {
    /// A literal `null` operand, which tuples are allowed to hold.
    Null,
    Ref(MdId),
    /// An inline `!"text"`.
    String(ByteString),
    /// A typed value, as in `!{i32 1}`.
    Value {
        ty: TypeId,
        value: Value,
    },
    /// A node written in place rather than referred to, which is how
    /// `!DIExpression()` appears inside a debug record.
    Inline(Box<Metadata>),
}

/// A field of a specialized node, such as `line: 42` or `scope: !3`.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum MdField {
    /// Wide enough for a `DIEnumerator` value, which is an arbitrary integer
    /// and routinely exceeds 64 bits.
    Unsigned(u128),
    Signed(i128),
    /// A number too wide for 128 bits, kept as written. `DIEnumerator` values
    /// are arbitrary-precision, and this dialect prints them back rather than
    /// reading them, the way it does with a data layout string.
    BigInt {
        negative: bool,
        digits: String,
    },
    Bool(bool),
    /// A quoted string field, such as a name or a filename. Bytes rather
    /// than characters, because a filename need not be UTF-8.
    Str(ByteString),
    Ref(MdId),
    Null,
    /// One or more bare words, which is how enumerators and flag sets are
    /// written: `DW_TAG_structure_type`, or `DIFlagPublic | DIFlagStaticMember`.
    Words(Vec<String>),
    /// A typed value, which appears in `DIArgList` and in the value fields of
    /// debug records.
    Value {
        ty: TypeId,
        value: Value,
    },
    /// A node written in place, as `!DIExpression()` is inside a field.
    Inline(Box<Metadata>),
}

/// How a specialized node's arguments are written.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum SpecializedArgs {
    /// `!DILocation(line: 1, scope: !2)`.
    Named(Vec<(String, MdField)>),
    /// `!DIExpression(DW_OP_deref)`, which is positional.
    Positional(Vec<MdField>),
}

impl SpecializedArgs {
    /// The values, whichever way they are written.
    pub fn fields(&self) -> impl Iterator<Item = &MdField> {
        let named: &[(String, MdField)] = match self {
            SpecializedArgs::Named(fields) => fields,
            SpecializedArgs::Positional(_) => &[],
        };
        let positional: &[MdField] = match self {
            SpecializedArgs::Named(_) => &[],
            SpecializedArgs::Positional(values) => values,
        };
        named
            .iter()
            .map(|(_, value)| value)
            .chain(positional.iter())
    }
}

/// A metadata node definition.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Metadata {
    /// `!0 = !"text"`.
    String(ByteString),
    /// `!0 = !{...}` or `!0 = distinct !{...}`.
    Tuple {
        distinct: bool,
        operands: Vec<MdOperand>,
    },
    /// `!0 = !DILocation(...)`.
    Specialized {
        distinct: bool,
        tag: String,
        args: SpecializedArgs,
    },
}

impl Metadata {
    pub fn is_distinct(&self) -> bool {
        match self {
            Metadata::String(_) => false,
            Metadata::Tuple { distinct, .. } | Metadata::Specialized { distinct, .. } => *distinct,
        }
    }

    /// The tag and identifier of a `DICompositeType` that carries one, which
    /// is the key upstream uniques it under rather than its contents.
    ///
    /// A type with an identifier is one the language gives a single
    /// definition across every translation unit, so upstream keeps one node
    /// per identifier and makes it `distinct` so that nothing merges two
    /// definitions that only happen to look alike. Measured one module at a
    /// time: with an identifier it comes back `distinct`, without one it does
    /// not, and `identifier: ""` comes back with the field gone and the node
    /// ordinary. Every tag behaves the same way, class, structure,
    /// enumeration, union, array and variant part alike.
    ///
    /// The tag is part of the key. Two nodes sharing an identifier merge when
    /// their tags agree, and when the tags differ the second is left where it
    /// is and not even made distinct, which is the one case where an
    /// identifier buys nothing.
    /// What a member of an ODR type is merged on: the scope it belongs to,
    /// and a key made of what upstream compares.
    ///
    /// A type that gives itself an identifier has one definition across every
    /// translation unit, and so do its members: a second member of the same
    /// scope under the same key is the same member however much else differs.
    /// Measured a field at a time, `file:`, `line:` and `size:` all turn out
    /// not to matter, where the key does.
    ///
    /// The key is the tag and the name for a `DIDerivedType` and the linkage
    /// name for a `DISubprogram`, which is measured rather than symmetric:
    /// two subprograms with different names and one linkage name merge, and
    /// two with one name and no linkage name do not. A node with no key
    /// merges with nothing, and no other kind is merged at all: a nested
    /// composite type has its own identifier rule, and an enumerator or a
    /// template parameter has no scope to be a member of.
    pub fn odr_member_key(&self) -> Option<(MdId, String)> {
        let Metadata::Specialized { tag, args, .. } = self else {
            return None;
        };
        let SpecializedArgs::Named(fields) = args else {
            return None;
        };
        let text = |wanted: &str| {
            fields.iter().find_map(|(name, value)| match value {
                MdField::Str(text) if name == wanted => text.as_str(),
                _ => None,
            })
        };
        let scope = fields.iter().find_map(|(name, value)| match value {
            MdField::Ref(id) if name == "scope" => Some(*id),
            _ => None,
        })?;
        let key = match tag.as_str() {
            "DIDerivedType" => {
                let number = fields.iter().find_map(|(name, value)| match value {
                    MdField::Unsigned(number) if name == "tag" => Some(*number),
                    _ => None,
                })?;
                format!("DIDerivedType {number} {}", text("name")?)
            }
            "DISubprogram" => format!("DISubprogram {}", text("linkageName")?),
            _ => return None,
        };
        Some((scope, key))
    }

    pub fn odr_key(&self) -> Option<(u128, &str)> {
        let Metadata::Specialized { tag, args, .. } = self else {
            return None;
        };
        if tag != "DICompositeType" {
            return None;
        }
        let SpecializedArgs::Named(fields) = args else {
            return None;
        };
        let identifier = fields.iter().find_map(|(name, value)| match value {
            MdField::Str(text) if name == "identifier" => text.as_str(),
            _ => None,
        })?;
        if identifier.is_empty() {
            return None;
        }
        // The tag is held as the number its word stands for, so two nodes
        // that spell it differently key the same way, which is the same
        // reason they unique the same way.
        let tag = fields.iter().find_map(|(name, value)| match value {
            MdField::Unsigned(number) if name == "tag" => Some(*number),
            _ => None,
        })?;
        Some((tag, identifier))
    }

    pub fn as_tuple(&self) -> Option<&[MdOperand]> {
        match self {
            Metadata::Tuple { operands, .. } => Some(operands),
            _ => None,
        }
    }

    /// The nodes this one names by number, which is what a walk over the
    /// graph follows. A node written in place is part of this one rather
    /// than a reference to another, so it contributes nothing.
    pub fn references(&self) -> Vec<MdId> {
        match self {
            Metadata::String(_) => Vec::new(),
            Metadata::Tuple { operands, .. } => operands
                .iter()
                .filter_map(|operand| match operand {
                    MdOperand::Ref(id) => Some(*id),
                    _ => None,
                })
                .collect(),
            Metadata::Specialized { args, .. } => args
                .fields()
                .filter_map(|field| match field {
                    MdField::Ref(id) => Some(*id),
                    _ => None,
                })
                .collect(),
        }
    }

    /// The value of a named field, for a node that has named fields.
    pub fn field(&self, wanted: &str) -> Option<&MdField> {
        let Metadata::Specialized {
            args: SpecializedArgs::Named(fields),
            ..
        } = self
        else {
            return None;
        };
        fields
            .iter()
            .find_map(|(name, value)| (name == wanted).then_some(value))
    }

    pub fn as_string(&self) -> Option<&str> {
        match self {
            Metadata::String(text) => text.as_str(),
            _ => None,
        }
    }
}

/// `!llvm.module.flags = !{!0, !1}`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct NamedMetadata {
    /// The name without its leading `!`. Bytes, because upstream escapes
    /// any byte into one with `\\FF`.
    pub name: ByteString,
    pub operands: Vec<MdId>,
}

/// A node an attachment or an operand points at: a number, or a node written
/// in place. `!dbg !7` is the first; `!dbg !DILocation(scope: !1)` is the
/// second, and upstream accepts both.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum MdRef {
    Id(MdId),
    Inline(Box<Metadata>),
}

/// A `!kind !node` attachment on an instruction, global or function.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MdAttachment {
    /// The kind name without its leading `!`, such as `dbg` or `range`.
    pub kind: ByteString,
    pub node: MdRef,
}

/// The order upstream writes a specialized node's fields in, which is
/// neither the order they were read nor the order the grammar lists them:
/// a `!DIBasicType` writes `flags` after `num_extra_inhabitants` and the
/// grammar has it before. Generated by `corpus/md-field-order.nu`. A field
/// this does not name is written after the ones it does, in the order it
/// was read; the only one is a `!DIStringType`'s tag, which never survives
/// its default.
///
/// Sorted by node kind, so the lookup can be a binary search.
pub static FIELD_ORDER: &[(&str, &[&str])] = &[
    (
        "DIBasicType",
        &[
            "tag",
            "name",
            "size",
            "align",
            "encoding",
            "num_extra_inhabitants",
            "flags",
        ],
    ),
    (
        "DICompileUnit",
        &[
            "language",
            "file",
            "producer",
            "isOptimized",
            "flags",
            "runtimeVersion",
            "splitDebugFilename",
            "emissionKind",
            "enums",
            "retainedTypes",
            "globals",
            "imports",
            "macros",
            "dwoId",
            "splitDebugInlining",
            "debugInfoForProfiling",
            "nameTableKind",
            "rangesBaseAddress",
            "sysroot",
            "sdk",
        ],
    ),
    (
        "DICompositeType",
        &[
            "tag",
            "name",
            "scope",
            "file",
            "line",
            // After `line`, which took a third probe to learn: the structure
            // probe carries no `baseType` and the array probe no `file`, so
            // between them nothing said where it goes and it sat next to
            // `name`. An enumeration carries both.
            "baseType",
            "size",
            "align",
            "offset",
            // Between `offset` and `flags`, which neither of the two probes
            // that kind takes was carrying: a fourth probe says so.
            "num_extra_inhabitants",
            "flags",
            "elements",
            "runtimeLang",
            "vtableHolder",
            "templateParams",
            "identifier",
            "dataLocation",
            "associated",
            "allocated",
            "rank",
            "annotations",
            "specification",
            "enumKind",
            "bitStride",
        ],
    ),
    (
        "DIDerivedType",
        &[
            "tag",
            "name",
            "scope",
            "file",
            "line",
            "baseType",
            "size",
            "align",
            "offset",
            "flags",
            "extraData",
            "dwarfAddressSpace",
            "annotations",
            "ptrAuthKey",
            "ptrAuthIsAddressDiscriminated",
            "ptrAuthExtraDiscriminator",
            "ptrAuthIsaPointer",
            "ptrAuthAuthenticatesNullValues",
        ],
    ),
    ("DIEnumerator", &["name", "value", "isUnsigned"]),
    (
        "DIFile",
        &[
            "filename",
            "directory",
            "checksumkind",
            "checksum",
            "source",
        ],
    ),
    (
        "DIGlobalVariable",
        &[
            "name",
            "linkageName",
            "scope",
            "file",
            "line",
            "type",
            "isLocal",
            "isDefinition",
            "declaration",
            "templateParams",
            "align",
            "annotations",
        ],
    ),
    (
        "DIImportedEntity",
        &["tag", "name", "scope", "entity", "file", "line", "elements"],
    ),
    (
        "DILabel",
        &[
            "scope",
            "name",
            "file",
            "line",
            "column",
            "isArtificial",
            "coroSuspendIdx",
        ],
    ),
    ("DILexicalBlock", &["scope", "file", "line", "column"]),
    ("DILexicalBlockFile", &["scope", "file", "discriminator"]),
    (
        "DILocalVariable",
        &[
            "name",
            "arg",
            "scope",
            "file",
            "line",
            "type",
            "flags",
            "align",
            "annotations",
        ],
    ),
    (
        "DILocation",
        &[
            "line",
            "column",
            "scope",
            "inlinedAt",
            "isImplicitCode",
            "atomGroup",
            "atomRank",
        ],
    ),
    ("DIMacro", &["type", "line", "name", "value"]),
    ("DIMacroFile", &["line", "file", "nodes"]),
    (
        "DIModule",
        &[
            "scope",
            "name",
            "configMacros",
            "includePath",
            "apinotes",
            "file",
            "line",
            "isDecl",
        ],
    ),
    ("DINamespace", &["name", "scope", "exportSymbols"]),
    (
        "DIObjCProperty",
        &[
            "name",
            "file",
            "line",
            "setter",
            "getter",
            "attributes",
            "type",
        ],
    ),
    (
        "DIStringType",
        &[
            "name",
            "stringLength",
            "stringLengthExpression",
            "stringLocationExpression",
            "size",
            "align",
            "encoding",
        ],
    ),
    (
        "DISubprogram",
        &[
            "name",
            "linkageName",
            "scope",
            "file",
            "line",
            "type",
            "scopeLine",
            "containingType",
            "virtualIndex",
            "thisAdjustment",
            "flags",
            "spFlags",
            "unit",
            "templateParams",
            "declaration",
            "retainedNodes",
            "thrownTypes",
            "annotations",
            "targetFuncName",
            "keyInstructions",
        ],
    ),
    // `upperBound` between the other two, which took a second probe: a
    // subrange is described from one end or the other and never both, so the
    // probe carrying `count` cannot carry `upperBound` and nothing said
    // where it goes.
    (
        "DISubrange",
        &["count", "lowerBound", "upperBound", "stride"],
    ),
    ("DISubroutineType", &["flags", "cc", "types"]),
    ("DITemplateTypeParameter", &["name", "type", "defaulted"]),
    (
        "DITemplateValueParameter",
        &["tag", "name", "type", "defaulted", "value"],
    ),
];

/// The fields a node keeps at nought and does not write back, measured by
/// `corpus/md-field-defaults.nu`. A size or an offset is held as an operand,
/// so nought is a size where nothing is not, and `!DIBasicType()` and
/// `!DIBasicType(size: 0)` print the same and are two nodes.
static STORED_AT_ZERO: &[(&str, &str)] = &[
    ("DIBasicType", "size"),
    ("DICompositeType", "offset"),
    ("DICompositeType", "size"),
    ("DIDerivedType", "offset"),
    ("DIDerivedType", "size"),
    ("DIStringType", "size"),
];

/// The words a field takes, when it takes words. A field the table does not
/// name holds a number or something else entirely.
pub fn vocabulary(tag: &str, field: &str) -> Option<&'static [(u64, &'static str)]> {
    match field {
        "tag" => Some(dwarf::TAG),
        "encoding" => Some(dwarf::ENCODING),
        "language" => Some(dwarf::LANGUAGE),
        // A composite type's `runtimeLang` is a language too, and takes the
        // same words. It was missing here, so a number written for it came
        // back a number where upstream writes the word: `runtimeLang: 6` is
        // `runtimeLang: DW_LANG_Cobol85`.
        "runtimeLang" => Some(dwarf::LANGUAGE),
        "emissionKind" => Some(dwarf::EMISSIONKIND),
        "nameTableKind" => Some(dwarf::NAMETABLEKIND),
        "virtuality" => Some(dwarf::VIRTUALITY),
        "cc" => Some(dwarf::CC),
        "checksumkind" => Some(dwarf::CHECKSUMKIND),
        // `type:` is a macinfo kind on a macro and a node reference on the
        // four kinds that name what something is, which is why the node kind
        // has to be asked as well as the field.
        "type" if tag == "DIMacro" => Some(dwarf::TYPE),
        _ => None,
    }
}

/// What upstream calls a field's vocabulary when it refuses a word that is
/// not in it, and nothing for the three whose tables are not complete.
///
/// `nameTableKind: Default`, `virtuality: DW_VIRTUALITY_none` and
/// `checksumkind: CSK_MD5` are words upstream takes and the sweep cannot
/// learn: a value equal to a field's own default never prints, and the two
/// probes that would show the others are refused for reasons of their own.
/// So those three print a number as a word and refuse nothing.
pub fn vocabulary_name(tag: &str, field: &str) -> Option<&'static str> {
    match field {
        "tag" => Some("DWARF tag"),
        // Both of these are languages and upstream refuses a word neither
        // knows with the same message, "invalid DWARF language '...'".
        "language" | "runtimeLang" => Some("DWARF language"),
        "encoding" => Some("DWARF type attribute encoding"),
        "cc" => Some("DWARF calling convention"),
        "emissionKind" => Some("emission kind"),
        "type" if tag == "DIMacro" => Some("macinfo type"),
        _ => None,
    }
}

/// The number a word stands for, in the vocabulary a field takes.
pub fn number(tag: &str, field: &str, word: &str) -> Option<u64> {
    vocabulary(tag, field)?
        .iter()
        .find(|(_, name)| *name == word)
        .map(|(value, _)| *value)
}

/// Whether a field is one of those: written back only when it is not nought,
/// and kept in the node either way.
pub fn stored_at_zero(tag: &str, field: &str) -> bool {
    STORED_AT_ZERO.binary_search(&(tag, field)).is_ok()
}

/// Where a field sits in the order its node kind writes them. A field the
/// table does not name sorts after the ones it does.
pub fn field_rank(tag: &str, field: &str) -> usize {
    let Ok(index) = FIELD_ORDER.binary_search_by_key(&tag, |(kind, _)| *kind) else {
        return usize::MAX;
    };
    let order = FIELD_ORDER[index].1;
    order
        .iter()
        .position(|name| *name == field)
        .unwrap_or(order.len())
}
