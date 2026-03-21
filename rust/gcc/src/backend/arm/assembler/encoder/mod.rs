//! AArch64 instruction encoder.
//!
//! Encodes AArch64 instructions into 32-bit machine code words.
//! This covers the subset of instructions emitted by our codegen.
//!
//! AArch64 instructions are always 4 bytes (32 bits), little-endian.
//! The encoding format varies by instruction class.

// Encoding helpers for all AArch64 instruction formats; not all formats used yet.
#![allow(dead_code)]

use super::parser::Operand;

mod bitfield;
mod compare_branch;
mod data_processing;
mod fp_scalar;
mod load_store;
mod neon;
mod system;

pub(crate) use bitfield::*;
pub(crate) use compare_branch::*;
pub(crate) use data_processing::*;
pub(crate) use fp_scalar::*;
pub(crate) use load_store::*;
pub(crate) use neon::*;
pub(crate) use system::*;

/// Result of encoding an instruction.
#[derive(Debug, Clone)]
pub enum EncodeResult {
    /// Successfully encoded as a 4-byte instruction word
    Word(u32),
    /// Instruction needs a relocation to be applied later
    WordWithReloc { word: u32, reloc: Relocation },
    /// Multiple encoded words (e.g., movz+movk sequence)
    Words(Vec<u32>),
    /// Skip this instruction (e.g., pseudo-instruction handled elsewhere)
    Skip,
}

/// Relocation types for AArch64 ELF
#[derive(Debug, Clone)]
pub enum RelocType {
    /// R_AARCH64_CALL26 - for BL instruction (26-bit PC-relative)
    Call26,
    /// R_AARCH64_JUMP26 - for B instruction (26-bit PC-relative)
    Jump26,
    /// R_AARCH64_ADR_PREL_PG_HI21 - for ADRP (page-relative, bits [32:12])
    AdrpPage21,
    /// R_AARCH64_ADD_ABS_LO12_NC - for ADD :lo12: (low 12 bits)
    AddAbsLo12,
    /// R_AARCH64_LDST8_ABS_LO12_NC
    Ldst8AbsLo12,
    /// R_AARCH64_LDST16_ABS_LO12_NC
    Ldst16AbsLo12,
    /// R_AARCH64_LDST32_ABS_LO12_NC
    Ldst32AbsLo12,
    /// R_AARCH64_LDST64_ABS_LO12_NC
    Ldst64AbsLo12,
    /// R_AARCH64_LDST128_ABS_LO12_NC
    Ldst128AbsLo12,
    /// R_AARCH64_ADR_GOT_PAGE21 - GOT-relative ADRP
    AdrGotPage21,
    /// R_AARCH64_LD64_GOT_LO12_NC - GOT entry LDR
    Ld64GotLo12,
    /// R_AARCH64_TLSLE_ADD_TPREL_HI12
    TlsLeAddTprelHi12,
    /// R_AARCH64_TLSLE_ADD_TPREL_LO12_NC
    TlsLeAddTprelLo12,
    /// R_AARCH64_CONDBR19 - conditional branch, 19-bit offset
    CondBr19,
    /// R_AARCH64_TSTBR14 - test-and-branch, 14-bit offset
    TstBr14,
    /// R_AARCH64_ADR_PREL_LO21 - for ADR (21-bit PC-relative)
    AdrPrelLo21,
    /// R_AARCH64_ABS64 - 64-bit absolute
    Abs64,
    /// R_AARCH64_ABS32 - 32-bit absolute
    Abs32,
    /// R_AARCH64_PREL32 - 32-bit PC-relative
    Prel32,
    /// R_AARCH64_PREL64 - 64-bit PC-relative
    Prel64,
    /// R_AARCH64_LD_PREL_LO19 - LDR literal, 19-bit PC-relative
    Ldr19,
}

impl RelocType {
    /// Get the ELF relocation type number
    pub fn elf_type(&self) -> u32 {
        match self {
            RelocType::Abs64 => 257,             // R_AARCH64_ABS64
            RelocType::Abs32 => 258,             // R_AARCH64_ABS32
            RelocType::Prel32 => 261,            // R_AARCH64_PREL32
            RelocType::Prel64 => 260,            // R_AARCH64_PREL64
            RelocType::Call26 => 283,            // R_AARCH64_CALL26
            RelocType::Jump26 => 282,            // R_AARCH64_JUMP26
            RelocType::AdrPrelLo21 => 274,       // R_AARCH64_ADR_PREL_LO21
            RelocType::AdrpPage21 => 275,        // R_AARCH64_ADR_PREL_PG_HI21
            RelocType::AddAbsLo12 => 277,        // R_AARCH64_ADD_ABS_LO12_NC
            RelocType::Ldst8AbsLo12 => 278,      // R_AARCH64_LDST8_ABS_LO12_NC
            RelocType::Ldst16AbsLo12 => 284,     // R_AARCH64_LDST16_ABS_LO12_NC
            RelocType::Ldst32AbsLo12 => 285,     // R_AARCH64_LDST32_ABS_LO12_NC
            RelocType::Ldst64AbsLo12 => 286,     // R_AARCH64_LDST64_ABS_LO12_NC
            RelocType::Ldst128AbsLo12 => 299,    // R_AARCH64_LDST128_ABS_LO12_NC
            RelocType::AdrGotPage21 => 311,      // R_AARCH64_ADR_GOT_PAGE21
            RelocType::Ld64GotLo12 => 312,       // R_AARCH64_LD64_GOT_LO12_NC
            RelocType::TlsLeAddTprelHi12 => 549, // R_AARCH64_TLSLE_ADD_TPREL_HI12
            RelocType::TlsLeAddTprelLo12 => 551, // R_AARCH64_TLSLE_ADD_TPREL_LO12_NC
            RelocType::CondBr19 => 280,          // R_AARCH64_CONDBR19
            RelocType::TstBr14 => 279,           // R_AARCH64_TSTBR14
            RelocType::Ldr19 => 273,             // R_AARCH64_LD_PREL_LO19
        }
    }
}

/// A relocation to be applied.
#[derive(Debug, Clone)]
pub struct Relocation {
    pub reloc_type: RelocType,
    pub symbol: String,
    pub addend: i64,
}

