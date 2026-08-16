//! RISC-V intrinsic/SIMD emission: software emulation of SSE-equivalent operations
//! (128-bit bitwise, byte compare, saturating subtract, pmovmskb) using scalar
//! RISC-V instructions, plus hardware intrinsics (fences, CRC32, sqrt, fabs).

use super::emit::RiscvCodegen;
use crate::ir::reexports::{IntrinsicOp, Operand, Value};

impl RiscvCodegen {
    pub(super) fn emit_intrinsic_rv(
        &mut self,
        dest: &Option<Value>,
        op: &IntrinsicOp,
        dest_ptr: &Option<Value>,
        args: &[Operand],
    ) {
        match op {
            IntrinsicOp::Lfence | IntrinsicOp::Mfence => {
                self.state.emit("    fence iorw, iorw");
            }
            IntrinsicOp::Sfence => {
                self.state.emit("    fence ow, ow");
            }
            IntrinsicOp::Pause => {
                // RISC-V pause hint (encoded as fence with specific args)
                self.state.emit("    fence.tso");
            }
            IntrinsicOp::Clflush => {
                // No RISC-V equivalent; emit fence as best approximation
                self.state.emit("    fence iorw, iorw");
            }
            IntrinsicOp::Movnti => {
                // Non-temporal 32-bit store: dest_ptr = target address, args[0] = value
                if let Some(ptr) = dest_ptr {
                    self.operand_to_t0(&args[0]);
                    self.state.emit("    mv t1, t0");
                    self.load_ptr_to_reg_rv(ptr, "t0");
                    self.state.emit("    sw t1, 0(t0)");
                }
            }
            IntrinsicOp::Movnti64 => {
                // Non-temporal 64-bit store
                if let Some(ptr) = dest_ptr {
                    self.operand_to_t0(&args[0]);
                    self.state.emit("    mv t1, t0");
                    self.load_ptr_to_reg_rv(ptr, "t0");
                    self.state.emit("    sd t1, 0(t0)");
                }
            }
            IntrinsicOp::Movntdq | IntrinsicOp::Movntpd => {
                // Non-temporal 128-bit store: dest_ptr = target, args[0] = source ptr
                if let Some(ptr) = dest_ptr {
                    self.operand_to_t0(&args[0]);
                    // t0 = source pointer, load 16 bytes
                    self.state.emit("    ld t1, 0(t0)");
                    self.state.emit("    ld t2, 8(t0)");
                    self.load_ptr_to_reg_rv(ptr, "t0");
                    self.state.emit("    sd t1, 0(t0)");
                    self.state.emit("    sd t2, 8(t0)");
                }
            }
            IntrinsicOp::Loaddqu => {
                // Load 128-bit unaligned
                if let Some(dptr) = dest_ptr {
                    self.operand_to_t0(&args[0]);
                    self.state.emit("    ld t1, 0(t0)");
                    self.state.emit("    ld t2, 8(t0)");
                    self.load_ptr_to_reg_rv(dptr, "t0");
                    self.state.emit("    sd t1, 0(t0)");
                    self.state.emit("    sd t2, 8(t0)");
                }
            }
            IntrinsicOp::Storedqu => {
                // Store 128-bit unaligned
                if let Some(ptr) = dest_ptr {
                    self.operand_to_t0(&args[0]);
                    self.state.emit("    ld t1, 0(t0)");
                    self.state.emit("    ld t2, 8(t0)");
                    self.load_ptr_to_reg_rv(ptr, "t0");
                    self.state.emit("    sd t1, 0(t0)");
                    self.state.emit("    sd t2, 8(t0)");
                }
            }
            IntrinsicOp::Pcmpeqb128 => {
                // Byte-wise compare equal: result[i] = (a[i] == b[i]) ? 0xFF : 0x00
                if let Some(dptr) = dest_ptr {
                    self.emit_rv_cmpeq_bytes(dptr, args);
                }
            }
            IntrinsicOp::Pcmpeqd128 => {
                // 32-bit lane compare equal
                if let Some(dptr) = dest_ptr {
                    self.emit_rv_cmpeq_dwords(dptr, args);
                }
            }
            IntrinsicOp::Psubusb128 => {
                // Unsigned saturating byte subtract
                if let Some(dptr) = dest_ptr {
                    self.emit_rv_binary_128_bytewise(dptr, args);
                }
            }
            IntrinsicOp::Psubsb128 => {
                // Signed saturating byte subtract
                if let Some(dptr) = dest_ptr {
                    self.emit_rv_psubsb_128(dptr, args);
                }
            }
            IntrinsicOp::Por128 => {
                if let Some(dptr) = dest_ptr {
                    self.emit_rv_binary_128(dptr, args, "or");
                }
            }
            IntrinsicOp::Pand128 => {
                if let Some(dptr) = dest_ptr {
                    self.emit_rv_binary_128(dptr, args, "and");
                }
            }
            IntrinsicOp::Pxor128 => {
                if let Some(dptr) = dest_ptr {
                    self.emit_rv_binary_128(dptr, args, "xor");
                }
            }
            IntrinsicOp::Pmovmskb128 => {
                // Extract high bit of each byte into a 16-bit mask
                self.emit_rv_pmovmskb(dest, args);
            }
            IntrinsicOp::SetEpi8 => {
                // Splat byte to all 16 positions
                if let Some(dptr) = dest_ptr {
                    self.operand_to_t0(&args[0]);
                    // Replicate byte to all 8 bytes of a 64-bit value
                    // t0 has the byte in low bits; mask to 8 bits
                    self.state.emit("    andi t0, t0, 0xff");
                    // Build 8-byte splat: t0 * 0x0101010101010101
                    self.state.emit("    li t1, 0x0101010101010101");
                    self.state.emit("    mul t0, t0, t1");
                    // Store twice for 16 bytes
                    self.load_ptr_to_reg_rv(dptr, "t1");
                    self.state.emit("    sd t0, 0(t1)");
                    self.state.emit("    sd t0, 8(t1)");
                }
            }
            IntrinsicOp::SetEpi32 => {
                // Splat 32-bit to all 4 positions
                if let Some(dptr) = dest_ptr {
                    self.operand_to_t0(&args[0]);
                    // Mask to 32 bits and replicate: (val << 32) | val
                    self.state.emit("    slli t1, t0, 32");
                    // Zero-extend t0 to clear upper 32 bits
                    self.state.emit("    slli t0, t0, 32");
                    self.state.emit("    srli t0, t0, 32");
                    self.state.emit("    or t0, t0, t1");
                    self.load_ptr_to_reg_rv(dptr, "t1");
                    self.state.emit("    sd t0, 0(t1)");
                    self.state.emit("    sd t0, 8(t1)");
                }
            }
            IntrinsicOp::Crc32_8
            | IntrinsicOp::Crc32_16
            | IntrinsicOp::Crc32_32
            | IntrinsicOp::Crc32_64 => {
                // Software CRC32C (Castagnoli) using bit-by-bit computation.
                // args[0] = current CRC accumulator, args[1] = data value
                let num_bytes = match op {
                    IntrinsicOp::Crc32_8 => 1,
                    IntrinsicOp::Crc32_16 => 2,
                    IntrinsicOp::Crc32_32 => 4,
                    IntrinsicOp::Crc32_64 => 8,
                    _ => unreachable!("CRC32 dispatch matched non-CRC32 op: {:?}", op),
                };
                self.operand_to_t0(&args[0]); // t0 = crc
                self.state.emit("    mv t3, t0");
                self.operand_to_t0(&args[1]); // t0 = data
                self.state.emit("    mv t4, t0");
                // XOR data into low bytes of CRC
                self.state.emit("    xor t3, t3, t4");
                // CRC32C polynomial: 0x82F63B78
                self.state.emit("    li t5, 0x82F63B78");
                // Process num_bytes * 8 bits
                let num_bits = num_bytes * 8;
                let loop_label = self.state.fresh_label("crc_loop");
                let done_label = self.state.fresh_label("crc_done");
                let skip_label = self.state.fresh_label("crc_skip");
                self.state.emit_fmt(format_args!("    li t6, {}", num_bits));
                self.state.emit_fmt(format_args!("{}:", loop_label));
                self.state
                    .emit_fmt(format_args!("    beqz t6, {}", done_label));
                // Check LSB of crc
                self.state.emit("    andi t0, t3, 1");
                // Shift CRC right by 1
                self.state.emit("    srli t3, t3, 1");
                // If LSB was set, XOR with polynomial
                self.state
                    .emit_fmt(format_args!("    beqz t0, {}", skip_label));
                self.state.emit("    xor t3, t3, t5");
                self.state.emit_fmt(format_args!("{}:", skip_label));
                self.state.emit("    addi t6, t6, -1");
                self.state.emit_fmt(format_args!("    j {}", loop_label));
                self.state.emit_fmt(format_args!("{}:", done_label));
                // Zero-extend result to 32 bits (CRC32 is always 32-bit)
                self.state.emit("    slli t3, t3, 32");
                self.state.emit("    srli t3, t3, 32");
                if let Some(d) = dest {
                    self.state.emit("    mv t0, t3");
                    self.store_t0_to(d);
                }
            }
            IntrinsicOp::FrameAddress => {
                // __builtin_frame_address(0): return current frame pointer (s0)
                self.state.emit("    mv t0, s0");
                if let Some(d) = dest {
                    self.store_t0_to(d);
                }
            }
            IntrinsicOp::ReturnAddress => {
                // __builtin_return_address(0): ra is saved at s0-8 in prologue;
                // ra itself gets clobbered by subsequent calls, so load from stack
                self.state.emit("    ld t0, -8(s0)");
                if let Some(d) = dest {
                    self.store_t0_to(d);
                }
            }
            IntrinsicOp::ThreadPointer => {
                // __builtin_thread_pointer(): read TLS base from tp register
                self.state.emit("    mv t0, tp");
                if let Some(d) = dest {
                    self.store_t0_to(d);
                }
            }
            IntrinsicOp::SqrtF64 => {
                self.operand_to_t0(&args[0]);
                self.state.emit("    fmv.d.x ft0, t0");
                self.state.emit("    fsqrt.d ft0, ft0");
                self.state.emit("    fmv.x.d t0, ft0");
                if let Some(d) = dest {
                    self.store_t0_to(d);
                }
            }
            IntrinsicOp::SqrtF32 => {
                self.operand_to_t0(&args[0]);
                self.state.emit("    fmv.w.x ft0, t0");
                self.state.emit("    fsqrt.s ft0, ft0");
                self.state.emit("    fmv.x.w t0, ft0");
                if let Some(d) = dest {
                    self.store_t0_to(d);
                }
            }
            IntrinsicOp::FabsF64 => {
                self.operand_to_t0(&args[0]);
                self.state.emit("    fmv.d.x ft0, t0");
                self.state.emit("    fabs.d ft0, ft0");
                self.state.emit("    fmv.x.d t0, ft0");
                if let Some(d) = dest {
                    self.store_t0_to(d);
                }
            }
            IntrinsicOp::FabsF32 => {
                self.operand_to_t0(&args[0]);
                self.state.emit("    fmv.w.x ft0, t0");
                self.state.emit("    fabs.s ft0, ft0");
                self.state.emit("    fmv.x.w t0, ft0");
                if let Some(d) = dest {
                    self.store_t0_to(d);
                }
            }
            // x86-specific SSE/AES-NI/CLMUL intrinsics - these are x86-only and should
            // not appear in RISC-V codegen in practice. Cross-compiled code that conditionally
            // uses these behind #ifdef __x86_64__ will have the calls dead-code eliminated.
            // TODO: consider emitting a runtime trap instead of silent zeros
            IntrinsicOp::Aesenc128
            | IntrinsicOp::Aesenclast128
            | IntrinsicOp::Aesdec128
            | IntrinsicOp::Aesdeclast128
            | IntrinsicOp::Aesimc128
            | IntrinsicOp::Aeskeygenassist128
            | IntrinsicOp::Pclmulqdq128
            | IntrinsicOp::Pslldqi128
            | IntrinsicOp::Psrldqi128
            | IntrinsicOp::Psllqi128
            | IntrinsicOp::Psrlqi128
            | IntrinsicOp::Pshufd128
            | IntrinsicOp::Loadldi128
            | IntrinsicOp::Paddw128
            | IntrinsicOp::Psubw128
            | IntrinsicOp::Pmulhw128
            | IntrinsicOp::Pmaddwd128
            | IntrinsicOp::Pcmpgtw128
            | IntrinsicOp::Pcmpgtb128
            | IntrinsicOp::Psllwi128
            | IntrinsicOp::Psrlwi128
            | IntrinsicOp::Psrawi128
            | IntrinsicOp::Psradi128
            | IntrinsicOp::Pslldi128
            | IntrinsicOp::Psrldi128
            | IntrinsicOp::Paddd128
            | IntrinsicOp::Psubd128
            | IntrinsicOp::Packssdw128
            | IntrinsicOp::Packsswb128
            | IntrinsicOp::Packuswb128
            | IntrinsicOp::Punpcklbw128
            | IntrinsicOp::Punpckhbw128
            | IntrinsicOp::Punpcklwd128
            | IntrinsicOp::Punpckhwd128
            | IntrinsicOp::SetEpi16
            | IntrinsicOp::Pinsrw128
            | IntrinsicOp::Pextrw128
            | IntrinsicOp::Storeldi128
            | IntrinsicOp::Cvtsi128Si32
            | IntrinsicOp::Cvtsi32Si128
            | IntrinsicOp::Cvtsi128Si64
            | IntrinsicOp::Pshuflw128
            | IntrinsicOp::Pshufhw128
            | IntrinsicOp::Pinsrd128
            | IntrinsicOp::Pextrd128
            | IntrinsicOp::Pinsrb128
            | IntrinsicOp::Pextrb128
            | IntrinsicOp::Pinsrq128
            | IntrinsicOp::Pextrq128 => {
                // x86-only: zero dest if present
                if let Some(dptr) = dest_ptr {
                    if let Some(slot) = self.state.get_slot(dptr.0) {
                        self.emit_store_to_s0("zero", slot.0, "sd");
                        self.emit_store_to_s0("zero", slot.0 + 8, "sd");
                    }
                }
            }
        }
    }

