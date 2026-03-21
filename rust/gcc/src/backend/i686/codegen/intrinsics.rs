//! i686 SSE/AES/CRC intrinsic emission and x87 FPU math intrinsics.
//!
//! Handles the `emit_intrinsic` trait method for the i686 backend, covering:
//! - Memory fences (lfence, mfence, sfence, pause)
//! - Non-temporal stores (movnti, movntdq, movntpd)
//! - SSE/SSE2 128-bit packed operations
//! - AES-NI encryption/decryption
//! - CRC32 instructions
//! - Frame/return address intrinsics
//! - x87 FPU math (sqrt, fabs) for F32/F64

use super::emit::I686Codegen;
use crate::emit;
use crate::ir::reexports::{IntrinsicOp, IrConst, Operand, Value};

impl I686Codegen {
    pub(super) fn emit_intrinsic_impl(
        &mut self,
        dest: &Option<Value>,
        op: &IntrinsicOp,
        dest_ptr: &Option<Value>,
        args: &[Operand],
    ) {
        match op {
            // --- Memory fences (same x86 instructions as x86-64) ---
            IntrinsicOp::Lfence => {
                self.state.emit("    lfence");
            }
            IntrinsicOp::Mfence => {
                self.state.emit("    mfence");
            }
            IntrinsicOp::Sfence => {
                self.state.emit("    sfence");
            }
            IntrinsicOp::Pause => {
                self.state.emit("    pause");
            }
            IntrinsicOp::Clflush => {
                self.operand_to_eax(&args[0]);
                self.state.emit("    clflush (%eax)");
            }

            // --- Non-temporal stores ---
            IntrinsicOp::Movnti
            | IntrinsicOp::Movnti64
            | IntrinsicOp::Movntdq
            | IntrinsicOp::Movntpd => {
                self.emit_nontemporal_store(op, dest_ptr, args);
            }

            // --- SSE 128-bit load/store ---
            IntrinsicOp::Loaddqu => {
                if let Some(dptr) = dest_ptr {
                    self.operand_to_eax(&args[0]);
                    self.state.emit("    movdqu (%eax), %xmm0");
                    self.operand_to_eax(&Operand::Value(*dptr));
                    self.state.emit("    movdqu %xmm0, (%eax)");
                }
            }
            IntrinsicOp::Storedqu => {
                if let Some(ptr) = dest_ptr {
                    self.operand_to_eax(&args[0]);
                    self.state.emit("    movdqu (%eax), %xmm0");
                    self.operand_to_eax(&Operand::Value(*ptr));
                    self.state.emit("    movdqu %xmm0, (%eax)");
                }
            }

            // SSE 128-bit binary operations
            IntrinsicOp::Pcmpeqb128
            | IntrinsicOp::Pcmpeqd128
            | IntrinsicOp::Psubusb128
            | IntrinsicOp::Psubsb128
            | IntrinsicOp::Por128
            | IntrinsicOp::Pand128
            | IntrinsicOp::Pxor128 => {
                if let Some(dptr) = dest_ptr {
                    let inst = match op {
                        IntrinsicOp::Pcmpeqb128 => "pcmpeqb",
                        IntrinsicOp::Pcmpeqd128 => "pcmpeqd",
                        IntrinsicOp::Psubusb128 => "psubusb",
                        IntrinsicOp::Psubsb128 => "psubsb",
                        IntrinsicOp::Por128 => "por",
                        IntrinsicOp::Pand128 => "pand",
                        IntrinsicOp::Pxor128 => "pxor",
                        _ => unreachable!("unexpected SSE binary op: {:?}", op),
                    };
                    self.emit_sse_binary_128(dptr, args, inst);
                }
            }
            IntrinsicOp::Pmovmskb128 => {
                self.operand_to_eax(&args[0]);
                self.state.emit("    movdqu (%eax), %xmm0");
                self.state.emit("    pmovmskb %xmm0, %eax");
                self.state.reg_cache.invalidate_acc();
                if let Some(d) = dest {
                    self.store_eax_to(d);
                }
            }
            IntrinsicOp::SetEpi8 => {
                if let Some(dptr) = dest_ptr {
                    self.operand_to_eax(&args[0]);
                    self.state.emit("    movd %eax, %xmm0");
                    self.state.emit("    punpcklbw %xmm0, %xmm0");
                    self.state.emit("    punpcklwd %xmm0, %xmm0");
                    self.state.emit("    pshufd $0, %xmm0, %xmm0");
                    self.operand_to_eax(&Operand::Value(*dptr));
                    self.state.emit("    movdqu %xmm0, (%eax)");
                }
            }
            IntrinsicOp::SetEpi32 => {
                if let Some(dptr) = dest_ptr {
                    self.operand_to_eax(&args[0]);
                    self.state.emit("    movd %eax, %xmm0");
                    self.state.emit("    pshufd $0, %xmm0, %xmm0");
                    self.operand_to_eax(&Operand::Value(*dptr));
                    self.state.emit("    movdqu %xmm0, (%eax)");
                }
            }

            // --- CRC32 ---
            IntrinsicOp::Crc32_8
            | IntrinsicOp::Crc32_16
            | IntrinsicOp::Crc32_32
            | IntrinsicOp::Crc32_64 => {
                self.emit_crc32_intrinsic(op, dest, args);
            }

            // --- Frame and return address ---
            IntrinsicOp::FrameAddress => {
                self.state.emit("    movl %ebp, %eax");
                self.state.reg_cache.invalidate_acc();
                if let Some(d) = dest {
                    self.store_eax_to(d);
                }
            }
            IntrinsicOp::ReturnAddress => {
                // On i686, return address is at 4(%ebp) (32-bit stack frame)
                // With FP omission: param_ref(4) computes the correct ESP-relative offset
                let ra = self.param_ref(4);
                emit!(self.state, "    movl {}, %eax", ra);
                self.state.reg_cache.invalidate_acc();
                if let Some(d) = dest {
                    self.store_eax_to(d);
                }
            }
            IntrinsicOp::ThreadPointer => {
                // __builtin_thread_pointer(): read TLS base from %gs:0 on i686
                self.state.emit("    movl %gs:0, %eax");
                self.state.reg_cache.invalidate_acc();
                if let Some(d) = dest {
                    self.store_eax_to(d);
                }
            }

            // --- Floating-point intrinsics via x87 FPU ---
            IntrinsicOp::SqrtF64 => {
                self.emit_f64_unary_x87(&args[0], "fsqrt", dest);
            }
            IntrinsicOp::SqrtF32 => {
                self.emit_f32_load_to_x87(&args[0]);
                self.state.emit("    fsqrt");
                self.emit_f32_store_from_x87(dest);
            }
            IntrinsicOp::FabsF64 => {
                self.emit_f64_unary_x87(&args[0], "fabs", dest);
            }
            IntrinsicOp::FabsF32 => {
                self.emit_f32_load_to_x87(&args[0]);
                self.state.emit("    fabs");
                self.emit_f32_store_from_x87(dest);
            }

            // --- AES-NI ---
            IntrinsicOp::Aesenc128
            | IntrinsicOp::Aesenclast128
            | IntrinsicOp::Aesdec128
            | IntrinsicOp::Aesdeclast128 => {
                if let Some(dptr) = dest_ptr {
                    let inst = match op {
                        IntrinsicOp::Aesenc128 => "aesenc",
                        IntrinsicOp::Aesenclast128 => "aesenclast",
                        IntrinsicOp::Aesdec128 => "aesdec",
                        IntrinsicOp::Aesdeclast128 => "aesdeclast",
                        _ => unreachable!("AES-NI dispatch matched non-AES op: {:?}", op),
                    };
                    self.emit_sse_binary_128(dptr, args, inst);
                }
            }
            IntrinsicOp::Aesimc128 => {
                if let Some(dptr) = dest_ptr {
                    self.operand_to_eax(&args[0]);
                    self.state.emit("    movdqu (%eax), %xmm0");
                    self.state.emit("    aesimc %xmm0, %xmm0");
                    self.operand_to_eax(&Operand::Value(*dptr));
                    self.state.emit("    movdqu %xmm0, (%eax)");
                }
            }
            IntrinsicOp::Aeskeygenassist128 => {
                if let Some(dptr) = dest_ptr {
                    self.operand_to_eax(&args[0]);
                    self.state.emit("    movdqu (%eax), %xmm0");
                    let imm = Self::operand_to_imm_i64(&args[1]);
                    self.state
                        .emit_fmt(format_args!("    aeskeygenassist ${}, %xmm0, %xmm0", imm));
                    self.operand_to_eax(&Operand::Value(*dptr));
                    self.state.emit("    movdqu %xmm0, (%eax)");
                }
            }
            IntrinsicOp::Pclmulqdq128 => {
                if let Some(dptr) = dest_ptr {
                    self.operand_to_eax(&args[0]);
                    self.state.emit("    movdqu (%eax), %xmm0");
                    self.operand_to_eax(&args[1]);
                    self.state.emit("    movdqu (%eax), %xmm1");
                    let imm = Self::operand_to_imm_i64(&args[2]);
                    self.state
                        .emit_fmt(format_args!("    pclmulqdq ${}, %xmm1, %xmm0", imm));
                    self.operand_to_eax(&Operand::Value(*dptr));
                    self.state.emit("    movdqu %xmm0, (%eax)");
                }
            }

            // SSE2 shift-by-immediate operations
            IntrinsicOp::Pslldqi128
            | IntrinsicOp::Psrldqi128
            | IntrinsicOp::Psllqi128
            | IntrinsicOp::Psrlqi128 => {
                if let Some(dptr) = dest_ptr {
                    let inst = match op {
                        IntrinsicOp::Pslldqi128 => "pslldq",
                        IntrinsicOp::Psrldqi128 => "psrldq",
                        IntrinsicOp::Psllqi128 => "psllq",
                        IntrinsicOp::Psrlqi128 => "psrlq",
                        _ => unreachable!("unexpected SSE shift-by-immediate op: {:?}", op),
                    };
                    self.emit_sse_unary_imm_128(dptr, args, inst);
                }
            }
            // SSE2 shuffle with immediate
            IntrinsicOp::Pshufd128 => {
                if let Some(dptr) = dest_ptr {
                    self.emit_sse_shuffle_imm_128(dptr, args, "pshufd");
                }
            }
            IntrinsicOp::Loadldi128 => {
                if let Some(dptr) = dest_ptr {
                    self.operand_to_eax(&args[0]);
                    self.state.emit("    movq (%eax), %xmm0");
                    self.operand_to_eax(&Operand::Value(*dptr));
                    self.state.emit("    movdqu %xmm0, (%eax)");
                }
            }

            // SSE2 binary 128-bit operations
            IntrinsicOp::Paddw128
            | IntrinsicOp::Psubw128
            | IntrinsicOp::Pmulhw128
            | IntrinsicOp::Pmaddwd128
            | IntrinsicOp::Pcmpgtw128
            | IntrinsicOp::Pcmpgtb128
            | IntrinsicOp::Paddd128
            | IntrinsicOp::Psubd128
            | IntrinsicOp::Packssdw128
            | IntrinsicOp::Packsswb128
            | IntrinsicOp::Packuswb128
            | IntrinsicOp::Punpcklbw128
            | IntrinsicOp::Punpckhbw128
            | IntrinsicOp::Punpcklwd128
            | IntrinsicOp::Punpckhwd128 => {
                if let Some(dptr) = dest_ptr {
                    let inst = match op {
                        IntrinsicOp::Paddw128 => "paddw",
                        IntrinsicOp::Psubw128 => "psubw",
                        IntrinsicOp::Pmulhw128 => "pmulhw",
                        IntrinsicOp::Pmaddwd128 => "pmaddwd",
                        IntrinsicOp::Pcmpgtw128 => "pcmpgtw",
                        IntrinsicOp::Pcmpgtb128 => "pcmpgtb",
                        IntrinsicOp::Paddd128 => "paddd",
                        IntrinsicOp::Psubd128 => "psubd",
                        IntrinsicOp::Packssdw128 => "packssdw",
                        IntrinsicOp::Packsswb128 => "packsswb",
                        IntrinsicOp::Packuswb128 => "packuswb",
                        IntrinsicOp::Punpcklbw128 => "punpcklbw",
                        IntrinsicOp::Punpckhbw128 => "punpckhbw",
                        IntrinsicOp::Punpcklwd128 => "punpcklwd",
                        IntrinsicOp::Punpckhwd128 => "punpckhwd",
                        _ => unreachable!("unexpected SSE binary op: {:?}", op),
                    };
                    self.emit_sse_binary_128(dptr, args, inst);
                }
            }

            // SSE2 element shift-by-immediate operations
            IntrinsicOp::Psllwi128
            | IntrinsicOp::Psrlwi128
            | IntrinsicOp::Psrawi128
            | IntrinsicOp::Psradi128
            | IntrinsicOp::Pslldi128
            | IntrinsicOp::Psrldi128 => {
                if let Some(dptr) = dest_ptr {
                    let inst = match op {
                        IntrinsicOp::Psllwi128 => "psllw",
                        IntrinsicOp::Psrlwi128 => "psrlw",
                        IntrinsicOp::Psrawi128 => "psraw",
                        IntrinsicOp::Psradi128 => "psrad",
                        IntrinsicOp::Pslldi128 => "pslld",
                        IntrinsicOp::Psrldi128 => "psrld",
                        _ => unreachable!("unexpected SSE element shift op: {:?}", op),
                    };
                    self.emit_sse_unary_imm_128(dptr, args, inst);
                }
            }

            // --- SSE2 set/insert/extract/convert ---
            IntrinsicOp::SetEpi16 => {
                if let Some(dptr) = dest_ptr {
                    self.operand_to_eax(&args[0]);
                    self.state.emit("    movd %eax, %xmm0");
                    self.state.emit("    punpcklwd %xmm0, %xmm0");
                    self.state.emit("    pshufd $0, %xmm0, %xmm0");
                    self.operand_to_eax(&Operand::Value(*dptr));
                    self.state.emit("    movdqu %xmm0, (%eax)");
                }
            }
            IntrinsicOp::Pinsrw128 => {
                if let Some(dptr) = dest_ptr {
                    self.operand_to_eax(&args[0]);
                    self.state.emit("    movdqu (%eax), %xmm0");
                    self.operand_to_ecx(&args[1]);
                    let imm = Self::operand_to_imm_i64(&args[2]);
                    self.state
                        .emit_fmt(format_args!("    pinsrw ${}, %ecx, %xmm0", imm));
                    self.operand_to_eax(&Operand::Value(*dptr));
                    self.state.emit("    movdqu %xmm0, (%eax)");
                }
            }
            IntrinsicOp::Pextrw128 => {
                self.operand_to_eax(&args[0]);
                self.state.emit("    movdqu (%eax), %xmm0");
                let imm = Self::operand_to_imm_i64(&args[1]);
                self.state
                    .emit_fmt(format_args!("    pextrw ${}, %xmm0, %eax", imm));
                self.state.reg_cache.invalidate_acc();
                if let Some(d) = dest {
                    self.store_eax_to(d);
                }
            }
            IntrinsicOp::Pinsrd128 => {
                // Insert 32-bit value at lane: pinsrd $imm, %eax, %xmm0 (SSE4.1)
                if let Some(dptr) = dest_ptr {
                    self.operand_to_eax(&args[0]);
                    self.state.emit("    movdqu (%eax), %xmm0");
                    self.operand_to_ecx(&args[1]);
                    let imm = Self::operand_to_imm_i64(&args[2]);
                    self.state
                        .emit_fmt(format_args!("    pinsrd ${}, %ecx, %xmm0", imm));
                    self.operand_to_eax(&Operand::Value(*dptr));
                    self.state.emit("    movdqu %xmm0, (%eax)");
                }
            }
            IntrinsicOp::Pextrd128 => {
                // Extract 32-bit value at lane: pextrd $imm, %xmm0, %eax (SSE4.1)
                self.operand_to_eax(&args[0]);
                self.state.emit("    movdqu (%eax), %xmm0");
                let imm = Self::operand_to_imm_i64(&args[1]);
                self.state
                    .emit_fmt(format_args!("    pextrd ${}, %xmm0, %eax", imm));
                self.state.reg_cache.invalidate_acc();
                if let Some(d) = dest {
                    self.store_eax_to(d);
                }
            }
            IntrinsicOp::Pinsrb128 => {
                // Insert 8-bit value at lane: pinsrb $imm, %eax, %xmm0 (SSE4.1)
                if let Some(dptr) = dest_ptr {
                    self.operand_to_eax(&args[0]);
                    self.state.emit("    movdqu (%eax), %xmm0");
                    self.operand_to_ecx(&args[1]);
                    let imm = Self::operand_to_imm_i64(&args[2]);
                    self.state
                        .emit_fmt(format_args!("    pinsrb ${}, %ecx, %xmm0", imm));
                    self.operand_to_eax(&Operand::Value(*dptr));
                    self.state.emit("    movdqu %xmm0, (%eax)");
                }
            }
            IntrinsicOp::Pextrb128 => {
                // Extract 8-bit value at lane: pextrb $imm, %xmm0, %eax (SSE4.1)
                self.operand_to_eax(&args[0]);
                self.state.emit("    movdqu (%eax), %xmm0");
                let imm = Self::operand_to_imm_i64(&args[1]);
                self.state
                    .emit_fmt(format_args!("    pextrb ${}, %xmm0, %eax", imm));
                self.state.reg_cache.invalidate_acc();
                if let Some(d) = dest {
                    self.store_eax_to(d);
                }
            }
            IntrinsicOp::Pinsrq128 => {
                // TODO: PINSRQ is not available on i686 - could emulate with two PINSRD
                // Currently just copies input unchanged (no-op)
                if let Some(dptr) = dest_ptr {
                    self.operand_to_eax(&args[0]);
                    self.state.emit("    movdqu (%eax), %xmm0");
                    self.operand_to_eax(&Operand::Value(*dptr));
                    self.state.emit("    movdqu %xmm0, (%eax)");
                }
            }
            IntrinsicOp::Pextrq128 => {
                // TODO: PEXTRQ is not available on i686 - could emulate with MOVQ or two PEXTRD
                // Currently only extracts low 32 bits as fallback
                self.operand_to_eax(&args[0]);
                self.state.emit("    movdqu (%eax), %xmm0");
                self.state.emit("    movd %xmm0, %eax");
                self.state.reg_cache.invalidate_acc();
                if let Some(d) = dest {
                    self.store_eax_to(d);
                }
            }
            IntrinsicOp::Storeldi128 => {
                if let Some(ptr) = dest_ptr {
                    self.operand_to_eax(&args[0]);
                    self.state.emit("    movdqu (%eax), %xmm0");
                    self.operand_to_eax(&Operand::Value(*ptr));
                    self.state.emit("    movq %xmm0, (%eax)");
                }
            }
            IntrinsicOp::Cvtsi128Si32 => {
                self.operand_to_eax(&args[0]);
                self.state.emit("    movdqu (%eax), %xmm0");
                self.state.emit("    movd %xmm0, %eax");
                self.state.reg_cache.invalidate_acc();
                if let Some(d) = dest {
                    self.store_eax_to(d);
                }
            }
            IntrinsicOp::Cvtsi32Si128 => {
                if let Some(dptr) = dest_ptr {
                    self.operand_to_eax(&args[0]);
                    self.state.emit("    movd %eax, %xmm0");
                    self.operand_to_eax(&Operand::Value(*dptr));
                    self.state.emit("    movdqu %xmm0, (%eax)");
                }
            }
            IntrinsicOp::Cvtsi128Si64 => {
                // On i686, only extracts the low 32 bits
                self.operand_to_eax(&args[0]);
                self.state.emit("    movdqu (%eax), %xmm0");
                self.state.emit("    movd %xmm0, %eax");
                self.state.reg_cache.invalidate_acc();
                if let Some(d) = dest {
                    self.store_eax_to(d);
                }
            }
            IntrinsicOp::Pshuflw128 | IntrinsicOp::Pshufhw128 => {
                if let Some(dptr) = dest_ptr {
                    let inst = match op {
                        IntrinsicOp::Pshuflw128 => "pshuflw",
                        IntrinsicOp::Pshufhw128 => "pshufhw",
                        _ => unreachable!("unexpected SSE shuffle op: {:?}", op),
                    };
                    self.emit_sse_shuffle_imm_128(dptr, args, inst);
                }
            }
        }
        self.state.reg_cache.invalidate_acc();
    }

