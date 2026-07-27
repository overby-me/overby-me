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

    pub fn as_tuple(&self) -> Option<&[MdOperand]> {
        match self {
            Metadata::Tuple { operands, .. } => Some(operands),
            _ => None,
        }
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