    /// Emit 128-bit binary op (or, and, xor) using two 64-bit operations.
    pub(super) fn emit_rv_binary_128(&mut self, dest_ptr: &Value, args: &[Operand], op: &str) {
        // Load args[0] pointer
        self.operand_to_t0(&args[0]);
        self.state.emit("    ld t1, 0(t0)");
        self.state.emit("    ld t2, 8(t0)");
        // Load args[1] pointer
        self.operand_to_t0(&args[1]);
        self.state.emit("    ld t3, 0(t0)");
        self.state.emit("    ld t4, 8(t0)");
        // Apply operation
        self.state.emit_fmt(format_args!("    {} t1, t1, t3", op));
        self.state.emit_fmt(format_args!("    {} t2, t2, t4", op));
        // Store to dest_ptr
        self.load_ptr_to_reg_rv(dest_ptr, "t0");
        self.state.emit("    sd t1, 0(t0)");
        self.state.emit("    sd t2, 8(t0)");
    }

    /// Byte-wise compare equal: for each of 16 bytes, result = (a == b) ? 0xFF : 0x00
    pub(super) fn emit_rv_cmpeq_bytes(&mut self, dest_ptr: &Value, args: &[Operand]) {
        // Load source pointers
        self.operand_to_t0(&args[0]);
        self.state.emit("    mv a6, t0");
        self.operand_to_t0(&args[1]);
        self.state.emit("    mv a7, t0");
        // Get dest address
        self.load_ptr_to_reg_rv(dest_ptr, "a5");
        // XOR corresponding 64-bit halves: zero bytes mean equal
        self.state.emit("    ld t1, 0(a6)");
        self.state.emit("    ld t2, 0(a7)");
        self.state.emit("    xor t1, t1, t2");
        // For each byte that is 0 in t1, we need 0xFF; for non-zero, 0x00.
        // Use the trick: byte == 0 iff ((byte - 1) & ~byte & 0x80) != 0
        // But simpler: negate each byte and saturate using bit manipulation
        // Actually simplest approach: process byte by byte using a loop-like unroll
        // For correctness, use a helper that processes 8 bytes of XOR result
        self.emit_rv_zero_byte_mask("t1", "t3"); // t3 = mask where equal bytes -> 0xFF
                                                 // Do the same for high 8 bytes
        self.state.emit("    ld t1, 8(a6)");
        self.state.emit("    ld t2, 8(a7)");
        self.state.emit("    xor t1, t1, t2");
        self.emit_rv_zero_byte_mask("t1", "t4"); // t4 = mask for high bytes
                                                 // Store results
        self.state.emit("    sd t3, 0(a5)");
        self.state.emit("    sd t4, 8(a5)");
    }

