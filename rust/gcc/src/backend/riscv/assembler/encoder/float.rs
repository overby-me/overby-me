use super::*;

// ── Floating-point instructions ──

pub(crate) fn encode_float_load(operands: &[Operand], funct3: u32) -> Result<EncodeResult, String> {
    let rd = get_freg(operands, 0)?;
    match &operands.get(1) {
        Some(Operand::Mem { base, offset }) => {
            let rs1 = reg_num(base).ok_or("invalid base register")?;
            Ok(EncodeResult::Word(encode_i(
                OP_LOAD_FP,
                rd,
                funct3,
                rs1,
                *offset as i32,
            )))
        }
        Some(Operand::MemSymbol { base, symbol, .. }) => {
            let rs1 = reg_num(base).ok_or("invalid base register")?;
            let (reloc_type, sym) = parse_reloc_modifier(symbol);
            let reloc_type = match reloc_type {
                RelocType::PcrelHi20 => RelocType::PcrelLo12I,
                RelocType::Hi20 => RelocType::Lo12I,
                other => other,
            };
            Ok(EncodeResult::WordWithReloc {
                word: encode_i(OP_LOAD_FP, rd, funct3, rs1, 0),
                reloc: Relocation {
                    reloc_type,
                    symbol: sym,
                    addend: 0,
                },
            })
        }
        _ => Err("float load: expected memory operand".to_string()),
    }
}

pub(crate) fn encode_float_store(
    operands: &[Operand],
    funct3: u32,
) -> Result<EncodeResult, String> {
    let rs2 = get_freg(operands, 0)?;
    match &operands.get(1) {
        Some(Operand::Mem { base, offset }) => {
            let rs1 = reg_num(base).ok_or("invalid base register")?;
            Ok(EncodeResult::Word(encode_s(
                OP_STORE_FP,
                funct3,
                rs1,
                rs2,
                *offset as i32,
            )))
        }
        Some(Operand::MemSymbol { base, symbol, .. }) => {
            let rs1 = reg_num(base).ok_or("invalid base register")?;
            let (reloc_type, sym) = parse_reloc_modifier(symbol);
            let reloc_type = match reloc_type {
                RelocType::PcrelHi20 => RelocType::PcrelLo12S,
                RelocType::Hi20 => RelocType::Lo12S,
                other => other,
            };
            Ok(EncodeResult::WordWithReloc {
                word: encode_s(OP_STORE_FP, funct3, rs1, rs2, 0),
                reloc: Relocation {
                    reloc_type,
                    symbol: sym,
                    addend: 0,
                },
            })
        }
        _ => Err("float store: expected memory operand".to_string()),
    }
}

pub(crate) fn encode_fp_arith(operands: &[Operand], funct7: u32) -> Result<EncodeResult, String> {
    let rd = get_freg(operands, 0)?;
    let rs1 = get_freg(operands, 1)?;
    let rs2 = get_freg(operands, 2)?;
    // Check for optional rounding mode
    let rm = if operands.len() > 3 {
        match &operands[3] {
            Operand::RoundingMode(s) => parse_rm(s),
            _ => 0b111, // dynamic
        }
    } else {
        0b111
    };
    Ok(EncodeResult::Word(encode_r(
        OP_OP_FP, rd, rm, rs1, rs2, funct7,
    )))
}

pub(crate) fn encode_fp_arith_d(operands: &[Operand], funct7: u32) -> Result<EncodeResult, String> {
    encode_fp_arith(operands, funct7)
}

pub(crate) fn encode_fp_unary(
    operands: &[Operand],
    funct7: u32,
    rs2: u32,
) -> Result<EncodeResult, String> {
    let rd = get_freg(operands, 0)?;
    let rs1 = get_freg(operands, 1)?;
    let rm = if operands.len() > 2 {
        match &operands[2] {
            Operand::RoundingMode(s) => parse_rm(s),
            _ => 0b111,
        }
    } else {
        0b111
    };
    Ok(EncodeResult::Word(encode_r(
        OP_OP_FP, rd, rm, rs1, rs2, funct7,
    )))
}

pub(crate) fn encode_fp_sgnj(
    operands: &[Operand],
    funct7: u32,
    funct3: u32,
) -> Result<EncodeResult, String> {
    let rd = get_freg(operands, 0)?;
    let rs1 = get_freg(operands, 1)?;
    let rs2 = get_freg(operands, 2)?;
    Ok(EncodeResult::Word(encode_r(
        OP_OP_FP, rd, funct3, rs1, rs2, funct7,
    )))
}

