//! The grammar of specialized metadata nodes.
//!
//! `!DILocation(line: 1, scope: !2)` is not a free-form keyed list: each node
//! kind has a fixed set of field names, some of them required, some of them
//! forbidden from being null, and several with a numeric range. Upstream
//! enforces all of that in its parser rather than its verifier, so a module
//! that gets it wrong is a parse error and not a verifier diagnostic.
//!
//! We model debug info syntactically rather than semantically (see
//! `docs/dialect-notes.md`), and this table is what keeps that from meaning
//! "anything goes". It says nothing about what a field *means*; DWARF
//! modelling is `llvm-debuginfo`'s job at T1.
//!
//! The field names come from upstream's own `.ll` tests and from LangRef,
//! which the project rules name as specifications. Being generous with a name
//! is safe and being stingy is not: an unknown-but-valid name would reject IR
//! upstream accepts, and the conformance ratchet would catch it. What is not
//! modelled here is the DWARF vocabulary itself, because there is no
//! specification in the tree that enumerates every `DW_TAG_*`, so a word in a
//! word-valued field is taken as written.

/// What a field may hold. Most fields are `Any`, which checks nothing: a
/// shape is here only where upstream's own tests show a rule.
#[derive(Clone, Copy)]
pub(crate) enum Shape {
    /// Anything the field grammar can produce.
    Any,
    /// An unsigned integer no larger than the limit, as `arg:` and `column:`
    /// are. A negative number is an error before the limit is considered.
    Unsigned(u64),
    /// A word, or the unsigned number behind one, no larger than the limit.
    /// The description names what a non-numeric, non-word value should have
    /// been: `expected DWARF tag`.
    Enumerator(u64, &'static str),
    /// Like `Enumerator`, but upstream reports the overflow without naming a
    /// limit, which `emissionKind` is the only case of.
    SmallEnumerator(u64),
    /// An integer in an inclusive range, which may instead be a node
    /// reference or an expression written in place.
    Bounded(i128, i128),
}

/// One field of one node kind.
pub(crate) struct Field {
    pub(crate) name: &'static str,
    pub(crate) shape: Shape,
    /// The node cannot be written without this field.
    pub(crate) required: bool,
    /// `null` is not one of the values this field may take.
    pub(crate) non_null: bool,
    /// The empty string is not one of the values this field may take.
    pub(crate) non_empty: bool,
}

/// When a node kind has to be written `distinct`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Distinct {
    /// Either spelling is fine.
    Optional,
    /// Always, as `!DICompileUnit` is.
    Always,
    /// When the node describes a definition, as `!DISubprogram` does when it
    /// carries `isDefinition: true` or `DISPFlagDefinition`.
    WhenDefinition,
}

pub(crate) struct Node {
    pub(crate) fields: &'static [Field],
    pub(crate) distinct: Distinct,
    /// A node written positionally, as `!DIExpression(DW_OP_deref)` is. Its
    /// arguments are not named, so the field table does not apply.
    pub(crate) positional: bool,
}

const fn field(name: &'static str) -> Field {
    Field {
        name,
        shape: Shape::Any,
        required: false,
        non_null: false,
        non_empty: false,
    }
}

const fn required(name: &'static str) -> Field {
    Field {
        required: true,
        ..field(name)
    }
}

const fn shaped(name: &'static str, shape: Shape) -> Field {
    Field {
        shape,
        ..field(name)
    }
}

const fn scope() -> Field {
    Field {
        required: true,
        non_null: true,
        ..field("scope")
    }
}