/// Parse a register name to its 5-bit encoding number (0-30, 31 for sp/zr).
pub fn parse_reg_num(name: &str) -> Option<u32> {
    let name = name.to_lowercase();
    match name.as_str() {
        "sp" | "wsp" => Some(31),
        "xzr" | "wzr" => Some(31),
        "lr" => Some(30),
        _ => {
            let prefix = name.chars().next()?;
            match prefix {
                'x' | 'w' | 'd' | 's' | 'q' | 'v' | 'h' | 'b' => {
                    let num: u32 = name[1..].parse().ok()?;
                    if num <= 31 {
                        Some(num)
                    } else {
                        None
                    }
                }
                _ => None,
            }
        }
    }
}

/// Check if a register name is a 64-bit (X) register or SP.
fn is_64bit_reg(name: &str) -> bool {
    let name = name.to_lowercase();
    name.starts_with('x') || name == "sp" || name == "xzr" || name == "lr"
}

/// Check if a register name is a 32-bit (W) register.
fn is_32bit_reg(name: &str) -> bool {
    let name = name.to_lowercase();
    name.starts_with('w') || name == "wsp" || name == "wzr"
}

/// Check if a register is a floating-point/SIMD register.
fn is_fp_reg(name: &str) -> bool {
    let c = name.chars().next().unwrap_or(' ').to_ascii_lowercase();
    matches!(c, 'd' | 's' | 'q' | 'v' | 'h' | 'b')
}

/// Encode a condition code string to 4-bit encoding.
fn encode_cond(cond: &str) -> Option<u32> {
    match cond.to_lowercase().as_str() {
        "eq" => Some(0),
        "ne" => Some(1),
        "cs" | "hs" => Some(2),
        "cc" | "lo" => Some(3),
        "mi" => Some(4),
        "pl" => Some(5),
        "vs" => Some(6),
        "vc" => Some(7),
        "hi" => Some(8),
        "ls" => Some(9),
        "ge" => Some(10),
        "lt" => Some(11),
        "gt" => Some(12),
        "le" => Some(13),
        "al" => Some(14),
        "nv" => Some(15),
        _ => None,
    }
}

