/// Target-independent intrinsic operations.
///
/// These represent SIMD, crypto, math, and hardware intrinsics that the IR
/// can express without target-specific details. Each backend emits the
/// appropriate native instructions for its architecture.
///
/// Organized by ISA extension:
/// - Fences/barriers (Lfence, Mfence, Sfence, Pause)
/// - Non-temporal stores (Movnti, Movntdq, etc.)
/// - SSE2 packed integer ops (Pcmpeqb, Paddw, Psubd, etc.)
/// - SSE2 shuffle/pack/unpack (Pshufd, Packssdw, Punpcklbw, etc.)
/// - SSE2/SSE4.1 insert/extract (Pinsrw, Pextrw, Pinsrd, etc.)
/// - AES-NI (Aesenc, Aesdec, etc.)
/// - CLMUL (Pclmulqdq)
/// - CRC32
/// - Scalar math (SqrtF32, SqrtF64, FabsF32, FabsF64)
/// - Frame/return address builtins

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IntrinsicOp {
    /// Memory fence operations (no dest, no args beyond optional ptr)
    Lfence,
    Mfence,
    Sfence,
    Pause,
    Clflush,
    /// Non-temporal stores: movnti (32-bit), movnti64 (64-bit), movntdq (128-bit), movntpd (128-bit double)
    Movnti,
    Movnti64,
    Movntdq,
    Movntpd,
    /// Load/store 128-bit unaligned
    Loaddqu,
    Storedqu,
    /// Compare equal packed bytes (16 bytes)
    Pcmpeqb128,
    /// Compare equal packed dwords (4x32)
    Pcmpeqd128,
    /// Subtract packed unsigned saturated bytes
    Psubusb128,
    /// Subtract packed signed saturated bytes
    Psubsb128,
    /// Bitwise OR/AND/XOR on 128-bit
    Por128,
    Pand128,
    Pxor128,
    /// Move byte mask (pmovmskb) - returns i32
    Pmovmskb128,
    /// Set all bytes to value (splat)
    SetEpi8,
    /// Set all dwords to value (splat)
    SetEpi32,
    /// CRC32 accumulate
    Crc32_8,
    Crc32_16,
    Crc32_32,
    Crc32_64,
    /// __builtin_frame_address(0) - returns current frame pointer
    FrameAddress,
    /// __builtin_return_address(0) - returns current return address
    ReturnAddress,
    /// __builtin_thread_pointer() - returns thread pointer (TLS base address)
    ThreadPointer,
    /// Scalar square root: sqrtsd/sqrtss on x86, fsqrt on ARM/RISC-V
    /// args[0] = input float value; dest = sqrt result
    SqrtF32,
    SqrtF64,
    /// Scalar absolute value: bitwise AND with sign mask on x86, fabs on ARM/RISC-V
    /// args[0] = input float value; dest = |x|
    FabsF32,
    FabsF64,
    /// AES-NI: aesenc (single round encrypt)
    /// args[0] = state ptr, args[1] = round key ptr; dest_ptr = result ptr
    Aesenc128,
    /// AES-NI: aesenclast (final round encrypt)
    Aesenclast128,
    /// AES-NI: aesdec (single round decrypt)
    Aesdec128,
    /// AES-NI: aesdeclast (final round decrypt)
    Aesdeclast128,
    /// AES-NI: aesimc (inverse mix columns)
    /// args[0] = input ptr; dest_ptr = result ptr
    Aesimc128,
    /// AES-NI: aeskeygenassist with immediate
    /// args[0] = input ptr, args[1] = imm8; dest_ptr = result ptr
    Aeskeygenassist128,
    /// CLMUL: pclmulqdq with immediate
    /// args[0] = src1 ptr, args[1] = src2 ptr, args[2] = imm8; dest_ptr = result ptr
    Pclmulqdq128,
    /// SSE2 byte shift left (PSLLDQ): shift by imm8 bytes
    /// args[0] = src ptr, args[1] = imm8; dest_ptr = result ptr
    Pslldqi128,
    /// SSE2 byte shift right (PSRLDQ): shift by imm8 bytes
    Psrldqi128,
    /// SSE2 bit shift left per 64-bit lane (PSLLQ)
    /// args[0] = src ptr, args[1] = count; dest_ptr = result ptr
    Psllqi128,
    /// SSE2 bit shift right per 64-bit lane (PSRLQ)
    Psrlqi128,
    /// SSE2 shuffle 32-bit integers (PSHUFD)
    /// args[0] = src ptr, args[1] = imm8; dest_ptr = result ptr
    Pshufd128,
    /// Load low 64 bits, zero upper (MOVQ)
    /// args[0] = src ptr; dest_ptr = result ptr
    Loadldi128,

    // --- SSE2 packed 16-bit integer operations ---
    /// Packed 16-bit add (PADDW)
    Paddw128,
    /// Packed 16-bit subtract (PSUBW)
    Psubw128,
    /// Packed 16-bit multiply high (PMULHW)
    Pmulhw128,
    /// Packed 16-bit multiply-add to 32-bit (PMADDWD)
    Pmaddwd128,
    /// Packed 16-bit compare greater-than (PCMPGTW)
    Pcmpgtw128,
    /// Packed 8-bit compare greater-than (PCMPGTB)
    Pcmpgtb128,
    /// Packed 16-bit shift left by imm (PSLLW)
    Psllwi128,
    /// Packed 16-bit shift right logical by imm (PSRLW)
    Psrlwi128,
    /// Packed 16-bit shift right arithmetic by imm (PSRAW)
    Psrawi128,
    /// Packed 32-bit shift right arithmetic by imm (PSRAD)
    Psradi128,
    /// Packed 32-bit shift left by imm (PSLLD)
    Pslldi128,
    /// Packed 32-bit shift right logical by imm (PSRLD)
    Psrldi128,

    // --- SSE2 packed 32-bit integer operations ---
    /// Packed 32-bit add (PADDD)
    Paddd128,
    /// Packed 32-bit subtract (PSUBD)
    Psubd128,

    // --- SSE2 pack/unpack operations ---
    /// Pack 32-bit to 16-bit signed saturate (PACKSSDW)
    Packssdw128,
    /// Pack 16-bit to 8-bit signed saturate (PACKSSWB)
    Packsswb128,
    /// Pack 16-bit to 8-bit unsigned saturate (PACKUSWB)
    Packuswb128,
    /// Unpack and interleave low 8-bit (PUNPCKLBW)
    Punpcklbw128,
    /// Unpack and interleave high 8-bit (PUNPCKHBW)
    Punpckhbw128,
    /// Unpack and interleave low 16-bit (PUNPCKLWD)
    Punpcklwd128,
    /// Unpack and interleave high 16-bit (PUNPCKHWD)
    Punpckhwd128,

    // --- SSE2 set/insert/extract/convert operations ---
    /// Set all 16-bit lanes to value (splat)
    SetEpi16,
    /// Insert 16-bit value at lane (PINSRW)
    Pinsrw128,
    /// Extract 16-bit value at lane (PEXTRW) - returns scalar i32
    Pextrw128,
    /// Store low 64 bits to memory (MOVQ store)
    Storeldi128,
    /// Convert low 32-bit of __m128i to int (MOVD) - returns scalar i32
    Cvtsi128Si32,
    /// Convert int to __m128i with zero extension (MOVD)
    Cvtsi32Si128,
    /// Convert low 64-bit of __m128i to long long - returns scalar i64
    Cvtsi128Si64,
    /// Shuffle low 16-bit integers (PSHUFLW)
    Pshuflw128,
    /// Shuffle high 16-bit integers (PSHUFHW)
    Pshufhw128,

    // --- SSE4.1 insert/extract operations ---
    /// Insert 32-bit value at lane (PINSRD)
    Pinsrd128,
    /// Extract 32-bit value at lane (PEXTRD) - returns scalar i32
    Pextrd128,
    /// Insert 8-bit value at lane (PINSRB)
    Pinsrb128,
    /// Extract 8-bit value at lane (PEXTRB) - returns scalar i32
    Pextrb128,
    /// Insert 64-bit value at lane (PINSRQ)
    Pinsrq128,
    /// Extract 64-bit value at lane (PEXTRQ) - returns scalar i64
    Pextrq128,
}