    fn emit_nontemporal_store(
        &mut self,
        op: &IntrinsicOp,
        dest_ptr: &Option<Value>,
        args: &[Operand],
    ) {
        let Some(ptr) = dest_ptr else { return };
        match op {
            IntrinsicOp::Movnti => {
                self.operand_to_eax(&args[0]);
                self.state.emit("    movl %eax, %ecx");
                self.operand_to_eax(&Operand::Value(*ptr));
                self.state.emit("    movnti %ecx, (%eax)");
            }
            IntrinsicOp::Movnti64 => {
                self.operand_to_eax(&Operand::Value(*ptr));
                self.state.emit("    movl %eax, %ecx");
                if let Operand::Value(v) = &args[0] {
                    if let Some(slot) = self.state.get_slot(v.0) {
                        let sr0 = self.slot_ref(slot);
                        let sr4 = self.slot_ref_offset(slot, 4);
                        emit!(self.state, "    movl {}, %eax", sr0);
                        self.state.emit("    movnti %eax, (%ecx)");
                        emit!(self.state, "    movl {}, %eax", sr4);
                        self.state.emit("    movnti %eax, 4(%ecx)");
                    } else {
                        self.operand_to_eax(&args[0]);
                        self.state.emit("    movnti %eax, (%ecx)");
                        self.state.emit("    xorl %eax, %eax");
                        self.state.emit("    movnti %eax, 4(%ecx)");
                    }
                } else {
                    self.operand_to_eax(&args[0]);
                    self.state.emit("    movnti %eax, (%ecx)");
                    self.state.emit("    xorl %eax, %eax");
                    self.state.emit("    movnti %eax, 4(%ecx)");
                }
            }
            IntrinsicOp::Movntdq => {
                self.operand_to_eax(&args[0]);
                self.state.emit("    movdqu (%eax), %xmm0");
                self.operand_to_eax(&Operand::Value(*ptr));
                self.state.emit("    movntdq %xmm0, (%eax)");
            }
            IntrinsicOp::Movntpd => {
                self.operand_to_eax(&args[0]);
                self.state.emit("    movupd (%eax), %xmm0");
                self.operand_to_eax(&Operand::Value(*ptr));
                self.state.emit("    movntpd %xmm0, (%eax)");
            }
            _ => {}
        }
    }