/// The table. A node kind that is not here is not a metadata node.
///
/// A `static` rather than a match returning references, because a reference
/// to a value built in a function body does not outlive the call.
static TABLE: &[(&str, Node)] = &[
    (
        "DIBasicType",
        Node {
            fields: &[
                field("tag"),
                field("name"),
                field("size"),
                field("align"),
                field("encoding"),
                field("flags"),
                field("num_extra_inhabitants"),
            ],
            distinct: Distinct::Optional,
            positional: false,
        },
    ),
    (
        "DICommonBlock",
        Node {
            fields: &[
                field("scope"),
                field("declaration"),
                field("name"),
                field("file"),
                field("line"),
            ],
            distinct: Distinct::Optional,
            positional: false,
        },
    ),
    (
        "DICompileUnit",
        Node {
            fields: &[
                Field {
                    shape: Shape::Enumerator(u16::MAX as u64, "DWARF language"),
                    required: true,
                    ..field("language")
                },
                Field {
                    required: true,
                    non_null: true,
                    ..field("file")
                },
                field("producer"),
                field("isOptimized"),
                field("flags"),
                field("runtimeVersion"),
                field("splitDebugFilename"),
                // Four kinds: no debug info, full, line tables only, and
                // directives only. Upstream's tests use every word and the
                // numbers behind them, and nothing above 3.
                shaped("emissionKind", Shape::SmallEnumerator(3)),
                field("enums"),
                field("retainedTypes"),
                field("globals"),
                field("imports"),
                field("macros"),
                field("dwoId"),
                field("splitDebugInlining"),
                field("debugInfoForProfiling"),
                field("nameTableKind"),
                field("rangesBaseAddress"),
                field("sysroot"),
                field("sdk"),
                field("subprograms"),
            ],
            distinct: Distinct::Always,
            positional: false,
        },
    ),
    (
        "DICompositeType",
        Node {
            fields: &[
                required("tag"),
                field("name"),
                field("file"),
                field("line"),
                field("scope"),
                field("baseType"),
                field("size"),
                field("align"),
                field("offset"),
                field("flags"),
                field("elements"),
                field("runtimeLang"),
                field("vtableHolder"),
                field("templateParams"),
                field("identifier"),
                field("discriminator"),
                field("dataLocation"),
                field("associated"),
                field("allocated"),
                field("rank"),
                field("annotations"),
                field("num_extra_inhabitants"),
                field("specification"),
                field("enumKind"),
                field("bitStride"),
            ],
            distinct: Distinct::Optional,
            positional: false,
        },
    ),
    (
        "DIDerivedType",
        Node {
            fields: &[
                required("tag"),
                required("baseType"),
                field("name"),
                field("file"),
                field("line"),
                field("scope"),
                field("size"),
                field("align"),
                field("offset"),
                field("flags"),
                field("extraData"),
                field("dwarfAddressSpace"),
                field("annotations"),
                field("ptrAuthKey"),
                field("ptrAuthIsAddressDiscriminated"),
                field("ptrAuthExtraDiscriminator"),
                field("ptrAuthIsaPointer"),
                field("ptrAuthAuthenticatesNullValues"),
            ],
            distinct: Distinct::Optional,
            positional: false,
        },
    ),
    (
        "DIEnumerator",
        Node {
            fields: &[required("name"), required("value"), field("isUnsigned")],
            distinct: Distinct::Optional,
            positional: false,
        },
    ),
    (
        "DIFile",
        Node {
            fields: &[
                required("filename"),
                required("directory"),
                field("checksumkind"),
                field("checksum"),
                field("source"),
            ],
            distinct: Distinct::Optional,
            positional: false,
        },
    ),
    (
        "DIFixedPointType",
        Node {
            fields: &[
                field("tag"),
                field("name"),
                field("size"),
                field("align"),
                field("encoding"),
                field("flags"),
                field("kind"),
                field("factor"),
                field("numerator"),
                field("denominator"),
            ],
            distinct: Distinct::Optional,
            positional: false,
        },
    ),
    (
        "DIGenericSubrange",
        Node {
            fields: &[
                field("count"),
                field("lowerBound"),
                field("upperBound"),
                field("stride"),
            ],
            distinct: Distinct::Optional,
            positional: false,
        },
    ),
    (
        "DIGlobalVariable",
        Node {
            fields: &[
                Field {
                    non_empty: true,
                    ..field("name")
                },
                field("scope"),
                field("linkageName"),
                field("file"),
                field("line"),
                field("type"),
                field("isLocal"),
                field("isDefinition"),
                field("templateParams"),
                field("declaration"),
                field("align"),
                field("annotations"),
            ],
            distinct: Distinct::Optional,
            positional: false,
        },
    ),
    (
        "DIGlobalVariableExpression",
        Node {
            fields: &[required("var"), required("expr")],
            distinct: Distinct::Optional,
            positional: false,
        },
    ),
    (
        "DIImportedEntity",
        Node {
            fields: &[
                required("tag"),
                required("scope"),
                field("entity"),
                field("file"),
                field("line"),
                field("name"),
                field("elements"),
            ],
            distinct: Distinct::Optional,
            positional: false,
        },
    ),
    (
        "DILabel",
        Node {
            fields: &[
                field("scope"),
                field("name"),
                field("file"),
                field("line"),
                field("column"),
                field("isArtificial"),
                field("coroSuspendIdx"),
            ],
            distinct: Distinct::Optional,
            positional: false,
        },
    ),
    (
        "DILexicalBlock",
        Node {
            fields: &[scope(), field("file"), field("line"), field("column")],
            distinct: Distinct::Optional,
            positional: false,
        },
    ),
    (
        "DILexicalBlockFile",
        Node {
            fields: &[scope(), field("file"), required("discriminator")],
            distinct: Distinct::Optional,
            positional: false,
        },
    ),
    (
        "DILocalVariable",
        Node {
            fields: &[
                scope(),
                field("name"),
                shaped("arg", Shape::Unsigned(u16::MAX as u64)),
                field("file"),
                field("line"),
                field("type"),
                field("flags"),
                field("align"),
                field("annotations"),
                field("tag"),
            ],
            distinct: Distinct::Optional,
            positional: false,
        },
    ),
    (
        "DILocation",
        Node {
            fields: &[
                shaped("line", Shape::Unsigned(u32::MAX as u64)),
                shaped("column", Shape::Unsigned(u16::MAX as u64)),
                scope(),
                field("inlinedAt"),
                field("isImplicitCode"),
                field("atomGroup"),
                field("atomRank"),
            ],
            distinct: Distinct::Optional,
            positional: false,
        },
    ),
    (
        "DIMacro",
        Node {
            fields: &[
                required("type"),
                field("line"),
                required("name"),
                field("value"),
            ],
            distinct: Distinct::Optional,
            positional: false,
        },
    ),
    (
        "DIMacroFile",
        Node {
            fields: &[field("type"), field("line"), field("file"), field("nodes")],
            distinct: Distinct::Optional,
            positional: false,
        },
    ),
    (
        "DIModule",
        Node {
            fields: &[
                required("scope"),
                required("name"),
                field("configMacros"),
                field("includePath"),
                field("apinotes"),
                field("file"),
                field("line"),
                field("isDecl"),
            ],
            distinct: Distinct::Optional,
            positional: false,
        },
    ),
    (
        "DINamespace",
        Node {
            fields: &[
                required("scope"),
                field("name"),
                field("exportSymbols"),
                field("file"),
                field("line"),
            ],
            distinct: Distinct::Optional,
            positional: false,
        },
    ),
    (
        "DIObjCProperty",
        Node {
            fields: &[
                field("name"),
                field("file"),
                field("line"),
                field("setter"),
                field("getter"),
                field("attributes"),
                field("type"),
            ],
            distinct: Distinct::Optional,
            positional: false,
        },
    ),
    (
        "DIStringType",
        Node {
            fields: &[
                field("tag"),
                field("name"),
                field("stringLength"),
                field("stringLengthExpression"),
                field("stringLocationExpression"),
                field("size"),
                field("align"),
                field("encoding"),
            ],
            distinct: Distinct::Optional,
            positional: false,
        },
    ),
    (
        "DISubprogram",
        Node {
            fields: &[
                field("scope"),
                field("name"),
                field("linkageName"),
                field("file"),
                field("line"),
                field("type"),
                field("isLocal"),
                field("isDefinition"),
                field("scopeLine"),
                field("containingType"),
                field("virtuality"),
                field("virtualIndex"),
                field("thisAdjustment"),
                field("flags"),
                field("spFlags"),
                field("isOptimized"),
                field("unit"),
                field("templateParams"),
                field("declaration"),
                field("retainedNodes"),
                field("thrownTypes"),
                field("annotations"),
                field("targetFuncName"),
                field("keyInstructions"),
            ],
            distinct: Distinct::WhenDefinition,
            positional: false,
        },
    ),
    (
        "DISubrange",
        Node {
            fields: &[
                Field {
                    shape: Shape::Bounded(-1, i64::MAX as i128),
                    non_null: true,
                    ..field("count")
                },
                shaped(
                    "lowerBound",
                    Shape::Bounded(i64::MIN as i128, i64::MAX as i128),
                ),
                shaped(
                    "upperBound",
                    Shape::Bounded(i64::MIN as i128, i64::MAX as i128),
                ),
                shaped("stride", Shape::Bounded(i64::MIN as i128, i64::MAX as i128)),
            ],
            distinct: Distinct::Optional,
            positional: false,
        },
    ),
    (
        "DISubrangeType",
        Node {
            fields: &[
                field("name"),
                field("scope"),
                field("file"),
                field("line"),
                field("size"),
                field("align"),
                field("flags"),
                field("baseType"),
                field("lowerBound"),
                field("upperBound"),
                field("stride"),
                field("bias"),
            ],
            distinct: Distinct::Optional,
            positional: false,
        },
    ),
    (
        "DISubroutineType",
        Node {
            fields: &[required("types"), field("flags"), field("cc")],
            distinct: Distinct::Optional,
            positional: false,
        },
    ),
    (
        "DITemplateTypeParameter",
        Node {
            fields: &[field("name"), required("type"), field("defaulted")],
            distinct: Distinct::Optional,
            positional: false,
        },
    ),
    (
        "DITemplateValueParameter",
        Node {
            fields: &[
                field("tag"),
                field("name"),
                field("type"),
                required("value"),
                field("defaulted"),
            ],
            distinct: Distinct::Optional,
            positional: false,
        },
    ),
    (
        "GenericDINode",
        Node {
            fields: &[
                Field {
                    shape: Shape::Enumerator(u16::MAX as u64, "DWARF tag"),
                    required: true,
                    ..field("tag")
                },
                field("header"),
                field("operands"),
            ],
            distinct: Distinct::Optional,
            positional: false,
        },
    ),
    (
        "DIAssignID",
        Node {
            fields: &[],
            distinct: Distinct::Optional,
            positional: true,
        },
    ),
    (
        "DIExpression",
        Node {
            fields: &[],
            distinct: Distinct::Optional,
            positional: true,
        },
    ),
    (
        "DIArgList",
        Node {
            fields: &[],
            distinct: Distinct::Optional,
            positional: true,
        },
    ),
];

pub(crate) fn node(tag: &str) -> Option<&'static Node> {
    TABLE
        .iter()
        .find(|(name, _)| *name == tag)
        .map(|(_, node)| node)
}
