//! Switch/case/default statement lowering.
//!
//! Handles the full switch dispatch pipeline: evaluating the controlling expression,
//! setting up the switch context, collecting case/default labels during body lowering,
//! and emitting the dispatch chain (Switch terminator for exact cases, if-else chain
//! for GNU case ranges).

use super::definitions::SwitchFrame;
use super::lower::Lowerer;
use crate::common::types::{AddressSpace, IrType};
use crate::frontend::parser::ast::{Expr, Stmt};
use crate::ir::reexports::{BlockId, Instruction, IrCmpOp, IrConst, Operand, Terminator, Value};

impl Lowerer {
    pub(super) fn lower_switch_stmt(&mut self, expr: &Expr, body: &Stmt) {
        // C99 6.8.4.2: Integer promotions are performed on the controlling expression.
        // Non-integer types (pointers, floats, etc.) are rejected by sema; this Ptr
        // arm is a defensive fallback to avoid crashes if sema is bypassed.
        let raw_expr_ty = self.get_expr_type(expr);
        let switch_expr_ty = match raw_expr_ty {
            IrType::I8 | IrType::U8 | IrType::I16 | IrType::U16 => IrType::I32,
            IrType::Ptr => crate::common::types::target_int_ir_type(),
            _ => raw_expr_ty,
        };
        let val = self.lower_expr(expr);
        let val = if switch_expr_ty != raw_expr_ty {
            self.emit_implicit_cast(val, raw_expr_ty, switch_expr_ty)
        } else {
            val
        };

        // Store switch value in an alloca for dispatch chain reloading
        let switch_alloca = self.fresh_value();
        let switch_size = switch_expr_ty.size();
        self.emit(Instruction::Alloca {
            dest: switch_alloca,
            ty: switch_expr_ty,
            size: switch_size,
            align: 0,
            volatile: false,
        });
        self.emit(Instruction::Store {
            val,
            ptr: switch_alloca,
            ty: switch_expr_ty,
            seg_override: AddressSpace::Default,
        });

        let dispatch_label = self.fresh_label();
        let end_label = self.fresh_label();
        let body_label = self.fresh_label();

        // Push switch context
        self.func_mut().switch_stack.push(SwitchFrame {
            cases: Vec::new(),
            case_ranges: Vec::new(),
            default_label: None,
            expr_type: switch_expr_ty,
        });
        let scope_depth = self.func().scope_stack.len();
        self.func_mut().break_labels.push((end_label, scope_depth));

        self.terminate(Terminator::Branch(dispatch_label));

        // Lower the body first; case/default stmts register their labels
        self.start_block(body_label);
        self.lower_stmt(body);
        self.terminate(Terminator::Branch(end_label));

        // Pop switch context and emit dispatch chain
        let switch_frame = self.func_mut().switch_stack.pop();
        self.func_mut().break_labels.pop();
        let cases = switch_frame
            .as_ref()
            .map(|f| f.cases.clone())
            .unwrap_or_default();
        let case_ranges = switch_frame
            .as_ref()
            .map(|f| f.case_ranges.clone())
            .unwrap_or_default();
        let default_label = switch_frame.as_ref().and_then(|f| f.default_label);

        let expr_type = switch_frame
            .as_ref()
            .map(|f| f.expr_type)
            .unwrap_or(IrType::I32);
        let fallback = default_label.unwrap_or(end_label);
        self.start_block(dispatch_label);
        self.emit_switch_dispatch(
            &val,
            switch_alloca,
            &cases,
            &case_ranges,
            fallback,
            expr_type,
        );
        self.start_block(end_label);
    }