    /// Given XOR result in `src_reg`, produce byte mask in `dst_reg`:
    /// For each byte that is 0x00, output 0xFF; for non-zero, output 0x00.
    /// Uses: t5, t6 as temporaries.
    pub(super) fn emit_rv_zero_byte_mask(&mut self, src_reg: &str, dst_reg: &str) {
        // Algorithm: for each byte b of src:
        //   result byte = (b == 0) ? 0xFF : 0x00
        // Using: has_zero = (x - 0x0101...) & ~x & 0x8080...
        // This detects zero bytes by checking if subtracting 1 from each byte
        // causes a borrow from a non-zero byte.
        self.state
            .emit_fmt(format_args!("    li t5, 0x0101010101010101"));
        self.state
            .emit_fmt(format_args!("    li t6, 0x8080808080808080"));
        self.state.emit_fmt(format_args!(
            "    sub {dst}, {src}, t5",
            dst = dst_reg,
            src = src_reg
        ));
        self.state
            .emit_fmt(format_args!("    not t5, {src}", src = src_reg));
        self.state
            .emit_fmt(format_args!("    and {dst}, {dst}, t5", dst = dst_reg));
        self.state
            .emit_fmt(format_args!("    and {dst}, {dst}, t6", dst = dst_reg));
        // Now dst has 0x80 in each byte position where src byte was 0x00.
        // Expand 0x80 -> 0xFF by shifting right 7 (get 0x01) then replicating
        // bit 0 to all 8 bit positions via: x | (x<<1) | (x<<2) | ... | (x<<7)
        self.state
            .emit_fmt(format_args!("    srli {dst}, {dst}, 7", dst = dst_reg));
        // Now dst has 0x01 where bytes were zero, 0x00 elsewhere.
        // Replicate: x * 0xFF works correctly here because each byte is 0 or 1,
        // and adjacent bytes can both be 1: 0x0101 * 0xFF = 0xFEFF (WRONG).
        // Instead use shift-or cascade:
        self.state
            .emit_fmt(format_args!("    slli t5, {dst}, 1", dst = dst_reg));
        self.state
            .emit_fmt(format_args!("    or {dst}, {dst}, t5", dst = dst_reg));
        self.state
            .emit_fmt(format_args!("    slli t5, {dst}, 2", dst = dst_reg));
        self.state
            .emit_fmt(format_args!("    or {dst}, {dst}, t5", dst = dst_reg));
        self.state
            .emit_fmt(format_args!("    slli t5, {dst}, 4", dst = dst_reg));
        self.state
            .emit_fmt(format_args!("    or {dst}, {dst}, t5", dst = dst_reg));
        // Now each byte that was 0x01 has become 0xFF (bits replicated),
        // and each byte that was 0x00 stays 0x00.
    }

