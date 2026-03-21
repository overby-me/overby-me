use super::*;

// ── Instruction encoders ──────────────────────────────────────────────

pub(crate) fn encode_lui(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    match &operands.get(1) {
        Some(Operand::Imm(imm)) => Ok(EncodeResult::Word(encode_u(
            OP_LUI,
            rd,
            (*imm as u32) << 12,
        ))),
        Some(Operand::Symbol(s)) => {
            // %hi(symbol)
            Ok(EncodeResult::WordWithReloc {
                word: encode_u(OP_LUI, rd, 0),
                reloc: Relocation {
                    reloc_type: if s.starts_with("%tprel_hi(") {
                        RelocType::TprelHi20
                    } else {
                        RelocType::Hi20
                    },
                    symbol: extract_modifier_symbol(s),
                    addend: 0,
                },
            })
        }
        _ => Err("lui: invalid operands".to_string()),
    }
}

pub(crate) fn encode_auipc(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    match &operands.get(1) {
        Some(Operand::Imm(imm)) => Ok(EncodeResult::Word(encode_u(
            OP_AUIPC,
            rd,
            (*imm as u32) << 12,
        ))),
        Some(Operand::Symbol(s)) => {
            let (reloc_type, symbol) = parse_reloc_modifier(s);
            Ok(EncodeResult::WordWithReloc {
                word: encode_u(OP_AUIPC, rd, 0),
                reloc: Relocation {
                    reloc_type,
                    symbol,
                    addend: 0,
                },
            })
        }
        _ => Err("auipc: invalid operands".to_string()),
    }
}

pub(crate) fn encode_jal(operands: &[Operand]) -> Result<EncodeResult, String> {
    // jal rd, offset  OR  jal offset (rd = ra)
    if operands.len() == 1 {
        // jal offset (implicit rd = ra)
        match &operands[0] {
            Operand::Imm(imm) => Ok(EncodeResult::Word(encode_j(OP_JAL, 1, *imm as i32))),
            Operand::Symbol(s) | Operand::Label(s) | Operand::Reg(s) => {
                Ok(EncodeResult::WordWithReloc {
                    word: encode_j(OP_JAL, 1, 0),
                    reloc: Relocation {
                        reloc_type: RelocType::Jal,
                        symbol: s.clone(),
                        addend: 0,
                    },
                })
            }
            _ => Err("jal: invalid operand".to_string()),
        }
    } else {
        let rd = get_reg(operands, 0)?;
        match &operands[1] {
            Operand::Imm(imm) => Ok(EncodeResult::Word(encode_j(OP_JAL, rd, *imm as i32))),
            Operand::Symbol(s) | Operand::Label(s) | Operand::Reg(s) => {
                Ok(EncodeResult::WordWithReloc {
                    word: encode_j(OP_JAL, rd, 0),
                    reloc: Relocation {
                        reloc_type: RelocType::Jal,
                        symbol: s.clone(),
                        addend: 0,
                    },
                })
            }
            _ => Err("jal: invalid operand".to_string()),
        }
    }
}

pub(crate) fn encode_jalr(operands: &[Operand]) -> Result<EncodeResult, String> {
    // jalr rd, rs1, offset  OR  jalr rd, offset(rs1)  OR  jalr rs1
    match operands.len() {
        1 => {
            // jalr rs1 (rd = ra, offset = 0)
            let rs1 = get_reg(operands, 0)?;
            Ok(EncodeResult::Word(encode_i(OP_JALR, 1, 0, rs1, 0)))
        }
        2 => {
            // jalr rd, rs1  (offset = 0)
            let rd = get_reg(operands, 0)?;
            match &operands[1] {
                Operand::Reg(name) => {
                    let rs1 = reg_num(name).ok_or("invalid register")?;
                    Ok(EncodeResult::Word(encode_i(OP_JALR, rd, 0, rs1, 0)))
                }
                Operand::Mem { base, offset } => {
                    let rs1 = reg_num(base).ok_or("invalid base register")?;
                    Ok(EncodeResult::Word(encode_i(
                        OP_JALR,
                        rd,
                        0,
                        rs1,
                        *offset as i32,
                    )))
                }
                _ => Err("jalr: invalid operands".to_string()),
            }
        }
        3 => {
            let rd = get_reg(operands, 0)?;
            let rs1 = get_reg(operands, 1)?;
            let imm = get_imm(operands, 2)?;
            Ok(EncodeResult::Word(encode_i(
                OP_JALR, rd, 0, rs1, imm as i32,
            )))
        }
        _ => Err("jalr: wrong number of operands".to_string()),
    }
}

