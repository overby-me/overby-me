//! RiscvCodegen: type conversion/casting operations.

use super::emit::RiscvCodegen;
use crate::backend::cast::{classify_cast, CastKind};
use crate::common::types::IrType;
use crate::ir::reexports::{Operand, Value};

impl RiscvCodegen {
    pub(super) fn emit_cast_instrs_impl(&mut self, from_ty: IrType, to_ty: IrType) {
        match classify_cast(from_ty, to_ty) {
            CastKind::Noop => {}

            CastKind::UnsignedToSignedSameSize { to_ty } => {
                // On RISC-V, 32-bit values must be sign-extended in 64-bit registers
                // per the ABI. When converting U32 to I32, we must sign-extend
                // to ensure correct behavior when the value is used in comparisons
                // or passed to functions compiled by other compilers (e.g. glibc).
                if to_ty == IrType::I32 {
                    self.state.emit("    sext.w t0, t0");
                }
                // U8->I8, U16->I16, U64->I64 are true noops
            }

            CastKind::FloatToSigned { from_f64 } => {
                if from_f64 {
                    self.state.emit("    fmv.d.x ft0, t0");
                    self.state.emit("    fcvt.l.d t0, ft0, rtz");
                } else {
                    self.state.emit("    fmv.w.x ft0, t0");
                    self.state.emit("    fcvt.l.s t0, ft0, rtz");
                }
                match to_ty {
                    IrType::I8 => {
                        self.state.emit("    slli t0, t0, 56");
                        self.state.emit("    srai t0, t0, 56");
                    }
                    IrType::I16 => {
                        self.state.emit("    slli t0, t0, 48");
                        self.state.emit("    srai t0, t0, 48");
                    }
                    IrType::I32 => self.state.emit("    sext.w t0, t0"),
                    _ => {}
                }
            }

            CastKind::FloatToUnsigned { from_f64, .. } => {
                if from_f64 {
                    self.state.emit("    fmv.d.x ft0, t0");
                    self.state.emit("    fcvt.lu.d t0, ft0, rtz");
                } else {
                    self.state.emit("    fmv.w.x ft0, t0");
                    self.state.emit("    fcvt.lu.s t0, ft0, rtz");
                }
                match to_ty {
                    IrType::U8 => self.state.emit("    andi t0, t0, 0xff"),
                    IrType::U16 => {
                        self.state.emit("    slli t0, t0, 48");
                        self.state.emit("    srli t0, t0, 48");
                    }
                    IrType::U32 => {
                        self.state.emit("    slli t0, t0, 32");
                        self.state.emit("    srli t0, t0, 32");
                    }
                    _ => {}
                }
            }

            CastKind::SignedToFloat { to_f64, from_ty } => {
                match from_ty.size() {
                    1 => {
                        self.state.emit("    slli t0, t0, 56");
                        self.state.emit("    srai t0, t0, 56");
                    }
                    2 => {
                        self.state.emit("    slli t0, t0, 48");
                        self.state.emit("    srai t0, t0, 48");
                    }
                    4 => self.state.emit("    sext.w t0, t0"),
                    _ => {}
                }
                if to_f64 {
                    self.state.emit("    fcvt.d.l ft0, t0");
                    self.state.emit("    fmv.x.d t0, ft0");
                } else {
                    self.state.emit("    fcvt.s.l ft0, t0");
                    self.state.emit("    fmv.x.w t0, ft0");
                }
            }

            CastKind::UnsignedToFloat { to_f64, from_ty } => {
                let from_u64 = from_ty == IrType::U64;
                if from_u64 {
                    if to_f64 {
                        self.state.emit("    fcvt.d.lu ft0, t0");
                        self.state.emit("    fmv.x.d t0, ft0");
                    } else {
                        self.state.emit("    fcvt.s.lu ft0, t0");
                        self.state.emit("    fmv.x.w t0, ft0");
                    }
                } else if to_f64 {
                    self.state.emit("    fcvt.d.wu ft0, t0");
                    self.state.emit("    fmv.x.d t0, ft0");
                } else {
                    self.state.emit("    fcvt.s.wu ft0, t0");
                    self.state.emit("    fmv.x.w t0, ft0");
                }
            }

            CastKind::FloatToFloat { widen } => {
                if widen {
                    self.state.emit("    fmv.w.x ft0, t0");
                    self.state.emit("    fcvt.d.s ft0, ft0");
                    self.state.emit("    fmv.x.d t0, ft0");
                } else {
                    self.state.emit("    fmv.d.x ft0, t0");
                    self.state.emit("    fcvt.s.d ft0, ft0");
                    self.state.emit("    fmv.x.w t0, ft0");
                }
            }

            CastKind::SignedToUnsignedSameSize { to_ty } => match to_ty {
                IrType::U8 => self.state.emit("    andi t0, t0, 0xff"),
                IrType::U16 => {
                    self.state.emit("    slli t0, t0, 48");
                    self.state.emit("    srli t0, t0, 48");
                }
                IrType::U32 => {
                    self.state.emit("    slli t0, t0, 32");
                    self.state.emit("    srli t0, t0, 32");
                }
                _ => {}
            },

            CastKind::IntWiden { from_ty, .. } => {
                if from_ty.is_unsigned() {
                    match from_ty {
                        IrType::U8 => self.state.emit("    andi t0, t0, 0xff"),
                        IrType::U16 => {
                            self.state.emit("    slli t0, t0, 48");
                            self.state.emit("    srli t0, t0, 48");
                        }
                        IrType::U32 => {
                            self.state.emit("    slli t0, t0, 32");
                            self.state.emit("    srli t0, t0, 32");
                        }
                        _ => {}
                    }
                } else {
                    match from_ty {
                        IrType::I8 => {
                            self.state.emit("    slli t0, t0, 56");
                            self.state.emit("    srai t0, t0, 56");
                        }
                        IrType::I16 => {
                            self.state.emit("    slli t0, t0, 48");
                            self.state.emit("    srai t0, t0, 48");
                        }
                        IrType::I32 => self.state.emit("    sext.w t0, t0"),
                        _ => {}
                    }
                }
            }

            CastKind::IntNarrow { to_ty } => match to_ty {
                IrType::I8 => {
                    self.state.emit("    slli t0, t0, 56");
                    self.state.emit("    srai t0, t0, 56");
                }
                IrType::U8 => self.state.emit("    andi t0, t0, 0xff"),
                IrType::I16 => {
                    self.state.emit("    slli t0, t0, 48");
                    self.state.emit("    srai t0, t0, 48");
                }
                IrType::U16 => {
                    self.state.emit("    slli t0, t0, 48");
                    self.state.emit("    srli t0, t0, 48");
                }
                IrType::I32 => self.state.emit("    sext.w t0, t0"),
                IrType::U32 => {
                    self.state.emit("    slli t0, t0, 32");
                    self.state.emit("    srli t0, t0, 32");
                }
                _ => {}
            },

            CastKind::SignedToF128 { .. }
            | CastKind::UnsignedToF128 { .. }
            | CastKind::F128ToSigned { .. }
            | CastKind::F128ToUnsigned { .. }
            | CastKind::FloatToF128 { .. }
            | CastKind::F128ToFloat { .. } => {
                unreachable!("F128 casts should be handled by emit_cast override");
            }
        }
    }

    pub(super) fn emit_cast_impl(
        &mut self,
        dest: &Value,
        src: &Operand,
        from_ty: IrType,
        to_ty: IrType,
    ) {
        if crate::backend::f128_softfloat::f128_emit_cast(self, dest, src, from_ty, to_ty) {
            return;
        }
        crate::backend::traits::emit_cast_default(self, dest, src, from_ty, to_ty);
    }
}
