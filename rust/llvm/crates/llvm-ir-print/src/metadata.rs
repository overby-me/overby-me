//! Printing metadata nodes and attachments.

use std::fmt::Write as _;

use llvm_ir::Value;

use crate::{Printer, escape_bytes, metadata_name};
use llvm_ir::metadata::{MdAttachment, MdField, MdOperand, MdRef, Metadata, SpecializedArgs};

impl Printer<'_> {
    // -------------------------------------------------------------- metadata

    pub(crate) fn metadata_attachments(&mut self, attachments: &[MdAttachment], separator: &str) {
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
                        for (index, (key, value)) in fields.iter().enumerate() {
                            if index > 0 {
                                self.push(", ");
                            }
                            let _ = write!(self.out, "{key}: ");
                            self.metadata_field(value);
                        }
                    }
                    SpecializedArgs::Positional(fields) => {
                        for (index, value) in fields.iter().enumerate() {
                            if index > 0 {
                                self.push(", ");
                            }
                            self.metadata_field(value);
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
