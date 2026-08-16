use super::*;
use crate::backend::arm::assembler::parser::Operand;

// ── NEON/SIMD ────────────────────────────────────────────────────────────

/// Helper to extract register number from a RegArrangement operand
pub(crate) fn get_neon_reg(operands: &[Operand], idx: usize) -> Result<(u32, String), String> {
    match operands.get(idx) {
        Some(Operand::RegArrangement { reg, arrangement }) => {
            let num =
                parse_reg_num(reg).ok_or_else(|| format!("invalid NEON register: {}", reg))?;
            Ok((num, arrangement.clone()))
        }
        Some(Operand::Reg(name)) => {
            let num = parse_reg_num(name).ok_or_else(|| format!("invalid register: {}", name))?;
            Ok((num, String::new()))
        }
        other => Err(format!(
            "expected NEON register at operand {}, got {:?}",
            idx, other
        )),
    }
}

pub(crate) fn encode_cnt(operands: &[Operand]) -> Result<EncodeResult, String> {
    // CNT Vd.<T>, Vn.<T>
    // Encoding: 0 Q 00 1110 size 10 0000 0101 10 Rn Rd
    // Only valid for .8b (Q=0) and .16b (Q=1)
    if operands.len() < 2 {
        return Err("cnt requires 2 operands".to_string());
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _arr_n) = get_neon_reg(operands, 1)?;

    let q: u32 = if arr_d == "16b" { 1 } else { 0 }; // .8b -> Q=0, .16b -> Q=1

    // 0 Q 00 1110 00 10 0000 0101 10 Rn Rd
    let word =
        ((q << 30) | (0b001110 << 24)) | (0b100000 << 16) | (0b010110 << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

// ── NEON three-same register operations ──────────────────────────────────

/// Get Q bit and size from arrangement specifier.
pub(crate) fn neon_arr_to_q_size(arr: &str) -> Result<(u32, u32), String> {
    match arr {
        "8b" => Ok((0, 0b00)),
        "16b" => Ok((1, 0b00)),
        "4h" => Ok((0, 0b01)),
        "8h" => Ok((1, 0b01)),
        "2s" => Ok((0, 0b10)),
        "4s" => Ok((1, 0b10)),
        "1d" => Ok((0, 0b11)),
        "2d" => Ok((1, 0b11)),
        _ => Err(format!("unsupported NEON arrangement: {}", arr)),
    }
}

/// Encode NEON three-same-register instructions: CMEQ, UQSUB, SQSUB, CMHI, etc.
///
/// Layout: 0 Q U 01110 size 1 Rm opcode 1 Rn Rd
///         31 30 29 28-24 23-22 21 20-16 15-11 10 9-5 4-0
///
/// `u_bit`: U field (bit 29) - 0 for signed, 1 for unsigned
/// `opcode`: instruction opcode (bits 15-11)
pub(crate) fn encode_neon_three_same(
    operands: &[Operand],
    u_bit: u32,
    opcode: u32,
) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("NEON three-same requires 3 operands".to_string());
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _arr_n) = get_neon_reg(operands, 1)?;
    let (rm, _arr_m) = get_neon_reg(operands, 2)?;

    let (q, size) = neon_arr_to_q_size(&arr_d)?;

    // 0 Q U 01110 size 1 Rm opcode 1 Rn Rd
    let word = (q << 30)
        | (u_bit << 29)
        | (0b01110 << 24)
        | (size << 22)
        | (1 << 21)
        | (rm << 16)
        | (opcode << 11)
        | (1 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON three-different instructions: USUBL, SSUBL, UADDL, SADDL, etc.
///
/// These instructions have wider destination than source operands.
/// Format: 0 Q U 01110 size 1 Rm opcode 00 Rn Rd
///
/// `u_bit`: 0 for signed, 1 for unsigned
/// `opcode`: 4-bit opcode (bits 15-12)
/// `is_high`: true for the "2" variant (upper half, Q=1)
pub(crate) fn encode_neon_three_diff(
    operands: &[Operand],
    u_bit: u32,
    opcode: u32,
    is_high: bool,
) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("NEON three-different requires 3 operands".to_string());
    }
    let (rd, _arr_d) = get_neon_reg(operands, 0)?;
    let (rn, arr_n) = get_neon_reg(operands, 1)?;
    let (rm, _arr_m) = get_neon_reg(operands, 2)?;

    // Size is determined from the source (narrow) arrangement
    let (q, size) = match arr_n.as_str() {
        "8b" => (0u32, 0b00u32), // base
        "16b" => (1, 0b00),      // "2" variant
        "4h" => (0, 0b01),
        "8h" => (1, 0b01),
        "2s" => (0, 0b10),
        "4s" => (1, 0b10),
        _ => {
            return Err(format!(
                "unsupported source arrangement for three-diff: {}",
                arr_n
            ))
        }
    };

    // For the "2" variant, override Q
    let q = if is_high { 1 } else { q };

    // 0 Q U 01110 size 1 Rm opcode 00 Rn Rd
    let word = (q << 30)
        | (u_bit << 29)
        | (0b01110 << 24)
        | (size << 22)
        | (1 << 21)
        | (rm << 16)
        | (opcode << 12)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON SQSHRUN/SQSHRUN2: Signed saturating shift right unsigned narrow
/// Format: 0 Q 1 011110 immh immb 100011 Rn Rd
pub(crate) fn encode_neon_sqshrun(
    operands: &[Operand],
    is_rounding: bool,
    is_high: bool,
) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("sqshrun requires 3 operands".to_string());
    }
    let (rd, _arr_d) = get_neon_reg(operands, 0)?;
    let (rn, arr_n) = get_neon_reg(operands, 1)?;
    let shift = match &operands[2] {
        Operand::Imm(v) => *v as u32,
        _ => return Err("sqshrun: expected immediate shift".to_string()),
    };

    // immh:immb encode element size and shift amount
    // For source .4s (dest .4h or .8h): immh=001x, shift_amount = 32 - (immh:immb)
    // For source .8h (dest .8b or .16b): immh=0001, shift_amount = 16 - (immh:immb)
    // For source .2d (dest .2s or .4s): immh=01xx, shift_amount = 64 - (immh:immb)
    let (element_bits, immh_base) = match arr_n.as_str() {
        "8h" => (16u32, 0b0001u32),
        "4s" => (32, 0b0010),
        "2d" => (64, 0b0100),
        _ => {
            return Err(format!(
                "sqshrun: unsupported source arrangement: {}",
                arr_n
            ))
        }
    };

    if shift == 0 || shift > element_bits {
        return Err(format!(
            "sqshrun: shift {} out of range for {}-bit elements",
            shift, element_bits
        ));
    }

    let immhb = (element_bits - shift) & 0x7F; // immh:immb combined
    let immh = (immhb >> 3) | immh_base;
    let immb = immhb & 0x7;

    let q = if is_high { 1u32 } else { 0 };

    // 0 Q 1 011110 immh immb opcode 1 Rn Rd
    // SQSHRUN: opcode = 100001, SQRSHRUN: opcode = 100011
    let opcode_bits: u32 = if is_rounding { 0b100011 } else { 0b100001 };
    let word = (q << 30)
        | (1 << 29)
        | (0b011110 << 23)
        | (immh << 19)
        | (immb << 16)
        | (opcode_bits << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON UXTL/SXTL (unsigned/signed extend long).
/// These are aliases for USHLL/SSHLL with shift #0.
///
/// Format: 0 Q U 011110 immh immb 10100 1 Rn Rd
pub(crate) fn encode_neon_xtl(
    operands: &[Operand],
    u_bit: u32,
    is_high: bool,
) -> Result<EncodeResult, String> {
    if operands.len() < 2 {
        return Err("NEON uxtl/sxtl requires 2 operands".to_string());
    }
    let (rd, _arr_d) = get_neon_reg(operands, 0)?;
    let (rn, arr_n) = get_neon_reg(operands, 1)?;

    // immh encodes the source element size, immb=0 (shift=0)
    let immh = match arr_n.as_str() {
        "8b" | "16b" => 0b0001u32,
        "4h" | "8h" => 0b0010,
        "2s" | "4s" => 0b0100,
        _ => {
            return Err(format!(
                "uxtl/sxtl: unsupported source arrangement: {}",
                arr_n
            ))
        }
    };

    let q = if is_high { 1u32 } else { 0 };

    // 0 Q U 011110 immh immb 10100 1 Rn Rd
    let word = (q << 30)
        | (u_bit << 29)
        | (0b011110 << 23)
        | (immh << 19)
        | (0b101001 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON compare-to-zero: CMEQ Vd, Vn, #0, CMGE Vd, Vn, #0, etc.
///
/// Format: 0 Q U 01110 size 10000 opcode 10 Rn Rd
pub(crate) fn encode_neon_cmp_zero(
    operands: &[Operand],
    u_bit: u32,
    opcode: u32,
) -> Result<EncodeResult, String> {
    if operands.len() < 2 {
        return Err("NEON compare-zero requires at least 2 operands".to_string());
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let (q, size) = neon_arr_to_q_size(&arr_d)?;

    // 0 Q U 01110 size 10000 opcode 10 Rn Rd
    let word = (q << 30)
        | (u_bit << 29)
        | (0b01110 << 24)
        | (size << 22)
        | (0b10000 << 17)
        | (opcode << 12)
        | (0b10 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON two-register miscellaneous narrowing: UQXTN, SQXTN, XTN
///
/// Format: 0 Q U 01110 size 10000 opcode 10 Rn Rd
pub(crate) fn encode_neon_two_misc_narrow(
    operands: &[Operand],
    u_bit: u32,
    opcode: u32,
    is_high: bool,
) -> Result<EncodeResult, String> {
    if operands.len() < 2 {
        return Err("NEON two-reg narrow requires 2 operands".to_string());
    }
    let (rd, _arr_d) = get_neon_reg(operands, 0)?;
    let (rn, arr_n) = get_neon_reg(operands, 1)?;

    // Size from source (wider) arrangement
    let size = match arr_n.as_str() {
        "8h" => 0b00u32,
        "4s" => 0b01,
        "2d" => 0b10,
        _ => {
            return Err(format!(
                "unsupported source arrangement for narrow: {}",
                arr_n
            ))
        }
    };

    let q = if is_high { 1u32 } else { 0 };

    // 0 Q U 01110 size 10000 opcode 10 Rn Rd
    let word = (q << 30)
        | (u_bit << 29)
        | (0b01110 << 24)
        | (size << 22)
        | (0b10000 << 17)
        | (opcode << 12)
        | (0b10 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON vector-by-element long instructions: SMULL/UMULL/SMLAL/UMLAL/SMLSL/UMLSL (elem)
///
/// Format: 0 Q U 01111 size L M Rm opcode H 0 Rn Rd
///
/// These are the widening multiply-by-element forms where the third operand
/// is a register lane (e.g., v0.h[2]).
pub(crate) fn encode_neon_elem_long(
    operands: &[Operand],
    u_bit: u32,
    opcode: u32,
    is_high: bool,
) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("NEON elem-long requires 3 operands".to_string());
    }
    let (rd, _arr_d) = get_neon_reg(operands, 0)?;
    let (rn, arr_n) = get_neon_reg(operands, 1)?;

    // Third operand is RegLane: v0.h[2]
    let (rm, index) = match &operands[2] {
        Operand::RegLane {
            reg,
            elem_size: _,
            index,
        } => {
            let rm = parse_reg_num(reg).ok_or("invalid NEON register")?;
            (rm, *index)
        }
        _ => {
            return Err(format!(
                "expected register lane operand, got {:?}",
                operands[2]
            ))
        }
    };

    // Determine size and Q from source arrangement
    let (q, size) = match arr_n.as_str() {
        "4h" => (0u32, 0b01u32),
        "8h" => (1, 0b01),
        "2s" => (0, 0b10),
        "4s" => (1, 0b10),
        _ => {
            return Err(format!(
                "unsupported source arrangement for elem-long: {}",
                arr_n
            ))
        }
    };
    let q = if is_high { 1 } else { q };

    // Encode index into H:L:M bits depending on element size
    let (h, l, m) = match size {
        0b01 => {
            // Half-word: index = H:L:M (3 bits), Rm limited to v0-v15
            if index > 7 {
                return Err(format!("element index {} out of range for .h", index));
            }
            let h = (index >> 2) & 1;
            let l = (index >> 1) & 1;
            let m = index & 1;
            (h, l, m)
        }
        0b10 => {
            // Word: index = H:L (2 bits), M=Rm[4]
            if index > 3 {
                return Err(format!("element index {} out of range for .s", index));
            }
            let h = (index >> 1) & 1;
            let l = index & 1;
            let m = (rm >> 4) & 1; // M bit from Rm[4]
            (h, l, m)
        }
        _ => return Err("unsupported element size for by-element".to_string()),
    };

    // Limit Rm for half-word indexing (only v0-v15)
    let rm_enc = if size == 0b01 { rm & 0xF } else { rm & 0x1F };

    // 0 Q U 01111 size L M Rm opcode H 0 Rn Rd
    let word = (q << 30)
        | (u_bit << 29)
        | (0b01111 << 24)
        | (size << 22)
        | (l << 21)
        | (m << 20)
        | (rm_enc << 16)
        | (opcode << 12)
        | (h << 11)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON logical operations: ORR/AND/EOR Vd.T, Vn.T, Vm.T
pub(crate) fn encode_neon_logical(operands: &[Operand], opc: u32) -> Result<EncodeResult, String> {
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _arr_n) = get_neon_reg(operands, 1)?;
    let (rm, _arr_m) = get_neon_reg(operands, 2)?;

    let q: u32 = if arr_d == "16b" { 1 } else { 0 };

    // NEON logical three-same:
    // ORR: 0 Q 0 01110 10 1 Rm 000111 Rn Rd  (opc=0b01 -> size=10)
    // AND: 0 Q 0 01110 00 1 Rm 000111 Rn Rd  (opc=0b00 -> size=00)
    // EOR: 0 Q 1 01110 00 1 Rm 000111 Rn Rd  (opc=0b10 -> size=00, U=1)
    // BIC: 0 Q 0 01110 01 1 Rm 000111 Rn Rd  (would be opc=0b01 with N=1... but not needed)
    let (u_bit, size_bits): (u32, u32) = match opc {
        0b00 => (0, 0b00), // AND
        0b01 => (0, 0b10), // ORR
        0b10 => (1, 0b00), // EOR
        0b11 => (1, 0b00), // ANDS - not valid for NEON, fall back
        _ => return Err("unsupported NEON logical opc".to_string()),
    };

    let word = (q << 30)
        | (u_bit << 29)
        | (0b01110 << 24)
        | (size_bits << 22)
        | (1 << 21)
        | (rm << 16)
        | (0b000111 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON MUL Vd.T, Vn.T, Vm.T
pub(crate) fn encode_neon_mul(operands: &[Operand]) -> Result<EncodeResult, String> {
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let (rm, _) = get_neon_reg(operands, 2)?;
    let (q, size) = neon_arr_to_q_size(&arr_d)?;

    // MUL (vector): 0 Q 0 01110 size 1 Rm 10011 1 Rn Rd
    let word = (q << 30)
        | (0b001110 << 24)
        | (size << 22)
        | (1 << 21)
        | (rm << 16)
        | (0b100111 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON PMUL Vd.T, Vn.T, Vm.T (polynomial multiply, bytes only)
pub(crate) fn encode_neon_pmul(operands: &[Operand]) -> Result<EncodeResult, String> {
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let (rm, _) = get_neon_reg(operands, 2)?;
    let q: u32 = if arr_d == "16b" { 1 } else { 0 };
    // PMUL: 0 Q 1 01110 00 1 Rm 10011 1 Rn Rd (size=00 for bytes, U=1)
    // PMUL encoding: size=00 (bytes) is implicit (zero bits at [23:22])
    let word = (q << 30)
        | (1 << 29)
        | (0b01110 << 24)
        | (1 << 21)
        | (rm << 16)
        | (0b100111 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON MLA Vd.T, Vn.T, Vm.T (multiply-accumulate)
pub(crate) fn encode_neon_mla(operands: &[Operand]) -> Result<EncodeResult, String> {
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let (rm, _) = get_neon_reg(operands, 2)?;
    let (q, size) = neon_arr_to_q_size(&arr_d)?;
    // MLA: 0 Q 0 01110 size 1 Rm 10010 1 Rn Rd
    let word = (q << 30)
        | (0b001110 << 24)
        | (size << 22)
        | (1 << 21)
        | (rm << 16)
        | (0b100101 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON MLS Vd.T, Vn.T, Vm.T (multiply-subtract)
pub(crate) fn encode_neon_mls(operands: &[Operand]) -> Result<EncodeResult, String> {
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let (rm, _) = get_neon_reg(operands, 2)?;
    let (q, size) = neon_arr_to_q_size(&arr_d)?;
    // MLS: 0 Q 1 01110 size 1 Rm 10010 1 Rn Rd (U=1)
    let word = (q << 30)
        | (1 << 29)
        | (0b01110 << 24)
        | (size << 22)
        | (1 << 21)
        | (rm << 16)
        | (0b100101 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON USHR Vd.T, Vn.T, #shift (unsigned shift right immediate)
pub(crate) fn encode_neon_shift_imm(
    operands: &[Operand],
    _is_unsigned: bool,
) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("ushr requires 3 operands".to_string());
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let shift = get_imm(operands, 2)?;

    let (q, _size) = neon_arr_to_q_size(&arr_d)?;

    // USHR: 0 Q 1 011110 immh:immb 00000 1 Rn Rd
    // For .16b (bytes, size=8): immh = 0001, immb = 8-shift (3 bits)
    // For .8h (halfwords, size=16): immh = 001x
    // For .4s (words, size=32): immh = 01xx
    // For .2d (doublewords, size=64): immh = 1xxx
    // immh:immb = (element_size * 2 - shift)
    let (elem_bits, immh_immb) = match arr_d.as_str() {
        "8b" | "16b" => (8u32, (16 - shift as u32) & 0xF), // immh:immb is 4 bits for 8-bit elems
        "4h" | "8h" => (16, (32 - shift as u32) & 0x1F),
        "2s" | "4s" => (32, (64 - shift as u32) & 0x3F),
        "2d" => (64, (128 - shift as u32) & 0x7F),
        _ => return Err(format!("unsupported USHR arrangement: {}", arr_d)),
    };
    let _ = elem_bits;

    // Full encoding: 0 Q 1 011110 immh:immb 000001 Rn Rd
    let word = (q << 30)
        | (1 << 29)
        | (0b011110 << 23)
        | (immh_immb << 16)
        | (0b000001 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON EXT Vd.T, Vn.T, Vm.T, #index
pub(crate) fn encode_neon_ext(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 4 {
        return Err("ext requires 4 operands".to_string());
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let (rm, _) = get_neon_reg(operands, 2)?;
    let index = get_imm(operands, 3)? as u32;

    let q: u32 = if arr_d == "16b" { 1 } else { 0 };

    // EXT Vd.T, Vn.T, Vm.T, #index
    // Encoding: 0 Q 10 1110 00 0 Rm 0 imm4 0 Rn Rd
    let word =
        ((((q << 30) | (0b101110 << 24)) | (rm << 16)) | ((index & 0xF) << 11)) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON ADDV: add across vector lanes
pub(crate) fn encode_neon_addv(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 2 {
        return Err("addv requires 2 operands".to_string());
    }
    let (rd, _) = get_neon_reg(operands, 0)?;
    let (rn, arr_n) = get_neon_reg(operands, 1)?;

    let (q, size) = neon_arr_to_q_size(&arr_n)?;

    // ADDV: 0 Q 0 01110 size 11000 11011 10 Rn Rd
    let word = (q << 30)
        | (0b001110 << 24)
        | (size << 22)
        | (0b11000 << 17)
        | (0b110111 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON across-vector instructions: UMAXV, UMINV, SMAXV, SMINV
///
/// Format: 0 Q U 01110 size 11000 opcode 10 Rn Rd
///
/// `u_bit`: 0 for signed, 1 for unsigned
/// `opcode`: 5-bit opcode (bits 16-12)
pub(crate) fn encode_neon_across(
    operands: &[Operand],
    u_bit: u32,
    opcode: u32,
) -> Result<EncodeResult, String> {
    if operands.len() < 2 {
        return Err("NEON across-vector requires 2 operands".to_string());
    }
    let (rd, _) = get_neon_reg(operands, 0)?;
    let (rn, arr_n) = get_neon_reg(operands, 1)?;

    let (q, size) = neon_arr_to_q_size(&arr_n)?;

    // 0 Q U 01110 size 11000 opcode 10 Rn Rd
    let word = (q << 30)
        | (u_bit << 29)
        | (0b01110 << 24)
        | (size << 22)
        | (0b11000 << 17)
        | (opcode << 12)
        | (0b10 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON UMOV: move element to GP register
pub(crate) fn encode_neon_umov(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 2 {
        return Err("umov requires 2 operands".to_string());
    }
    let (rd, is_64) = get_reg(operands, 0)?;

    // Second operand should be a RegLane (v0.b[0])
    match operands.get(1) {
        Some(Operand::RegLane {
            reg,
            elem_size,
            index,
        }) => {
            let rn = parse_reg_num(reg).ok_or("invalid NEON register")?;
            let q = if is_64 { 1u32 } else { 0 };

            let imm5 = match elem_size.as_str() {
                "b" => ((*index & 0xF) << 1) | 0b00001,
                "h" => ((*index & 0x7) << 2) | 0b00010,
                "s" => ((*index & 0x3) << 3) | 0b00100,
                "d" => ((*index & 0x1) << 4) | 0b01000,
                _ => return Err(format!("unsupported umov element size: {}", elem_size)),
            };

            // UMOV Rd, Vn.Ts[index]: 0 Q 0 01110 000 imm5 0 0111 1 Rn Rd
            let word = (q << 30)
                | (0b001110000u32 << 21)
                | (imm5 << 16)
                | (0b001111 << 10)
                | (rn << 5)
                | rd;
            Ok(EncodeResult::Word(word))
        }
        _ => Err("umov: expected register lane operand".to_string()),
    }
}

/// Encode NEON DUP: broadcast GP register to all vector lanes
pub(crate) fn encode_neon_dup(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 2 {
        return Err("dup requires 2 operands".to_string());
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;

    // DUP Vd.T, Rn (general form - broadcast GP reg to vector)
    if let Some(Operand::Reg(rn_name)) = operands.get(1) {
        let rn = parse_reg_num(rn_name).ok_or("invalid rn")?;
        let (q, _) = neon_arr_to_q_size(&arr_d)?;

        // imm5 encoding for element size:
        // .8b/.16b: imm5 = 00001
        // .4h/.8h:  imm5 = 00010
        // .2s/.4s:  imm5 = 00100
        // .2d:      imm5 = 01000
        let imm5 = match arr_d.as_str() {
            "8b" | "16b" => 0b00001u32,
            "4h" | "8h" => 0b00010,
            "2s" | "4s" => 0b00100,
            "2d" => 0b01000,
            _ => return Err(format!("unsupported dup arrangement: {}", arr_d)),
        };

        // DUP Vd.T, Rn: 0 Q 0 01110 000 imm5 0 0001 1 Rn Rd
        let word =
            (q << 30) | (0b001110000u32 << 21) | (imm5 << 16) | (0b000011 << 10) | (rn << 5) | rd;
        return Ok(EncodeResult::Word(word));
    }

    // DUP Vd.T, Vn.Ts[index] (broadcast element to all lanes)
    if let Some(Operand::RegLane {
        reg,
        elem_size,
        index,
    }) = operands.get(1)
    {
        let rn = parse_reg_num(reg).ok_or("invalid NEON register")?;
        let (q, _) = neon_arr_to_q_size(&arr_d)?;

        // imm5 encodes both element size and index:
        // .b[i]: imm5 = (i << 1) | 0b00001
        // .h[i]: imm5 = (i << 2) | 0b00010
        // .s[i]: imm5 = (i << 3) | 0b00100
        // .d[i]: imm5 = (i << 4) | 0b01000
        let imm5 = match elem_size.as_str() {
            "b" => ((*index & 0xF) << 1) | 0b00001,
            "h" => ((*index & 0x7) << 2) | 0b00010,
            "s" => ((*index & 0x3) << 3) | 0b00100,
            "d" => ((*index & 0x1) << 4) | 0b01000,
            _ => return Err(format!("unsupported dup element size: {}", elem_size)),
        };

        // DUP Vd.T, Vn.Ts[i]: 0 Q 0 01110 000 imm5 0 0000 1 Rn Rd
        let word =
            (q << 30) | (0b001110000u32 << 21) | (imm5 << 16) | (0b000001 << 10) | (rn << 5) | rd;
        return Ok(EncodeResult::Word(word));
    }

    Err("unsupported dup operands".to_string())
}

/// Encode NEON INS (insert element from GP register): INS Vd.Ts[index], Xn
pub(crate) fn encode_neon_ins(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 2 {
        return Err("ins requires 2 operands".to_string());
    }
    match (&operands[0], &operands[1]) {
        // INS Vd.Ts[dst_idx], Xn (general register to element)
        (
            Operand::RegLane {
                reg,
                elem_size,
                index,
            },
            Operand::Reg(rn_name),
        ) => {
            let rd = parse_reg_num(reg).ok_or("invalid NEON register")?;
            let rn = parse_reg_num(rn_name).ok_or("invalid register")?;

            let imm5 = match elem_size.as_str() {
                "b" => ((*index & 0xF) << 1) | 0b00001,
                "h" => ((*index & 0x7) << 2) | 0b00010,
                "s" => ((*index & 0x3) << 3) | 0b00100,
                "d" => ((*index & 0x1) << 4) | 0b01000,
                _ => return Err(format!("unsupported ins element size: {}", elem_size)),
            };

            // INS Vd.Ts[i], Xn: 0 1 0 01110 000 imm5 0 0011 1 Rn Rd
            let word = (0b01001110000u32 << 21) | (imm5 << 16) | (0b000111 << 10) | (rn << 5) | rd;
            Ok(EncodeResult::Word(word))
        }
        // INS Vd.Ts[dst_idx], Vn.Ts[src_idx] (element to element)
        (
            Operand::RegLane {
                reg: rd_name,
                elem_size: dst_size,
                index: dst_idx,
            },
            Operand::RegLane {
                reg: rn_name,
                elem_size: _src_size,
                index: src_idx,
            },
        ) => {
            let rd = parse_reg_num(rd_name).ok_or("invalid NEON rd")?;
            let rn = parse_reg_num(rn_name).ok_or("invalid NEON rn")?;

            let (imm5, imm4) = match dst_size.as_str() {
                "b" => (((*dst_idx & 0xF) << 1) | 0b00001, *src_idx & 0xF),
                "h" => (((*dst_idx & 0x7) << 2) | 0b00010, (*src_idx & 0x7) << 1),
                "s" => (((*dst_idx & 0x3) << 3) | 0b00100, (*src_idx & 0x3) << 2),
                "d" => (((*dst_idx & 0x1) << 4) | 0b01000, (*src_idx & 0x1) << 3),
                _ => return Err(format!("unsupported ins element size: {}", dst_size)),
            };

            // INS Vd.Ts[dst], Vn.Ts[src]: 0 1 1 01110 000 imm5 0 imm4 1 Rn Rd
            let word =
                (0b01101110000u32 << 21) | (imm5 << 16) | (imm4 << 11) | (1 << 10) | (rn << 5) | rd;
            Ok(EncodeResult::Word(word))
        }
        _ => Err("ins: expected (RegLane, Reg) or (RegLane, RegLane) operands".to_string()),
    }
}

/// Encode NEON NOT (bitwise NOT): NOT Vd.T, Vn.T
pub(crate) fn encode_neon_not(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 2 {
        return Err("not requires 2 operands".to_string());
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;

    let q: u32 = if arr_d == "16b" { 1 } else { 0 };

    // NOT Vd.T, Vn.T (alias of MVN): 0 Q 1 01110 00 10000 00101 10 Rn Rd
    let word = ((q << 30) | (1 << 29) | (0b01110 << 24))
        | (0b10000 << 17)
        | (0b00101 << 12)
        | (0b10 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON MOVI (move immediate to vector)
pub(crate) fn encode_neon_movi(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 2 {
        return Err("movi requires 2 operands".to_string());
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let imm = get_imm(operands, 1)?;

    match arr_d.as_str() {
        "16b" | "8b" => {
            // MOVI Vd.16b, #imm8
            // Encoding: 0 Q 00 1111 00000 abc 1110 01 defgh Rd
            // where imm8 = abc:defgh
            let q: u32 = if arr_d == "16b" { 1 } else { 0 };
            let imm8 = imm as u32 & 0xFF;
            let abc = (imm8 >> 5) & 0x7;
            let defgh = imm8 & 0x1F;
            // 0 Q op 0 1111 0 a b c cmode(1110) o2(0) 1 defgh Rd
            let word = (q << 30)
                | (0b0011110 << 23)
                | ((abc >> 2) << 18)
                | (((abc >> 1) & 1) << 17)
                | ((abc & 1) << 16)
                | (0b1110 << 12)
                | (0b01 << 10)
                | (defgh << 5)
                | rd;
            Ok(EncodeResult::Word(word))
        }
        "2d" => {
            // MOVI Vd.2d, #imm
            // The 64-bit immediate is encoded as 8 bits, where each bit expands
            // to 8 bits (0x00 or 0xFF) in the result.
            // Convert the 64-bit value to the 8-bit encoding.
            let imm64 = imm as u64;
            let mut imm8 = 0u32;
            for i in 0..8 {
                let byte_val = (imm64 >> (i * 8)) & 0xFF;
                if byte_val == 0xFF {
                    imm8 |= 1 << i;
                } else if byte_val != 0 {
                    return Err(format!("movi .2d: each byte of immediate must be 0x00 or 0xFF, got 0x{:02x} at byte {}", byte_val, i));
                }
            }
            let abc = (imm8 >> 5) & 0x7;
            let defgh = imm8 & 0x1F;
            // MOVI Vd.2d, #imm: 0 1 1 0 1111 00 abc 1110 01 defgh Rd  (op=1, Q=1)
            let word = (0b01101111 << 24)
                | ((abc >> 2) << 18)
                | (((abc >> 1) & 1) << 17)
                | ((abc & 1) << 16)
                | (0b111001 << 10)
                | (defgh << 5)
                | rd;
            Ok(EncodeResult::Word(word))
        }
        "2s" | "4s" => {
            // MOVI Vd.2s/4s, #imm8 (32-bit element, no shift)
            // Encoding: 0 Q op(0) 0 1111 0 abc cmode(0000) o2(0) 1 defgh Rd
            let q: u32 = if arr_d == "4s" { 1 } else { 0 };
            let imm8 = imm as u32 & 0xFF;
            let abc = (imm8 >> 5) & 0x7;
            let defgh = imm8 & 0x1F;

            // Check for optional LSL shift operand
            let (cmode, shift_val) = if operands.len() > 2 {
                if let Some(Operand::Shift { kind, amount }) = operands.get(2) {
                    if kind == "lsl" {
                        match amount {
                            0 => (0b0000u32, 0),
                            8 => (0b0010, 8),
                            16 => (0b0100, 16),
                            24 => (0b0110, 24),
                            _ => return Err(format!("movi: unsupported shift amount: {}", amount)),
                        }
                    } else {
                        (0b0000, 0)
                    }
                } else {
                    (0b0000, 0)
                }
            } else {
                (0b0000, 0)
            };
            let _ = shift_val;

            let word = (q << 30)
                | (0b0011110 << 23)
                | ((abc >> 2) << 18)
                | (((abc >> 1) & 1) << 17)
                | ((abc & 1) << 16)
                | (cmode << 12)
                | (0b01 << 10)
                | (defgh << 5)
                | rd;
            Ok(EncodeResult::Word(word))
        }
        "4h" | "8h" => {
            // MOVI Vd.4h/8h, #imm8
            let q: u32 = if arr_d == "8h" { 1 } else { 0 };
            let imm8 = imm as u32 & 0xFF;
            let abc = (imm8 >> 5) & 0x7;
            let defgh = imm8 & 0x1F;
            // cmode=1000 for .4h/.8h with no shift
            let word = (q << 30)
                | (0b0011110 << 23)
                | ((abc >> 2) << 18)
                | (((abc >> 1) & 1) << 17)
                | ((abc & 1) << 16)
                | (0b1000 << 12)
                | (0b01 << 10)
                | (defgh << 5)
                | rd;
            Ok(EncodeResult::Word(word))
        }
        _ => Err(format!("movi: unsupported arrangement: {}", arr_d)),
    }
}

/// Encode NEON BIC (bitwise clear vector): BIC Vd.T, Vn.T, Vm.T
pub(crate) fn encode_neon_bic(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("bic requires 3 operands".to_string());
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let (rm, _) = get_neon_reg(operands, 2)?;

    let q: u32 = if arr_d == "16b" { 1 } else { 0 };

    // BIC Vd.T, Vn.T, Vm.T: 0 Q 0 01110 01 1 Rm 000111 Rn Rd
    let word = (q << 30)
        | (0b001110 << 24)
        | (0b01 << 22)
        | (1 << 21)
        | (rm << 16)
        | (0b000111 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON BSL (bitwise select): BSL Vd.T, Vn.T, Vm.T
pub(crate) fn encode_neon_bsl(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("bsl requires 3 operands".to_string());
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let (rm, _) = get_neon_reg(operands, 2)?;

    let q: u32 = if arr_d == "16b" { 1 } else { 0 };

    // BSL Vd.T, Vn.T, Vm.T: 0 Q 1 01110 01 1 Rm 000111 Rn Rd
    let word = (q << 30)
        | (1 << 29)
        | (0b01110 << 24)
        | (0b01 << 22)
        | (1 << 21)
        | (rm << 16)
        | (0b000111 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON REV64: reverse elements within 64-bit doublewords
pub(crate) fn encode_neon_rev64(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 2 {
        return Err("rev64 requires 2 operands".to_string());
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;

    let (q, size) = neon_arr_to_q_size(&arr_d)?;

    // REV64 Vd.T, Vn.T: 0 Q 0 01110 size 10 0000 0000 10 Rn Rd
    let word = (q << 30)
        | (0b001110 << 24)
        | (size << 22)
        | (0b100000 << 16)
        | (0b000010 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON TBL: table vector lookup
pub(crate) fn encode_neon_tbl(operands: &[Operand]) -> Result<EncodeResult, String> {
    // TBL Vd.T, {Vn.T}, Vm.T  (single register table)
    // TBL Vd.T, {Vn.T, Vn+1.T}, Vm.T  (two register table)
    // etc.
    if operands.len() < 3 {
        return Err("tbl requires 3 operands".to_string());
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let q: u32 = if arr_d == "16b" { 1 } else { 0 };

    // The second operand is a register list
    let (rn, num_regs) = match &operands[1] {
        Operand::RegList(regs) => {
            let first_reg = match &regs[0] {
                Operand::RegArrangement { reg, .. } => parse_reg_num(reg).ok_or("invalid reg")?,
                Operand::Reg(name) => parse_reg_num(name).ok_or("invalid reg")?,
                _ => return Err("tbl: expected register in list".to_string()),
            };
            (first_reg, regs.len() as u32)
        }
        _ => return Err("tbl: expected register list as second operand".to_string()),
    };

    let (rm, _) = get_neon_reg(operands, 2)?;

    // len field: 1 reg -> 00, 2 -> 01, 3 -> 10, 4 -> 11
    let len = (num_regs - 1) & 0x3;

    // TBL: 0 Q 00 1110 000 Rm 0 len 0 00 Rn Rd
    let word = ((((q << 30) | (0b001110 << 24)) | (rm << 16)) | (len << 13)) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON TBX: table vector lookup with insert (preserves out-of-range lanes)
pub(crate) fn encode_neon_tbx(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("tbx requires 3 operands".to_string());
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let q: u32 = if arr_d == "16b" { 1 } else { 0 };

    let (rn, num_regs) = match &operands[1] {
        Operand::RegList(regs) => {
            let first_reg = match &regs[0] {
                Operand::RegArrangement { reg, .. } => parse_reg_num(reg).ok_or("invalid reg")?,
                Operand::Reg(name) => parse_reg_num(name).ok_or("invalid reg")?,
                _ => return Err("tbx: expected register in list".to_string()),
            };
            (first_reg, regs.len() as u32)
        }
        _ => return Err("tbx: expected register list as second operand".to_string()),
    };

    let (rm, _) = get_neon_reg(operands, 2)?;
    let len = (num_regs - 1) & 0x3;

    // TBX: 0 Q 00 1110 000 Rm 0 len 1 00 Rn Rd (op=1 for TBX vs op=0 for TBL)
    let word = (q << 30) | (0b001110 << 24) | (rm << 16) | (len << 13) | (1 << 12) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON LD1R: load single structure and replicate to all lanes
pub(crate) fn encode_neon_ld1r(operands: &[Operand]) -> Result<EncodeResult, String> {
    // LD1R {Vt.T}, [Xn]
    if operands.len() < 2 {
        return Err("ld1r requires 2 operands".to_string());
    }

    let (rt, arr) = match &operands[0] {
        Operand::RegList(regs) => {
            if regs.len() != 1 {
                return Err("ld1r expects exactly one register in list".to_string());
            }
            match &regs[0] {
                Operand::RegArrangement { reg, arrangement } => {
                    let num = parse_reg_num(reg).ok_or("invalid reg")?;
                    (num, arrangement.clone())
                }
                _ => return Err("ld1r: expected register with arrangement".to_string()),
            }
        }
        _ => return Err("ld1r: expected register list as first operand".to_string()),
    };

    let (q, size) = match arr.as_str() {
        "8b" => (0u32, 0b00u32),
        "16b" => (1, 0b00),
        "4h" => (0, 0b01),
        "8h" => (1, 0b01),
        "2s" => (0, 0b10),
        "4s" => (1, 0b10),
        "1d" => (0, 0b11),
        "2d" => (1, 0b11),
        _ => return Err(format!("ld1r: unsupported arrangement: {}", arr)),
    };

    match &operands[1] {
        Operand::Mem { base, offset: 0 } => {
            let rn = parse_reg_num(base).ok_or("invalid base reg")?;
            // LD1R: 0 Q 0 01101 0 1 0 00000 110 0 size Rn Rt (no post-index)
            let word = (q << 30)
                | (0b001101 << 24)
                | (1 << 22)
                | (0b110 << 13)
                | (size << 10)
                | (rn << 5)
                | rt;
            Ok(EncodeResult::Word(word))
        }
        Operand::MemPostIndex { base, offset } => {
            let rn = parse_reg_num(base).ok_or("invalid base reg")?;
            // LD1R post-index (immediate): 0 Q 0 01101 1 1 0 11111 110 0 size Rn Rt
            // Rm=11111 means post-index by element size
            let _ = offset; // offset must match element size, not encoded separately
            let word = (q << 30)
                | (0b001101 << 24)
                | (1 << 23)
                | (1 << 22)
                | (0b11111 << 16)
                | (0b110 << 13)
                | (size << 10)
                | (rn << 5)
                | rt;
            Ok(EncodeResult::Word(word))
        }
        _ => Err("ld1r: expected [Xn] or [Xn], #imm memory operand".to_string()),
    }
}

/// Encode NEON LD1 (vector load, multiple structures)
/// Dispatch LD/ST1-4: choose between "multiple structures" and "single structure (element)" encoding.
pub(crate) fn encode_neon_ld_st_dispatch(
    operands: &[Operand],
    is_load: bool,
    num_structs: u32,
) -> Result<EncodeResult, String> {
    // If the first operand is a RegListIndexed, use single-element encoding
    if let Some(Operand::RegListIndexed { .. }) = operands.first() {
        return encode_neon_ld_st_single(operands, is_load, num_structs);
    }
    // Multiple-structures encoding for ld1-4/st1-4
    encode_neon_ld_st_multi(operands, is_load, num_structs)
}

/// Encode NEON LD/ST single structure (element):
/// st1 {v0.s}[0], [x3]
/// st2 {v0.s, v1.s}[0], [x3]
/// st4 {v0.s, v1.s, v2.s, v3.s}[0], [x3]
/// ld2 {v0.s, v1.s}[0], [x3]
// TODO: add post-index form [Xn], #imm
pub(crate) fn encode_neon_ld_st_single(
    operands: &[Operand],
    is_load: bool,
    num_structs: u32,
) -> Result<EncodeResult, String> {
    if operands.len() < 2 {
        return Err(format!(
            "ld/st{} single element requires at least 2 operands",
            num_structs
        ));
    }

    let (regs, index) = match &operands[0] {
        Operand::RegListIndexed { regs, index } => (regs, *index),
        _ => return Err("expected register list with index".to_string()),
    };

    if regs.len() as u32 != num_structs {
        return Err(format!(
            "expected {} registers in list, got {}",
            num_structs,
            regs.len()
        ));
    }
    // TODO: validate that registers in the list are consecutive (ARM ISA requirement)

    // Get element size and first register from the list
    let (rt, elem_size) = match &regs[0] {
        Operand::RegArrangement { reg, arrangement } => (
            parse_reg_num(reg).ok_or("invalid register in list")?,
            arrangement.clone(),
        ),
        _ => return Err("expected register with arrangement in list".to_string()),
    };

    // Get base register and check for post-index
    let (rn, post_index) = match &operands[1] {
        Operand::Mem { base, offset: 0 } => {
            let rn =
                parse_reg_num(base).ok_or_else(|| format!("invalid base register: {}", base))?;
            // Check for post-index immediate: operands[2] is the post-index offset
            let pi = if operands.len() > 2 {
                match &operands[2] {
                    Operand::Imm(off) => Some(*off),
                    _ => None,
                }
            } else {
                None
            };
            (rn, pi)
        }
        Operand::MemPostIndex { base, offset } => {
            let rn =
                parse_reg_num(base).ok_or_else(|| format!("invalid base register: {}", base))?;
            (rn, Some(*offset))
        }
        _ => return Err("expected [Xn] memory operand".to_string()),
    };

    let l_bit = if is_load { 1u32 } else { 0u32 };

    // R bit: 0 for 1,3 registers; 1 for 2,4 registers
    let r_bit = match num_structs {
        1 | 3 => 0u32,
        2 | 4 => 1u32,
        _ => return Err(format!("unsupported struct count: {}", num_structs)),
    };

    // Compute opcode, S, Q, size based on element size and index
    let (opcode, s_bit, q_bit, size_field) = match elem_size.as_str() {
        "b" => {
            // opcode = 000 (1,2 regs) or 001 (3,4 regs)
            let base_opc = if num_structs <= 2 { 0b000u32 } else { 0b001u32 };
            // index bits: Q:S:size[1]:size[0] = 4 bits for 0-15
            let q = (index >> 3) & 1;
            let s = (index >> 2) & 1;
            let sz = index & 3;
            (base_opc, s, q, sz)
        }
        "h" => {
            let base_opc = if num_structs <= 2 { 0b010u32 } else { 0b011u32 };
            // index bits: Q:S:size[1] = 3 bits for 0-7, size[0]=0
            let q = (index >> 2) & 1;
            let s = (index >> 1) & 1;
            let sz = (index & 1) << 1;
            (base_opc, s, q, sz)
        }
        "s" => {
            let base_opc = if num_structs <= 2 { 0b100u32 } else { 0b101u32 };
            // index bits: Q:S = 2 bits for 0-3, size=00
            let q = (index >> 1) & 1;
            let s = index & 1;
            (base_opc, s, q, 0b00u32)
        }
        "d" => {
            let base_opc = if num_structs <= 2 { 0b100u32 } else { 0b101u32 };
            // index bits: Q = 1 bit for 0-1, S=0, size=01
            let q = index & 1;
            (base_opc, 0u32, q, 0b01u32)
        }
        _ => {
            return Err(format!(
                "unsupported element size for ld/st single: {}",
                elem_size
            ))
        }
    };

    if let Some(_offset) = post_index {
        // Post-index form: Q 0011011 L R 11111 opcode S size Rn Rt
        // (Rm=11111 means immediate post-index, the amount is implicit from element size)
        let word = (q_bit << 30)
            | (0b0011011 << 23)
            | (l_bit << 22)
            | (r_bit << 21)
            | (0b11111 << 16)
            | (opcode << 13)
            | (s_bit << 12)
            | (size_field << 10)
            | (rn << 5)
            | rt;
        Ok(EncodeResult::Word(word))
    } else {
        // No post-index: Q 0011010 L R 00000 opcode S size Rn Rt
        let word = (q_bit << 30)
            | (0b0011010 << 23)
            | (l_bit << 22)
            | (r_bit << 21)
            | (opcode << 13)
            | (s_bit << 12)
            | (size_field << 10)
            | (rn << 5)
            | rt;
        Ok(EncodeResult::Word(word))
    }
}

/// Common encoder for LD1/ST1 (multiple structures)
pub(crate) fn encode_neon_ld_st_multi(
    operands: &[Operand],
    is_load: bool,
    num_structs: u32,
) -> Result<EncodeResult, String> {
    if operands.len() < 2 {
        return Err(format!(
            "ld{}/st{} requires at least 2 operands",
            num_structs, num_structs
        ));
    }

    // First operand: register list {Vt.T} or {Vt.T, Vt+1.T, ...}
    let (rt, arr, num_regs) = match &operands[0] {
        Operand::RegList(regs) => {
            let (first_reg, arrangement) = match &regs[0] {
                Operand::RegArrangement { reg, arrangement } => (
                    parse_reg_num(reg).ok_or("invalid reg")?,
                    arrangement.clone(),
                ),
                _ => {
                    return Err(format!(
                        "ld{}/st{}: expected RegArrangement in list",
                        num_structs, num_structs
                    ))
                }
            };
            (first_reg, arrangement, regs.len() as u32)
        }
        _ => {
            return Err(format!(
                "ld{}/st{}: expected register list",
                num_structs, num_structs
            ))
        }
    };

    let (q, size) = neon_arr_to_q_size(&arr)?;

    // Second operand: [Xn] memory base or [Xn], #imm (post-index, merged by parser)
    let (rn, post_index) = match &operands[1] {
        Operand::Mem { base, offset: 0 } => {
            let r =
                parse_reg_num(base).ok_or_else(|| format!("invalid base register: {}", base))?;
            (r, None)
        }
        Operand::MemPostIndex { base, offset } => {
            let r =
                parse_reg_num(base).ok_or_else(|| format!("invalid base register: {}", base))?;
            (r, Some(*offset))
        }
        _ => {
            return Err(format!(
                "ld{}/st{}: expected [Xn] memory operand",
                num_structs, num_structs
            ))
        }
    };

    // opcode field based on structure count and number of registers:
    // LD1/ST1: 1 reg=0111, 2 reg=1010, 3 reg=0110, 4 reg=0010
    // LD2/ST2: 2 reg=1000
    // LD3/ST3: 3 reg=0100
    // LD4/ST4: 4 reg=0000
    let opcode = match num_structs {
        1 => match num_regs {
            1 => 0b0111u32,
            2 => 0b1010,
            3 => 0b0110,
            4 => 0b0010,
            _ => return Err(format!("ld1/st1: unsupported register count: {}", num_regs)),
        },
        2 => 0b1000u32,
        3 => 0b0100,
        4 => 0b0000,
        _ => return Err(format!("unsupported structure count: {}", num_structs)),
    };

    let l_bit = if is_load { 1u32 } else { 0u32 };

    // Handle post-index form from merged MemPostIndex
    if let Some(_imm) = post_index {
        // Post-index with immediate: use Rm=11111 (0x1F)
        let word = ((q << 30) | (0b001100 << 24) | (1 << 23) | (l_bit << 22))
            | (0b11111 << 16)
            | (opcode << 12)
            | (size << 10)
            | (rn << 5)
            | rt;
        return Ok(EncodeResult::Word(word));
    }

    // Check for post-index form via separate operands: [Xn], Xm
    if operands.len() > 2 {
        match &operands[2] {
            Operand::Imm(_) => {
                // Post-index with immediate: use Rm=11111
                let word = ((q << 30) | (0b001100 << 24) | (1 << 23) | (l_bit << 22))
                    | (0b11111 << 16)
                    | (opcode << 12)
                    | (size << 10)
                    | (rn << 5)
                    | rt;
                return Ok(EncodeResult::Word(word));
            }
            Operand::Reg(rm_name) => {
                let rm = parse_reg_num(rm_name).ok_or("invalid rm")?;
                let word = ((q << 30) | (0b001100 << 24) | (1 << 23) | (l_bit << 22))
                    | (rm << 16)
                    | (opcode << 12)
                    | (size << 10)
                    | (rn << 5)
                    | rt;
                return Ok(EncodeResult::Word(word));
            }
            _ => {}
        }
    }

    // No post-index: LD1/ST1 {Vt.T...}, [Xn]
    // 0 Q 001100 0 L 0 00000 opcode size Rn Rt
    let word = (((q << 30) | (0b001100 << 24)) | (l_bit << 22))
        | (opcode << 12)
        | (size << 10)
        | (rn << 5)
        | rt;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON UZP1/UZP2/ZIP1/ZIP2
pub(crate) fn encode_neon_zip_uzp(
    operands: &[Operand],
    op_bits: u32,
    _is_zip: bool,
) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("uzp/zip requires 3 operands".to_string());
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let (rm, _) = get_neon_reg(operands, 2)?;
    let (q, size) = neon_arr_to_q_size(&arr_d)?;

    // UZP1: 0 Q 0 01110 size 0 Rm 0 001 10 Rn Rd  (op_bits=001)
    // UZP2: 0 Q 0 01110 size 0 Rm 0 101 10 Rn Rd  (op_bits=101)
    // ZIP1: 0 Q 0 01110 size 0 Rm 0 011 10 Rn Rd  (op_bits=011)
    // ZIP2: 0 Q 0 01110 size 0 Rm 0 111 10 Rn Rd  (op_bits=111)
    let word = (((q << 30) | (0b001110 << 24) | (size << 22)) | (rm << 16))
        | (op_bits << 12)
        | (0b10 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON EOR3 (three-way XOR, SHA3 extension): EOR3 Vd.16b, Vn.16b, Vm.16b, Vk.16b
pub(crate) fn encode_neon_eor3(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 4 {
        return Err("eor3 requires 4 operands".to_string());
    }
    let (rd, _) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let (rm, _) = get_neon_reg(operands, 2)?;
    let (rk, _) = get_neon_reg(operands, 3)?;

    // EOR3 Vd.16b, Vn.16b, Vm.16b, Vk.16b
    // Encoding: 11001110 000 Rm 0 Rk(4:0) 00 Rn Rd
    let word = ((0b11001110u32 << 24) | (rm << 16)) | (rk << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON PMULL/PMULL2 (polynomial multiply long)
pub(crate) fn encode_neon_pmull(
    operands: &[Operand],
    is_pmull2: bool,
) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("pmull requires 3 operands".to_string());
    }
    let (rd, _) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let (rm, _) = get_neon_reg(operands, 2)?;

    let q = if is_pmull2 { 1u32 } else { 0 };

    // PMULL  Vd.1q, Vn.1d, Vm.1d: 0 0 00 1110 11 1 Rm 11100 0 Rn Rd  (size=11)
    // PMULL2 Vd.1q, Vn.2d, Vm.2d: 0 1 00 1110 11 1 Rm 11100 0 Rn Rd
    let word =
        ((q << 30) | (0b001110 << 24) | (0b11 << 22) | (1 << 21) | (rm << 16) | (0b11100 << 11))
            | (rn << 5)
            | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON AES instructions (AESE, AESD, AESMC, AESIMC)
pub(crate) fn encode_neon_aes(operands: &[Operand], opcode: u32) -> Result<EncodeResult, String> {
    if operands.len() < 2 {
        return Err("aes instruction requires 2 operands".to_string());
    }
    let (rd, _) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;

    // AES instructions: 0100 1110 0010 1000 opcode 10 Rn Rd
    // AESE:  opcode = 00100 (0x4)
    // AESD:  opcode = 00101 (0x5)
    // AESMC: opcode = 00110 (0x6)
    // AESIMC:opcode = 00111 (0x7)
    let word =
        (0b01001110 << 24) | (0b0010100 << 17) | (opcode << 12) | (0b10 << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON ADD/SUB (vector integer): ADD/SUB Vd.T, Vn.T, Vm.T
pub(crate) fn encode_neon_add_sub(
    operands: &[Operand],
    is_sub: bool,
) -> Result<EncodeResult, String> {
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let (rm, _) = get_neon_reg(operands, 2)?;
    let (q, size) = neon_arr_to_q_size(&arr_d)?;
    let u = if is_sub { 1u32 } else { 0u32 };

    // ADD: 0 Q 0 01110 size 1 Rm 10000 1 Rn Rd
    // SUB: 0 Q 1 01110 size 1 Rm 10000 1 Rn Rd
    let word = (q << 30)
        | (u << 29)
        | (0b01110 << 24)
        | (size << 22)
        | (1 << 21)
        | (rm << 16)
        | (0b10000 << 11)
        | (1 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON USHR (unsigned shift right immediate)
pub(crate) fn encode_neon_ushr(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("ushr requires 3 operands".to_string());
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let shift = get_imm(operands, 2)? as u32;

    let (q, _) = neon_arr_to_q_size(&arr_d)?;

    // USHR Vd.T, Vn.T, #shift
    // 0 Q 1 0 11110 immh:immb 000001 Rn Rd
    let immh_immb = match arr_d.as_str() {
        "8b" | "16b" => (16 - shift) & 0xF,
        "4h" | "8h" => (32 - shift) & 0x1F,
        "2s" | "4s" => (64 - shift) & 0x3F,
        "2d" => (128 - shift) & 0x7F,
        _ => return Err(format!("unsupported ushr arrangement: {}", arr_d)),
    };

    let word = (q << 30)
        | (1 << 29)
        | (0b011110 << 23)
        | (immh_immb << 16)
        | (0b000001 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON SSHR (signed shift right immediate)
pub(crate) fn encode_neon_sshr(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("sshr requires 3 operands".to_string());
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let shift = get_imm(operands, 2)? as u32;

    let (q, _) = neon_arr_to_q_size(&arr_d)?;

    // SSHR Vd.T, Vn.T, #shift
    // 0 Q 0 0 11110 immh:immb 000001 Rn Rd  (U=0)
    let immh_immb = match arr_d.as_str() {
        "8b" | "16b" => (16 - shift) & 0xF,
        "4h" | "8h" => (32 - shift) & 0x1F,
        "2s" | "4s" => (64 - shift) & 0x3F,
        "2d" => (128 - shift) & 0x7F,
        _ => return Err(format!("unsupported sshr arrangement: {}", arr_d)),
    };

    let word = (q << 30) | (0b011110 << 23) | (immh_immb << 16) | (0b000001 << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON SHL (shift left immediate)
pub(crate) fn encode_neon_shl(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("shl requires 3 operands".to_string());
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let shift = get_imm(operands, 2)? as u32;

    let (q, _) = neon_arr_to_q_size(&arr_d)?;

    // SHL Vd.T, Vn.T, #shift
    // 0 Q 0 0 11110 immh:immb 010101 Rn Rd
    // immh:immb = element_size + shift
    let immh_immb = match arr_d.as_str() {
        "8b" | "16b" => (8 + shift) & 0xF,
        "4h" | "8h" => (16 + shift) & 0x1F,
        "2s" | "4s" => (32 + shift) & 0x3F,
        "2d" => (64 + shift) & 0x7F,
        _ => return Err(format!("unsupported shl arrangement: {}", arr_d)),
    };

    let word = (q << 30) | (0b011110 << 23) | (immh_immb << 16) | (0b010101 << 10) | (rn << 5) | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode NEON SLI (shift left and insert)
pub(crate) fn encode_neon_sli(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("sli requires 3 operands".to_string());
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let shift = get_imm(operands, 2)? as u32;

    let (q, _) = neon_arr_to_q_size(&arr_d)?;

    // SLI Vd.T, Vn.T, #shift
    // 0 Q 1 0 11110 immh:immb 010101 Rn Rd  (U=1)
    let immh_immb = match arr_d.as_str() {
        "8b" | "16b" => (8 + shift) & 0xF,
        "4h" | "8h" => (16 + shift) & 0x1F,
        "2s" | "4s" => (32 + shift) & 0x3F,
        "2d" => (64 + shift) & 0x7F,
        _ => return Err(format!("unsupported sli arrangement: {}", arr_d)),
    };

    let word = (q << 30)
        | (1 << 29)
        | (0b011110 << 23)
        | (immh_immb << 16)
        | (0b010101 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// Encode SRI (Shift Right and Insert) immediate.
/// SRI Vd.T, Vn.T, #shift: 0 Q 1 0 11110 immh:immb 010001 Rn Rd  (U=1)
pub(crate) fn encode_neon_sri(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("sri requires 3 operands".to_string());
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let shift = get_imm(operands, 2)? as u32;

    let (q, _) = neon_arr_to_q_size(&arr_d)?;

    // immh:immb = (2*esize - shift) for right shift
    let immh_immb = match arr_d.as_str() {
        "8b" | "16b" => (16 - shift) & 0xF,
        "4h" | "8h" => (32 - shift) & 0x1F,
        "2s" | "4s" => (64 - shift) & 0x3F,
        "2d" => (128 - shift) & 0x7F,
        _ => return Err(format!("unsupported sri arrangement: {}", arr_d)),
    };

    let word = (q << 30)
        | (1 << 29)
        | (0b011110 << 23)
        | (immh_immb << 16)
        | (0b010001 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

// ── NEON RBIT (vector bit reverse) ───────────────────────────────────────

/// Encode NEON RBIT Vd.T, Vn.T (per-byte bit reversal in each element).
pub(crate) fn encode_neon_rbit(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 2 {
        return Err("neon rbit requires 2 operands".to_string());
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;

    // Only .8b and .16b arrangements are valid for NEON RBIT
    if arr_d != "8b" && arr_d != "16b" {
        return Err(format!(
            "neon rbit: unsupported arrangement .{}, expected .8b or .16b",
            arr_d
        ));
    }
    let q: u32 = if arr_d == "16b" { 1 } else { 0 };
    // RBIT Vd.T, Vn.T: 0 Q 1 01110 01 10000 00101 10 Rn Rd
    let word = (q << 30)
        | (1 << 29)
        | (0b01110 << 24)
        | (0b01 << 22)
        | (0b10000 << 17)
        | (0b00101 << 12)
        | (0b10 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

// ── NEON MVNI (move NOT immediate) ───────────────────────────────────────

/// Encode NEON MVNI Vd.T, #imm (move bitwise NOT immediate to vector).
pub(crate) fn encode_neon_mvni(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 2 {
        return Err("mvni requires 2 operands".to_string());
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let imm = get_imm(operands, 1)?;
    let imm8 = imm as u32 & 0xFF;

    // Extract abc:defgh for encoding
    let abc = (imm8 >> 5) & 0x7;
    let defgh = imm8 & 0x1f;

    match arr_d.as_str() {
        "2s" | "4s" => {
            let q: u32 = if arr_d == "4s" { 1 } else { 0 };
            // Check for optional shift
            let cmode = if let Some(Operand::Shift { kind, amount }) = operands.get(2) {
                if kind.to_lowercase() == "lsl" {
                    match *amount {
                        0 => 0b0000u32,
                        8 => 0b0010,
                        16 => 0b0100,
                        24 => 0b0110,
                        _ => return Err(format!("mvni: unsupported shift amount: {}", amount)),
                    }
                } else if kind.to_lowercase() == "msl" {
                    match *amount {
                        8 => 0b1100u32,
                        16 => 0b1101,
                        _ => return Err(format!("mvni: unsupported MSL shift: {}", amount)),
                    }
                } else {
                    0b0000
                }
            } else {
                0b0000
            };
            // MVNI: 0 Q 1 0 1111 00 abc cmode 01 defgh Rd  (op=1)
            let word = (q << 30)
                | (1 << 29)
                | (0b0111100 << 22)
                | (abc << 16)
                | (cmode << 12)
                | (0b01 << 10)
                | (defgh << 5)
                | rd;
            Ok(EncodeResult::Word(word))
        }
        "4h" | "8h" => {
            let q: u32 = if arr_d == "8h" { 1 } else { 0 };
            // MVNI 16-bit: cmode=1000, op=1
            let word = (q << 30)
                | (1 << 29)
                | (0b0111100 << 22)
                | (abc << 16)
                | (0b1000 << 12)
                | (0b01 << 10)
                | (defgh << 5)
                | rd;
            Ok(EncodeResult::Word(word))
        }
        _ => Err(format!("mvni: unsupported arrangement: {}", arr_d)),
    }
}

// ── NEON float three-same ────────────────────────────────────────────────
/// Encode NEON float three-same: FADD, FSUB, FMUL, FDIV, FMLA, FMLS, etc.
/// Format: 0 Q U 01110 size 1 Rm opcode 1 Rn Rd
/// size[1]=size_hi (0 or 1), size[0]=sz (0=single, 1=double)
pub(crate) fn encode_neon_float_three_same(
    operands: &[Operand],
    u_bit: u32,
    size_hi: u32,
    opcode: u32,
) -> Result<EncodeResult, String> {
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let (rm, _) = get_neon_reg(operands, 2)?;
    let (q, sz) = match arr_d.as_str() {
        "2s" => (0u32, 0u32),
        "4s" => (1, 0),
        "2d" => (1, 1),
        _ => {
            return Err(format!(
                "float three-same: unsupported arrangement: {}",
                arr_d
            ))
        }
    };
    let size = (size_hi << 1) | sz;
    let word = (q << 30)
        | (u_bit << 29)
        | (0b01110 << 24)
        | (size << 22)
        | (1 << 21)
        | (rm << 16)
        | (opcode << 11)
        | (1 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

// ── NEON two-register misc (integer) ─────────────────────────────────────
/// Encode NEON two-reg misc: ABS, NEG, CLS, CLZ, etc.
/// Format: 0 Q U 01110 size 10000 opcode 10 Rn Rd
pub(crate) fn encode_neon_two_misc(
    operands: &[Operand],
    u_bit: u32,
    opcode: u32,
) -> Result<EncodeResult, String> {
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let (q, size) = neon_arr_to_q_size(&arr_d)?;
    let word = (q << 30)
        | (u_bit << 29)
        | (0b01110 << 24)
        | (size << 22)
        | (0b10000 << 17)
        | (opcode << 12)
        | (0b10 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

// ── NEON float two-register misc ─────────────────────────────────────────
/// Encode NEON float two-reg misc: UCVTF, SCVTF, FCVTZS, FCVTZU, FNEG, FABS, etc. (vector)
/// Format: 0 Q U 01110 size 10000 opcode 10 Rn Rd
/// size[1]=size_hi, size[0]=sz (0=single, 1=double)
pub(crate) fn encode_neon_float_two_misc(
    operands: &[Operand],
    u_bit: u32,
    size_hi: u32,
    opcode: u32,
) -> Result<EncodeResult, String> {
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let (q, sz) = match arr_d.as_str() {
        "2s" => (0u32, 0u32),
        "4s" => (1, 0),
        "2d" => (1, 1),
        _ => {
            return Err(format!(
                "float two-misc: unsupported arrangement: {}",
                arr_d
            ))
        }
    };
    let size = (size_hi << 1) | sz;
    let word = (q << 30)
        | (u_bit << 29)
        | (0b01110 << 24)
        | (size << 22)
        | (0b10000 << 17)
        | (opcode << 12)
        | (0b10 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

// ── NEON shift right narrow (SHRN/RSHRN) ─────────────────────────────────
/// Format: 0 Q 0 01111 0 immh immb opcode 1 Rn Rd
/// SHRN opcode=10000, RSHRN opcode=10001
pub(crate) fn encode_neon_shrn(
    operands: &[Operand],
    opcode: u32,
    is_high: bool,
) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("shrn/rshrn requires 3 operands".to_string());
    }
    let (rd, _) = get_neon_reg(operands, 0)?;
    let (rn, arr_n) = get_neon_reg(operands, 1)?;
    let shift = get_imm(operands, 2)? as u32;
    let element_bits = match arr_n.as_str() {
        "8h" => 16u32,
        "4s" => 32,
        "2d" => 64,
        _ => return Err(format!("shrn: unsupported source: {}", arr_n)),
    };
    let half_bits = element_bits / 2;
    if shift == 0 || shift > half_bits {
        return Err(format!("shrn: shift {} out of range", shift));
    }
    let immhb = element_bits - shift;
    let q = if is_high { 1u32 } else { 0 };
    let word = (q << 30)
        | (0b011110 << 23)
        | ((immhb >> 3) << 19)
        | ((immhb & 7) << 16)
        | (opcode << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

// ── NEON shift right accumulate (SSRA/USRA/SRSHR/URSHR) ─────────────────
/// Format: 0 Q U 01111 0 immh immb opcode 1 Rn Rd
pub(crate) fn encode_neon_shift_right(
    operands: &[Operand],
    u_bit: u32,
    opcode: u32,
) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("shift-right requires 3 operands".to_string());
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let shift = get_imm(operands, 2)? as u32;
    let (q, _) = neon_arr_to_q_size(&arr_d)?;
    let element_bits: u32 = match arr_d.as_str() {
        "8b" | "16b" => 8,
        "4h" | "8h" => 16,
        "2s" | "4s" => 32,
        "2d" => 64,
        _ => return Err(format!("shift-right: unsupported: {}", arr_d)),
    };
    if shift == 0 || shift > element_bits {
        return Err(format!("shift {} out of range", shift));
    }
    let immhb = (element_bits * 2) - shift;
    let word = (q << 30)
        | (u_bit << 29)
        | (0b011110 << 23)
        | ((immhb >> 3) << 19)
        | ((immhb & 7) << 16)
        | (opcode << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

// ── NEON SSHLL/USHLL (shift left long) ───────────────────────────────────
/// Format: 0 Q U 011110 immh immb 10100 1 Rn Rd
pub(crate) fn encode_neon_shll(
    operands: &[Operand],
    u_bit: u32,
    is_high: bool,
) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("sshll/ushll requires 3 operands".to_string());
    }
    let (rd, _) = get_neon_reg(operands, 0)?;
    let (rn, arr_n) = get_neon_reg(operands, 1)?;
    let shift = get_imm(operands, 2)? as u32;
    let base_val = match arr_n.as_str() {
        "8b" | "16b" => 8u32,
        "4h" | "8h" => 16,
        "2s" | "4s" => 32,
        _ => return Err(format!("sshll/ushll: unsupported source: {}", arr_n)),
    };
    let immhb = base_val + shift;
    let q = if is_high { 1u32 } else { 0 };
    let word = (q << 30)
        | (u_bit << 29)
        | (0b011110 << 23)
        | ((immhb >> 3) << 19)
        | ((immhb & 7) << 16)
        | (0b101001 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

// ── NEON pairwise add (UADDLP/SADDLP/UADALP/SADALP) ────────────────────

// ── NEON three-different extras: UABAL/SABAL/ADDHN/RADDHN/SUBHN/RSUBHN ──
// Already have encode_neon_three_diff which handles these opcodes.

// ── NEON SQXTUN ──────────────────────────────────────────────────────────
// Two-reg misc with U=1, opcode=10010. Reuse encode_neon_two_misc_narrow.

// ── NEON shift right narrow saturating (SQSHRN/UQSHRN/SQRSHRN/UQRSHRN) ─
pub(crate) fn encode_neon_qshrn(
    operands: &[Operand],
    u_bit: u32,
    is_rounding: bool,
    is_high: bool,
) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("qshrn requires 3 operands".to_string());
    }
    let (rd, _) = get_neon_reg(operands, 0)?;
    let (rn, arr_n) = get_neon_reg(operands, 1)?;
    let shift = get_imm(operands, 2)? as u32;
    let element_bits = match arr_n.as_str() {
        "8h" => 16u32,
        "4s" => 32,
        "2d" => 64,
        _ => return Err(format!("qshrn: unsupported source: {}", arr_n)),
    };
    if shift == 0 || shift > element_bits {
        return Err(format!(
            "qshrn: shift {} out of range for {}-bit elements",
            shift, element_bits
        ));
    }
    let immhb = element_bits - shift;
    let q = if is_high { 1u32 } else { 0 };
    let opcode_bits: u32 = if is_rounding { 0b100111 } else { 0b100101 };
    let word = (q << 30)
        | (u_bit << 29)
        | (0b011110 << 23)
        | ((immhb >> 3) << 19)
        | ((immhb & 7) << 16)
        | (opcode_bits << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

// ── NEON ADDHN/RADDHN/SUBHN/RSUBHN ──────────────────────────────────────
/// Three-different narrowing high: Format: 0 Q U 01110 size 1 Rm opcode 00 Rn Rd
pub(crate) fn encode_neon_three_diff_narrow(
    operands: &[Operand],
    u_bit: u32,
    opcode: u32,
    is_high: bool,
) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("addhn/subhn requires 3 operands".to_string());
    }
    let (rd, _) = get_neon_reg(operands, 0)?;
    let (rn, arr_n) = get_neon_reg(operands, 1)?;
    let (rm, _) = get_neon_reg(operands, 2)?;
    let size = match arr_n.as_str() {
        "8h" => 0b00u32,
        "4s" => 0b01,
        "2d" => 0b10,
        _ => return Err(format!("addhn: unsupported source: {}", arr_n)),
    };
    let q = if is_high { 1u32 } else { 0 };
    let word = (q << 30)
        | (u_bit << 29)
        | (0b01110 << 24)
        | (size << 22)
        | (1 << 21)
        | (rm << 16)
        | (opcode << 12)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

// ── NEON LD2R/LD3R/LD4R ──────────────────────────────────────────────────
pub(crate) fn encode_neon_ldnr(
    operands: &[Operand],
    num_structs: u32,
) -> Result<EncodeResult, String> {
    if operands.len() < 2 {
        return Err(format!("ld{}r requires 2 operands", num_structs));
    }
    let (rt, arr, num_regs) = match &operands[0] {
        Operand::RegList(regs) => {
            let (first_reg, arrangement) = match &regs[0] {
                Operand::RegArrangement { reg, arrangement } => (
                    parse_reg_num(reg).ok_or("invalid reg")?,
                    arrangement.clone(),
                ),
                _ => return Err("expected RegArrangement in list".to_string()),
            };
            (first_reg, arrangement, regs.len() as u32)
        }
        _ => return Err("expected register list".to_string()),
    };
    if num_regs != num_structs {
        return Err(format!(
            "ld{}r: expected {} regs, got {}",
            num_structs, num_structs, num_regs
        ));
    }
    let (q, size) = match arr.as_str() {
        "8b" => (0u32, 0b00u32),
        "16b" => (1, 0b00),
        "4h" => (0, 0b01),
        "8h" => (1, 0b01),
        "2s" => (0, 0b10),
        "4s" => (1, 0b10),
        "1d" => (0, 0b11),
        "2d" => (1, 0b11),
        _ => {
            return Err(format!(
                "ld{}r: unsupported arrangement: {}",
                num_structs, arr
            ))
        }
    };
    // opcode: ld1r=110, ld2r=110(S=1), ld3r=111, ld4r=111(S=1)
    let (opcode, s_bit) = match num_structs {
        1 => (0b110u32, 0u32),
        2 => (0b110, 1),
        3 => (0b111, 0),
        4 => (0b111, 1),
        _ => return Err(format!("unsupported: ld{}r", num_structs)),
    };
    let base = match &operands[1] {
        Operand::Mem { base, .. } => parse_reg_num(base).ok_or("invalid base")?,
        Operand::MemPostIndex { base, .. } => parse_reg_num(base).ok_or("invalid base")?,
        _ => return Err("expected memory operand".to_string()),
    };
    // check for post-index
    let rm = match &operands[1] {
        Operand::MemPostIndex { .. } => 0b11111u32, // immediate post-index
        _ => 0u32,
    };
    let has_post = rm != 0;
    let word = (q << 30)
        | (0b001101 << 24)
        | (if has_post { 1u32 } else { 0 } << 23)
        | (1 << 22)
        | (if has_post { rm } else { 0 } << 16)
        | (opcode << 13)
        | (s_bit << 12)
        | (size << 10)
        | (base << 5)
        | rt;
    Ok(EncodeResult::Word(word))
}

// ── NEON float compare-to-zero ───────────────────────────────────────────
/// FCMEQ/FCMLE/FCMLT/FCMGE/FCMGT to zero
/// Format: 0 Q U 01110 size 10000 opcode 10 Rn Rd (float, size = 0sz)
pub(crate) fn encode_neon_float_cmp_zero(
    operands: &[Operand],
    u_bit: u32,
    size_hi: u32,
    opcode: u32,
) -> Result<EncodeResult, String> {
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let (q, sz) = match arr_d.as_str() {
        "2s" => (0u32, 0u32),
        "4s" => (1, 0),
        "2d" => (1, 1),
        _ => return Err(format!("float cmp zero: unsupported: {}", arr_d)),
    };
    let size = (size_hi << 1) | sz;
    let word = (q << 30)
        | (u_bit << 29)
        | (0b01110 << 24)
        | (size << 22)
        | (0b10000 << 17)
        | (opcode << 12)
        | (0b10 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

// ── NEON by-element (non-long) ───────────────────────────────────────────
/// MUL/MLA/MLS by element: 0 Q U 01111 size L M Rm opcode H 0 Rn Rd
pub(crate) fn encode_neon_elem(
    operands: &[Operand],
    u_bit: u32,
    opcode: u32,
) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("NEON by-element requires 3 operands".to_string());
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let (rm, index) = match &operands[2] {
        Operand::RegLane { reg, index, .. } => (parse_reg_num(reg).ok_or("invalid reg")?, *index),
        _ => return Err(format!("expected register lane, got {:?}", operands[2])),
    };
    let (q, size) = neon_arr_to_q_size(&arr_d)?;
    let (h, l, m_bit) = match size {
        0b01 => ((index >> 2) & 1, (index >> 1) & 1, index & 1),
        0b10 => ((index >> 1) & 1, index & 1, (rm >> 4) & 1),
        _ => return Err("unsupported element size for by-element".to_string()),
    };
    let rm_enc = if size == 0b01 { rm & 0xF } else { rm & 0x1F };
    let word = (q << 30)
        | (u_bit << 29)
        | (0b01111 << 24)
        | (size << 22)
        | (l << 21)
        | (m_bit << 20)
        | (rm_enc << 16)
        | (opcode << 12)
        | (h << 11)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

// ── NEON float by-element ────────────────────────────────────────────────
pub(crate) fn encode_neon_float_elem(
    operands: &[Operand],
    u_bit: u32,
    opcode: u32,
) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("NEON float by-element requires 3 operands".to_string());
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let (rm, index) = match &operands[2] {
        Operand::RegLane { reg, index, .. } => (parse_reg_num(reg).ok_or("invalid reg")?, *index),
        _ => return Err(format!("expected register lane, got {:?}", operands[2])),
    };
    let (q, sz) = match arr_d.as_str() {
        "2s" => (0u32, 0u32),
        "4s" => (1, 0),
        "2d" => (1, 1),
        _ => return Err(format!("float by-element: unsupported: {}", arr_d)),
    };
    let (h, l, m_bit) = if sz == 0 {
        ((index >> 1) & 1, index & 1, (rm >> 4) & 1)
    } else {
        (index & 1, 0u32, (rm >> 4) & 1)
    };
    let rm_enc = rm & 0x1F;
    let word = (q << 30)
        | (u_bit << 29)
        | (0b01111 << 24)
        | (sz << 22)
        | (l << 21)
        | (m_bit << 20)
        | (rm_enc << 16)
        | (opcode << 12)
        | (h << 11)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

// ── NEON FCVTL/FCVTN ────────────────────────────────────────────────────
/// FCVTL: half→single or single→double widening float convert
/// Format: 0 Q 0 01110 0 sz 10000 10111 10 Rn Rd
pub(crate) fn encode_neon_fcvtl(
    operands: &[Operand],
    is_high: bool,
) -> Result<EncodeResult, String> {
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let sz = match arr_d.as_str() {
        "4s" | "2s" => 0u32,
        "2d" => 1,
        _ => return Err(format!("fcvtl: unsupported dest: {}", arr_d)),
    };
    let q = if is_high { 1u32 } else { 0 };
    let word = (q << 30)
        | (0b01110 << 24)
        | (sz << 22)
        | (0b10000 << 17)
        | (0b10111 << 12)
        | (0b10 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

/// FCVTN: single→half or double→single narrowing float convert
pub(crate) fn encode_neon_fcvtn(
    operands: &[Operand],
    is_high: bool,
) -> Result<EncodeResult, String> {
    let (rd, _) = get_neon_reg(operands, 0)?;
    let (rn, arr_n) = get_neon_reg(operands, 1)?;
    let sz = match arr_n.as_str() {
        "4s" | "2s" => 0u32,
        "2d" => 1,
        _ => return Err(format!("fcvtn: unsupported source: {}", arr_n)),
    };
    let q = if is_high { 1u32 } else { 0 };
    let word = (q << 30)
        | (0b01110 << 24)
        | (sz << 22)
        | (0b10000 << 17)
        | (0b10110 << 12)
        | (0b10 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

// ── BIT/BIF (bitwise insert if true/false) ──────────────────────────────
/// Encodes BIT (size=10) and BIF (size=11) instructions.
/// Same format as BSL but with different size field.
/// Format: 0 Q 1 01110 ss 1 Rm 000111 Rn Rd
pub(crate) fn encode_neon_bitwise_insert(
    operands: &[Operand],
    size: u32,
) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("bit/bif requires 3 operands".to_string());
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let (rm, _) = get_neon_reg(operands, 2)?;
    let q: u32 = if arr_d == "16b" { 1 } else { 0 };
    let word = (q << 30)
        | (1 << 29)
        | (0b01110 << 24)
        | (size << 22)
        | (1 << 21)
        | (rm << 16)
        | (0b000111 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

// ── FADDP (float pairwise add) ──────────────────────────────────────────
/// FADDP — float add pairwise
/// Vector form: FADDP Vd.T, Vn.T, Vm.T
///   Format: 0 Q 1 01110 0 sz 1 Rm 110101 Rn Rd
/// Scalar form: FADDP Sd, Vn.2S  or FADDP Dd, Vn.2D
///   Format: 01 1 11110 0 sz 11000 01101 10 Rn Rd
pub(crate) fn encode_neon_faddp(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() >= 3 {
        // Vector form: 3 operands
        let (rd, arr_d) = get_neon_reg(operands, 0)?;
        let (rn, _) = get_neon_reg(operands, 1)?;
        let (rm, _) = get_neon_reg(operands, 2)?;
        let (q, sz) = match arr_d.as_str() {
            "2s" => (0u32, 0u32),
            "4s" => (1, 0),
            "2d" => (1, 1),
            _ => return Err(format!("faddp: unsupported arrangement: {}", arr_d)),
        };
        let word = (q << 30)
            | (1 << 29)
            | (0b01110 << 24)
            | (sz << 22)
            | (1 << 21)
            | (rm << 16)
            | (0b110101 << 10)
            | (rn << 5)
            | rd;
        Ok(EncodeResult::Word(word))
    } else if operands.len() == 2 {
        // Scalar form: FADDP Sd, Vn.2S or FADDP Dd, Vn.2D
        let rd = match &operands[0] {
            Operand::Reg(r) => parse_reg_num(r).ok_or("invalid dest reg")?,
            _ => return Err("faddp scalar: expected register".to_string()),
        };
        let (rn, arr_n) = get_neon_reg(operands, 1)?;
        let sz = match arr_n.as_str() {
            "2s" => 0u32,
            "2d" => 1,
            _ => return Err(format!("faddp scalar: unsupported source: {}", arr_n)),
        };
        // 01 1 11110 0 sz 11000 01101 10 Rn Rd
        let word = (0b01 << 30)
            | (1 << 29)
            | (0b11110 << 24)
            | (sz << 22)
            | (0b11000 << 17)
            | (0b01101 << 12)
            | (0b10 << 10)
            | (rn << 5)
            | rd;
        Ok(EncodeResult::Word(word))
    } else {
        Err("faddp requires 2 or 3 operands".to_string())
    }
}

// ── SADDLV/UADDLV (signed/unsigned add long across vector) ─────────────
/// Format: 0 Q U 01110 size 11000 00011 10 Rn Rd
pub(crate) fn encode_neon_across_long(
    operands: &[Operand],
    u: u32,
    opcode: u32,
) -> Result<EncodeResult, String> {
    if operands.len() < 2 {
        return Err("saddlv/uaddlv requires 2 operands".to_string());
    }
    // Destination is a scalar register (e.g., s16), source is a vector arrangement
    let rd = match &operands[0] {
        Operand::Reg(r) => parse_reg_num(r).ok_or("invalid dest reg")?,
        Operand::RegArrangement { reg, .. } => parse_reg_num(reg).ok_or("invalid dest reg")?,
        _ => return Err("saddlv: expected register".to_string()),
    };
    let (rn, arr_n) = get_neon_reg(operands, 1)?;
    let (q, size) = neon_arr_to_q_size(&arr_n)?;
    let word = (q << 30)
        | (u << 29)
        | (0b01110 << 24)
        | (size << 22)
        | (0b11000 << 17)
        | (opcode << 12)
        | (0b10 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

// ── NEON shift left by immediate (SQSHL, UQSHL, SHL, etc.) ─────────────
/// Format: 0 Q U 011110 immh:immb opcode 1 Rn Rd
/// immh:immb encodes both the element size and the shift amount.
pub(crate) fn encode_neon_shift_left_imm(
    operands: &[Operand],
    u: u32,
    opcode: u32,
) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("shift left immediate requires 3 operands".to_string());
    }
    let (rd, arr_d) = get_neon_reg(operands, 0)?;
    let (rn, _) = get_neon_reg(operands, 1)?;
    let shift = get_imm(operands, 2)? as u32;

    let (q, _immh_base, esize) = match arr_d.as_str() {
        "8b" => (0u32, 0b0001u32, 8u32),
        "16b" => (1, 0b0001, 8),
        "4h" => (0, 0b0010, 16),
        "8h" => (1, 0b0010, 16),
        "2s" => (0, 0b0100, 32),
        "4s" => (1, 0b0100, 32),
        "2d" => (1, 0b1000, 64),
        _ => {
            return Err(format!(
                "shift left imm: unsupported arrangement: {}",
                arr_d
            ))
        }
    };

    // immh:immb = esize + shift_amount
    // For 8-bit: immh=0001, shift in 0..7 => immh:immb = 8 + shift
    // For 16-bit: immh=001x, shift in 0..15 => immh:immb = 16 + shift
    // For 32-bit: immh=01xx, shift in 0..31 => immh:immb = 32 + shift
    // For 64-bit: immh=1xxx, shift in 0..63 => immh:immb = 64 + shift
    let immhb = esize + shift;
    let immh = (immhb >> 3) & 0xF;
    let immb = immhb & 0x7;

    let word = (q << 30)
        | (u << 29)
        | (0b011110 << 23)
        | (immh << 19)
        | (immb << 16)
        | (opcode << 11)
        | (1 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

// ── Helper: detect scalar d-register 3-operand NEON operations ──────────────
pub(crate) fn is_neon_scalar_d_reg_op(operands: &[Operand]) -> bool {
    if operands.len() < 3 {
        return false;
    }
    match &operands[0] {
        Operand::Reg(r) => {
            let r = r.to_lowercase();
            r.starts_with('d') && r[1..].parse::<u32>().is_ok()
        }
        _ => false,
    }
}

// ── NEON scalar three-same: ADD/SUB Dd, Dn, Dm ────────────────────────────
/// Encode scalar NEON three-same: 01 U 11110 size 1 Rm opcode 1 Rn Rd
pub(crate) fn encode_neon_scalar_three_same(
    operands: &[Operand],
    u_bit: u32,
    opcode: u32,
    size: u32,
) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("scalar three-same requires 3 operands".to_string());
    }
    let rd = match &operands[0] {
        Operand::Reg(r) => parse_reg_num(r).ok_or("invalid reg")?,
        _ => return Err("expected register".to_string()),
    };
    let rn = match &operands[1] {
        Operand::Reg(r) => parse_reg_num(r).ok_or("invalid reg")?,
        _ => return Err("expected register".to_string()),
    };
    let rm = match &operands[2] {
        Operand::Reg(r) => parse_reg_num(r).ok_or("invalid reg")?,
        _ => return Err("expected register".to_string()),
    };
    let word = (0b01 << 30)
        | (u_bit << 29)
        | (0b11110 << 24)
        | (size << 22)
        | (1 << 21)
        | (rm << 16)
        | (opcode << 11)
        | (1 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

// ── NEON scalar ADDP: addp Dd, Vn.2d ──────────────────────────────────────
pub(crate) fn encode_neon_scalar_addp(operands: &[Operand]) -> Result<EncodeResult, String> {
    if operands.len() < 2 {
        return Err("scalar addp requires 2 operands".to_string());
    }
    let rd = match &operands[0] {
        Operand::Reg(r) => parse_reg_num(r).ok_or("invalid reg")?,
        _ => return Err("expected d register".to_string()),
    };
    let rn = match &operands[1] {
        Operand::RegArrangement { reg, arrangement } => {
            if arrangement != "2d" {
                return Err(format!(
                    "scalar addp requires .2d source, got .{}",
                    arrangement
                ));
            }
            parse_reg_num(reg).ok_or("invalid reg")?
        }
        _ => return Err("scalar addp: expected Vn.2d source".to_string()),
    };
    // Scalar ADDP: 01 0 11110 11 11000 11011 10 Rn Rd
    let word = (0b01 << 30)
        | (0b011110 << 24)
        | (0b11 << 22)
        | (0b11000 << 17)
        | (0b11011 << 12)
        | (0b10 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

// ── NEON scalar two-reg misc: SQABS/SQNEG Hd,Hn / Sd,Sn / Dd,Dn ──────────
pub(crate) fn encode_neon_scalar_two_misc(
    operands: &[Operand],
    u_bit: u32,
    opcode: u32,
) -> Result<EncodeResult, String> {
    if operands.len() < 2 {
        return Err("scalar two-misc requires 2 operands".to_string());
    }
    let (rd, rd_name) = match &operands[0] {
        Operand::Reg(r) => (parse_reg_num(r).ok_or("invalid reg")?, r.to_lowercase()),
        _ => return Err("expected register".to_string()),
    };
    let rn = match &operands[1] {
        Operand::Reg(r) => parse_reg_num(r).ok_or("invalid reg")?,
        _ => return Err("expected register".to_string()),
    };
    let size = if rd_name.starts_with('b') {
        0b00u32
    } else if rd_name.starts_with('h') {
        0b01
    } else if rd_name.starts_with('s') {
        0b10
    } else if rd_name.starts_with('d') {
        0b11
    } else {
        return Err(format!(
            "scalar two-misc: unsupported register type: {}",
            rd_name
        ));
    };
    // 01 U 11110 size 10000 opcode 10 Rn Rd
    let word = (0b01 << 30)
        | (u_bit << 29)
        | (0b11110 << 24)
        | (size << 22)
        | (0b10000 << 17)
        | (opcode << 12)
        | (0b10 << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

// ── NEON scalar SQSHRN: sqshrn Hd,Sn,#shift / sqshrn Sd,Dn,#shift ────────
pub(crate) fn encode_neon_scalar_qshrn(
    operands: &[Operand],
    u_bit: u32,
    is_rounding: bool,
) -> Result<EncodeResult, String> {
    if operands.len() < 3 {
        return Err("scalar qshrn requires 3 operands".to_string());
    }
    let (rd, rd_name) = match &operands[0] {
        Operand::Reg(r) => (parse_reg_num(r).ok_or("invalid reg")?, r.to_lowercase()),
        _ => return Err("expected register".to_string()),
    };
    let rn = match &operands[1] {
        Operand::Reg(r) => parse_reg_num(r).ok_or("invalid reg")?,
        _ => return Err("expected register".to_string()),
    };
    let shift = get_imm(operands, 2)? as u32;
    // Determine element bits from destination register type
    let element_bits = if rd_name.starts_with('b') {
        8u32
    }
    // b <- h (narrow from 16-bit)
    else if rd_name.starts_with('h') {
        16
    }
    // h <- s (narrow from 32-bit), immh base = 16
    else if rd_name.starts_with('s') {
        32
    }
    // s <- d (narrow from 64-bit), immh base = 32
    else {
        return Err(format!("scalar qshrn: unsupported dest: {}", rd_name));
    };
    if shift == 0 || shift > element_bits {
        return Err(format!("scalar qshrn: shift {} out of range", shift));
    }
    let immhb = (element_bits * 2) - shift; // source element bits - shift
    let opcode_bits: u32 = if is_rounding { 0b100111 } else { 0b100101 };
    // 01 U 11110 immh:immb opcode 1 Rn Rd
    let word = (0b01 << 30)
        | (u_bit << 29)
        | (0b011110 << 23)
        | ((immhb >> 3) << 19)
        | ((immhb & 7) << 16)
        | (opcode_bits << 10)
        | (rn << 5)
        | rd;
    Ok(EncodeResult::Word(word))
}

// ── NEON addp (integer pairwise add) — already handled in three-same as addp ──
