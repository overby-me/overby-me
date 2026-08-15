//! Printing metadata nodes and attachments.

use std::fmt::Write as _;

use llvm_ir::Value;

use crate::{Printer, escape_bytes, metadata_name};
use llvm_ir::metadata::{MdAttachment, MdField, MdOperand, MdRef, Metadata, SpecializedArgs};

impl Printer<'_> {
    // -------------------------------------------------------------- metadata

    pub(crate) fn metadata_attachments(&mut self, attachments: &[MdAttachment], separator: &str) {
        // Upstream numbers each attachment kind as it first meets it and
        // writes them in that order rather than the order they were read, so
        // `!prof` comes before `!llvm.loop` however the module wrote them.
        let mut attachments: Vec<&MdAttachment> = attachments.iter().collect();
        attachments.sort_by_key(|attachment| attachment_rank(attachment));
        for attachment in attachments {
            let _ = write!(self.out, "{separator}!{} ", metadata_name(&attachment.kind));
            match &attachment.node {
                MdRef::Id(id) => self.metadata_reference(*id),
                MdRef::Inline(node) => self.metadata_definition(node),
            }
        }
    }

    pub(crate) fn metadata_definition(&mut self, node: &Metadata) {
        match node {
            Metadata::String(text) => {
                let _ = write!(self.out, "!\"{}\"", escape_bytes(text.as_bytes()));
            }
            Metadata::Tuple { distinct, operands } => {
                if *distinct {
                    self.push("distinct ");
                }
                self.push("!{");
                for (index, operand) in operands.iter().enumerate() {
                    if index > 0 {
                        self.push(", ");
                    }
                    self.metadata_operand(operand);
                }
                self.push("}");
            }
            Metadata::Specialized {
                distinct,
                tag,
                args,
            } => {
                if *distinct {
                    self.push("distinct ");
                }
                let _ = write!(self.out, "!{tag}(");
                match args {
                    SpecializedArgs::Named(fields) => {
                        // Upstream writes the fields in an order of its own
                        // rather than the one they were read in, so a module
                        // that wrote them differently still prints the same.
                        // A size or an offset is held even when it is nought,
                        // so that two nodes writing it differently stay two
                        // nodes, and is written back only when it is not.
                        let mut fields: Vec<_> = fields
                            .iter()
                            .filter(|(key, value)| {
                                !llvm_ir::metadata::stored_at_zero(tag, key)
                                    || !matches!(value, MdField::Unsigned(0) | MdField::Signed(0))
                            })
                            .collect();
                        fields.sort_by_key(|(key, _)| llvm_ir::metadata::field_rank(tag, key));
                        for (index, (key, value)) in fields.iter().enumerate() {
                            if index > 0 {
                                self.push(", ");
                            }
                            let _ = write!(self.out, "{key}: ");
                            // `operands:` is written with braces and no `!`,
                            // being the node's own operands rather than a
                            // reference to a node that holds them.
                            if key.as_str() == "operands"
                                && let MdField::Inline(node) = value
                                && let Metadata::Tuple { operands, .. } = &**node
                            {
                                self.push("{");
                                for (index, operand) in operands.iter().enumerate() {
                                    if index > 0 {
                                        self.push(", ");
                                    }
                                    self.metadata_operand(operand);
                                }
                                self.push("}");
                                continue;
                            }
                            // A field that takes a word takes the number
                            // behind it too, and upstream writes the word
                            // back either way.
                            match (llvm_ir::metadata::vocabulary(tag, key), value) {
                                (Some(words), MdField::Unsigned(number)) => {
                                    match u64::try_from(*number).ok().and_then(|number| {
                                        llvm_ir::metadata::dwarf::word(words, number)
                                    }) {
                                        Some(word) => self.push(word),
                                        None => self.metadata_field(value),
                                    }
                                }
                                _ => self.metadata_field(value),
                            }
                        }
                    }
                    SpecializedArgs::Positional(fields) => {
                        // An expression holds numbers, and upstream writes
                        // each one back as the word it stands for, so long as
                        // it reads the expression at all. One it does not is
                        // written out as the numbers it holds.
                        let written = (tag == "DIExpression")
                            .then(|| llvm_ir::metadata::expression::elements(fields))
                            .flatten()
                            .and_then(|elements| {
                                llvm_ir::metadata::expression::written_words(&elements)
                            });
                        for (index, value) in fields.iter().enumerate() {
                            if index > 0 {
                                self.push(", ");
                            }
                            match written.as_ref().and_then(|written| written[index]) {
                                Some(word) => self.push(word),
                                None => self.metadata_field(value),
                            }
                        }
                    }
                }
                self.push(")");
            }
        }
    }

    pub(crate) fn metadata_operand(&mut self, operand: &MdOperand) {
        match operand {
            MdOperand::Null => self.push("null"),
            MdOperand::Ref(id) => self.metadata_reference(*id),
            MdOperand::String(text) => {
                let _ = write!(self.out, "!\"{}\"", escape_bytes(text.as_bytes()));
            }
            MdOperand::Value { ty, value } => {
                self.ty(*ty);
                self.push(" ");
                self.metadata_value(*value);
            }
            MdOperand::Inline(node) => self.metadata_definition(node),
        }
    }

    /// A value inside metadata. Locals appear here in debug records, and they
    /// print as their slot, so the enclosing function's slots have to be the
    /// ones in scope.
    pub(crate) fn metadata_value(&mut self, value: Value) {
        match value {
            Value::Constant(id) => self.constant(id),
            Value::Instruction(id) => {
                let text = match self.slots.instruction_name(id) {
                    Some(name) => format!("%{name}"),
                    None => match self.slots.instruction(id) {
                        Some(slot) => format!("%{slot}"),
                        None => "%<badref>".to_string(),
                    },
                };
                self.push(&text);
            }
            Value::Argument(index) => {
                let text = match self.slots.argument_name(index) {
                    Some(name) => format!("%{name}"),
                    None => match self.slots.argument(index) {
                        Some(slot) => format!("%{slot}"),
                        None => "%<badref>".to_string(),
                    },
                };
                self.push(&text);
            }
            Value::Metadata(id) => self.metadata_reference(id),
            Value::Block(_) => self.push("<block>"),
        }
    }

    pub(crate) fn metadata_field(&mut self, field: &MdField) {
        match field {
            MdField::Unsigned(value) => {
                let _ = write!(self.out, "{value}");
            }
            MdField::BigInt { negative, digits } => {
                if *negative {
                    self.push("-");
                }
                self.push(digits);
            }
            MdField::Signed(value) => {
                let _ = write!(self.out, "{value}");
            }
            MdField::Bool(value) => {
                let _ = write!(self.out, "{value}");
            }
            MdField::Str(text) => {
                let _ = write!(self.out, "\"{}\"", escape_bytes(text.as_bytes()));
            }
            MdField::Ref(id) => self.metadata_reference(*id),
            MdField::Null => self.push("null"),
            MdField::Words(words) => self.push(&words.join(" | ")),
            MdField::Value { ty, value } => {
                self.ty(*ty);
                self.push(" ");
                self.metadata_value(*value);
            }
            MdField::Inline(node) => self.metadata_definition(node),
        }
    }

    /// A reference to a node: its number, or the node itself when it is one
    /// of the kinds that print in place.
    pub(crate) fn metadata_reference(&mut self, id: llvm_ir::MdId) {
        let canonical = self.metadata.resolve(id);
        if let Some(node) = self.module.metadata_node(canonical)
            && crate::md_slots::prints_in_place(node)
        {
            let node = node.clone();
            self.metadata_definition(&node);
            return;
        }
        match self.metadata.number(id) {
            Some(number) => {
                let _ = write!(self.out, "!{number}");
            }
            // A reference the traversal never reached, which the verifier
            // reports separately; printing the original number keeps the
            // output readable rather than silently dropping it.
            None => {
                let _ = write!(self.out, "!{}", canonical.0);
            }
        }
    }
}