pub(crate) fn encode_branch_instr(
    operands: &[Operand],
    funct3: u32,
) -> Result<EncodeResult, String> {
    let rs1 = get_reg(operands, 0)?;
    let rs2 = get_reg(operands, 1)?;

    match &operands.get(2) {
        Some(Operand::Imm(imm)) => Ok(EncodeResult::Word(encode_b(
            OP_BRANCH,
            funct3,
            rs1,
            rs2,
            *imm as i32,
        ))),
        Some(Operand::Symbol(s)) | Some(Operand::Label(s)) | Some(Operand::Reg(s)) => {
            Ok(EncodeResult::WordWithReloc {
                word: encode_b(OP_BRANCH, funct3, rs1, rs2, 0),
                reloc: Relocation {
                    reloc_type: RelocType::Branch,
                    symbol: s.clone(),
                    addend: 0,
                },
            })
        }
        _ => Err("branch: expected offset or label as 3rd operand".to_string()),
    }
}

pub(crate) fn encode_load(operands: &[Operand], funct3: u32) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    match &operands.get(1) {
        Some(Operand::Mem { base, offset }) => {
            let rs1 = reg_num(base).ok_or("invalid base register")?;
            Ok(EncodeResult::Word(encode_i(
                OP_LOAD,
                rd,
                funct3,
                rs1,
                *offset as i32,
            )))
        }
        Some(Operand::MemSymbol { base, symbol, .. }) => {
            let rs1 = reg_num(base).ok_or("invalid base register")?;
            let (reloc_type, sym) = parse_reloc_modifier(symbol);
            // Use Lo12I for load-type relocations
            let reloc_type = match reloc_type {
                RelocType::PcrelHi20 => RelocType::PcrelLo12I,
                RelocType::Hi20 => RelocType::Lo12I,
                RelocType::TprelHi20 => RelocType::TprelLo12I,
                other => other,
            };
            Ok(EncodeResult::WordWithReloc {
                word: encode_i(OP_LOAD, rd, funct3, rs1, 0),
                reloc: Relocation {
                    reloc_type,
                    symbol: sym,
                    addend: 0,
                },
            })
        }
        // Bare symbol: "ld rd, symbol" pseudo-instruction
        // Expand to: auipc rd, %pcrel_hi(symbol) ; ld rd, 0(rd)
        // with R_RISCV_PCREL_HI20 on auipc and R_RISCV_PCREL_LO12_I on ld
        Some(Operand::Symbol(s)) | Some(Operand::Label(s)) => {
            Ok(EncodeResult::WordsWithRelocs(vec![
                (
                    encode_u(OP_AUIPC, rd, 0),
                    Some(Relocation {
                        reloc_type: RelocType::PcrelHi20,
                        symbol: s.clone(),
                        addend: 0,
                    }),
                ),
                (
                    encode_i(OP_LOAD, rd, funct3, rd, 0),
                    Some(Relocation {
                        reloc_type: RelocType::PcrelLo12I,
                        symbol: s.clone(),
                        addend: 0,
                    }),
                ),
            ]))
        }
        _ => Err("load: expected memory operand".to_string()),
    }
}

pub(crate) fn encode_store(operands: &[Operand], funct3: u32) -> Result<EncodeResult, String> {
    let rs2 = get_reg(operands, 0)?;
    match &operands.get(1) {
        Some(Operand::Mem { base, offset }) => {
            let rs1 = reg_num(base).ok_or("invalid base register")?;
            Ok(EncodeResult::Word(encode_s(
                OP_STORE,
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
                RelocType::TprelHi20 => RelocType::TprelLo12S,
                other => other,
            };
            Ok(EncodeResult::WordWithReloc {
                word: encode_s(OP_STORE, funct3, rs1, rs2, 0),
                reloc: Relocation {
                    reloc_type,
                    symbol: sym,
                    addend: 0,
                },
            })
        }
        _ => Err("store: expected memory operand".to_string()),
    }
}

pub(crate) fn encode_alu_imm(operands: &[Operand], funct3: u32) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let rs1 = get_reg(operands, 1)?;
    match &operands.get(2) {
        Some(Operand::Imm(imm)) => Ok(EncodeResult::Word(encode_i(
            OP_OP_IMM,
            rd,
            funct3,
            rs1,
            *imm as i32,
        ))),
        Some(Operand::Symbol(s)) => {
            let (reloc_type, sym) = parse_reloc_modifier(s);
            let reloc_type = match reloc_type {
                RelocType::PcrelHi20 => RelocType::PcrelLo12I,
                RelocType::Hi20 => RelocType::Lo12I,
                RelocType::TprelHi20 => RelocType::TprelLo12I,
                other => other,
            };
            Ok(EncodeResult::WordWithReloc {
                word: encode_i(OP_OP_IMM, rd, funct3, rs1, 0),
                reloc: Relocation {
                    reloc_type,
                    symbol: sym,
                    addend: 0,
                },
            })
        }
        _ => Err("alu_imm: expected immediate".to_string()),
    }
}

pub(crate) fn encode_shift_imm(
    operands: &[Operand],
    funct3: u32,
    funct6: u32,
) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let rs1 = get_reg(operands, 1)?;
    let shamt = get_imm(operands, 2)? as u32;
    // For RV64, shift amount is 6 bits
    let imm = (funct6 << 6) | (shamt & 0x3F);
    Ok(EncodeResult::Word(encode_i(
        OP_OP_IMM, rd, funct3, rs1, imm as i32,
    )))
}