    fn emit_crc32_intrinsic(&mut self, op: &IntrinsicOp, dest: &Option<Value>, args: &[Operand]) {
        if *op == IntrinsicOp::Crc32_64 {
            // On i686, no 64-bit CRC32; do two 32-bit CRC32s
            self.operand_to_eax(&args[0]);
            self.state.emit("    movl %eax, %edx");
            if let Operand::Value(v) = &args[1] {
                if let Some(slot) = self.state.get_slot(v.0) {
                    let sr0 = self.slot_ref(slot);
                    let sr4 = self.slot_ref_offset(slot, 4);
                    emit!(self.state, "    movl {}, %ecx", sr0);
                    self.state.emit("    movl %edx, %eax");
                    self.state.emit("    crc32l %ecx, %eax");
                    emit!(self.state, "    movl {}, %ecx", sr4);
                    self.state.emit("    crc32l %ecx, %eax");
                } else {
                    self.operand_to_ecx(&args[1]);
                    self.state.emit("    movl %edx, %eax");
                    self.state.emit("    crc32l %ecx, %eax");
                }
            } else {
                self.operand_to_ecx(&args[1]);
                self.state.emit("    movl %edx, %eax");
                self.state.emit("    crc32l %ecx, %eax");
            }
        } else {
            self.operand_to_eax(&args[0]);
            self.state.emit("    movl %eax, %ecx");
            self.operand_to_eax(&args[1]);
            self.state.emit("    xchgl %eax, %ecx");
            let inst = match op {
                IntrinsicOp::Crc32_8 => "crc32b %cl, %eax",
                IntrinsicOp::Crc32_16 => "crc32w %cx, %eax",
                IntrinsicOp::Crc32_32 => "crc32l %ecx, %eax",
                _ => unreachable!("unexpected CRC32 op: {:?}", op),
            };
            self.state.emit_fmt(format_args!("    {}", inst));
        }
        self.state.reg_cache.invalidate_acc();
        if let Some(d) = dest {
            self.store_eax_to(d);
        }
    }