/// Encode an AArch64 instruction from its mnemonic and parsed operands.
pub fn encode_instruction(
    mnemonic: &str,
    operands: &[Operand],
    raw_operands: &str,
) -> Result<EncodeResult, String> {
    let mn = mnemonic.to_lowercase();

    // Handle condition-code suffixed branches: b.eq, b.ne, b.lt, etc.
    if let Some(cond) = mn.strip_prefix("b.") {
        return encode_cond_branch(cond, operands);
    }

    // Handle condition-code branches without the dot: beq, bne, bge, blt, etc.
    // These are common aliases used in GNU assembler syntax.
    {
        let cond_aliases: &[(&str, &str)] = &[
            ("beq", "eq"),
            ("bne", "ne"),
            ("bcs", "cs"),
            ("bhs", "hs"),
            ("bcc", "cc"),
            ("blo", "lo"),
            ("bmi", "mi"),
            ("bpl", "pl"),
            ("bvs", "vs"),
            ("bvc", "vc"),
            ("bhi", "hi"),
            ("bls", "ls"),
            ("bge", "ge"),
            ("blt", "lt"),
            ("bgt", "gt"),
            ("ble", "le"),
            ("bal", "al"),
        ];
        for &(alias, cond) in cond_aliases {
            if mn == alias {
                return encode_cond_branch(cond, operands);
            }
        }
    }

    match mn.as_str() {
        // Data processing - register
        "mov" => encode_mov(operands),
        "movz" => encode_movz(operands),
        "movk" => encode_movk(operands),
        "movn" => encode_movn(operands),
        "add" => {
            if is_neon_scalar_d_reg_op(operands) {
                encode_neon_scalar_three_same(operands, 0, 0b10000, 0b11)
            } else {
                encode_add_sub(operands, false, false)
            }
        }
        "adds" => encode_add_sub(operands, false, true),
        "sub" => {
            if is_neon_scalar_d_reg_op(operands) {
                encode_neon_scalar_three_same(operands, 1, 0b10000, 0b11)
            } else {
                encode_add_sub(operands, true, false)
            }
        }
        "subs" => encode_add_sub(operands, true, true),
        "and" => encode_logical(operands, 0b00),
        "orr" => encode_logical(operands, 0b01),
        "eor" => encode_logical(operands, 0b10),
        "ands" => encode_logical(operands, 0b11),
        "orn" => encode_orn(operands),
        "eon" => encode_eon(operands),
        "bics" => encode_bics(operands),
        "mul" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                    encode_neon_elem(operands, 0, 0b1000)
                } else {
                    encode_neon_three_same(operands, 0, 0b10011)
                }
            } else {
                encode_mul(operands)
            }
        }
        "madd" => encode_madd(operands),
        "msub" => encode_msub(operands),
        "smull" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                    encode_neon_elem_long(operands, 0, 0b1010, false) // SMULL (by element)
                } else {
                    encode_neon_three_diff(operands, 0, 0b1100, false) // SMULL (vector)
                }
            } else {
                encode_smull(operands) // SMULL (scalar)
            }
        }
        "umull" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                    encode_neon_elem_long(operands, 1, 0b1010, false) // UMULL (by element)
                } else {
                    encode_neon_three_diff(operands, 1, 0b1100, false) // UMULL (vector)
                }
            } else {
                encode_umull(operands) // UMULL (scalar)
            }
        }
        "smaddl" => encode_smaddl(operands),
        "umaddl" => encode_umaddl(operands),
        "mneg" => encode_mneg(operands),
        "udiv" => encode_div(operands, true),
        "sdiv" => encode_div(operands, false),
        "umulh" => encode_umulh(operands),
        "smulh" => encode_smulh(operands),
        "neg" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_two_misc(operands, 1, 0b01011)
            } else {
                encode_neg(operands)
            }
        }
        "negs" => encode_negs(operands),
        "mvn" => encode_mvn(operands),
        "adc" => encode_adc(operands, false),
        "adcs" => encode_adc(operands, true),
        "sbc" => encode_sbc(operands, false),
        "sbcs" => encode_sbc(operands, true),

        // Shifts
        "lsl" => encode_shift(operands, 0b00),
        "lsr" => encode_shift(operands, 0b01),
        "asr" => encode_shift(operands, 0b10),
        "ror" => encode_shift(operands, 0b11),

        // Extensions
        "sxtw" => encode_sxtw(operands),
        "sxth" => encode_sxth(operands),
        "sxtb" => encode_sxtb(operands),
        "uxtw" => encode_uxtw(operands),
        "uxth" => encode_uxth(operands),
        "uxtb" => encode_uxtb(operands),

        // Compare
        "cmp" => encode_cmp(operands),
        "cmn" => encode_cmn(operands),
        "tst" => encode_tst(operands),
        "ccmp" => encode_ccmp_ccmn(operands, true),
        "ccmn" => encode_ccmp_ccmn(operands, false),

        // Conditional select
        "csel" => encode_csel(operands),
        "csinc" => encode_csinc(operands),
        "csinv" => encode_csinv(operands),
        "csneg" => encode_csneg(operands),
        "cset" => encode_cset(operands),
        "csetm" => encode_csetm(operands),

        // Branches
        "b" => encode_branch(operands),
        "bl" => encode_bl(operands),
        "br" => encode_br(operands),
        "blr" => encode_blr(operands),
        "ret" => encode_ret(operands),
        "cbz" => encode_cbz(operands, false),
        "cbnz" => encode_cbz(operands, true),
        "tbz" => encode_tbz(operands, false),
        "tbnz" => encode_tbz(operands, true),

        // Loads/stores - size determined from register width
        "ldr" => encode_ldr_str_auto(operands, true),
        "str" => encode_ldr_str_auto(operands, false),
        "ldrb" => encode_ldr_str(operands, true, 0b00, false, false), // byte load
        "strb" => encode_ldr_str(operands, false, 0b00, false, false),
        "ldrh" => encode_ldr_str(operands, true, 0b01, false, false), // halfword load
        "strh" => encode_ldr_str(operands, false, 0b01, false, false),
        "ldrw" | "ldrsw" => encode_ldrsw(operands),
        "ldrsb" => encode_ldrs(operands, 0b00),
        "ldrsh" => encode_ldrs(operands, 0b01),
        "ldur" => encode_ldur_stur(operands, true, 0b00),
        "stur" => encode_ldur_stur(operands, false, 0b00),
        "ldtr" => encode_ldur_stur(operands, true, 0b10),
        "sttr" => encode_ldur_stur(operands, false, 0b10),
        "ldtrh" => encode_ldtr_sized(operands, true, 0b01),
        "sttrh" => encode_ldtr_sized(operands, false, 0b01),
        "ldtrb" => encode_ldtr_sized(operands, true, 0b00),
        "sttrb" => encode_ldtr_sized(operands, false, 0b00),
        "ldp" => encode_ldp_stp(operands, true),
        "stp" => encode_ldp_stp(operands, false),
        "ldnp" => encode_ldnp_stnp(operands, true),
        "stnp" => encode_ldnp_stnp(operands, false),
        "ldxr" => encode_ldxr_stxr(operands, true, None),
        "stxr" => encode_ldxr_stxr(operands, false, None),
        "ldxrb" => encode_ldxr_stxr(operands, true, Some(0b00)),
        "stxrb" => encode_ldxr_stxr(operands, false, Some(0b00)),
        "ldxrh" => encode_ldxr_stxr(operands, true, Some(0b01)),
        "stxrh" => encode_ldxr_stxr(operands, false, Some(0b01)),
        "ldaxr" => encode_ldaxr_stlxr(operands, true, None),
        "stlxr" => encode_ldaxr_stlxr(operands, false, None),
        "ldaxrb" => encode_ldaxr_stlxr(operands, true, Some(0b00)),
        "stlxrb" => encode_ldaxr_stlxr(operands, false, Some(0b00)),
        "ldaxrh" => encode_ldaxr_stlxr(operands, true, Some(0b01)),
        "stlxrh" => encode_ldaxr_stlxr(operands, false, Some(0b01)),
        "ldar" => encode_ldar_stlr(operands, true, None),
        "stlr" => encode_ldar_stlr(operands, false, None),
        "ldarb" => encode_ldar_stlr(operands, true, Some(0b00)),
        "stlrb" => encode_ldar_stlr(operands, false, Some(0b00)),
        "ldarh" => encode_ldar_stlr(operands, true, Some(0b01)),
        "stlrh" => encode_ldar_stlr(operands, false, Some(0b01)),
        "ldxp" => encode_ldxp_stxp(operands, true, false),
        "ldaxp" => encode_ldxp_stxp(operands, true, true),
        "stxp" => encode_ldxp_stxp(operands, false, false),
        "stlxp" => encode_ldxp_stxp(operands, false, true),

        // Address computation
        "adrp" => encode_adrp(operands),
        "adr" => encode_adr(operands),

        // Floating point (scalar or vector based on operand type)
        "fmov" => encode_fmov(operands),
        "fadd" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_float_three_same(operands, 0, 0, 0b11010)
            } else {
                encode_fp_arith(operands, 0b0010)
            }
        }
        "fsub" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_float_three_same(operands, 0, 1, 0b11010)
            } else {
                encode_fp_arith(operands, 0b0011)
            }
        }
        "fmul" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                    encode_neon_float_elem(operands, 1, 0b1001)
                } else {
                    encode_neon_float_three_same(operands, 1, 0, 0b11011)
                }
            } else {
                encode_fp_arith(operands, 0b0000)
            }
        }
        "fdiv" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_float_three_same(operands, 1, 0, 0b11111)
            } else {
                encode_fp_arith(operands, 0b0001)
            }
        }
        "fmax" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_float_three_same(operands, 0, 0, 0b11110)
            } else {
                encode_fp_arith(operands, 0b0100)
            }
        }
        "fmin" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_float_three_same(operands, 0, 1, 0b11110)
            } else {
                encode_fp_arith(operands, 0b0101)
            }
        }
        "fmaxnm" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_float_three_same(operands, 0, 0, 0b11000)
            } else {
                encode_fp_arith(operands, 0b0110)
            }
        }
        "fminnm" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_float_three_same(operands, 0, 1, 0b11000)
            } else {
                encode_fp_arith(operands, 0b0111)
            }
        }
        "fneg" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_float_two_misc(operands, 1, 0, 0b01111)
            } else {
                encode_fneg(operands)
            }
        }
        "fabs" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_float_two_misc(operands, 0, 1, 0b01111)
            } else {
                encode_fabs(operands)
            }
        }
        "fsqrt" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_float_two_misc(operands, 1, 1, 0b11111)
            } else {
                encode_fsqrt(operands)
            }
        }
        "frintn" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_float_two_misc(operands, 0, 0, 0b11000)
            } else {
                encode_fp_1src(operands, 0b001000)
            }
        }
        "frintp" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_float_two_misc(operands, 0, 1, 0b11000)
            } else {
                encode_fp_1src(operands, 0b001001)
            }
        }
        "frintm" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_float_two_misc(operands, 0, 0, 0b11001)
            } else {
                encode_fp_1src(operands, 0b001010)
            }
        }
        "frintz" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_float_two_misc(operands, 0, 1, 0b11001)
            } else {
                encode_fp_1src(operands, 0b001011)
            }
        }
        "frinta" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_float_two_misc(operands, 1, 0, 0b11000)
            } else {
                encode_fp_1src(operands, 0b001100)
            }
        }
        "frintx" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_float_two_misc(operands, 1, 0, 0b11001)
            } else {
                encode_fp_1src(operands, 0b001110)
            }
        }
        "frinti" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_float_two_misc(operands, 1, 1, 0b11001)
            } else {
                encode_fp_1src(operands, 0b001111)
            }
        }
        "fmadd" => encode_fmadd_fmsub(operands, false),
        "fmsub" => encode_fmadd_fmsub(operands, true),
        "fnmadd" => encode_fnmadd_fnmsub(operands, false),
        "fnmsub" => encode_fnmadd_fnmsub(operands, true),
        "fcmp" => encode_fcmp(operands),
        "fcvtzs" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_float_two_misc(operands, 0, 1, 0b11011)
            } else {
                encode_fcvt_rounding(operands, 0b11, 0b000)
            }
        }
        "fcvtzu" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_float_two_misc(operands, 1, 1, 0b11011)
            } else {
                encode_fcvt_rounding(operands, 0b11, 0b001)
            }
        }
        "fcvtas" => encode_fcvt_rounding(operands, 0b00, 0b100),
        "fcvtau" => encode_fcvt_rounding(operands, 0b00, 0b101),
        "fcvtns" => encode_fcvt_rounding(operands, 0b00, 0b000),
        "fcvtnu" => encode_fcvt_rounding(operands, 0b00, 0b001),
        "fcvtms" => encode_fcvt_rounding(operands, 0b10, 0b000),
        "fcvtmu" => encode_fcvt_rounding(operands, 0b10, 0b001),
        "fcvtps" => encode_fcvt_rounding(operands, 0b01, 0b000),
        "fcvtpu" => encode_fcvt_rounding(operands, 0b01, 0b001),
        "ucvtf" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_float_two_misc(operands, 1, 0, 0b11101)
            } else {
                encode_ucvtf(operands)
            }
        }
        "scvtf" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_float_two_misc(operands, 0, 0, 0b11101)
            } else {
                encode_scvtf(operands)
            }
        }
        "fcvt" => encode_fcvt_precision(operands),
        "fcvtl" => encode_neon_fcvtl(operands, false),
        "fcvtl2" => encode_neon_fcvtl(operands, true),
        "fcvtn" => encode_neon_fcvtn(operands, false),
        "fcvtn2" => encode_neon_fcvtn(operands, true),
        // NEON float three-same instructions (vector-only)
        "fmla" => {
            if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                encode_neon_float_elem(operands, 0, 0b0001)
            } else {
                encode_neon_float_three_same(operands, 0, 0, 0b11001)
            }
        }
        "fmls" => {
            if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                encode_neon_float_elem(operands, 0, 0b0101)
            } else {
                encode_neon_float_three_same(operands, 0, 1, 0b11001)
            }
        }
        "frecps" => encode_neon_float_three_same(operands, 0, 0, 0b11111),
        "frsqrts" => encode_neon_float_three_same(operands, 0, 1, 0b11111),
        "fcmeq" => {
            if matches!(operands.get(2), Some(Operand::Imm(0))) {
                encode_neon_float_cmp_zero(operands, 0, 0, 0b01101)
            } else {
                encode_neon_float_three_same(operands, 0, 0, 0b11100)
            }
        }
        "fcmge" => {
            if matches!(operands.get(2), Some(Operand::Imm(0))) {
                encode_neon_float_cmp_zero(operands, 1, 0, 0b01100)
            } else {
                encode_neon_float_three_same(operands, 1, 0, 0b11100)
            }
        }
        "fcmgt" => {
            if matches!(operands.get(2), Some(Operand::Imm(0))) {
                encode_neon_float_cmp_zero(operands, 0, 1, 0b01100)
            } else {
                encode_neon_float_three_same(operands, 1, 1, 0b11100)
            }
        }
        "fcmle" => encode_neon_float_cmp_zero(operands, 1, 0, 0b01101),
        "fcmlt" => encode_neon_float_cmp_zero(operands, 0, 1, 0b01101),
        "facge" => encode_neon_float_three_same(operands, 1, 0, 0b11101),
        "facgt" => encode_neon_float_three_same(operands, 1, 1, 0b11101),

        // NEON/SIMD
        "cnt" => encode_cnt(operands),
        "cmeq" => {
            // CMEQ has two forms:
            // - CMEQ Vd, Vn, Vm (three-same, U=1): compare registers
            // - CMEQ Vd, Vn, #0 (two-reg misc, U=0): compare to zero
            if matches!(operands.get(2), Some(Operand::Imm(0))) {
                encode_neon_cmp_zero(operands, 0, 0b01001)
            } else {
                encode_neon_three_same(operands, 1, 0b10001)
            }
        }
        "cmhi" => encode_neon_three_same(operands, 1, 0b00110),
        "cmhs" => encode_neon_three_same(operands, 1, 0b00111),
        "cmge" => {
            if matches!(operands.get(2), Some(Operand::Imm(0))) {
                encode_neon_cmp_zero(operands, 1, 0b01000) // CMGE #0
            } else {
                encode_neon_three_same(operands, 0, 0b00111)
            }
        }
        "cmgt" => {
            if matches!(operands.get(2), Some(Operand::Imm(0))) {
                encode_neon_cmp_zero(operands, 0, 0b01000) // CMGT #0
            } else {
                encode_neon_three_same(operands, 0, 0b00110)
            }
        }
        "cmtst" => encode_neon_three_same(operands, 0, 0b10001),
        "uqsub" => encode_neon_three_same(operands, 1, 0b00101),
        "sqsub" => encode_neon_three_same(operands, 0, 0b00101),
        "uhadd" => encode_neon_three_same(operands, 1, 0b00000),
        "shadd" => encode_neon_three_same(operands, 0, 0b00000),
        "urhadd" => encode_neon_three_same(operands, 1, 0b00010),
        "srhadd" => encode_neon_three_same(operands, 0, 0b00010),
        "uhsub" => encode_neon_three_same(operands, 1, 0b00100),
        "shsub" => encode_neon_three_same(operands, 0, 0b00100),
        "umax" => encode_neon_three_same(operands, 1, 0b01100),
        "smax" => encode_neon_three_same(operands, 0, 0b01100),
        "umin" => encode_neon_three_same(operands, 1, 0b01101),
        "smin" => encode_neon_three_same(operands, 0, 0b01101),
        "uabd" => encode_neon_three_same(operands, 1, 0b01110),
        "sabd" => encode_neon_three_same(operands, 0, 0b01110),
        "uaba" => encode_neon_three_same(operands, 1, 0b01111),
        "saba" => encode_neon_three_same(operands, 0, 0b01111),
        "uqadd" => encode_neon_three_same(operands, 1, 0b00001),
        "sqadd" => encode_neon_three_same(operands, 0, 0b00001),
        "sshl" => encode_neon_three_same(operands, 0, 0b01000),
        "ushl" => encode_neon_three_same(operands, 1, 0b01000),
        "sqshl" => {
            if matches!(operands.get(2), Some(Operand::Imm(_))) {
                encode_neon_shift_left_imm(operands, 0, 0b01110)
            } else {
                encode_neon_three_same(operands, 0, 0b01001)
            }
        }
        "uqshl" => {
            if matches!(operands.get(2), Some(Operand::Imm(_))) {
                encode_neon_shift_left_imm(operands, 1, 0b01110)
            } else {
                encode_neon_three_same(operands, 1, 0b01001)
            }
        }
        "srshl" => encode_neon_three_same(operands, 0, 0b01010),
        "urshl" => encode_neon_three_same(operands, 1, 0b01010),
        "sqrshl" => encode_neon_three_same(operands, 0, 0b01011),
        "uqrshl" => encode_neon_three_same(operands, 1, 0b01011),
        "addp" => {
            if operands.len() == 2
                && matches!(operands.first(), Some(Operand::Reg(r)) if r.starts_with('d') || r.starts_with('D'))
            {
                // Scalar ADDP: addp Dd, Vn.2d
                encode_neon_scalar_addp(operands)
            } else {
                encode_neon_three_same(operands, 0, 0b10111)
            }
        }
        "uminp" => encode_neon_three_same(operands, 1, 0b10101),
        "umaxp" => encode_neon_three_same(operands, 1, 0b10100),
        "sminp" => encode_neon_three_same(operands, 0, 0b10101),
        "smaxp" => encode_neon_three_same(operands, 0, 0b10100),
        // NEON two-reg misc (integer)
        "abs" => encode_neon_two_misc(operands, 0, 0b01011),
        // neg dispatch moved to early scalar section
        "cls" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_two_misc(operands, 0, 0b00100)
            } else {
                encode_cls(operands)
            }
        }
        "clz" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_two_misc(operands, 1, 0b00100)
            } else {
                encode_clz(operands)
            }
        }
        "rev16" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_two_misc(operands, 0, 0b00001)
            } else {
                encode_rev16(operands)
            }
        }
        "rev32" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_two_misc(operands, 1, 0b00000)
            } else {
                encode_rev32(operands)
            }
        }
        "saddlp" => encode_neon_two_misc(operands, 0, 0b00010),
        "uaddlp" => encode_neon_two_misc(operands, 1, 0b00010),
        "sadalp" => encode_neon_two_misc(operands, 0, 0b00110),
        "uadalp" => encode_neon_two_misc(operands, 1, 0b00110),
        "sqxtun" => encode_neon_two_misc_narrow(operands, 1, 0b10010, false),
        "sqxtun2" => encode_neon_two_misc_narrow(operands, 1, 0b10010, true),
        "sqabs" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_two_misc(operands, 0, 0b00111)
            } else {
                encode_neon_scalar_two_misc(operands, 0, 0b00111)
            }
        }
        "sqneg" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_two_misc(operands, 0, 0b01000)
            } else {
                encode_neon_scalar_two_misc(operands, 0, 0b01000)
            }
        }
        // Compare to zero forms
        "cmlt" => encode_neon_cmp_zero(operands, 0, 0b01010), // CMLT #0
        "cmle" => encode_neon_cmp_zero(operands, 1, 0b01001), // CMLE #0
        // NEON shift right narrow
        "shrn" => encode_neon_shrn(operands, 0b100001, false),
        "shrn2" => encode_neon_shrn(operands, 0b100001, true),
        "rshrn" => encode_neon_shrn(operands, 0b100011, false),
        "rshrn2" => encode_neon_shrn(operands, 0b100011, true),
        // NEON shift right accumulate and rounding shift right
        "srshr" => encode_neon_shift_right(operands, 0, 0b001001),
        "urshr" => encode_neon_shift_right(operands, 1, 0b001001),
        "ssra" => encode_neon_shift_right(operands, 0, 0b000101),
        "usra" => encode_neon_shift_right(operands, 1, 0b000101),
        "srsra" => encode_neon_shift_right(operands, 0, 0b001101),
        "ursra" => encode_neon_shift_right(operands, 1, 0b001101),
        // NEON shift left long
        "ushll" => encode_neon_shll(operands, 1, false),
        "ushll2" => encode_neon_shll(operands, 1, true),
        "sshll" => encode_neon_shll(operands, 0, false),
        "sshll2" => encode_neon_shll(operands, 0, true),
        // NEON three-different extras
        "uabal" => encode_neon_three_diff(operands, 1, 0b0101, false),
        "uabal2" => encode_neon_three_diff(operands, 1, 0b0101, true),
        "sabal" => encode_neon_three_diff(operands, 0, 0b0101, false),
        "sabal2" => encode_neon_three_diff(operands, 0, 0b0101, true),
        "uabdl" => encode_neon_three_diff(operands, 1, 0b0111, false),
        "uabdl2" => encode_neon_three_diff(operands, 1, 0b0111, true),
        "sabdl" => encode_neon_three_diff(operands, 0, 0b0111, false),
        "sabdl2" => encode_neon_three_diff(operands, 0, 0b0111, true),
        // ADDHN/RADDHN/SUBHN/RSUBHN (narrowing high)
        "addhn" => encode_neon_three_diff_narrow(operands, 0, 0b0100, false),
        "addhn2" => encode_neon_three_diff_narrow(operands, 0, 0b0100, true),
        "raddhn" => encode_neon_three_diff_narrow(operands, 1, 0b0100, false),
        "raddhn2" => encode_neon_three_diff_narrow(operands, 1, 0b0100, true),
        "subhn" => encode_neon_three_diff_narrow(operands, 0, 0b0110, false),
        "subhn2" => encode_neon_three_diff_narrow(operands, 0, 0b0110, true),
        "rsubhn" => encode_neon_three_diff_narrow(operands, 1, 0b0110, false),
        "rsubhn2" => encode_neon_three_diff_narrow(operands, 1, 0b0110, true),
        // NEON sat shift right narrow
        "sqshrn" => {
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_qshrn(operands, 0, false, false)
            } else {
                encode_neon_scalar_qshrn(operands, 0, false)
            }
        }
        "sqshrn2" => encode_neon_qshrn(operands, 0, false, true),
        "uqshrn" => encode_neon_qshrn(operands, 1, false, false),
        "uqshrn2" => encode_neon_qshrn(operands, 1, false, true),
        "sqrshrn" => encode_neon_qshrn(operands, 0, true, false),
        "sqrshrn2" => encode_neon_qshrn(operands, 0, true, true),
        "uqrshrn" => encode_neon_qshrn(operands, 1, true, false),
        "uqrshrn2" => encode_neon_qshrn(operands, 1, true, true),
        "sqrshrun" => encode_neon_sqshrun(operands, true, false),
        "sqrshrun2" => encode_neon_sqshrun(operands, true, true),
        // NEON permute: TRN1/TRN2
        "trn1" => encode_neon_zip_uzp(operands, 0b010, false),
        "trn2" => encode_neon_zip_uzp(operands, 0b110, false),
        // NEON replicate loads
        "ld2r" => encode_neon_ldnr(operands, 2),
        "ld3r" => encode_neon_ldnr(operands, 3),
        "ld4r" => encode_neon_ldnr(operands, 4),
        "ushr" => encode_neon_ushr(operands),
        "sshr" => encode_neon_sshr(operands),
        "shl" => encode_neon_shl(operands),
        "sli" => encode_neon_sli(operands),
        "sri" => encode_neon_sri(operands),
        "ext" => encode_neon_ext(operands),
        "addv" => encode_neon_addv(operands),
        "umaxv" => encode_neon_across(operands, 1, 0b01010),
        "uminv" => encode_neon_across(operands, 1, 0b11010),
        "smaxv" => encode_neon_across(operands, 0, 0b01010),
        "sminv" => encode_neon_across(operands, 0, 0b11010),
        "umov" => encode_neon_umov(operands),
        "dup" => encode_neon_dup(operands),
        "ins" => encode_neon_ins(operands),
        "not" => encode_neon_not(operands),
        "movi" => encode_neon_movi(operands),
        "bic" => encode_bic(operands),
        "bsl" => encode_neon_bsl(operands),
        "bit" => encode_neon_bitwise_insert(operands, 0b10),
        "bif" => encode_neon_bitwise_insert(operands, 0b11),
        "faddp" => encode_neon_faddp(operands),
        "saddlv" => encode_neon_across_long(operands, 0, 0b00011),
        "uaddlv" => encode_neon_across_long(operands, 1, 0b00011),
        "sqdmlal" => {
            if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                encode_neon_elem_long(operands, 0, 0b0011, false)
            } else {
                encode_neon_three_diff(operands, 0, 0b1001, false)
            }
        }
        "sqdmlal2" => {
            if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                encode_neon_elem_long(operands, 0, 0b0011, true)
            } else {
                encode_neon_three_diff(operands, 0, 0b1001, true)
            }
        }
        "sqdmlsl" => {
            if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                encode_neon_elem_long(operands, 0, 0b0111, false)
            } else {
                encode_neon_three_diff(operands, 0, 0b1011, false)
            }
        }
        "sqdmlsl2" => {
            if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                encode_neon_elem_long(operands, 0, 0b0111, true)
            } else {
                encode_neon_three_diff(operands, 0, 0b1011, true)
            }
        }
        "sqdmull" => {
            if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                encode_neon_elem_long(operands, 0, 0b1011, false)
            } else {
                encode_neon_three_diff(operands, 0, 0b1101, false)
            }
        }
        "sqdmull2" => {
            if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                encode_neon_elem_long(operands, 0, 0b1011, true)
            } else {
                encode_neon_three_diff(operands, 0, 0b1101, true)
            }
        }
        "sqdmulh" => {
            if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                encode_neon_elem(operands, 0, 0b1100)
            } else {
                encode_neon_three_same(operands, 0, 0b10110)
            }
        }
        "sqrdmulh" => {
            if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                encode_neon_elem(operands, 0, 0b1101)
            } else {
                encode_neon_three_same(operands, 1, 0b10110)
            }
        }
        "pmul" => encode_neon_pmul(operands),
        "mla" => {
            if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                encode_neon_elem(operands, 0, 0b0000)
            } else {
                encode_neon_mla(operands)
            }
        }
        "mls" => {
            if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                encode_neon_elem(operands, 0, 0b0100)
            } else {
                encode_neon_mls(operands)
            }
        }
        "rev64" => encode_neon_rev64(operands),
        "tbl" => encode_neon_tbl(operands),
        "tbx" => encode_neon_tbx(operands),
        "ld1" => encode_neon_ld_st_dispatch(operands, true, 1),
        "ld1r" => encode_neon_ld1r(operands),
        "ld2" => encode_neon_ld_st_dispatch(operands, true, 2),
        "ld3" => encode_neon_ld_st_dispatch(operands, true, 3),
        "ld4" => encode_neon_ld_st_dispatch(operands, true, 4),
        "st1" => encode_neon_ld_st_dispatch(operands, false, 1),
        "st2" => encode_neon_ld_st_dispatch(operands, false, 2),
        "st3" => encode_neon_ld_st_dispatch(operands, false, 3),
        "st4" => encode_neon_ld_st_dispatch(operands, false, 4),
        "uzp1" => encode_neon_zip_uzp(operands, 0b001, false),
        "uzp2" => encode_neon_zip_uzp(operands, 0b101, false),
        "zip1" => encode_neon_zip_uzp(operands, 0b011, false),
        "zip2" => encode_neon_zip_uzp(operands, 0b111, false),
        "eor3" => encode_neon_eor3(operands),
        "pmull" => encode_neon_pmull(operands, false),
        "pmull2" => encode_neon_pmull(operands, true),
        "aese" => encode_neon_aes(operands, 0b00100),
        "aesd" => encode_neon_aes(operands, 0b00101),
        "aesmc" => encode_neon_aes(operands, 0b00110),
        "aesimc" => encode_neon_aes(operands, 0b00111),

        // NEON three-different (widening/narrowing)
        "usubl" => encode_neon_three_diff(operands, 1, 0b0010, false),
        "usubl2" => encode_neon_three_diff(operands, 1, 0b0010, true),
        "ssubl" => encode_neon_three_diff(operands, 0, 0b0010, false),
        "ssubl2" => encode_neon_three_diff(operands, 0, 0b0010, true),
        "usubw" => encode_neon_three_diff(operands, 1, 0b0011, false),
        "usubw2" => encode_neon_three_diff(operands, 1, 0b0011, true),
        "ssubw" => encode_neon_three_diff(operands, 0, 0b0011, false),
        "ssubw2" => encode_neon_three_diff(operands, 0, 0b0011, true),
        "uaddl" => encode_neon_three_diff(operands, 1, 0b0000, false),
        "uaddl2" => encode_neon_three_diff(operands, 1, 0b0000, true),
        "saddl" => encode_neon_three_diff(operands, 0, 0b0000, false),
        "saddl2" => encode_neon_three_diff(operands, 0, 0b0000, true),
        "uaddw" => encode_neon_three_diff(operands, 1, 0b0001, false),
        "uaddw2" => encode_neon_three_diff(operands, 1, 0b0001, true),
        "saddw" => encode_neon_three_diff(operands, 0, 0b0001, false),
        "saddw2" => encode_neon_three_diff(operands, 0, 0b0001, true),
        "umlal" => {
            if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                encode_neon_elem_long(operands, 1, 0b0010, false)
            } else {
                encode_neon_three_diff(operands, 1, 0b1000, false)
            }
        }
        "umlal2" => {
            if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                encode_neon_elem_long(operands, 1, 0b0010, true)
            } else {
                encode_neon_three_diff(operands, 1, 0b1000, true)
            }
        }
        "smlal" => {
            if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                encode_neon_elem_long(operands, 0, 0b0010, false)
            } else {
                encode_neon_three_diff(operands, 0, 0b1000, false)
            }
        }
        "smlal2" => {
            if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                encode_neon_elem_long(operands, 0, 0b0010, true)
            } else {
                encode_neon_three_diff(operands, 0, 0b1000, true)
            }
        }
        "umlsl" => {
            if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                encode_neon_elem_long(operands, 1, 0b0110, false)
            } else {
                encode_neon_three_diff(operands, 1, 0b1010, false)
            }
        }
        "umlsl2" => {
            if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                encode_neon_elem_long(operands, 1, 0b0110, true)
            } else {
                encode_neon_three_diff(operands, 1, 0b1010, true)
            }
        }
        "smlsl" => {
            if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                encode_neon_elem_long(operands, 0, 0b0110, false)
            } else {
                encode_neon_three_diff(operands, 0, 0b1010, false)
            }
        }
        "smlsl2" => {
            if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                encode_neon_elem_long(operands, 0, 0b0110, true)
            } else {
                encode_neon_three_diff(operands, 0, 0b1010, true)
            }
        }
        "umull2" => {
            if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                encode_neon_elem_long(operands, 1, 0b1010, true)
            } else {
                encode_neon_three_diff(operands, 1, 0b1100, true)
            }
        }
        "smull2" => {
            if matches!(operands.get(2), Some(Operand::RegLane { .. })) {
                encode_neon_elem_long(operands, 0, 0b1010, true)
            } else {
                encode_neon_three_diff(operands, 0, 0b1100, true)
            }
        }

        // NEON saturating shift right narrow
        "sqshrun" => encode_neon_sqshrun(operands, false, false),
        "sqshrun2" => encode_neon_sqshrun(operands, false, true),

        // NEON extend long (aliases for USHLL/SSHLL #0)
        "uxtl" => encode_neon_xtl(operands, 1, false),
        "uxtl2" => encode_neon_xtl(operands, 1, true),
        "sxtl" => encode_neon_xtl(operands, 0, false),
        "sxtl2" => encode_neon_xtl(operands, 0, true),

        // NEON two-register narrowing
        "uqxtn" => encode_neon_two_misc_narrow(operands, 1, 0b10100, false),
        "uqxtn2" => encode_neon_two_misc_narrow(operands, 1, 0b10100, true),
        "sqxtn" => encode_neon_two_misc_narrow(operands, 0, 0b10100, false),
        "sqxtn2" => encode_neon_two_misc_narrow(operands, 0, 0b10100, true),
        "xtn" => encode_neon_two_misc_narrow(operands, 0, 0b10010, false),
        "xtn2" => encode_neon_two_misc_narrow(operands, 0, 0b10010, true),

        // System
        "hint" => encode_hint(operands),
        "bti" => encode_bti(raw_operands),
        "nop" => Ok(EncodeResult::Word(0xd503201f)),
        "yield" => Ok(EncodeResult::Word(0xd503203f)),
        "wfe" => Ok(EncodeResult::Word(0xd503205f)),
        "wfi" => Ok(EncodeResult::Word(0xd503207f)),
        "sev" => Ok(EncodeResult::Word(0xd503209f)),
        "sevl" => Ok(EncodeResult::Word(0xd50320bf)),
        "eret" => Ok(EncodeResult::Word(0xd69f03e0)),
        "clrex" => Ok(EncodeResult::Word(0xd503305f)),
        "dc" => encode_dc(operands, raw_operands),
        "tlbi" => encode_tlbi(operands, raw_operands),
        "ic" => encode_ic(raw_operands),
        "dmb" => encode_dmb(operands),
        "dsb" => encode_dsb(operands),
        "isb" => Ok(EncodeResult::Word(0xd5033fdf)),
        "mrs" => encode_mrs(operands),
        "msr" => encode_msr(operands),
        "svc" => encode_svc(operands),
        "hvc" => encode_hvc(operands),
        "smc" => encode_smc(operands),
        "at" => encode_at(operands, raw_operands),
        "sys" => encode_sys(raw_operands),
        "brk" => encode_brk(operands),

        // Bitfield extract/insert
        "ubfx" => encode_ubfx(operands),
        "sbfx" => encode_sbfx(operands),
        "ubfm" => encode_ubfm(operands),
        "sbfm" => encode_sbfm(operands),
        "ubfiz" => encode_ubfiz(operands),
        "sbfiz" => encode_sbfiz(operands),
        "bfm" => encode_bfm(operands),
        "bfi" => encode_bfi(operands),
        "bfxil" => encode_bfxil(operands),
        "extr" => encode_extr(operands),

        // Additional conditional operations
        "cneg" => encode_cneg(operands),
        "cinc" => encode_cinc(operands),
        "cinv" => encode_cinv(operands),

        // Bit manipulation
        "rbit" => {
            // RBIT has both scalar and NEON forms
            if matches!(operands.first(), Some(Operand::RegArrangement { .. })) {
                encode_neon_rbit(operands)
            } else {
                encode_rbit(operands)
            }
        }
        "rev" => encode_rev(operands),

        // CRC32
        "crc32b" | "crc32h" | "crc32w" | "crc32x" | "crc32cb" | "crc32ch" | "crc32cw"
        | "crc32cx" => encode_crc32(mnemonic, operands),

        // Prefetch
        "prfm" => encode_prfm(operands),

        // LSE atomics
        "cas" | "casa" | "casal" | "casl" | "casb" | "casab" | "casalb" | "caslb" | "cash"
        | "casah" | "casalh" | "caslh" => encode_cas(mnemonic, operands),
        "swp" | "swpa" | "swpal" | "swpl" | "swpb" | "swpab" | "swpalb" | "swplb" | "swph"
        | "swpah" | "swpalh" | "swplh" => encode_swp(mnemonic, operands),
        "ldadd" | "ldadda" | "ldaddal" | "ldaddl" | "ldaddb" | "ldaddab" | "ldaddalb"
        | "ldaddlb" | "ldaddh" | "ldaddah" | "ldaddalh" | "ldaddlh" | "ldclr" | "ldclra"
        | "ldclral" | "ldclrl" | "ldclrb" | "ldclrab" | "ldclralb" | "ldclrlb" | "ldclrh"
        | "ldclrah" | "ldclralh" | "ldclrlh" | "ldeor" | "ldeora" | "ldeoral" | "ldeorl"
        | "ldeorb" | "ldeorab" | "ldeoralb" | "ldeorlb" | "ldeorh" | "ldeorah" | "ldeoralh"
        | "ldeorlh" | "ldset" | "ldseta" | "ldsetal" | "ldsetl" | "ldsetb" | "ldsetab"
        | "ldsetalb" | "ldsetlb" | "ldseth" | "ldsetah" | "ldsetalh" | "ldsetlh" => {
            encode_ldop(mnemonic, operands)
        }
        // LSE atomic store aliases (Rt=XZR, discard result)
        "stadd" | "staddl" | "staddb" | "staddlb" | "staddh" | "staddlh" | "stclr" | "stclrl"
        | "stclrb" | "stclrlb" | "stclrh" | "stclrlh" | "steor" | "steorl" | "steorb"
        | "steorlb" | "steorh" | "steorlh" | "stset" | "stsetl" | "stsetb" | "stsetlb"
        | "stseth" | "stsetlh" => encode_stop(mnemonic, operands),

        // NEON move-not-immediate
        "mvni" => encode_neon_mvni(operands),

        _ => {
            // TODO: handle remaining instructions
            Err(format!(
                "unsupported instruction: {} {}",
                mnemonic, raw_operands
            ))
        }
    }
}

