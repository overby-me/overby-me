//! Atomic builtin lowering: __atomic_* and __sync_* operations.
//!
//! Extracted from expr.rs. This module handles all atomic/sync builtins:
//! - fetch-op / op-fetch (table-driven via classify_fetch_op / classify_op_fetch)
//! - exchange, compare-exchange
//! - atomic load/store
//! - test-and-set, clear
//! - sync CAS (val and bool variants)
//! - lock operations, fences, lock-free queries
//! - Helper: parse_ordering, emit_post_rmw_compute, emit_atomic_xchg, etc.

use super::lower::Lowerer;
use crate::common::types::{AddressSpace, IrType};
use crate::frontend::parser::ast::Expr;
use crate::ir::reexports::{
    AtomicOrdering, AtomicRmwOp, Instruction, IrBinOp, IrCmpOp, IrConst, IrUnaryOp, Operand, Value,
};

impl Lowerer {
    /// Try to lower a GCC atomic builtin (__atomic_* or __sync_*).
    ///
    /// Uses table-driven dispatch for fetch-op and op-fetch families.
    /// The __atomic_* variants take an explicit ordering argument; __sync_* always use SeqCst.
    pub(super) fn try_lower_atomic_builtin(
        &mut self,
        name: &str,
        args: &[Expr],
    ) -> Option<Operand> {
        // Strip size suffix (_1, _2, _4, _8, _16) from __sync_* builtins so that
        // e.g. __sync_fetch_and_add_8 dispatches the same as __sync_fetch_and_add.
        // Also normalize __atomic_*_N size-suffixed variants to their _n equivalents
        // (e.g., __atomic_load_4 -> __atomic_load_n).
        let normalized_atomic = crate::frontend::sema::builtins::normalize_atomic_size_suffix(name);
        let name = if let Some(norm) = normalized_atomic {
            norm
        } else {
            crate::frontend::sema::builtins::strip_sync_size_suffix(name)
        };

        let val_ty = if !args.is_empty() {
            self.get_pointee_ir_type(&args[0])
                .unwrap_or(crate::common::types::target_int_ir_type())
        } else {
            crate::common::types::target_int_ir_type()
        };

        // --- Fetch-op: atomic RMW returning old value ---
        if let Some((op, is_sync)) = Self::classify_fetch_op(name) {
            let min_args = if is_sync { 2 } else { 3 };
            if args.len() >= min_args {
                let ptr = self.lower_expr(&args[0]);
                let val = self.lower_expr(&args[1]);
                let ordering = if is_sync {
                    AtomicOrdering::SeqCst
                } else {
                    Self::parse_ordering(&args[2])
                };
                let dest = self.fresh_value();
                self.emit(Instruction::AtomicRmw {
                    dest,
                    op,
                    ptr,
                    val,
                    ty: val_ty,
                    ordering,
                });
                return Some(Operand::Value(dest));
            }
        }

        // --- Op-fetch: atomic RMW returning new value ---
        if let Some((rmw_op, bin_op, is_nand, is_sync)) = Self::classify_op_fetch(name) {
            let min_args = if is_sync { 2 } else { 3 };
            if args.len() >= min_args {
                let ptr = self.lower_expr(&args[0]);
                let val_expr = self.lower_expr(&args[1]);
                let ordering = if is_sync {
                    AtomicOrdering::SeqCst
                } else {
                    Self::parse_ordering(&args[2])
                };
                let old_val = self.fresh_value();
                self.emit(Instruction::AtomicRmw {
                    dest: old_val,
                    op: rmw_op,
                    ptr,
                    val: val_expr,
                    ty: val_ty,
                    ordering,
                });
                let new_val =
                    self.emit_post_rmw_compute(old_val, val_expr, bin_op, is_nand, val_ty);
                return Some(Operand::Value(new_val));
            }
        }

        // --- Exchange ---
        if name == "__atomic_exchange_n" && args.len() >= 3 {
            let ptr = self.lower_expr(&args[0]);
            let val = self.lower_expr(&args[1]);
            let ordering = Self::parse_ordering(&args[2]);
            return Some(Operand::Value(
                self.emit_atomic_xchg(ptr, val, val_ty, ordering),
            ));
        }
        if name == "__atomic_exchange" && args.len() >= 4 {
            let ptr = self.lower_expr(&args[0]);
            let val_ptr_op = self.lower_expr(&args[1]);
            let ret_ptr_op = self.lower_expr(&args[2]);
            let ordering = Self::parse_ordering(&args[3]);
            let val = self.load_through_ptr(val_ptr_op, val_ty);
            let old_val = self.emit_atomic_xchg(ptr, Operand::Value(val), val_ty, ordering);
            self.store_through_ptr(ret_ptr_op, Operand::Value(old_val), val_ty);
            return Some(Operand::Const(IrConst::I64(0)));
        }

        // --- Compare-exchange ---
        if name == "__atomic_compare_exchange_n" && args.len() >= 6 {
            let ptr = self.lower_expr(&args[0]);
            let expected_ptr_op = self.lower_expr(&args[1]);
            let expected = self.load_through_ptr(expected_ptr_op, val_ty);
            let desired = self.lower_expr(&args[2]);
            return Some(self.emit_cmpxchg_with_writeback(
                ptr,
                expected_ptr_op,
                expected,
                desired,
                val_ty,
                Self::parse_ordering(&args[4]),
                Self::parse_ordering(&args[5]),
            ));
        }
        if name == "__atomic_compare_exchange" && args.len() >= 6 {
            let ptr = self.lower_expr(&args[0]);
            let expected_ptr_op = self.lower_expr(&args[1]);
            let desired_ptr_op = self.lower_expr(&args[2]);
            let expected = self.load_through_ptr(expected_ptr_op, val_ty);
            let desired = self.load_through_ptr(desired_ptr_op, val_ty);
            return Some(self.emit_cmpxchg_with_writeback(
                ptr,
                expected_ptr_op,
                expected,
                Operand::Value(desired),
                val_ty,
                Self::parse_ordering(&args[4]),
                Self::parse_ordering(&args[5]),
            ));
        }

        // --- Load ---
        if name == "__atomic_load_n" && args.len() >= 2 {
            let ptr = self.lower_expr(&args[0]);
            let ordering = Self::parse_ordering(&args[1]);
            let dest = self.fresh_value();
            self.emit(Instruction::AtomicLoad {
                dest,
                ptr,
                ty: val_ty,
                ordering,
            });
            return Some(Operand::Value(dest));
        }
        if name == "__atomic_load" && args.len() >= 3 {
            let ptr = self.lower_expr(&args[0]);
            let ret_ptr = self.lower_expr(&args[1]);
            let ordering = Self::parse_ordering(&args[2]);
            let loaded = self.fresh_value();
            self.emit(Instruction::AtomicLoad {
                dest: loaded,
                ptr,
                ty: val_ty,
                ordering,
            });
            self.store_through_ptr(ret_ptr, Operand::Value(loaded), val_ty);
            return Some(Operand::Const(IrConst::I64(0)));
        }

        // --- Store ---
        if name == "__atomic_store_n" && args.len() >= 3 {
            let ptr = self.lower_expr(&args[0]);
            let val = self.lower_expr(&args[1]);
            let ordering = Self::parse_ordering(&args[2]);
            self.emit(Instruction::AtomicStore {
                ptr,
                val,
                ty: val_ty,
                ordering,
            });
            return Some(Operand::Const(IrConst::I64(0)));
        }
        if name == "__atomic_store" && args.len() >= 3 {
            let ptr = self.lower_expr(&args[0]);
            let val_ptr = self.lower_expr(&args[1]);
            let ordering = Self::parse_ordering(&args[2]);
            let loaded = self.load_through_ptr(val_ptr, val_ty);
            self.emit(Instruction::AtomicStore {
                ptr,
                val: Operand::Value(loaded),
                ty: val_ty,
                ordering,
            });
            return Some(Operand::Const(IrConst::I64(0)));
        }

        // --- Test-and-set / clear ---
        if name == "__atomic_test_and_set" && args.len() >= 2 {
            let ptr = self.lower_expr(&args[0]);
            let ordering = Self::parse_ordering(&args[1]);
            let dest = self.fresh_value();
            self.emit(Instruction::AtomicRmw {
                dest,
                op: AtomicRmwOp::TestAndSet,
                ptr,
                val: Operand::Const(IrConst::I64(1)),
                ty: IrType::I8,
                ordering,
            });
            return Some(Operand::Value(dest));
        }
        if name == "__atomic_clear" && args.len() >= 2 {
            let ptr = self.lower_expr(&args[0]);
            let ordering = Self::parse_ordering(&args[1]);
            self.emit(Instruction::AtomicStore {
                ptr,
                val: Operand::Const(IrConst::I8(0)),
                ty: IrType::I8,
                ordering,
            });
            return Some(Operand::Const(IrConst::I64(0)));
        }

        // --- Sync compare-and-swap ---
        if (name == "__sync_val_compare_and_swap" || name == "__sync_bool_compare_and_swap")
            && args.len() >= 3
        {
            let ptr = self.lower_expr(&args[0]);
            let expected = self.lower_expr(&args[1]);
            let desired = self.lower_expr(&args[2]);
            let returns_bool = name == "__sync_bool_compare_and_swap";
            let dest = self.fresh_value();
            self.emit(Instruction::AtomicCmpxchg {
                dest,
                ptr,
                expected,
                desired,
                ty: val_ty,
                success_ordering: AtomicOrdering::SeqCst,
                failure_ordering: AtomicOrdering::SeqCst,
                returns_bool,
            });
            return Some(Operand::Value(dest));
        }

        // --- Sync lock operations ---
        if name == "__sync_lock_test_and_set" && args.len() >= 2 {
            let ptr = self.lower_expr(&args[0]);
            let val = self.lower_expr(&args[1]);
            return Some(Operand::Value(self.emit_atomic_xchg(
                ptr,
                val,
                val_ty,
                AtomicOrdering::Acquire,
            )));
        }
        if name == "__sync_lock_release" && !args.is_empty() {
            let ptr = self.lower_expr(&args[0]);
            self.emit(Instruction::AtomicStore {
                ptr,
                val: Operand::Const(IrConst::I64(0)),
                ty: val_ty,
                ordering: AtomicOrdering::Release,
            });
            return Some(Operand::Const(IrConst::I64(0)));
        }

        // --- Fences ---
        if name == "__sync_synchronize" {
            self.emit(Instruction::Fence {
                ordering: AtomicOrdering::SeqCst,
            });
            return Some(Operand::Const(IrConst::I64(0)));
        }
        if name == "__atomic_thread_fence" || name == "__atomic_signal_fence" {
            let ordering = if !args.is_empty() {
                Self::parse_ordering(&args[0])
            } else {
                AtomicOrdering::SeqCst
            };
            self.emit(Instruction::Fence { ordering });
            return Some(Operand::Const(IrConst::I64(0)));
        }

        // --- Lock-free queries (always true for sizes <= 8) ---
        if name == "__atomic_is_lock_free" || name == "__atomic_always_lock_free" {
            return Some(Operand::Const(IrConst::I64(1)));
        }

        None
    }

