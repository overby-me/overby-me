//! The textual IR printer.
//!
//! The target is byte-identical output to upstream's assembly writer for the
//! constructs LLVM-rs models, which is a stronger and much more useful
//! property than "produces something that parses back". The corpus is
//! canonical `llvm-dis` output, so any divergence shows up as a diff rather
//! than as an opinion.
//!
//! The rules that look arbitrary are upstream's, and each one is noted where
//! it is implemented: labels pad to column 50, continuation lines indent ten
//! spaces, predecessors list in reverse order, `$` forces an identifier to be
//! quoted even though the lexer would accept it bare.

mod constants;
mod instructions;
mod md_slots;
mod metadata;
mod printer;
mod slots;
mod symbols;
mod type_finder;

use llvm_ir::constant::ConstId;
use llvm_ir::function::Function;
use llvm_ir::{Module, TypeId};

use printer::Printer;
use slots::FunctionSlots;

pub(crate) use printer::{
    CONTINUATION, LABEL_COMMENT_COLUMN, align_text, attribute_list, attribute_text, escape_bytes,
    escape_string, identifier, metadata_name, name_text, predecessors,
};

/// Prints a whole module.
pub fn print_module(module: &Module) -> String {
    let mut printer = Printer::new(module);
    printer.module();
    printer.out
}

/// Prints one type, for diagnostics and tests.
pub fn print_type(module: &Module, ty: TypeId) -> String {
    let mut printer = Printer::new(module);
    printer.ty(ty);
    printer.out
}

/// Prints one constant with its type, the way it appears as an operand.
pub fn print_constant(module: &Module, constant: ConstId) -> String {
    let mut printer = Printer::new(module);
    printer.constant_with_type(constant);
    printer.out
}

/// Prints one instruction, without its indentation, for diagnostics.
pub fn print_instruction(module: &Module, function: &Function, id: llvm_ir::InstId) -> String {
    let mut printer = Printer::new(module);
    printer.slots = FunctionSlots::compute(module, function);
    printer.instruction(function, id);
    printer.out.trim_start().to_string()
}