impl IntrinsicOp {
    /// Returns true if this intrinsic is a pure function (no side effects, result depends
    /// only on inputs). Pure intrinsics can be dead-code eliminated if their result is unused.
    pub fn is_pure(&self) -> bool {
        matches!(
            self,
            IntrinsicOp::SqrtF32 | IntrinsicOp::SqrtF64 |
            IntrinsicOp::FabsF32 | IntrinsicOp::FabsF64 |
            IntrinsicOp::Aesenc128 | IntrinsicOp::Aesenclast128 |
            IntrinsicOp::Aesdec128 | IntrinsicOp::Aesdeclast128 |
            IntrinsicOp::Aesimc128 | IntrinsicOp::Aeskeygenassist128 |
            IntrinsicOp::Pclmulqdq128 |
            IntrinsicOp::Pslldqi128 | IntrinsicOp::Psrldqi128 |
            IntrinsicOp::Psllqi128 | IntrinsicOp::Psrlqi128 |
            IntrinsicOp::Pshufd128 |
            // SSE2 packed operations are all pure
            IntrinsicOp::Psubsb128 |
            IntrinsicOp::Paddw128 | IntrinsicOp::Psubw128 |
            IntrinsicOp::Pmulhw128 | IntrinsicOp::Pmaddwd128 |
            IntrinsicOp::Pcmpgtw128 | IntrinsicOp::Pcmpgtb128 |
            IntrinsicOp::Psllwi128 | IntrinsicOp::Psrlwi128 |
            IntrinsicOp::Psrawi128 | IntrinsicOp::Psradi128 |
            IntrinsicOp::Pslldi128 | IntrinsicOp::Psrldi128 |
            IntrinsicOp::Paddd128 | IntrinsicOp::Psubd128 |
            IntrinsicOp::Packssdw128 | IntrinsicOp::Packsswb128 | IntrinsicOp::Packuswb128 |
            IntrinsicOp::Punpcklbw128 | IntrinsicOp::Punpckhbw128 |
            IntrinsicOp::Punpcklwd128 | IntrinsicOp::Punpckhwd128 |
            IntrinsicOp::SetEpi16 | IntrinsicOp::Pinsrw128 |
            IntrinsicOp::Pextrw128 | IntrinsicOp::Cvtsi128Si32 |
            IntrinsicOp::Cvtsi32Si128 | IntrinsicOp::Cvtsi128Si64 |
            IntrinsicOp::Pshuflw128 | IntrinsicOp::Pshufhw128 |
            // SSE4.1 insert/extract are pure
            IntrinsicOp::Pinsrd128 | IntrinsicOp::Pextrd128 |
            IntrinsicOp::Pinsrb128 | IntrinsicOp::Pextrb128 |
            IntrinsicOp::Pinsrq128 | IntrinsicOp::Pextrq128
        )
    }
}