    // -----------------------------------------------------------------------
    // Atomic helpers
    // -----------------------------------------------------------------------

    /// Parse a memory ordering constant from an expression.
    pub(super) fn parse_ordering(arg: &Expr) -> AtomicOrdering {
        match arg {
            Expr::IntLiteral(v, _) => match *v as i32 {
                0 => AtomicOrdering::Relaxed,
                1 | 2 => AtomicOrdering::Acquire, // consume maps to acquire
                3 => AtomicOrdering::Release,
                4 => AtomicOrdering::AcqRel,
                _ => AtomicOrdering::SeqCst,
            },
            _ => AtomicOrdering::SeqCst,
        }
    }

    /// Classify __atomic_fetch_OP / __sync_fetch_and_OP builtins.
    /// Returns (rmw_op, is_sync).
    fn classify_fetch_op(name: &str) -> Option<(AtomicRmwOp, bool)> {
        match name {
            "__atomic_fetch_add" => Some((AtomicRmwOp::Add, false)),
            "__atomic_fetch_sub" => Some((AtomicRmwOp::Sub, false)),
            "__atomic_fetch_and" => Some((AtomicRmwOp::And, false)),
            "__atomic_fetch_or" => Some((AtomicRmwOp::Or, false)),
            "__atomic_fetch_xor" => Some((AtomicRmwOp::Xor, false)),
            "__atomic_fetch_nand" => Some((AtomicRmwOp::Nand, false)),
            "__sync_fetch_and_add" => Some((AtomicRmwOp::Add, true)),
            "__sync_fetch_and_sub" => Some((AtomicRmwOp::Sub, true)),
            "__sync_fetch_and_and" => Some((AtomicRmwOp::And, true)),
            "__sync_fetch_and_or" => Some((AtomicRmwOp::Or, true)),
            "__sync_fetch_and_xor" => Some((AtomicRmwOp::Xor, true)),
            "__sync_fetch_and_nand" => Some((AtomicRmwOp::Nand, true)),
            _ => None,
        }
    }

