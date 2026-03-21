//! ArmCodegen: ALU operations (integer arithmetic, bitwise, unary).

use super::emit::{arm_alu_mnemonic, callee_saved_name, callee_saved_name_32, ArmCodegen};
use crate::common::types::IrType;
use crate::ir::reexports::{IrBinOp, Operand, Value};

impl ArmCodegen {
    pub(super) fn emit_float_neg_impl(&mut self, ty: IrType) {
        if ty == IrType::F32 {
            self.state.emit("    fmov s0, w0");
            self.state.emit("    fneg s0, s0");
            self.state.emit("    fmov w0, s0");
            self.state.emit("    mov w0, w0"); // zero-extend
        } else {
            self.state.emit("    fmov d0, x0");
            self.state.emit("    fneg d0, d0");
            self.state.emit("    fmov x0, d0");
        }
    }

    pub(super) fn emit_int_neg_impl(&mut self, _ty: IrType) {
        self.state.emit("    neg x0, x0");
    }

    pub(super) fn emit_int_not_impl(&mut self, _ty: IrType) {
        self.state.emit("    mvn x0, x0");
    }

    pub(super) fn emit_int_clz_impl(&mut self, ty: IrType) {
        if ty == IrType::I32 || ty == IrType::U32 {
            self.state.emit("    clz w0, w0");
        } else {
            self.state.emit("    clz x0, x0");
        }
    }

    pub(super) fn emit_int_ctz_impl(&mut self, ty: IrType) {
        if ty == IrType::I32 || ty == IrType::U32 {
            self.state.emit("    rbit w0, w0");
            self.state.emit("    clz w0, w0");
        } else {
            self.state.emit("    rbit x0, x0");
            self.state.emit("    clz x0, x0");
        }
    }

    pub(super) fn emit_int_bswap_impl(&mut self, ty: IrType) {
        if ty == IrType::I16 || ty == IrType::U16 {
            self.state.emit("    rev w0, w0");
            self.state.emit("    lsr w0, w0, #16");
        } else if ty == IrType::I32 || ty == IrType::U32 {
            self.state.emit("    rev w0, w0");
        } else {
            self.state.emit("    rev x0, x0");
        }
    }

    pub(super) fn emit_int_popcount_impl(&mut self, ty: IrType) {
        if ty == IrType::I32 || ty == IrType::U32 {
            self.state.emit("    fmov s0, w0");
        } else {
            self.state.emit("    fmov d0, x0");
        }
        self.state.emit("    cnt v0.8b, v0.8b");
        self.state.emit("    uaddlv h0, v0.8b");
        self.state.emit("    fmov w0, s0");
    }

