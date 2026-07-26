//! Printing metadata nodes and attachments.

use std::fmt::Write as _;

use llvm_ir::Value;

use crate::{Printer, escape_string, metadata_name};
use llvm_ir::metadata::{MdAttachment, MdField, MdOperand, MdRef, Metadata, SpecializedArgs};

impl Printer<'_> {
    // -------------------------------------------------------------- metadata

    pub(crate) fn metadata_attachments(&mut self, attachments: &[MdAttachment], separator: &str) {
        for attachment in attachments {
            let _ = write!(self.out, "{separator}!{} ", metadata_name(&attachment.kind));
            match &attachment.node {
                MdRef::Id(id) => {
                    let _ = write!(self.out, "!{}", id.0);
                }
                MdRef::Inline(node) => self.metadata_definition(node),
            }
        }
    }

    pub(crate) fn metadata_definition(&mut self, node: &Metadata) {
        match node {
            Metadata::String(text) => {
                let _ = write!(self.out, "!\"{}\"", escape_string(text));
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
            MdOperand::Ref(id) => {
                let _ = write!(self.out, "!{}", id.0);
            }
            MdOperand::String(text) => {
                let _ = write!(self.out, "!\"{}\"", escape_string(text));
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
                let text = match self.slots.instruction(id) {
                    Some(slot) => format!("%{slot}"),
                    None => "%<badref>".to_string(),
                };
                self.push(&text);
            }
            Value::Argument(index) => {
                let text = match self.slots.argument(index) {
                    Some(slot) => format!("%{slot}"),
                    None => "%<badref>".to_string(),
                };
                self.push(&text);
            }
            Value::Metadata(id) => {
                let _ = write!(self.out, "!{}", id.0);
            }
            Value::Block(_) => self.push("<block>"),
        }
    }

    pub(crate) fn metadata_field(&mut self, field: &MdField) {
        match field {
            MdField::Unsigned(value) => {
                let _ = write!(self.out, "{value}");
            }
            MdField::Signed(value) => {
                let _ = write!(self.out, "{value}");
            }
            MdField::Bool(value) => {
                let _ = write!(self.out, "{value}");
            }
            MdField::Str(text) => {
                let _ = write!(self.out, "\"{}\"", escape_string(text));
            }
            MdField::Ref(id) => {
                let _ = write!(self.out, "!{}", id.0);
            }
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
}