    /// Classify __atomic_OP_fetch / __sync_OP_and_fetch builtins.
    /// Returns (rmw_op, bin_op_for_recompute, is_nand, is_sync).
    fn classify_op_fetch(name: &str) -> Option<(AtomicRmwOp, IrBinOp, bool, bool)> {
        match name {
            "__atomic_add_fetch" => Some((AtomicRmwOp::Add, IrBinOp::Add, false, false)),
            "__atomic_sub_fetch" => Some((AtomicRmwOp::Sub, IrBinOp::Sub, false, false)),
            "__atomic_and_fetch" => Some((AtomicRmwOp::And, IrBinOp::And, false, false)),
            "__atomic_or_fetch" => Some((AtomicRmwOp::Or, IrBinOp::Or, false, false)),
            "__atomic_xor_fetch" => Some((AtomicRmwOp::Xor, IrBinOp::Xor, false, false)),
            "__atomic_nand_fetch" => Some((AtomicRmwOp::Nand, IrBinOp::And, true, false)),
            "__sync_add_and_fetch" => Some((AtomicRmwOp::Add, IrBinOp::Add, false, true)),
            "__sync_sub_and_fetch" => Some((AtomicRmwOp::Sub, IrBinOp::Sub, false, true)),
            "__sync_and_and_fetch" => Some((AtomicRmwOp::And, IrBinOp::And, false, true)),
            "__sync_or_and_fetch" => Some((AtomicRmwOp::Or, IrBinOp::Or, false, true)),
            "__sync_xor_and_fetch" => Some((AtomicRmwOp::Xor, IrBinOp::Xor, false, true)),
            "__sync_nand_and_fetch" => Some((AtomicRmwOp::Nand, IrBinOp::And, true, true)),
            _ => None,
        }
    }

