//! RiscvCodegen: comparisons, fused cmp+branch, select.

use super::emit::RiscvCodegen;
use crate::common::types::IrType;
use crate::ir::reexports::{IrCmpOp, Operand, Value};

impl RiscvCodegen {
    // ---- Float comparison ----

    pub(super) fn emit_float_cmp_impl(
        &mut self,
        dest: &Value,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
    ) {
        // Float comparison (F32/F64): load operands into t1/t2, then move to float regs.
        self.operand_to_t0(lhs);
        self.state.emit("    mv t1, t0");
        self.operand_to_t0(rhs);
        self.state.emit("    mv t2, t0");
        let s = if ty == IrType::F64 { "d" } else { "s" };
        let fmv = if s == "d" { "fmv.d.x" } else { "fmv.w.x" };
        self.state.emit_fmt(format_args!("    {} ft0, t1", fmv));
        self.state.emit_fmt(format_args!("    {} ft1, t2", fmv));
        match op {
            IrCmpOp::Eq => self
                .state
                .emit_fmt(format_args!("    feq.{} t0, ft0, ft1", s)),
            IrCmpOp::Ne => {
                self.state
                    .emit_fmt(format_args!("    feq.{} t0, ft0, ft1", s));
                self.state.emit("    xori t0, t0, 1");
            }
            IrCmpOp::Slt | IrCmpOp::Ult => self
                .state
                .emit_fmt(format_args!("    flt.{} t0, ft0, ft1", s)),
            IrCmpOp::Sle | IrCmpOp::Ule => self
                .state
                .emit_fmt(format_args!("    fle.{} t0, ft0, ft1", s)),
            IrCmpOp::Sgt | IrCmpOp::Ugt => self
                .state
                .emit_fmt(format_args!("    flt.{} t0, ft1, ft0", s)),
            IrCmpOp::Sge | IrCmpOp::Uge => self
                .state
                .emit_fmt(format_args!("    fle.{} t0, ft1, ft0", s)),
        }
        self.store_t0_to(dest);
    }

    // ---- Integer comparison ----

    pub(super) fn emit_int_cmp_impl(
        &mut self,
        dest: &Value,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
    ) {
        // Integer comparison: load + sign/zero-extend, then compare
        self.emit_cmp_operand_load(lhs, rhs, ty);
        match op {
            IrCmpOp::Eq => {
                self.state.emit("    sub t0, t1, t2");
                self.state.emit("    seqz t0, t0");
            }
            IrCmpOp::Ne => {
                self.state.emit("    sub t0, t1, t2");
                self.state.emit("    snez t0, t0");
            }
            IrCmpOp::Slt => self.state.emit("    slt t0, t1, t2"),
            IrCmpOp::Ult => self.state.emit("    sltu t0, t1, t2"),
            IrCmpOp::Sge => {
                self.state.emit("    slt t0, t1, t2");
                self.state.emit("    xori t0, t0, 1");
            }
            IrCmpOp::Uge => {
                self.state.emit("    sltu t0, t1, t2");
                self.state.emit("    xori t0, t0, 1");
            }
            IrCmpOp::Sgt => self.state.emit("    slt t0, t2, t1"),
            IrCmpOp::Ugt => self.state.emit("    sltu t0, t2, t1"),
            IrCmpOp::Sle => {
                self.state.emit("    slt t0, t2, t1");
                self.state.emit("    xori t0, t0, 1");
            }
            IrCmpOp::Ule => {
                self.state.emit("    sltu t0, t2, t1");
                self.state.emit("    xori t0, t0, 1");
            }
        }

        self.store_t0_to(dest);
    }

    // ---- Fused compare-and-branch ----

    pub(super) fn emit_fused_cmp_branch_impl(
        &mut self,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
        true_label: &str,
        false_label: &str,
    ) {
        // Load operands into t1, t2 (with sign/zero-extension for sub-64-bit types)
        self.emit_cmp_operand_load(lhs, rhs, ty);

        // Emit inverted branch to skip over the true-path jump.
        let (inv_branch, r1, r2) = match op {
            IrCmpOp::Eq => ("bne", "t1", "t2"),
            IrCmpOp::Ne => ("beq", "t1", "t2"),
            IrCmpOp::Slt => ("bge", "t1", "t2"),
            IrCmpOp::Sge => ("blt", "t1", "t2"),
            IrCmpOp::Ult => ("bgeu", "t1", "t2"),
            IrCmpOp::Uge => ("bltu", "t1", "t2"),
            // Swap operands for > and <=, and invert
            IrCmpOp::Sgt => ("bge", "t2", "t1"), // NOT(a > b)  = b >= a
            IrCmpOp::Sle => ("blt", "t2", "t1"), // NOT(a <= b) = b < a
            IrCmpOp::Ugt => ("bgeu", "t2", "t1"), // NOT(a > b)  = b >= a (unsigned)
            IrCmpOp::Ule => ("bltu", "t2", "t1"), // NOT(a <= b) = b < a (unsigned)
        };
        let skip = self.state.fresh_label("skip");
        self.state
            .emit_fmt(format_args!("    {} {}, {}, {}", inv_branch, r1, r2, skip));
        self.state
            .emit_fmt(format_args!("    jump {}, t6", true_label));
        self.state.emit_fmt(format_args!("{}:", skip));
        self.state
            .emit_fmt(format_args!("    jump {}, t6", false_label));
        self.state.reg_cache.invalidate_all();
    }

    // ---- Select ----

    pub(super) fn emit_select_impl(
        &mut self,
        dest: &Value,
        cond: &Operand,
        true_val: &Operand,
        false_val: &Operand,
        _ty: IrType,
    ) {
        let label_id = self.state.next_label_id();
        let skip_label = format!(".Lsel_skip_{}", label_id);

        // Load false_val (default) into t0
        self.operand_to_t0(false_val);
        // Save to t2 so we can test the condition
        self.state.emit("    mv t2, t0");

        // Load condition
        self.operand_to_t0(cond);
        // Branch to skip if condition is zero (keep false_val)
        self.state
            .emit_fmt(format_args!("    beqz t0, {}", skip_label));

        // Load true_val into t2 (overrides the false_val)
        self.operand_to_t0(true_val);
        self.state.emit("    mv t2, t0");

        // Skip label
        self.state.emit_fmt(format_args!("{}:", skip_label));

        // Move result to t0 and store
        self.state.emit("    mv t0, t2");
        self.state.reg_cache.invalidate_acc();
        self.store_t0_to(dest);
    }

    // ---- F128 comparison ----

    pub(super) fn emit_f128_cmp_impl(
        &mut self,
        dest: &Value,
        op: IrCmpOp,
        lhs: &Operand,
        rhs: &Operand,
    ) {
        crate::backend::f128_softfloat::f128_cmp(self, dest, op, lhs, rhs);
    }
}