pub(crate) fn encode_fp_cmp(
    operands: &[Operand],
    funct7: u32,
    funct3: u32,
) -> Result<EncodeResult, String> {
    // Result goes to integer register
    let rd = get_reg(operands, 0)?;
    let rs1 = get_freg(operands, 1)?;
    let rs2 = get_freg(operands, 2)?;
    Ok(EncodeResult::Word(encode_r(
        OP_OP_FP, rd, funct3, rs1, rs2, funct7,
    )))
}

pub(crate) fn encode_fclass(operands: &[Operand], funct7: u32) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let rs1 = get_freg(operands, 1)?;
    Ok(EncodeResult::Word(encode_r(
        OP_OP_FP, rd, 0b001, rs1, 0, funct7,
    )))
}

pub(crate) fn encode_fcvt_int(
    operands: &[Operand],
    funct7: u32,
    rs2: u32,
) -> Result<EncodeResult, String> {
    // Float to integer: result in integer register, source in float register
    let rd = get_reg(operands, 0)?;
    let rs1 = get_freg(operands, 1)?;
    let rm = if operands.len() > 2 {
        match &operands[2] {
            Operand::RoundingMode(s) => parse_rm(s),
            _ => 0b111,
        }
    } else {
        0b111
    };
    Ok(EncodeResult::Word(encode_r(
        OP_OP_FP, rd, rm, rs1, rs2, funct7,
    )))
}

pub(crate) fn encode_fcvt_from_int(
    operands: &[Operand],
    funct7: u32,
    rs2: u32,
) -> Result<EncodeResult, String> {
    // Integer to float: result in float register, source in integer register
    let rd = get_freg(operands, 0)?;
    let rs1 = get_reg(operands, 1)?;
    let rm = if operands.len() > 2 {
        match &operands[2] {
            Operand::RoundingMode(s) => parse_rm(s),
            _ => 0b111,
        }
    } else {
        0b111
    };
    Ok(EncodeResult::Word(encode_r(
        OP_OP_FP, rd, rm, rs1, rs2, funct7,
    )))
}

pub(crate) fn encode_fcvt_fp(
    operands: &[Operand],
    funct7: u32,
    rs2: u32,
) -> Result<EncodeResult, String> {
    // Float to float conversion (e.g., fcvt.s.d, fcvt.d.s)
    let rd = get_freg(operands, 0)?;
    let rs1 = get_freg(operands, 1)?;
    let rm = if operands.len() > 2 {
        match &operands[2] {
            Operand::RoundingMode(s) => parse_rm(s),
            _ => 0b111,
        }
    } else {
        0b111
    };
    Ok(EncodeResult::Word(encode_r(
        OP_OP_FP, rd, rm, rs1, rs2, funct7,
    )))
}

pub(crate) fn encode_fmv_x_f(
    operands: &[Operand],
    funct7: u32,
    _fmt: u32,
) -> Result<EncodeResult, String> {
    // Float to integer register move
    let rd = get_reg(operands, 0)?;
    let rs1 = get_freg(operands, 1)?;
    Ok(EncodeResult::Word(encode_r(
        OP_OP_FP, rd, 0b000, rs1, 0, funct7,
    )))
}

pub(crate) fn encode_fmv_f_x(
    operands: &[Operand],
    funct7: u32,
    _fmt: u32,
) -> Result<EncodeResult, String> {
    // Integer to float register move
    let rd = get_freg(operands, 0)?;
    let rs1 = get_reg(operands, 1)?;
    Ok(EncodeResult::Word(encode_r(
        OP_OP_FP, rd, 0b000, rs1, 0, funct7,
    )))
}

pub(crate) fn encode_fma(
    operands: &[Operand],
    opcode: u32,
    fmt: u32,
) -> Result<EncodeResult, String> {
    let rd = get_freg(operands, 0)?;
    let rs1 = get_freg(operands, 1)?;
    let rs2 = get_freg(operands, 2)?;
    let rs3 = get_freg(operands, 3)?;
    let rm = if operands.len() > 4 {
        match &operands[4] {
            Operand::RoundingMode(s) => parse_rm(s),
            _ => 0b111,
        }
    } else {
        0b111
    };
    // R4-type: rs3[31:27] | fmt[26:25] | rs2[24:20] | rs1[19:15] | rm[14:12] | rd[11:7] | opcode[6:0]
    let word =
        (rs3 << 27) | (fmt << 25) | (rs2 << 20) | (rs1 << 15) | (rm << 12) | (rd << 7) | opcode;
    Ok(EncodeResult::Word(word))
}