    pub(super) fn emit_int_binop_impl(
        &mut self,
        dest: &Value,
        op: IrBinOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: IrType,
    ) {
        let use_32bit = ty == IrType::I32 || ty == IrType::U32;
        let is_unsigned = ty.is_unsigned();

        // Strength reduction: UDiv/URem by power-of-2 constant
        if let Some(shift) = Self::const_as_power_of_2(rhs) {
            if op == IrBinOp::UDiv {
                self.operand_to_x0(lhs);
                if use_32bit {
                    self.state
                        .emit_fmt(format_args!("    lsr w0, w0, #{}", shift));
                } else {
                    self.state
                        .emit_fmt(format_args!("    lsr x0, x0, #{}", shift));
                }
                self.store_x0_to(dest);
                return;
            }
            if op == IrBinOp::URem {
                self.operand_to_x0(lhs);
                let mask = (1u64 << shift) - 1;
                if use_32bit {
                    self.state
                        .emit_fmt(format_args!("    and w0, w0, #{}", mask));
                } else {
                    self.state
                        .emit_fmt(format_args!("    and x0, x0, #{}", mask));
                }
                self.store_x0_to(dest);
                return;
            }
        }

        // Register-direct path
        if let Some(dest_phys) = self.dest_reg(dest) {
            let dest_name = callee_saved_name(dest_phys);
            let dest_name_32 = callee_saved_name_32(dest_phys);

            let is_simple_alu = matches!(
                op,
                IrBinOp::Add
                    | IrBinOp::Sub
                    | IrBinOp::And
                    | IrBinOp::Or
                    | IrBinOp::Xor
                    | IrBinOp::Mul
            );
            if is_simple_alu {
                let mnemonic = arm_alu_mnemonic(op);

                if matches!(op, IrBinOp::Add | IrBinOp::Sub) {
                    if let Some(imm) = Self::const_as_imm12(rhs) {
                        self.operand_to_callee_reg(lhs, dest_phys);
                        if use_32bit {
                            self.state.emit_fmt(format_args!(
                                "    {} {}, {}, #{}",
                                mnemonic, dest_name_32, dest_name_32, imm
                            ));
                            if !is_unsigned {
                                self.state.emit_fmt(format_args!(
                                    "    sxtw {}, {}",
                                    dest_name, dest_name_32
                                ));
                            }
                        } else {
                            self.state.emit_fmt(format_args!(
                                "    {} {}, {}, #{}",
                                mnemonic, dest_name, dest_name, imm
                            ));
                        }
                        self.state.reg_cache.invalidate_acc();
                        return;
                    }
                }

                let rhs_phys = self.operand_reg(rhs);
                let rhs_conflicts = rhs_phys.is_some_and(|r| r.0 == dest_phys.0);
                let rhs_reg: String = if rhs_conflicts {
                    self.operand_to_x0(rhs);
                    self.operand_to_callee_reg(lhs, dest_phys);
                    "x0".to_string()
                } else {
                    self.operand_to_callee_reg(lhs, dest_phys);
                    if let Some(rhs_phys) = rhs_phys {
                        callee_saved_name(rhs_phys).to_string()
                    } else {
                        self.operand_to_x0(rhs);
                        "x0".to_string()
                    }
                };
                let rhs_32: String = if rhs_reg == "x0" {
                    "w0".to_string()
                } else {
                    rhs_reg.replace('x', "w")
                };

                if use_32bit {
                    self.state.emit_fmt(format_args!(
                        "    {} {}, {}, {}",
                        mnemonic, dest_name_32, dest_name_32, rhs_32
                    ));
                    if !is_unsigned {
                        self.state
                            .emit_fmt(format_args!("    sxtw {}, {}", dest_name, dest_name_32));
                    }
                } else {
                    self.state.emit_fmt(format_args!(
                        "    {} {}, {}, {}",
                        mnemonic, dest_name, dest_name, rhs_reg
                    ));
                }
                self.state.reg_cache.invalidate_acc();
                return;
            }
        }

        // Fallback: accumulator path
        self.operand_to_x0(lhs);
        self.state.emit("    mov x1, x0");
        self.operand_to_x0(rhs);
        self.state.emit("    mov x2, x0");

        if use_32bit {
            match op {
                IrBinOp::Add => {
                    self.state.emit("    add w0, w1, w2");
                    if !is_unsigned {
                        self.state.emit("    sxtw x0, w0");
                    }
                }
                IrBinOp::Sub => {
                    self.state.emit("    sub w0, w1, w2");
                    if !is_unsigned {
                        self.state.emit("    sxtw x0, w0");
                    }
                }
                IrBinOp::Mul => {
                    self.state.emit("    mul w0, w1, w2");
                    if !is_unsigned {
                        self.state.emit("    sxtw x0, w0");
                    }
                }
                IrBinOp::SDiv => {
                    self.state.emit("    sdiv w0, w1, w2");
                    self.state.emit("    sxtw x0, w0");
                }
                IrBinOp::UDiv => self.state.emit("    udiv w0, w1, w2"),
                IrBinOp::SRem => {
                    self.state.emit("    sdiv w3, w1, w2");
                    self.state.emit("    msub w0, w3, w2, w1");
                    self.state.emit("    sxtw x0, w0");
                }
                IrBinOp::URem => {
                    self.state.emit("    udiv w3, w1, w2");
                    self.state.emit("    msub w0, w3, w2, w1");
                }
                IrBinOp::And => self.state.emit("    and w0, w1, w2"),
                IrBinOp::Or => self.state.emit("    orr w0, w1, w2"),
                IrBinOp::Xor => self.state.emit("    eor w0, w1, w2"),
                IrBinOp::Shl => {
                    self.state.emit("    lsl w0, w1, w2");
                    if !is_unsigned {
                        self.state.emit("    sxtw x0, w0");
                    }
                }
                IrBinOp::AShr => {
                    self.state.emit("    asr w0, w1, w2");
                    if !is_unsigned {
                        self.state.emit("    sxtw x0, w0");
                    }
                }
                IrBinOp::LShr => self.state.emit("    lsr w0, w1, w2"),
            }
        } else {
            match op {
                IrBinOp::Add => self.state.emit("    add x0, x1, x2"),
                IrBinOp::Sub => self.state.emit("    sub x0, x1, x2"),
                IrBinOp::Mul => self.state.emit("    mul x0, x1, x2"),
                IrBinOp::SDiv => self.state.emit("    sdiv x0, x1, x2"),
                IrBinOp::UDiv => self.state.emit("    udiv x0, x1, x2"),
                IrBinOp::SRem => {
                    self.state.emit("    sdiv x3, x1, x2");
                    self.state.emit("    msub x0, x3, x2, x1");
                }
                IrBinOp::URem => {
                    self.state.emit("    udiv x3, x1, x2");
                    self.state.emit("    msub x0, x3, x2, x1");
                }
                IrBinOp::And => self.state.emit("    and x0, x1, x2"),
                IrBinOp::Or => self.state.emit("    orr x0, x1, x2"),
                IrBinOp::Xor => self.state.emit("    eor x0, x1, x2"),
                IrBinOp::Shl => self.state.emit("    lsl x0, x1, x2"),
                IrBinOp::AShr => self.state.emit("    asr x0, x1, x2"),
                IrBinOp::LShr => self.state.emit("    lsr x0, x1, x2"),
            }
        }

        self.store_x0_to(dest);
    }

    pub(super) fn emit_copy_i128_impl(&mut self, dest: &Value, src: &Operand) {
        self.operand_to_x0_x1(src);
        self.store_x0_x1_to(dest);
    }
}