    /// 32-bit lane compare equal
    pub(super) fn emit_rv_cmpeq_dwords(&mut self, dest_ptr: &Value, args: &[Operand]) {
        // Load source pointers
        self.operand_to_t0(&args[0]);
        self.state.emit("    mv a6, t0");
        self.operand_to_t0(&args[1]);
        self.state.emit("    mv a7, t0");
        self.load_ptr_to_reg_rv(dest_ptr, "a5");
        // Process 4 dwords (each 32-bit)
        // Dword 0 and 1 are in the low 64 bits of each source
        self.state.emit("    ld t1, 0(a6)");
        self.state.emit("    ld t2, 0(a7)");
        // Compare low dword (bits 0-31)
        self.state.emit("    slli t3, t1, 32");
        self.state.emit("    slli t4, t2, 32");
        self.state.emit("    srli t3, t3, 32"); // zero-extend low 32 bits of t1
        self.state.emit("    srli t4, t4, 32"); // zero-extend low 32 bits of t2
        self.state.emit("    sub t3, t3, t4");
        self.state.emit("    snez t3, t3");
        self.state.emit("    neg t3, t3"); // 0 if equal, -1 if not
        self.state.emit("    not t3, t3"); // -1 if equal, 0 if not -> 0xFFFFFFFF or 0
        self.state.emit("    slli t3, t3, 32");
        self.state.emit("    srli t3, t3, 32"); // mask to 32 bits
                                                // Compare high dword (bits 32-63)
        self.state.emit("    srli t5, t1, 32");
        self.state.emit("    srli t6, t2, 32");
        self.state.emit("    sub t5, t5, t6");
        self.state.emit("    snez t5, t5");
        self.state.emit("    neg t5, t5");
        self.state.emit("    not t5, t5");
        self.state.emit("    slli t5, t5, 32");
        // Combine: low dword result in t3, high in t5 (already shifted)
        self.state.emit("    or t3, t3, t5");
        self.state.emit("    sd t3, 0(a5)");
        // Process high 64 bits (dwords 2 and 3)
        self.state.emit("    ld t1, 8(a6)");
        self.state.emit("    ld t2, 8(a7)");
        self.state.emit("    slli t3, t1, 32");
        self.state.emit("    slli t4, t2, 32");
        self.state.emit("    srli t3, t3, 32");
        self.state.emit("    srli t4, t4, 32");
        self.state.emit("    sub t3, t3, t4");
        self.state.emit("    snez t3, t3");
        self.state.emit("    neg t3, t3");
        self.state.emit("    not t3, t3");
        self.state.emit("    slli t3, t3, 32");
        self.state.emit("    srli t3, t3, 32");
        self.state.emit("    srli t5, t1, 32");
        self.state.emit("    srli t6, t2, 32");
        self.state.emit("    sub t5, t5, t6");
        self.state.emit("    snez t5, t5");
        self.state.emit("    neg t5, t5");
        self.state.emit("    not t5, t5");
        self.state.emit("    slli t5, t5, 32");
        self.state.emit("    or t3, t3, t5");
        self.state.emit("    sd t3, 8(a5)");
    }