    /// After an atomic RMW that returns the old value, compute the new value.
    /// For nand: ~(old & val). For others: old bin_op val.
    fn emit_post_rmw_compute(
        &mut self,
        old_val: Value,
        val_expr: Operand,
        bin_op: IrBinOp,
        is_nand: bool,
        ty: IrType,
    ) -> Value {
        if is_nand {
            let and_val = self.emit_binop_val(IrBinOp::And, Operand::Value(old_val), val_expr, ty);
            let result = self.fresh_value();
            self.emit(Instruction::UnaryOp {
                dest: result,
                op: IrUnaryOp::Not,
                src: Operand::Value(and_val),
                ty,
            });
            result
        } else {
            self.emit_binop_val(bin_op, Operand::Value(old_val), val_expr, ty)
        }
    }

    /// Emit an atomic exchange (xchg) instruction, returning the old value.
    fn emit_atomic_xchg(
        &mut self,
        ptr: Operand,
        val: Operand,
        ty: IrType,
        ordering: AtomicOrdering,
    ) -> Value {
        let dest = self.fresh_value();
        self.emit(Instruction::AtomicRmw {
            dest,
            op: AtomicRmwOp::Xchg,
            ptr,
            val,
            ty,
            ordering,
        });
        dest
    }

    /// Emit a compare-exchange with writeback to expected_ptr and equality comparison.
    fn emit_cmpxchg_with_writeback(
        &mut self,
        ptr: Operand,
        expected_ptr_op: Operand,
        expected: Value,
        desired: Operand,
        ty: IrType,
        success_ordering: AtomicOrdering,
        failure_ordering: AtomicOrdering,
    ) -> Operand {
        let old_val = self.fresh_value();
        self.emit(Instruction::AtomicCmpxchg {
            dest: old_val,
            ptr,
            expected: Operand::Value(expected),
            desired,
            ty,
            success_ordering,
            failure_ordering,
            returns_bool: false,
        });
        self.store_through_ptr(expected_ptr_op, Operand::Value(old_val), ty);
        let result = self.emit_cmp_val(
            IrCmpOp::Eq,
            Operand::Value(old_val),
            Operand::Value(expected),
            ty,
        );
        Operand::Value(result)
    }

    /// Load a value through a pointer operand.
    pub(super) fn load_through_ptr(&mut self, ptr_op: Operand, ty: IrType) -> Value {
        let ptr_val = self.operand_to_value(ptr_op);
        let dest = self.fresh_value();
        self.emit(Instruction::Load {
            dest,
            ptr: ptr_val,
            ty,
            seg_override: AddressSpace::Default,
        });
        dest
    }

    /// Store a value through a pointer operand.
    pub(super) fn store_through_ptr(&mut self, ptr_op: Operand, val: Operand, ty: IrType) {
        let ptr_val = self.operand_to_value(ptr_op);
        self.emit(Instruction::Store {
            val,
            ptr: ptr_val,
            ty,
            seg_override: AddressSpace::Default,
        });
    }

    /// Get the IR type of the pointee for a pointer expression.
    pub(super) fn get_pointee_ir_type(&self, expr: &Expr) -> Option<IrType> {
        if let Some(crate::common::types::CType::Pointer(inner, _)) = self.get_expr_ctype(expr) {
            return Some(IrType::from_ctype(&inner));
        }
        None
    }
}
