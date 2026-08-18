use super::*;

// ── Explicit compressed instruction encoders ──

// c.lui rd, nzimm
pub(crate) fn encode_c_lui(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    if rd == 0 || rd == 2 {
        return Err("c.lui: rd cannot be x0 or x2".into());
    }
    let imm = get_imm(operands, 1)?;
    let nzimm = imm as i32;
    if nzimm == 0 {
        return Err("c.lui: nzimm must not be zero".into());
    }
    let bit17 = ((nzimm >> 5) & 1) as u16;
    let bits16_12 = (nzimm & 0x1F) as u16;
    Ok(EncodeResult::Half(
        0b01 | ((bits16_12 & 0x1F) << 2) | ((rd as u16) << 7) | (bit17 << 12) | (0b011 << 13),
    ))
}

// c.li rd, imm
pub(crate) fn encode_c_li(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let imm = get_imm(operands, 1)? as i32;
    let bit5 = ((imm >> 5) & 1) as u16;
    let bits4_0 = (imm & 0x1F) as u16;
    Ok(EncodeResult::Half(
        0b01 | (bits4_0 << 2) | ((rd as u16) << 7) | (bit5 << 12) | (0b010 << 13),
    ))
}

// c.addi rd, nzimm
pub(crate) fn encode_c_addi(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let imm = get_imm(operands, 1)? as i32;
    let bit5 = ((imm >> 5) & 1) as u16;
    let bits4_0 = (imm & 0x1F) as u16;
    Ok(EncodeResult::Half(
        0b01 | (bits4_0 << 2) | ((rd as u16) << 7) | (bit5 << 12),
    ))
}

// c.mv rd, rs2
pub(crate) fn encode_c_mv(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let rs2 = get_reg(operands, 1)?;
    Ok(EncodeResult::Half(
        0b10 | ((rs2 as u16) << 2) | ((rd as u16) << 7) | (0b100 << 13),
    ))
}

// c.add rd, rs2
pub(crate) fn encode_c_add(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let rs2 = get_reg(operands, 1)?;
    Ok(EncodeResult::Half(
        0b10 | ((rs2 as u16) << 2) | ((rd as u16) << 7) | (1 << 12) | (0b100 << 13),
    ))
}

// c.jr rs1
pub(crate) fn encode_c_jr(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rs1 = get_reg(operands, 0)?;
    Ok(EncodeResult::Half(
        0b10 | ((rs1 as u16) << 7) | (0b100 << 13),
    ))
}

// c.jalr rs1
pub(crate) fn encode_c_jalr(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rs1 = get_reg(operands, 0)?;
    Ok(EncodeResult::Half(
        0b10 | ((rs1 as u16) << 7) | (1 << 12) | (0b100 << 13),
    ))
}

// ── .insn directive encoder ──

/// Encode an .insn directive that allows arbitrary instruction encoding
pub fn encode_insn_directive(args: &str) -> Result<EncodeResult, String> {
    let args = args.trim();

    // Parse the format: .insn <format> <opcode>, <funct3>, <rd>, <rs1>, <imm>
    // or .insn <format> <opcode>, <funct3>, <funct7>, <rd>, <rs1>, <rs2>
    let parts: Vec<&str> = args
        .splitn(2, |c: char| c.is_whitespace() || c == ',')
        .collect();
    if parts.is_empty() {
        return Err("empty .insn directive".into());
    }

    let format = parts[0].trim().to_lowercase();

    // Get remaining args after the format keyword
    let rest = if parts.len() > 1 {
        parts[1].trim_start_matches(',').trim()
    } else {
        ""
    };
    let fields: Vec<&str> = rest
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();

    match format.as_str() {
        "r" => encode_insn_r(&fields),
        "i" => encode_insn_i(&fields),
        "s" => encode_insn_s(&fields),
        "b" | "sb" => encode_insn_b(&fields),
        "u" => encode_insn_u(&fields),
        "j" | "uj" => encode_insn_j(&fields),
        // Raw 32-bit word: .insn 0x12345678
        _ => {
            // Try parsing as a raw 32-bit value
            if let Ok(word) = parse_insn_int(parts[0]) {
                Ok(EncodeResult::Word(word as u32))
            } else {
                Err(format!("unsupported .insn format: {}", format))
            }
        }
    }
}

pub(crate) fn parse_insn_int(s: &str) -> Result<i64, String> {
    let s = s.trim();
    if s.starts_with("0x") || s.starts_with("0X") {
        i64::from_str_radix(&s[2..], 16).map_err(|e| format!("invalid hex in .insn: {}: {}", s, e))
    } else if s.starts_with("0b") || s.starts_with("0B") {
        i64::from_str_radix(&s[2..], 2).map_err(|e| format!("invalid bin in .insn: {}: {}", s, e))
    } else {
        s.parse::<i64>()
            .map_err(|e| format!("invalid int in .insn: {}: {}", s, e))
    }
}