    /// Unsigned saturating byte subtract: result[i] = saturate(a[i] - b[i])
    /// Uses SWAR (SIMD Within A Register) technique to process 8 bytes at a time.
    /// Algorithm: For unsigned bytes, saturating subtract is:
    ///   result = (a | 0x80) - (b & 0x7F)  -- subtract with borrow protection
    ///   but actually: result = ((a | H) - (b & ~H)) ^ ((a ^ ~b) & H)
    ///   where H = 0x8080808080808080 (MSB of each byte lane)
    /// Simpler approach: result = a - min(a, b) where min uses standard SWAR trick.
    pub(super) fn emit_rv_binary_128_bytewise(&mut self, dest_ptr: &Value, args: &[Operand]) {
        // Load args[0] pointer (a)
        self.operand_to_t0(&args[0]);
        self.state.emit("    mv a6, t0");
        // Load args[1] pointer (b)
        self.operand_to_t0(&args[1]);
        self.state.emit("    mv a7, t0");
        // Get dest address
        self.load_ptr_to_reg_rv(dest_ptr, "a5");
        // Process low 8 bytes
        self.state.emit("    ld t1, 0(a6)"); // a_lo
        self.state.emit("    ld t2, 0(a7)"); // b_lo
        self.emit_rv_psubusb_8bytes("t1", "t2", "t3"); // t3 = saturate(a_lo - b_lo)
                                                       // Process high 8 bytes
        self.state.emit("    ld t1, 8(a6)"); // a_hi
        self.state.emit("    ld t2, 8(a7)"); // b_hi
        self.emit_rv_psubusb_8bytes("t1", "t2", "t4"); // t4 = saturate(a_hi - b_hi)
                                                       // Store results
        self.state.emit("    sd t3, 0(a5)");
        self.state.emit("    sd t4, 8(a5)");
    }

