use super::*;

// ── Atomics ──

pub(crate) fn encode_lr(operands: &[Operand], funct3: u32) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let (rs1, _offset) = get_mem(operands, 1)?;
    // LR: funct7 = 00010 | aq | rl, rs2 = 0
    let funct7 = 0b0001000; // aq=0, rl=0 by default
    Ok(EncodeResult::Word(encode_r(
        OP_AMO, rd, funct3, rs1, 0, funct7,
    )))
}

pub(crate) fn encode_sc(operands: &[Operand], funct3: u32) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let rs2 = get_reg(operands, 1)?;
    let (rs1, _offset) = get_mem(operands, 2)?;
    let funct7 = 0b0001100; // SC: 00011 | aq=0 | rl=0
    Ok(EncodeResult::Word(encode_r(
        OP_AMO, rd, funct3, rs1, rs2, funct7,
    )))
}

pub(crate) fn encode_amo(
    operands: &[Operand],
    funct3: u32,
    funct5: u32,
) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let rs2 = get_reg(operands, 1)?;
    let (rs1, _offset) = get_mem(operands, 2)?;
    let funct7 = funct5 << 2; // aq=0, rl=0
    Ok(EncodeResult::Word(encode_r(
        OP_AMO, rd, funct3, rs1, rs2, funct7,
    )))
}

pub(crate) fn encode_lr_suffixed(
    mnemonic: &str,
    operands: &[Operand],
) -> Result<EncodeResult, String> {
    // Parse lr.w, lr.d, lr.w.aq, lr.w.rl, lr.w.aqrl, etc.
    let parts: Vec<&str> = mnemonic.split('.').collect();
    let funct3 = match parts.get(1).copied() {
        Some("w") => 0b010,
        Some("d") => 0b011,
        _ => return Err(format!("lr: invalid width: {}", mnemonic)),
    };
    let (aq, rl) = parse_aq_rl(&parts[2..]);
    let rd = get_reg(operands, 0)?;
    let (rs1, _) = get_mem(operands, 1)?;
    let funct7 = (0b00010 << 2) | (aq << 1) | rl;
    Ok(EncodeResult::Word(encode_r(
        OP_AMO, rd, funct3, rs1, 0, funct7,
    )))
}

pub(crate) fn encode_sc_suffixed(
    mnemonic: &str,
    operands: &[Operand],
) -> Result<EncodeResult, String> {
    let parts: Vec<&str> = mnemonic.split('.').collect();
    let funct3 = match parts.get(1).copied() {
        Some("w") => 0b010,
        Some("d") => 0b011,
        _ => return Err(format!("sc: invalid width: {}", mnemonic)),
    };
    let (aq, rl) = parse_aq_rl(&parts[2..]);
    let rd = get_reg(operands, 0)?;
    let rs2 = get_reg(operands, 1)?;
    let (rs1, _) = get_mem(operands, 2)?;
    let funct7 = (0b00011 << 2) | (aq << 1) | rl;
    Ok(EncodeResult::Word(encode_r(
        OP_AMO, rd, funct3, rs1, rs2, funct7,
    )))
}

pub(crate) fn encode_amo_suffixed(
    mnemonic: &str,
    operands: &[Operand],
) -> Result<EncodeResult, String> {
    // Parse e.g. amoswap.w.aqrl, amoadd.d.aq, etc.
    let parts: Vec<&str> = mnemonic.split('.').collect();
    if parts.len() < 2 {
        return Err(format!("amo: invalid mnemonic: {}", mnemonic));
    }

    let op_name = parts[0]; // e.g., "amoswap", "amoadd"
    let funct3 = match parts.get(1).copied() {
        Some("w") => 0b010,
        Some("d") => 0b011,
        _ => return Err(format!("amo: invalid width in {}", mnemonic)),
    };
    let (aq, rl) = parse_aq_rl(&parts[2..]);

    let funct5 = match op_name {
        "amoswap" => 0b00001,
        "amoadd" => 0b00000,
        "amoxor" => 0b00100,
        "amoand" => 0b01100,
        "amoor" => 0b01000,
        "amomin" => 0b10000,
        "amomax" => 0b10100,
        "amominu" => 0b11000,
        "amomaxu" => 0b11100,
        _ => return Err(format!("amo: unknown op: {}", op_name)),
    };

    let rd = get_reg(operands, 0)?;
    let rs2 = get_reg(operands, 1)?;
    let (rs1, _) = get_mem(operands, 2)?;
    let funct7 = (funct5 << 2) | (aq << 1) | rl;
    Ok(EncodeResult::Word(encode_r(
        OP_AMO, rd, funct3, rs1, rs2, funct7,
    )))
}

pub(crate) fn parse_aq_rl(suffixes: &[&str]) -> (u32, u32) {
    let mut aq = 0u32;
    let mut rl = 0u32;
    for s in suffixes {
        match *s {
            "aq" => aq = 1,
            "rl" => rl = 1,
            "aqrl" => {
                aq = 1;
                rl = 1;
            }
            _ => {}
        }
    }
    (aq, rl)
}