/// The order upstream writes instruction metadata attachments in, which is
/// the order it numbers the kinds rather than the order they were read.
/// Measured by writing twenty of them on one call, backwards, and reading
/// the order they came back in; the three that only a terminator takes came
/// from a second probe, and where they sit relative to the load-only kinds
/// cannot be seen, no instruction taking both.
///
/// A kind this does not name is written after the ones it does, in the order
/// it was read.
/// Where an attachment sits in that order. A kind the table does not name
/// sorts after the ones it does.
pub(crate) fn attachment_rank(attachment: &MdAttachment) -> usize {
    let name = attachment.kind.to_lossy();
    ATTACHMENT_ORDER
        .iter()
        .position(|kind| *kind == name)
        .unwrap_or(ATTACHMENT_ORDER.len())
}

static ATTACHMENT_ORDER: &[&str] = &[
    "dbg",
    "tbaa",
    "prof",
    "fpmath",
    "range",
    "make.implicit",
    "unpredictable",
    "llvm.loop",
    "invariant.load",
    "alias.scope",
    "noalias",
    "nontemporal",
    "nonnull",
    "dereferenceable",
    "invariant.group",
    "align",
    "callees",
    "llvm.access.group",
    "noundef",
    "annotation",
    "nosanitize",
    "memprof",
    "callsite",
    "mmra",
];