    /// Emit unsigned saturating byte subtract for 8 bytes packed in registers.
    /// dst = saturate(a - b) for each byte lane.
    /// Processes each byte individually to guarantee correctness.
    pub(super) fn emit_rv_psubusb_8bytes(&mut self, a_reg: &str, b_reg: &str, dst_reg: &str) {
        // Process 8 bytes one at a time using shift-and-mask
        // Result accumulates in dst_reg
        self.state
            .emit_fmt(format_args!("    li {dst}, 0", dst = dst_reg));
        for i in 0..8 {
            let shift = i * 8;
            // Extract byte i from a into t5
            if shift == 0 {
                self.state
                    .emit_fmt(format_args!("    andi t5, {a}, 0xff", a = a_reg));
            } else {
                self.state
                    .emit_fmt(format_args!("    srli t5, {a}, {shift}", a = a_reg));
                self.state.emit("    andi t5, t5, 0xff");
            }
            // Extract byte i from b into t6
            if shift == 0 {
                self.state
                    .emit_fmt(format_args!("    andi t6, {b}, 0xff", b = b_reg));
            } else {
                self.state
                    .emit_fmt(format_args!("    srli t6, {b}, {shift}", b = b_reg));
                self.state.emit("    andi t6, t6, 0xff");
            }
            // Saturating subtract: max(a_byte - b_byte, 0)
            let skip_label = self.state.fresh_label("psub_skip");
            self.state
                .emit_fmt(format_args!("    bltu t5, t6, {skip}", skip = skip_label));
            self.state.emit("    sub t5, t5, t6");
            if shift > 0 {
                self.state
                    .emit_fmt(format_args!("    slli t5, t5, {shift}"));
            }
            self.state
                .emit_fmt(format_args!("    or {dst}, {dst}, t5", dst = dst_reg));
            self.state
                .emit_fmt(format_args!("{skip}:", skip = skip_label));
        }
    }