// ── Encoding helpers ──────────────────────────────────────────────────────

fn get_reg(operands: &[Operand], idx: usize) -> Result<(u32, bool), String> {
    match operands.get(idx) {
        Some(Operand::Reg(name)) => {
            let num = parse_reg_num(name).ok_or_else(|| format!("invalid register: {}", name))?;
            let is_64 = is_64bit_reg(name);
            Ok((num, is_64))
        }
        other => Err(format!(
            "expected register at operand {}, got {:?}",
            idx, other
        )),
    }
}

fn get_imm(operands: &[Operand], idx: usize) -> Result<i64, String> {
    match operands.get(idx) {
        Some(Operand::Imm(v)) => Ok(*v),
        other => Err(format!(
            "expected immediate at operand {}, got {:?}",
            idx, other
        )),
    }
}

fn get_symbol(operands: &[Operand], idx: usize) -> Result<(String, i64), String> {
    match operands.get(idx) {
        Some(Operand::Symbol(s)) => Ok((s.clone(), 0)),
        Some(Operand::Label(s)) => Ok((s.clone(), 0)),
        Some(Operand::SymbolOffset(s, off)) => Ok((s.clone(), *off)),
        Some(Operand::Modifier { symbol, .. }) => Ok((symbol.clone(), 0)),
        Some(Operand::ModifierOffset { symbol, offset, .. }) => Ok((symbol.clone(), *offset)),
        // The parser misclassifies symbol names that collide with register names,
        // condition codes, or barrier names. These are valid symbols in context.
        Some(Operand::Reg(name)) => Ok((name.clone(), 0)),
        Some(Operand::Cond(name)) => Ok((name.clone(), 0)),
        Some(Operand::Barrier(name)) => Ok((name.clone(), 0)),
        other => Err(format!(
            "expected symbol at operand {}, got {:?}",
            idx, other
        )),
    }
}

fn sf_bit(is_64: bool) -> u32 {
    if is_64 {
        1
    } else {
        0
    }
}