pub(crate) fn parse_insn_reg(s: &str) -> Result<u32, String> {
    let s = s.trim();
    reg_num(s).ok_or_else(|| format!("invalid register in .insn: {}", s))
}

pub(crate) fn encode_insn_r(fields: &[&str]) -> Result<EncodeResult, String> {
    // .insn r opcode, funct3, funct7, rd, rs1, rs2
    if fields.len() < 6 {
        return Err(format!(
            ".insn r requires 6 fields (opcode, funct3, funct7, rd, rs1, rs2), got {}",
            fields.len()
        ));
    }
    let opcode = parse_insn_int(fields[0])? as u32;
    let funct3 = parse_insn_int(fields[1])? as u32;
    let funct7 = parse_insn_int(fields[2])? as u32;
    let rd = parse_insn_reg(fields[3])?;
    let rs1 = parse_insn_reg(fields[4])?;
    let rs2 = parse_insn_reg(fields[5])?;
    Ok(EncodeResult::Word(encode_r(
        opcode, rd, funct3, rs1, rs2, funct7,
    )))
}

pub(crate) fn encode_insn_i(fields: &[&str]) -> Result<EncodeResult, String> {
    // .insn i opcode, funct3, rd, rs1, imm
    if fields.len() < 5 {
        return Err(format!(
            ".insn i requires 5 fields (opcode, funct3, rd, rs1, imm), got {}",
            fields.len()
        ));
    }
    let opcode = parse_insn_int(fields[0])? as u32;
    let funct3 = parse_insn_int(fields[1])? as u32;
    let rd = parse_insn_reg(fields[2])?;
    let rs1 = parse_insn_reg(fields[3])?;
    let imm = parse_insn_int(fields[4])? as i32;
    Ok(EncodeResult::Word(encode_i(opcode, rd, funct3, rs1, imm)))
}

pub(crate) fn encode_insn_s(fields: &[&str]) -> Result<EncodeResult, String> {
    // .insn s opcode, funct3, rs2, imm(rs1)
    if fields.len() < 4 {
        return Err(format!(".insn s requires 4 fields, got {}", fields.len()));
    }
    let opcode = parse_insn_int(fields[0])? as u32;
    let funct3 = parse_insn_int(fields[1])? as u32;
    let rs2 = parse_insn_reg(fields[2])?;
    // Parse imm(rs1)
    let last = fields[3].trim();
    if let Some(paren_pos) = last.find('(') {
        let imm_str = &last[..paren_pos];
        let rs1_str = last[paren_pos + 1..].trim_end_matches(')');
        let imm = parse_insn_int(imm_str)? as i32;
        let rs1 = parse_insn_reg(rs1_str)?;
        Ok(EncodeResult::Word(encode_s(opcode, funct3, rs1, rs2, imm)))
    } else {
        Err(".insn s: expected imm(rs1) format for last field".into())
    }
}

pub(crate) fn encode_insn_b(fields: &[&str]) -> Result<EncodeResult, String> {
    // .insn b/sb opcode, funct3, rs1, rs2, offset
    if fields.len() < 5 {
        return Err(format!(".insn b requires 5 fields, got {}", fields.len()));
    }
    let opcode = parse_insn_int(fields[0])? as u32;
    let funct3 = parse_insn_int(fields[1])? as u32;
    let rs1 = parse_insn_reg(fields[2])?;
    let rs2 = parse_insn_reg(fields[3])?;
    let imm = parse_insn_int(fields[4])? as i32;
    Ok(EncodeResult::Word(encode_b(opcode, funct3, rs1, rs2, imm)))
}

pub(crate) fn encode_insn_u(fields: &[&str]) -> Result<EncodeResult, String> {
    // .insn u opcode, rd, imm
    if fields.len() < 3 {
        return Err(format!(".insn u requires 3 fields, got {}", fields.len()));
    }
    let opcode = parse_insn_int(fields[0])? as u32;
    let rd = parse_insn_reg(fields[1])?;
    let imm = parse_insn_int(fields[2])? as u32;
    Ok(EncodeResult::Word(encode_u(opcode, rd, imm)))
}

pub(crate) fn encode_insn_j(fields: &[&str]) -> Result<EncodeResult, String> {
    // .insn j/uj opcode, rd, imm
    if fields.len() < 3 {
        return Err(format!(".insn j requires 3 fields, got {}", fields.len()));
    }
    let opcode = parse_insn_int(fields[0])? as u32;
    let rd = parse_insn_reg(fields[1])?;
    let imm = parse_insn_int(fields[2])? as i32;
    Ok(EncodeResult::Word(encode_j(opcode, rd, imm)))
}