    /// Apply an x87 unary FPU op on an f64 operand and store the result.
    fn emit_f64_unary_x87(&mut self, arg: &Operand, x87_op: &str, dest: &Option<Value>) {
        self.emit_f64_load_to_x87(arg);
        self.state.emit_fmt(format_args!("    {}", x87_op));
        if let Some(d) = dest {
            if let Some(slot) = self.state.get_slot(d.0) {
                let sr = self.slot_ref(slot);
                emit!(self.state, "    fstpl {}", sr);
            } else {
                self.state.emit("    fstp %st(0)");
            }
        } else {
            self.state.emit("    fstp %st(0)");
        }
    }

    /// Emit a binary SSE 128-bit operation: load two 128-bit operands from
    /// pointers, apply the operation, and store the result to dest_ptr.
    fn emit_sse_binary_128(&mut self, dptr: &Value, args: &[Operand], sse_inst: &str) {
        self.operand_to_eax(&args[0]);
        self.state.emit("    movdqu (%eax), %xmm0");
        self.operand_to_eax(&args[1]);
        self.state.emit("    movdqu (%eax), %xmm1");
        self.state
            .emit_fmt(format_args!("    {} %xmm1, %xmm0", sse_inst));
        self.operand_to_eax(&Operand::Value(*dptr));
        self.state.emit("    movdqu %xmm0, (%eax)");
    }