    /// Emit signed saturating byte subtract for 128 bits (16 bytes).
    /// Equivalent to x86 PSUBSB / _mm_subs_epi8.
    pub(super) fn emit_rv_psubsb_128(&mut self, dest_ptr: &Value, args: &[Operand]) {
        // Load args[0] pointer (a)
        self.operand_to_t0(&args[0]);
        self.state.emit("    mv a6, t0");
        // Load args[1] pointer (b)
        self.operand_to_t0(&args[1]);
        self.state.emit("    mv a7, t0");
        // Get dest address
        self.load_ptr_to_reg_rv(dest_ptr, "a5");
        // Process low 8 bytes
        self.state.emit("    ld t1, 0(a6)"); // a_lo
        self.state.emit("    ld t2, 0(a7)"); // b_lo
        self.emit_rv_psubsb_8bytes("t1", "t2", "t3"); // t3 = signed_saturate(a_lo - b_lo)
                                                      // Process high 8 bytes
        self.state.emit("    ld t1, 8(a6)"); // a_hi
        self.state.emit("    ld t2, 8(a7)"); // b_hi
        self.emit_rv_psubsb_8bytes("t1", "t2", "t4"); // t4 = signed_saturate(a_hi - b_hi)
                                                      // Store results
        self.state.emit("    sd t3, 0(a5)");
        self.state.emit("    sd t4, 8(a5)");
    }

