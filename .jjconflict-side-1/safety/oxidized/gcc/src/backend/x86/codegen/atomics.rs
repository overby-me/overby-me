//! X86Codegen: atomic operations (RMW, cmpxchg, load, store, fence).

use super::emit::X86Codegen;
use crate::common::types::IrType;
use crate::ir::reexports::{AtomicOrdering, AtomicRmwOp, Operand, Value};

impl X86Codegen {
    pub(super) fn emit_atomic_rmw_impl(
        &mut self,
        dest: &Value,
        op: AtomicRmwOp,
        ptr: &Operand,
        val: &Operand,
        ty: IrType,
        _ordering: AtomicOrdering,
    ) {
        self.operand_to_rax(ptr);
        self.state.emit("    movq %rax, %rcx");
        self.operand_to_rax(val);
        self.state.reg_cache.invalidate_all();
        let size_suffix = Self::type_suffix(ty);
        let val_reg = Self::reg_for_type("rax", ty);
        match op {
            AtomicRmwOp::Add => {
                self.state.emit_fmt(format_args!(
                    "    lock xadd{} %{}, (%rcx)",
                    size_suffix, val_reg
                ));
            }
            AtomicRmwOp::Xchg => {
                self.state
                    .emit_fmt(format_args!("    xchg{} %{}, (%rcx)", size_suffix, val_reg));
            }
            AtomicRmwOp::TestAndSet => {
                self.state.emit("    movb $1, %al");
                self.state.emit("    xchgb %al, (%rcx)");
                // Zero-extend %al to %eax: xchgb only sets the low byte,
                // leaving upper bytes with garbage from prior register usage.
                self.state.emit("    movzbl %al, %eax");
            }
            AtomicRmwOp::Sub => {
                self.emit_x86_atomic_op_loop(ty, "sub");
            }
            AtomicRmwOp::And => {
                self.emit_x86_atomic_op_loop(ty, "and");
            }
            AtomicRmwOp::Or => {
                self.emit_x86_atomic_op_loop(ty, "or");
            }
            AtomicRmwOp::Xor => {
                self.emit_x86_atomic_op_loop(ty, "xor");
            }
            AtomicRmwOp::Nand => {
                self.emit_x86_atomic_op_loop(ty, "nand");
            }
        }
        self.store_rax_to(dest);
    }

    pub(super) fn emit_atomic_cmpxchg_impl(
        &mut self,
        dest: &Value,
        ptr: &Operand,
        expected: &Operand,
        desired: &Operand,
        ty: IrType,
        _success_ordering: AtomicOrdering,
        _failure_ordering: AtomicOrdering,
        returns_bool: bool,
    ) {
        self.operand_to_rax(ptr);
        self.state.emit("    movq %rax, %rcx");
        self.operand_to_rax(desired);
        self.state.emit("    movq %rax, %rdx");
        self.operand_to_rax(expected);
        self.state.reg_cache.invalidate_all();
        let size_suffix = Self::type_suffix(ty);
        let desired_reg = Self::reg_for_type("rdx", ty);
        self.state.emit_fmt(format_args!(
            "    lock cmpxchg{} %{}, (%rcx)",
            size_suffix, desired_reg
        ));
        if returns_bool {
            self.state.emit("    sete %al");
            self.state.emit("    movzbl %al, %eax");
        }
        self.store_rax_to(dest);
    }

    pub(super) fn emit_atomic_load_impl(
        &mut self,
        dest: &Value,
        ptr: &Operand,
        ty: IrType,
        _ordering: AtomicOrdering,
    ) {
        self.operand_to_rax(ptr);
        self.state.reg_cache.invalidate_all();
        let load_instr = Self::mov_load_for_type(ty);
        let dest_reg = Self::load_dest_reg(ty);
        self.state
            .emit_fmt(format_args!("    {} (%rax), {}", load_instr, dest_reg));
        self.store_rax_to(dest);
    }

    pub(super) fn emit_atomic_store_impl(
        &mut self,
        ptr: &Operand,
        val: &Operand,
        ty: IrType,
        ordering: AtomicOrdering,
    ) {
        self.operand_to_rax(val);
        self.state.emit("    movq %rax, %rdx");
        self.operand_to_rax(ptr);
        self.state.reg_cache.invalidate_all();
        let store_reg = Self::reg_for_type("rdx", ty);
        let store_instr = Self::mov_store_for_type(ty);
        self.state
            .emit_fmt(format_args!("    {} %{}, (%rax)", store_instr, store_reg));
        if matches!(ordering, AtomicOrdering::SeqCst) {
            self.state.emit("    mfence");
        }
    }

    pub(super) fn emit_fence_impl(&mut self, ordering: AtomicOrdering) {
        match ordering {
            AtomicOrdering::Relaxed => {}
            _ => self.state.emit("    mfence"),
        }
    }
}