    /// Emit SSE unary 128-bit op with immediate: load xmm0 from arg0 ptr,
    /// apply `inst $imm, %xmm0`, store result xmm0 to dest_ptr.
    fn emit_sse_unary_imm_128(&mut self, dptr: &Value, args: &[Operand], sse_inst: &str) {
        self.operand_to_eax(&args[0]);
        self.state.emit("    movdqu (%eax), %xmm0");
        let imm = Self::operand_to_imm_i64(&args[1]);
        self.state
            .emit_fmt(format_args!("    {} ${}, %xmm0", sse_inst, imm));
        self.operand_to_eax(&Operand::Value(*dptr));
        self.state.emit("    movdqu %xmm0, (%eax)");
    }

    /// Emit SSE shuffle with immediate: load xmm0, apply `inst $imm, %xmm0, %xmm0`,
    /// store result. Used for pshufd/pshuflw/pshufhw.
    fn emit_sse_shuffle_imm_128(&mut self, dptr: &Value, args: &[Operand], sse_inst: &str) {
        self.operand_to_eax(&args[0]);
        self.state.emit("    movdqu (%eax), %xmm0");
        let imm = Self::operand_to_imm_i64(&args[1]);
        self.state
            .emit_fmt(format_args!("    {} ${}, %xmm0, %xmm0", sse_inst, imm));
        self.operand_to_eax(&Operand::Value(*dptr));
        self.state.emit("    movdqu %xmm0, (%eax)");
    }