    /// Emit signed saturating byte subtract for 8 bytes packed in registers.
    /// dst = clamp(a - b, -128, 127) for each byte lane.
    /// Processes each byte individually.
    pub(super) fn emit_rv_psubsb_8bytes(&mut self, a_reg: &str, b_reg: &str, dst_reg: &str) {
        self.state
            .emit_fmt(format_args!("    li {dst}, 0", dst = dst_reg));
        for i in 0..8 {
            let shift = i * 8;
            // Extract byte i from a into t5 (as signed: sign-extend from 8 bits)
            if shift == 0 {
                self.state
                    .emit_fmt(format_args!("    slli t5, {a}, 56", a = a_reg));
            } else {
                self.state.emit_fmt(format_args!(
                    "    slli t5, {a}, {s}",
                    a = a_reg,
                    s = 56 - shift
                ));
            }
            self.state.emit("    srai t5, t5, 56"); // sign-extend byte to 64-bit
                                                    // Extract byte i from b into t6 (as signed)
            if shift == 0 {
                self.state
                    .emit_fmt(format_args!("    slli t6, {b}, 56", b = b_reg));
            } else {
                self.state.emit_fmt(format_args!(
                    "    slli t6, {b}, {s}",
                    b = b_reg,
                    s = 56 - shift
                ));
            }
            self.state.emit("    srai t6, t6, 56"); // sign-extend byte to 64-bit
                                                    // Compute difference in t5
            self.state.emit("    sub t5, t5, t6");
            // Clamp to [-128, 127]
            let no_clamp_hi = self.state.fresh_label("psubsb_noclamp_hi");
            let done = self.state.fresh_label("psubsb_done");
            self.state.emit_fmt(format_args!("    li t6, 127"));
            self.state
                .emit_fmt(format_args!("    ble t5, t6, {}", no_clamp_hi));
            self.state.emit_fmt(format_args!("    li t5, 127"));
            self.state.emit_fmt(format_args!("    j {}", done));
            self.state.emit_fmt(format_args!("{}:", no_clamp_hi));
            self.state.emit_fmt(format_args!("    li t6, -128"));
            let no_clamp_lo = self.state.fresh_label("psubsb_noclamp_lo");
            self.state
                .emit_fmt(format_args!("    bge t5, t6, {}", no_clamp_lo));
            self.state.emit_fmt(format_args!("    li t5, -128"));
            self.state.emit_fmt(format_args!("{}:", no_clamp_lo));
            self.state.emit_fmt(format_args!("{}:", done));
            // Mask to 8 bits and place in correct position
            self.state.emit("    andi t5, t5, 0xff");
            if shift > 0 {
                self.state
                    .emit_fmt(format_args!("    slli t5, t5, {shift}"));
            }
            self.state
                .emit_fmt(format_args!("    or {dst}, {dst}, t5", dst = dst_reg));
        }
    }

    /// Extract high bit of each of 16 bytes into a 16-bit mask (pmovmskb equivalent)
    pub(super) fn emit_rv_pmovmskb(&mut self, dest: &Option<Value>, args: &[Operand]) {
        // Load 128-bit source
        self.operand_to_t0(&args[0]);
        self.state.emit("    ld t1, 0(t0)"); // low 8 bytes
        self.state.emit("    ld t2, 8(t0)"); // high 8 bytes
                                             // Extract bit 7 of each byte and collect into a mask.
                                             // For low 8 bytes (t1) -> bits 0-7 of result
                                             // For high 8 bytes (t2) -> bits 8-15 of result
                                             // Method: AND with 0x8080808080808080, then compress.
                                             // After AND, each byte is either 0x80 or 0x00.
                                             // Multiply by magic constant to pack bits together:
                                             //   0x0002040810204081 will shift each 0x80 bit to accumulate in the high byte
        self.state.emit("    li t3, 0x8080808080808080");
        self.state.emit("    and t1, t1, t3");
        self.state.emit("    and t2, t2, t3");
        // Multiply to pack: each byte's bit 7 gets shifted to different positions
        // Magic: 0x0002040810204081 makes bits collect in byte 7
        self.state.emit("    li t3, 0x0002040810204081");
        self.state.emit("    mul t1, t1, t3");
        self.state.emit("    srli t1, t1, 56"); // extract byte 7 = low 8-bit mask
        self.state.emit("    mul t2, t2, t3");
        self.state.emit("    srli t2, t2, 56"); // high 8-bit mask
                                                // Combine
        self.state.emit("    slli t2, t2, 8");
        self.state.emit("    or t0, t1, t2");
        // Store scalar result
        if let Some(d) = dest {
            self.store_t0_to(d);
        }
    }
}
