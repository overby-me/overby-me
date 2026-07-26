//! Printing constants, constant expressions and inline assembly.

use std::fmt::Write as _;

use llvm_ir::TypeKind;

use crate::{Printer, escape_bytes, escape_string, name_text};
use llvm_ir::constant::{ConstExpr, ConstId, Constant, GepFlags, InlineAsm};
use llvm_ir::value::GlobalRef;

impl Printer<'_> {
    // ------------------------------------------------------------- constants

    pub(crate) fn constant_with_type(&mut self, id: ConstId) {
        let ty = self.module.ctx.constant(id).ty();
        self.ty(ty);
        self.push(" ");
        self.constant(id);
    }

    pub(crate) fn constant(&mut self, id: ConstId) {
        let constant = self.module.ctx.constant(id).clone();
        match constant {
            Constant::Integer { ty, value } => {
                let is_bool = matches!(self.module.ctx.type_kind(ty), TypeKind::Integer(1));
                if is_bool {
                    self.push(if value.is_zero() { "false" } else { "true" });
                } else {
                    self.push(&value.to_string_signed());
                }
            }
            Constant::Float { value, .. } => {
                let text = value.to_llvm_text();
                self.push(&text);
            }
            Constant::Null(_) => self.push("null"),
            Constant::NoneToken(_) => self.push("none"),
            Constant::Undef(_) => self.push("undef"),
            Constant::Poison(_) => self.push("poison"),
            Constant::ZeroInitializer(_) => self.push("zeroinitializer"),
            Constant::Struct { ty, fields } => {
                let packed = matches!(
                    self.module.ctx.type_kind(ty),
                    TypeKind::Struct { packed: true, .. }
                ) || matches!(self.module.ctx.type_kind(ty),
                    TypeKind::NamedStruct(id) if self.module.ctx.struct_def(*id).packed);
                if packed {
                    self.push("<");
                }
                if fields.is_empty() {
                    self.push("{}");
                } else {
                    self.push("{ ");
                    for (index, field) in fields.iter().enumerate() {
                        if index > 0 {
                            self.push(", ");
                        }
                        self.constant_with_type(*field);
                    }
                    self.push(" }");
                }
                if packed {
                    self.push(">");
                }
            }
            Constant::Array { elements, .. } => {
                self.push("[");
                for (index, element) in elements.iter().enumerate() {
                    if index > 0 {
                        self.push(", ");
                    }
                    self.constant_with_type(*element);
                }
                self.push("]");
            }
            Constant::String { bytes, .. } => {
                let text = format!("c\"{}\"", escape_bytes(&bytes));
                self.push(&text);
            }
            Constant::Vector { elements, .. } => {
                self.push("<");
                for (index, element) in elements.iter().enumerate() {
                    if index > 0 {
                        self.push(", ");
                    }
                    self.constant_with_type(*element);
                }
                self.push(">");
            }
            Constant::Global { target, .. } => {
                let text = self.global_ref_text(target);
                self.push(&text);
            }
            Constant::BlockAddress {
                function, block, ..
            } => {
                let function_text = self.global_ref_text(function);
                let _ = write!(
                    self.out,
                    "blockaddress({function_text}, %{})",
                    name_text(&block)
                );
            }
            Constant::DsoLocalEquivalent { target, .. } => {
                let text = self.global_ref_text(target);
                let _ = write!(self.out, "dso_local_equivalent {text}");
            }
            Constant::NoCfiValue { target, .. } => {
                let text = self.global_ref_text(target);
                let _ = write!(self.out, "no_cfi {text}");
            }
            Constant::Metadata { node, .. } => {
                let _ = write!(self.out, "!{}", node.0);
            }
            Constant::InlineAsm(asm) => self.inline_asm(&asm),
            Constant::Expression(expr) => self.const_expr(&expr),
        }
    }

    pub(crate) fn global_ref_text(&self, target: GlobalRef) -> String {
        format!("@{}", name_text(self.module.global_name(target)))
    }

    pub(crate) fn inline_asm(&mut self, asm: &InlineAsm) {
        self.push("asm ");
        if asm.has_side_effects {
            self.push("sideeffect ");
        }
        if asm.align_stack {
            self.push("alignstack ");
        }
        if asm.dialect == llvm_ir::attribute::AsmDialect::Intel {
            self.push("inteldialect ");
        }
        if asm.can_unwind {
            self.push("unwind ");
        }
        let _ = write!(
            self.out,
            "\"{}\", \"{}\"",
            escape_string(&asm.assembly),
            escape_string(&asm.constraints)
        );
    }

    pub(crate) fn const_expr(&mut self, expr: &ConstExpr) {
        match expr {
            ConstExpr::Cast { op, operand, ty } => {
                let _ = write!(self.out, "{} (", op.keyword());
                self.constant_with_type(*operand);
                self.push(" to ");
                self.ty(*ty);
                self.push(")");
            }
            ConstExpr::GetElementPtr {
                source_type,
                base,
                indices,
                flags,
                inrange,
                ..
            } => {
                self.push("getelementptr ");
                self.gep_flags(*flags);
                if let Some((low, high)) = inrange {
                    let _ = write!(self.out, "inrange({low}, {high}) ");
                }
                self.push("(");
                self.ty(*source_type);
                self.push(", ");
                self.constant_with_type(*base);
                for index in indices {
                    self.push(", ");
                    self.constant_with_type(*index);
                }
                self.push(")");
            }
            ConstExpr::ExtractElement { vector, index, .. } => {
                self.push("extractelement (");
                self.constant_with_type(*vector);
                self.push(", ");
                self.constant_with_type(*index);
                self.push(")");
            }
            ConstExpr::InsertElement {
                vector,
                element,
                index,
                ..
            } => {
                self.push("insertelement (");
                self.constant_with_type(*vector);
                self.push(", ");
                self.constant_with_type(*element);
                self.push(", ");
                self.constant_with_type(*index);
                self.push(")");
            }
            ConstExpr::ShuffleVector {
                first,
                second,
                mask,
                ..
            } => {
                self.push("shufflevector (");
                self.constant_with_type(*first);
                self.push(", ");
                self.constant_with_type(*second);
                self.push(", ");
                self.constant_with_type(*mask);
                self.push(")");
            }
        }
    }

    pub(crate) fn gep_flags(&mut self, flags: GepFlags) {
        if flags.inbounds {
            self.push("inbounds ");
        }
        if flags.nusw {
            self.push("nusw ");
        }
        if flags.nuw {
            self.push("nuw ");
        }
    }
}