    /// Emit the dispatch chain for a switch statement. For compile-time constant
    /// switch values, directly jumps to the matching case. Otherwise emits a
    /// chain of comparison branches.
    fn emit_switch_dispatch(
        &mut self,
        val: &Operand,
        switch_alloca: Value,
        cases: &[(i64, BlockId)],
        case_ranges: &[(i64, i64, BlockId)],
        fallback: BlockId,
        switch_ty: IrType,
    ) {
        let total_checks = cases.len() + case_ranges.len();

        // Constant folding: if switch expression is a compile-time constant,
        // jump directly to the matching case (avoids dead dispatch branches
        // with type-mismatched inline asm).
        if let Operand::Const(c) = val {
            if let Some(switch_int) = self.const_to_i64(c) {
                let is_unsigned = switch_ty.is_unsigned();
                let target = cases
                    .iter()
                    .find(|(cv, _)| *cv == switch_int)
                    .map(|(_, label)| *label)
                    .or_else(|| {
                        case_ranges
                            .iter()
                            .find(|(low, high, _)| {
                                if is_unsigned {
                                    (switch_int as u64) >= (*low as u64)
                                        && (switch_int as u64) <= (*high as u64)
                                } else {
                                    switch_int >= *low && switch_int <= *high
                                }
                            })
                            .map(|(_, _, label)| *label)
                    })
                    .unwrap_or(fallback);
                self.terminate(Terminator::Branch(target));
                return;
            }
        }

        if total_checks == 0 {
            self.terminate(Terminator::Branch(fallback));
            return;
        }

        // Use Switch terminator for non-range cases (enables jump table in backend).
        // Range cases still use the if-else chain since they can't be in a jump table.
        if !cases.is_empty() && case_ranges.is_empty() {
            // Load the switch value once and use Switch terminator
            let loaded = self.fresh_value();
            self.emit(Instruction::Load {
                dest: loaded,
                ptr: switch_alloca,
                ty: switch_ty,
                seg_override: AddressSpace::Default,
            });
            self.terminate(Terminator::Switch {
                val: Operand::Value(loaded),
                cases: cases.to_vec(),
                default: fallback,
                ty: switch_ty,
            });
            return;
        }

        // Mixed case: handle non-range cases with Switch if any, range cases with if-else.
        if !cases.is_empty() {
            // Switch terminator for the simple cases, with fallthrough to range checks
            let range_check_block = if !case_ranges.is_empty() {
                self.fresh_label()
            } else {
                fallback
            };
            let loaded = self.fresh_value();
            self.emit(Instruction::Load {
                dest: loaded,
                ptr: switch_alloca,
                ty: switch_ty,
                seg_override: AddressSpace::Default,
            });
            self.terminate(Terminator::Switch {
                val: Operand::Value(loaded),
                cases: cases.to_vec(),
                default: range_check_block,
                ty: switch_ty,
            });
            if !case_ranges.is_empty() {
                self.start_block(range_check_block);
            }
        }

        // Emit range checks: val >= low && val <= high
        // Use unsigned comparisons when the switch expression type is unsigned
        let is_unsigned = switch_ty.is_unsigned();
        let ge_op = if is_unsigned {
            IrCmpOp::Uge
        } else {
            IrCmpOp::Sge
        };
        let le_op = if is_unsigned {
            IrCmpOp::Ule
        } else {
            IrCmpOp::Sle
        };
        let range_count = case_ranges.len();
        let make_case_const = |v: i64| -> IrConst { IrConst::from_i64(v, switch_ty) };
        for (ri, (low, high, range_label)) in case_ranges.iter().enumerate() {
            let loaded = self.fresh_value();
            self.emit(Instruction::Load {
                dest: loaded,
                ptr: switch_alloca,
                ty: switch_ty,
                seg_override: AddressSpace::Default,
            });
            let ge_result = self.emit_cmp_val(
                ge_op,
                Operand::Value(loaded),
                Operand::Const(make_case_const(*low)),
                switch_ty,
            );
            let le_result = self.emit_cmp_val(
                le_op,
                Operand::Value(loaded),
                Operand::Const(make_case_const(*high)),
                switch_ty,
            );
            let and_result = self.fresh_value();
            self.emit(Instruction::BinOp {
                dest: and_result,
                op: crate::ir::reexports::IrBinOp::And,
                lhs: Operand::Value(ge_result),
                rhs: Operand::Value(le_result),
                ty: IrType::I32,
            });

            let next_check = if ri + 1 < range_count {
                self.fresh_label()
            } else {
                fallback
            };
            self.terminate(Terminator::CondBranch {
                cond: Operand::Value(and_result),
                true_label: *range_label,
                false_label: next_check,
            });
            if ri + 1 < range_count {
                self.start_block(next_check);
            }
        }
    }

    /// Evaluate a case constant and truncate to the switch expression type.
    pub(super) fn eval_case_constant(&mut self, expr: &Expr) -> i64 {
        let mut val = self
            .eval_const_expr(expr)
            .and_then(|c| self.const_to_i64(&c))
            .unwrap_or(0);
        if let Some(switch_ty) = self.func_mut().switch_stack.last().map(|f| &f.expr_type) {
            val = switch_ty.truncate_i64(val);
        }
        val
    }

    pub(super) fn lower_case_stmt(&mut self, expr: &Expr, stmt: &Stmt) {
        let case_val = self.eval_case_constant(expr);
        let label = self.fresh_label();
        if let Some(frame) = self.func_mut().switch_stack.last_mut() {
            frame.cases.push((case_val, label));
        }
        // Fallthrough from previous case
        self.terminate(Terminator::Branch(label));
        self.start_block(label);
        self.lower_stmt(stmt);
    }

    pub(super) fn lower_case_range_stmt(&mut self, low_expr: &Expr, high_expr: &Expr, stmt: &Stmt) {
        let low_val = self.eval_case_constant(low_expr);
        let high_val = self.eval_case_constant(high_expr);
        let label = self.fresh_label();
        if let Some(frame) = self.func_mut().switch_stack.last_mut() {
            frame.case_ranges.push((low_val, high_val, label));
        }
        self.terminate(Terminator::Branch(label));
        self.start_block(label);
        self.lower_stmt(stmt);
    }

    pub(super) fn lower_default_stmt(&mut self, stmt: &Stmt) {
        let label = self.fresh_label();
        if let Some(frame) = self.func_mut().switch_stack.last_mut() {
            frame.default_label = Some(label);
        }
        self.terminate(Terminator::Branch(label));
        self.start_block(label);
        self.lower_stmt(stmt);
    }
}
