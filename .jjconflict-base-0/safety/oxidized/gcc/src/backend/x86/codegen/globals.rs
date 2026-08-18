//! X86Codegen: global address, label address, TLS global address operations.

use super::emit::X86Codegen;
use crate::common::types::IrType;
use crate::ir::reexports::{Operand, Value};

impl X86Codegen {
    pub(super) fn emit_global_addr_impl(&mut self, dest: &Value, name: &str) {
        if self.state.needs_got_for_addr(name) {
            self.state
                .emit_fmt(format_args!("    movq {}@GOTPCREL(%rip), %rax", name));
        } else {
            self.state
                .out
                .emit_instr_sym_base_reg("    leaq", name, "rip", "rax");
        }
        self.store_rax_to(dest);
    }

    pub(super) fn emit_tls_global_addr_impl(&mut self, dest: &Value, name: &str) {
        if self.state.pic_mode {
            self.state
                .emit_fmt(format_args!("    movq {}@GOTTPOFF(%rip), %rax", name));
            self.state.emit("    addq %fs:0, %rax");
        } else {
            self.state.emit("    movq %fs:0, %rax");
            self.state
                .emit_fmt(format_args!("    leaq {}@TPOFF(%rax), %rax", name));
        }
        self.store_rax_to(dest);
    }

    pub(super) fn emit_global_addr_absolute_impl(&mut self, dest: &Value, name: &str) {
        self.state
            .out
            .emit_instr_sym_imm_reg("    movq", name, "rax");
        self.store_rax_to(dest);
    }

    pub(super) fn emit_global_load_rip_rel_impl(&mut self, dest: &Value, sym: &str, ty: IrType) {
        let load_instr = Self::mov_load_for_type(ty);
        let dest_reg = Self::load_dest_reg(ty);
        self.state.emit_fmt(format_args!(
            "    {} {}(%rip), {}",
            load_instr, sym, dest_reg
        ));
        self.emit_store_result_impl(dest);
    }

    pub(super) fn emit_global_store_rip_rel_impl(&mut self, val: &Operand, sym: &str, ty: IrType) {
        self.emit_load_operand_impl(val);
        let store_instr = Self::mov_store_for_type(ty);
        let store_reg = Self::reg_for_type("rax", ty);
        self.state.emit_fmt(format_args!(
            "    {} %{}, {}(%rip)",
            store_instr, store_reg, sym
        ));
    }

    pub(super) fn emit_label_addr_impl(&mut self, dest: &Value, label: &str) {
        self.state
            .out
            .emit_instr_sym_base_reg("    leaq", label, "rip", "rax");
        self.store_rax_to(dest);
    }

    // These thin helpers avoid circular delegation issues:
    fn emit_store_result_impl(&mut self, dest: &Value) {
        self.store_rax_to(dest);
    }

    fn emit_load_operand_impl(&mut self, op: &Operand) {
        self.operand_to_rax(op);
    }
}