pub(crate) fn encode_alu_reg(
    operands: &[Operand],
    funct3: u32,
    funct7: u32,
) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let rs1 = get_reg(operands, 1)?;
    let rs2 = get_reg(operands, 2)?;
    Ok(EncodeResult::Word(encode_r(
        OP_OP, rd, funct3, rs1, rs2, funct7,
    )))
}

pub(crate) fn encode_alu_imm_w(operands: &[Operand], funct3: u32) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let rs1 = get_reg(operands, 1)?;
    let imm = get_imm(operands, 2)? as i32;
    Ok(EncodeResult::Word(encode_i(
        OP_OP_IMM_32,
        rd,
        funct3,
        rs1,
        imm,
    )))
}

pub(crate) fn encode_shift_imm_w(
    operands: &[Operand],
    funct3: u32,
    funct7: u32,
) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let rs1 = get_reg(operands, 1)?;
    let shamt = get_imm(operands, 2)? as u32;
    // For RV32/W operations, shift amount is 5 bits
    let imm = (funct7 << 5) | (shamt & 0x1F);
    Ok(EncodeResult::Word(encode_i(
        OP_OP_IMM_32,
        rd,
        funct3,
        rs1,
        imm as i32,
    )))
}

pub(crate) fn encode_alu_reg_w(
    operands: &[Operand],
    funct3: u32,
    funct7: u32,
) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let rs1 = get_reg(operands, 1)?;
    let rs2 = get_reg(operands, 2)?;
    Ok(EncodeResult::Word(encode_r(
        OP_OP_32, rd, funct3, rs1, rs2, funct7,
    )))
}

// ── Zbb (bit manipulation) helpers ──

/// Encode a Zbb unary instruction (clz, ctz, cpop, sext.b, sext.h, rev8).
/// These are I-type with funct3=001 and the 12-bit immediate encoding the operation.
pub(crate) fn encode_zbb_unary(operands: &[Operand], imm12: u32) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let rs1 = get_reg(operands, 1)?;
    Ok(EncodeResult::Word(encode_i(
        OP_OP_IMM,
        rd,
        0b001,
        rs1,
        imm12 as i32,
    )))
}

/// Encode a Zbb unary instruction with funct3=101 (rev8, orc.b).
pub(crate) fn encode_zbb_unary_f5(
    operands: &[Operand],
    imm12: u32,
) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let rs1 = get_reg(operands, 1)?;
    Ok(EncodeResult::Word(encode_i(
        OP_OP_IMM,
        rd,
        0b101,
        rs1,
        imm12 as i32,
    )))
}

/// Encode a Zbb unary word instruction (clzw, ctzw, cpopw).
/// These are I-type on OP-IMM-32 with funct3=001.
pub(crate) fn encode_zbb_unary_w(operands: &[Operand], imm12: u32) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let rs1 = get_reg(operands, 1)?;
    Ok(EncodeResult::Word(encode_i(
        OP_OP_IMM_32,
        rd,
        0b001,
        rs1,
        imm12 as i32,
    )))
}

/// Encode zext.h rd, rs1 (R-type on OP-32: funct7=0000100, rs2=0, funct3=100).
pub(crate) fn encode_zbb_zexth(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let rs1 = get_reg(operands, 1)?;
    Ok(EncodeResult::Word(encode_r(
        OP_OP_32, rd, 0b100, rs1, 0, 0b0000100,
    )))
}
