//! x86-64 SSE/AES/CRC intrinsic emission and floating-point math intrinsics.
//!
//! Handles the `emit_intrinsic` trait method for the x86-64 backend, covering:
//! - Memory fences (lfence, mfence, sfence, pause, clflush)
//! - Non-temporal stores (movnti, movntdq, movntpd)
//! - SSE/SSE2 128-bit packed operations (arithmetic, compare, shuffle, shift)
//! - SSE2 element insertion/extraction and type conversion
//! - AES-NI encryption/decryption and key generation
//! - CLMUL carry-less multiplication
//! - CRC32 instructions
//! - Frame/return address intrinsics
//! - SSE scalar float math (sqrt, fabs) for F32/F64

use super::emit::X86Codegen;
use crate::ir::reexports::{IntrinsicOp, IrConst, Operand, Value};

impl X86Codegen {
    /// Load a float operand into %xmm0. Handles both Value operands (from stack)
    /// and float constants (loaded via their bit pattern into rax first).
    fn float_operand_to_xmm0(&mut self, op: &Operand, is_f32: bool) {
        match op {
            Operand::Const(c) => {
                match c {
                    IrConst::F64(v) => {
                        let bits = v.to_bits() as i64;
                        if bits == 0 {
                            self.state.emit("    xorpd %xmm0, %xmm0");
                        } else if bits >= i32::MIN as i64 && bits <= i32::MAX as i64 {
                            self.state.out.emit_instr_imm_reg("    movq", bits, "rax");
                            self.state.emit("    movq %rax, %xmm0");
                        } else {
                            self.state
                                .out
                                .emit_instr_imm_reg("    movabsq", bits, "rax");
                            self.state.emit("    movq %rax, %xmm0");
                        }
                    }
                    IrConst::F32(v) => {
                        let bits = v.to_bits() as i32;
                        if bits == 0 {
                            self.state.emit("    xorps %xmm0, %xmm0");
                        } else {
                            self.state
                                .out
                                .emit_instr_imm_reg("    movl", bits as i64, "eax");
                            self.state.emit("    movd %eax, %xmm0");
                        }
                    }
                    _ => {
                        // Integer or other constants - load to rax and move to xmm
                        self.operand_to_reg(op, "rax");
                        if is_f32 {
                            self.state.emit("    movd %eax, %xmm0");
                        } else {
                            self.state.emit("    movq %rax, %xmm0");
                        }
                    }
                }
            }
            Operand::Value(_) => {
                // Load from stack slot to rax, then to xmm0
                self.operand_to_reg(op, "rax");
                if is_f32 {
                    self.state.emit("    movd %eax, %xmm0");
                } else {
                    self.state.emit("    movq %rax, %xmm0");
                }
            }
        }
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
                self.operand_to_reg(&args[0], "rcx");
                self.value_to_reg(ptr, "rax");
                self.state.emit("    movnti %ecx, (%rax)");
            }
            IntrinsicOp::Movnti64 => {
                self.operand_to_reg(&args[0], "rcx");
                self.value_to_reg(ptr, "rax");
                self.state.emit("    movnti %rcx, (%rax)");
            }
            IntrinsicOp::Movntdq => {
                self.operand_to_reg(&args[0], "rcx");
                self.state.emit("    movdqu (%rcx), %xmm0");
                self.value_to_reg(ptr, "rax");
                self.state.emit("    movntdq %xmm0, (%rax)");
            }
            IntrinsicOp::Movntpd => {
                self.operand_to_reg(&args[0], "rcx");
                self.state.emit("    movupd (%rcx), %xmm0");
                self.value_to_reg(ptr, "rax");
                self.state.emit("    movntpd %xmm0, (%rax)");
            }
            _ => {}
        }
    }

    /// Emit SSE binary 128-bit op: load xmm0 from arg0 ptr, xmm1 from arg1 ptr,
    /// apply the given SSE instruction, store result xmm0 to dest_ptr.
    fn emit_sse_binary_128(&mut self, dest_ptr: &Value, args: &[Operand], sse_inst: &str) {
        self.operand_to_reg(&args[0], "rax");
        self.state.emit("    movdqu (%rax), %xmm0");
        self.operand_to_reg(&args[1], "rcx");
        self.state.emit("    movdqu (%rcx), %xmm1");
        self.state
            .emit_fmt(format_args!("    {} %xmm1, %xmm0", sse_inst));
        self.value_to_reg(dest_ptr, "rax");
        self.state.emit("    movdqu %xmm0, (%rax)");
    }

    /// Emit SSE unary 128-bit op with immediate: load xmm0 from arg0 ptr,
    /// apply `inst $imm, %xmm0`, store result xmm0 to dest_ptr.
    fn emit_sse_unary_imm_128(&mut self, dest_ptr: &Value, args: &[Operand], sse_inst: &str) {
        self.operand_to_reg(&args[0], "rax");
        self.state.emit("    movdqu (%rax), %xmm0");
        let imm = self.operand_to_imm_i64(&args[1]);
        self.state
            .emit_fmt(format_args!("    {} ${}, %xmm0", sse_inst, imm));
        self.value_to_reg(dest_ptr, "rax");
        self.state.emit("    movdqu %xmm0, (%rax)");
    }

    /// Emit SSE shuffle with immediate: load xmm0, apply `inst $imm, %xmm0, %xmm0`,
    /// store result. Used for pshufd/pshuflw/pshufhw which read and write same register.
    fn emit_sse_shuffle_imm_128(&mut self, dest_ptr: &Value, args: &[Operand], sse_inst: &str) {
        self.operand_to_reg(&args[0], "rax");
        self.state.emit("    movdqu (%rax), %xmm0");
        let imm = self.operand_to_imm_i64(&args[1]);
        self.state
            .emit_fmt(format_args!("    {} ${}, %xmm0, %xmm0", sse_inst, imm));
        self.value_to_reg(dest_ptr, "rax");
        self.state.emit("    movdqu %xmm0, (%rax)");
    }

    pub(super) fn emit_intrinsic_impl(
        &mut self,
        dest: &Option<Value>,
        op: &IntrinsicOp,
        dest_ptr: &Option<Value>,
        args: &[Operand],
    ) {
        match op {
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
                // args[0] = pointer to flush
                self.operand_to_reg(&args[0], "rax");
                self.state.emit("    clflush (%rax)");
            }
            IntrinsicOp::Movnti
            | IntrinsicOp::Movnti64
            | IntrinsicOp::Movntdq
            | IntrinsicOp::Movntpd => {
                self.emit_nontemporal_store(op, dest_ptr, args);
            }
            IntrinsicOp::Loaddqu => {
                if let Some(dptr) = dest_ptr {
                    self.operand_to_reg(&args[0], "rax");
                    self.state.emit("    movdqu (%rax), %xmm0");
                    self.value_to_reg(dptr, "rax");
                    self.state.emit("    movdqu %xmm0, (%rax)");
                }
            }
            IntrinsicOp::Storedqu => {
                if let Some(ptr) = dest_ptr {
                    self.operand_to_reg(&args[0], "rcx");
                    self.state.emit("    movdqu (%rcx), %xmm0");
                    self.value_to_reg(ptr, "rax");
                    self.state.emit("    movdqu %xmm0, (%rax)");
                }
            }
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
                self.operand_to_reg(&args[0], "rax");
                self.state.emit("    movdqu (%rax), %xmm0");
                self.state.emit("    pmovmskb %xmm0, %eax");
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
            }
            IntrinsicOp::SetEpi8 => {
                if let Some(dptr) = dest_ptr {
                    self.operand_to_reg(&args[0], "rax");
                    self.state.emit("    movd %eax, %xmm0");
                    self.state.emit("    punpcklbw %xmm0, %xmm0");
                    self.state.emit("    punpcklwd %xmm0, %xmm0");
                    self.state.emit("    pshufd $0, %xmm0, %xmm0");
                    self.value_to_reg(dptr, "rax");
                    self.state.emit("    movdqu %xmm0, (%rax)");
                }
            }
            IntrinsicOp::SetEpi32 => {
                if let Some(dptr) = dest_ptr {
                    self.operand_to_reg(&args[0], "rax");
                    self.state.emit("    movd %eax, %xmm0");
                    self.state.emit("    pshufd $0, %xmm0, %xmm0");
                    self.value_to_reg(dptr, "rax");
                    self.state.emit("    movdqu %xmm0, (%rax)");
                }
            }
            IntrinsicOp::Crc32_8
            | IntrinsicOp::Crc32_16
            | IntrinsicOp::Crc32_32
            | IntrinsicOp::Crc32_64 => {
                self.operand_to_reg(&args[0], "rax");
                self.operand_to_reg(&args[1], "rcx");
                let inst = match op {
                    IntrinsicOp::Crc32_8 => "crc32b %cl, %eax",
                    IntrinsicOp::Crc32_16 => "crc32w %cx, %eax",
                    IntrinsicOp::Crc32_32 => "crc32l %ecx, %eax",
                    IntrinsicOp::Crc32_64 => "crc32q %rcx, %rax",
                    _ => unreachable!("CRC32 dispatch matched non-CRC32 op: {:?}", op),
                };
                self.state.emit_fmt(format_args!("    {}", inst));
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
            }
            IntrinsicOp::FrameAddress => {
                // __builtin_frame_address(0): return current frame pointer (rbp)
                self.state.emit("    movq %rbp, %rax");
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
            }
            IntrinsicOp::ReturnAddress => {
                // __builtin_return_address(0): return address is at (%rbp)+8
                self.state.emit("    movq 8(%rbp), %rax");
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
            }
            IntrinsicOp::ThreadPointer => {
                // __builtin_thread_pointer(): read the TLS base from %fs:0
                self.state.emit("    movq %fs:0, %rax");
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
            }
            IntrinsicOp::SqrtF64 => {
                // sqrtsd: scalar double-precision square root
                self.float_operand_to_xmm0(&args[0], false);
                self.state.emit("    sqrtsd %xmm0, %xmm0");
                self.state.emit("    movq %xmm0, %rax");
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
            }
            IntrinsicOp::SqrtF32 => {
                // sqrtss: scalar single-precision square root
                self.float_operand_to_xmm0(&args[0], true);
                self.state.emit("    sqrtss %xmm0, %xmm0");
                self.state.emit("    movd %xmm0, %eax");
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
            }
            IntrinsicOp::FabsF64 => {
                // Clear sign bit for double-precision absolute value
                self.float_operand_to_xmm0(&args[0], false);
                self.state.emit("    movabsq $0x7FFFFFFFFFFFFFFF, %rcx");
                self.state.emit("    movq %rcx, %xmm1");
                self.state.emit("    andpd %xmm1, %xmm0");
                self.state.emit("    movq %xmm0, %rax");
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
            }
            IntrinsicOp::FabsF32 => {
                // Clear sign bit for single-precision absolute value
                self.float_operand_to_xmm0(&args[0], true);
                self.state.emit("    movl $0x7FFFFFFF, %ecx");
                self.state.emit("    movd %ecx, %xmm1");
                self.state.emit("    andps %xmm1, %xmm0");
                self.state.emit("    movd %xmm0, %eax");
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
            }
            // AES-NI binary ops: aesenc, aesenclast, aesdec, aesdeclast
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
            // AES-NI unary: aesimc
            IntrinsicOp::Aesimc128 => {
                if let Some(dptr) = dest_ptr {
                    self.operand_to_reg(&args[0], "rax");
                    self.state.emit("    movdqu (%rax), %xmm0");
                    self.state.emit("    aesimc %xmm0, %xmm0");
                    self.value_to_reg(dptr, "rax");
                    self.state.emit("    movdqu %xmm0, (%rax)");
                }
            }
            // AES-NI: aeskeygenassist with immediate
            IntrinsicOp::Aeskeygenassist128 => {
                if let Some(dptr) = dest_ptr {
                    self.operand_to_reg(&args[0], "rax");
                    self.state.emit("    movdqu (%rax), %xmm0");
                    // args[1] is the immediate value
                    let imm = self.operand_to_imm_i64(&args[1]);
                    self.state
                        .emit_fmt(format_args!("    aeskeygenassist ${}, %xmm0, %xmm0", imm));
                    self.value_to_reg(dptr, "rax");
                    self.state.emit("    movdqu %xmm0, (%rax)");
                }
            }
            // CLMUL: pclmulqdq with immediate
            IntrinsicOp::Pclmulqdq128 => {
                if let Some(dptr) = dest_ptr {
                    self.operand_to_reg(&args[0], "rax");
                    self.state.emit("    movdqu (%rax), %xmm0");
                    self.operand_to_reg(&args[1], "rcx");
                    self.state.emit("    movdqu (%rcx), %xmm1");
                    let imm = self.operand_to_imm_i64(&args[2]);
                    self.state
                        .emit_fmt(format_args!("    pclmulqdq ${}, %xmm1, %xmm0", imm));
                    self.value_to_reg(dptr, "rax");
                    self.state.emit("    movdqu %xmm0, (%rax)");
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
            // SSE2 shuffle with immediate (3-operand form: inst $imm, %src, %dst)
            IntrinsicOp::Pshufd128 => {
                if let Some(dptr) = dest_ptr {
                    self.emit_sse_shuffle_imm_128(dptr, args, "pshufd");
                }
            }
            // Load low 64 bits, zero upper (MOVQ)
            IntrinsicOp::Loadldi128 => {
                if let Some(dptr) = dest_ptr {
                    self.operand_to_reg(&args[0], "rax");
                    self.state.emit("    movq (%rax), %xmm0");
                    self.value_to_reg(dptr, "rax");
                    self.state.emit("    movdqu %xmm0, (%rax)");
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
                // Broadcast 16-bit value to all 8 lanes
                if let Some(dptr) = dest_ptr {
                    self.operand_to_reg(&args[0], "rax");
                    self.state.emit("    movd %eax, %xmm0");
                    self.state.emit("    punpcklwd %xmm0, %xmm0");
                    self.state.emit("    pshufd $0, %xmm0, %xmm0");
                    self.value_to_reg(dptr, "rax");
                    self.state.emit("    movdqu %xmm0, (%rax)");
                }
            }
            IntrinsicOp::Pinsrw128 => {
                // Insert 16-bit value at lane: pinsrw $imm, %eax, %xmm0
                if let Some(dptr) = dest_ptr {
                    self.operand_to_reg(&args[0], "rax");
                    self.state.emit("    movdqu (%rax), %xmm0");
                    self.operand_to_reg(&args[1], "rcx");
                    let imm = self.operand_to_imm_i64(&args[2]);
                    self.state
                        .emit_fmt(format_args!("    pinsrw ${}, %ecx, %xmm0", imm));
                    self.value_to_reg(dptr, "rax");
                    self.state.emit("    movdqu %xmm0, (%rax)");
                }
            }
            IntrinsicOp::Pextrw128 => {
                // Extract 16-bit value at lane: pextrw $imm, %xmm0, %eax
                self.operand_to_reg(&args[0], "rax");
                self.state.emit("    movdqu (%rax), %xmm0");
                let imm = self.operand_to_imm_i64(&args[1]);
                self.state
                    .emit_fmt(format_args!("    pextrw ${}, %xmm0, %eax", imm));
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
            }
            IntrinsicOp::Pinsrd128 => {
                // Insert 32-bit value at lane: pinsrd $imm, %eax, %xmm0 (SSE4.1)
                if let Some(dptr) = dest_ptr {
                    self.operand_to_reg(&args[0], "rax");
                    self.state.emit("    movdqu (%rax), %xmm0");
                    self.operand_to_reg(&args[1], "rcx");
                    let imm = self.operand_to_imm_i64(&args[2]);
                    self.state
                        .emit_fmt(format_args!("    pinsrd ${}, %ecx, %xmm0", imm));
                    self.value_to_reg(dptr, "rax");
                    self.state.emit("    movdqu %xmm0, (%rax)");
                }
            }
            IntrinsicOp::Pextrd128 => {
                // Extract 32-bit value at lane: pextrd $imm, %xmm0, %eax (SSE4.1)
                self.operand_to_reg(&args[0], "rax");
                self.state.emit("    movdqu (%rax), %xmm0");
                let imm = self.operand_to_imm_i64(&args[1]);
                self.state
                    .emit_fmt(format_args!("    pextrd ${}, %xmm0, %eax", imm));
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
            }
            IntrinsicOp::Pinsrb128 => {
                // Insert 8-bit value at lane: pinsrb $imm, %eax, %xmm0 (SSE4.1)
                if let Some(dptr) = dest_ptr {
                    self.operand_to_reg(&args[0], "rax");
                    self.state.emit("    movdqu (%rax), %xmm0");
                    self.operand_to_reg(&args[1], "rcx");
                    let imm = self.operand_to_imm_i64(&args[2]);
                    self.state
                        .emit_fmt(format_args!("    pinsrb ${}, %ecx, %xmm0", imm));
                    self.value_to_reg(dptr, "rax");
                    self.state.emit("    movdqu %xmm0, (%rax)");
                }
            }
            IntrinsicOp::Pextrb128 => {
                // Extract 8-bit value at lane: pextrb $imm, %xmm0, %eax (SSE4.1)
                self.operand_to_reg(&args[0], "rax");
                self.state.emit("    movdqu (%rax), %xmm0");
                let imm = self.operand_to_imm_i64(&args[1]);
                self.state
                    .emit_fmt(format_args!("    pextrb ${}, %xmm0, %eax", imm));
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
            }
            IntrinsicOp::Pinsrq128 => {
                // Insert 64-bit value at lane: pinsrq $imm, %rax, %xmm0 (SSE4.1)
                if let Some(dptr) = dest_ptr {
                    self.operand_to_reg(&args[0], "rax");
                    self.state.emit("    movdqu (%rax), %xmm0");
                    self.operand_to_reg(&args[1], "rcx");
                    let imm = self.operand_to_imm_i64(&args[2]);
                    self.state
                        .emit_fmt(format_args!("    pinsrq ${}, %rcx, %xmm0", imm));
                    self.value_to_reg(dptr, "rax");
                    self.state.emit("    movdqu %xmm0, (%rax)");
                }
            }
            IntrinsicOp::Pextrq128 => {
                // Extract 64-bit value at lane: pextrq $imm, %xmm0, %rax (SSE4.1)
                self.operand_to_reg(&args[0], "rax");
                self.state.emit("    movdqu (%rax), %xmm0");
                let imm = self.operand_to_imm_i64(&args[1]);
                self.state
                    .emit_fmt(format_args!("    pextrq ${}, %xmm0, %rax", imm));
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
            }
            IntrinsicOp::Storeldi128 => {
                // Store low 64 bits to memory (MOVQ)
                if let Some(ptr) = dest_ptr {
                    self.operand_to_reg(&args[0], "rcx");
                    self.state.emit("    movdqu (%rcx), %xmm0");
                    self.value_to_reg(ptr, "rax");
                    self.state.emit("    movq %xmm0, (%rax)");
                }
            }
            IntrinsicOp::Cvtsi128Si32 => {
                // Extract low 32-bit integer (MOVD)
                self.operand_to_reg(&args[0], "rax");
                self.state.emit("    movdqu (%rax), %xmm0");
                self.state.emit("    movd %xmm0, %eax");
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
            }
            IntrinsicOp::Cvtsi32Si128 => {
                // Convert int to __m128i (MOVD, zero-extends upper bits)
                if let Some(dptr) = dest_ptr {
                    self.operand_to_reg(&args[0], "rax");
                    self.state.emit("    movd %eax, %xmm0");
                    self.value_to_reg(dptr, "rax");
                    self.state.emit("    movdqu %xmm0, (%rax)");
                }
            }
            IntrinsicOp::Cvtsi128Si64 => {
                // Extract low 64-bit integer (MOVQ)
                self.operand_to_reg(&args[0], "rax");
                self.state.emit("    movdqu (%rax), %xmm0");
                self.state.emit("    movq %xmm0, %rax");
                if let Some(d) = dest {
                    self.store_rax_to(d);
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
    }
}