    /// Load an F32 operand onto the x87 FPU stack.
    fn emit_f32_load_to_x87(&mut self, op: &Operand) {
        match op {
            Operand::Value(v) if self.state.get_slot(v.0).is_some() => {
                let slot = self
                    .state
                    .get_slot(v.0)
                    .expect("slot exists (guarded by is_some)");
                let sr = self.slot_ref(slot);
                emit!(self.state, "    flds {}", sr);
            }
            Operand::Const(IrConst::F32(fval)) => {
                emit!(self.state, "    movl ${}, %eax", fval.to_bits() as i32);
                self.state.emit("    pushl %eax");
                self.state.emit("    flds (%esp)");
                self.state.emit("    addl $4, %esp");
            }
            _ => {
                self.operand_to_eax(op);
                self.state.emit("    pushl %eax");
                self.state.emit("    flds (%esp)");
                self.state.emit("    addl $4, %esp");
            }
        }
    }

    /// Store an x87 FPU result as F32 to a destination value.
    fn emit_f32_store_from_x87(&mut self, dest: &Option<Value>) {
        if let Some(d) = dest {
            if let Some(slot) = self.state.get_slot(d.0) {
                let sr = self.slot_ref(slot);
                emit!(self.state, "    fstps {}", sr);
            } else {
                self.state.emit("    fstp %st(0)");
            }
        } else {
            self.state.emit("    fstp %st(0)");
        }
    }
}
