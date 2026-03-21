use super::*;

// ── Pseudo-instruction encoders ──────────────────────────────────────

pub(crate) fn encode_li(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let imm = get_imm(operands, 1)?;

    let words = encode_li_immediate(rd, imm);
    if words.len() == 1 {
        Ok(EncodeResult::Word(words[0]))
    } else {
        Ok(EncodeResult::Words(words))
    }
}

/// Sign-extend a value from `bits` width to i64.
pub(crate) fn sign_extend_li(val: i64, bits: u32) -> i64 {
    let shift = 64 - bits;
    (val << shift) >> shift
}

/// Emit lui + addiw (or just addi) for a 32-bit signed value into register `rd`.
///
/// On RV64, the `li` pseudo-instruction uses `lui + addiw` (not `lui + addi`)
/// to ensure proper 32-bit sign extension. GAS always uses `addiw` after `lui`
/// for `li` on RV64. For small values (fits in 12 bits), `addi rd, x0, imm`
/// is sufficient since the result is the same.
pub(crate) fn encode_li_32bit(rd: u32, imm: i32) -> Vec<u32> {
    if (-2048..=2047).contains(&imm) {
        return vec![encode_i(OP_OP_IMM, rd, 0, 0, imm)]; // addi rd, x0, imm
    }
    let lo = (imm << 20) >> 20; // sign-extend low 12 bits
    let hi = ((imm as u32).wrapping_add(if lo < 0 { 0x1000 } else { 0 })) & 0xFFFFF000;
    let mut words = vec![encode_u(OP_LUI, rd, hi)];
    if lo != 0 {
        // Use addiw (OP_OP_IMM_32) to match GAS behavior on RV64.
        // lui sign-extends the 20-bit immediate to 64 bits, and addiw
        // ensures the final 32-bit result is properly sign-extended.
        words.push(encode_i(OP_OP_IMM_32, rd, 0, rd, lo)); // addiw rd, rd, lo
    }
    words
}

/// Encode `li` pseudo-instruction for an arbitrary 64-bit immediate.
///
/// Decomposes the value into a sequence of lui/addiw/slli/addi instructions.
/// For 64-bit values that don't fit in 32 bits, finds optimal shift amounts
/// such that the value = ((upper << shift1) + lo1) << shift2 + lo2 ...
/// where upper fits in 32 bits and each lo fits in 12 signed bits.
pub(crate) fn encode_li_immediate(rd: u32, imm: i64) -> Vec<u32> {
    // Case 1: fits in 12 bits (addi rd, x0, imm)
    if (-2048..=2047).contains(&imm) {
        return vec![encode_i(OP_OP_IMM, rd, 0, 0, imm as i32)];
    }

    // Case 2: fits in 32 bits (lui + addi)
    if (-0x80000000..=0x7FFFFFFF).contains(&imm) {
        return encode_li_32bit(rd, imm as i32);
    }

    // Case 3: 64-bit — try single shift: imm = (upper << shift) + lo12
    let lo12 = sign_extend_li(imm & 0xFFF, 12);
    let mut best: Option<Vec<u32>> = None;

    for shift in 12..45 {
        let remainder = imm.wrapping_sub(lo12);
        if remainder & ((1i64 << shift) - 1) != 0 {
            continue;
        }
        let upper = remainder >> shift;
        if !(-0x80000000..=0x7FFFFFFF).contains(&upper) {
            continue;
        }

        let mut words = encode_li_32bit(rd, upper as i32);
        // Convert addi to addiw after lui for proper 64-bit sign extension
        if words.len() == 2 {
            let first_opcode = words[0] & 0x7F;
            let second_opcode = words[1] & 0x7F;
            if first_opcode == OP_LUI && second_opcode == OP_OP_IMM {
                words[1] = (words[1] & !0x7F) | OP_OP_IMM_32;
            }
        }
        words.push(encode_i(OP_OP_IMM, rd, 0b001, rd, shift)); // slli
        if lo12 != 0 {
            words.push(encode_i(OP_OP_IMM, rd, 0, rd, lo12 as i32)); // addi
        }

        if best.is_none() || words.len() < best.as_ref().unwrap().len() {
            best = Some(words);
        }
    }

    if let Some(words) = best {
        return words;
    }

    // Case 4: two-level shift — imm = ((A << shift1) + lo_b) << shift2 + lo_c
    for shift2 in 12..33 {
        let remainder_c = imm.wrapping_sub(lo12);
        if remainder_c & ((1i64 << shift2) - 1) != 0 {
            continue;
        }
        let inner = remainder_c >> shift2;
        let lo12_b = sign_extend_li(inner & 0xFFF, 12);

        for shift1 in 12..33 {
            let remainder_b = inner.wrapping_sub(lo12_b);
            if remainder_b & ((1i64 << shift1) - 1) != 0 {
                continue;
            }
            let upper = remainder_b >> shift1;
            if !(-0x80000000..=0x7FFFFFFF).contains(&upper) {
                continue;
            }

            let mut words = encode_li_32bit(rd, upper as i32);
            if words.len() == 2 {
                let first_opcode = words[0] & 0x7F;
                let second_opcode = words[1] & 0x7F;
                if first_opcode == OP_LUI && second_opcode == OP_OP_IMM {
                    words[1] = (words[1] & !0x7F) | OP_OP_IMM_32;
                }
            }
            words.push(encode_i(OP_OP_IMM, rd, 0b001, rd, shift1)); // slli
            if lo12_b != 0 {
                words.push(encode_i(OP_OP_IMM, rd, 0, rd, lo12_b as i32)); // addi
            }
            words.push(encode_i(OP_OP_IMM, rd, 0b001, rd, shift2)); // slli
            if lo12 != 0 {
                words.push(encode_i(OP_OP_IMM, rd, 0, rd, lo12 as i32)); // addi
            }

            if best.is_none() || words.len() < best.as_ref().unwrap().len() {
                best = Some(words);
            }
        }
    }

    if let Some(words) = best {
        return words;
    }

    // Case 5: three-level shift (needed for dense bit patterns across all 64 bits)
    for shift3 in 12..23 {
        let rem_c = imm.wrapping_sub(lo12);
        if rem_c & ((1i64 << shift3) - 1) != 0 {
            continue;
        }
        let v2 = rem_c >> shift3;
        let lo12_b = sign_extend_li(v2 & 0xFFF, 12);

        for shift2 in 12..23 {
            let rem_b = v2.wrapping_sub(lo12_b);
            if rem_b & ((1i64 << shift2) - 1) != 0 {
                continue;
            }
            let v1 = rem_b >> shift2;
            let lo12_a = sign_extend_li(v1 & 0xFFF, 12);

            for shift1 in 12..23 {
                let rem_a = v1.wrapping_sub(lo12_a);
                if rem_a & ((1i64 << shift1) - 1) != 0 {
                    continue;
                }
                let upper = rem_a >> shift1;
                if !(-0x80000000..=0x7FFFFFFF).contains(&upper) {
                    continue;
                }

                let mut words = encode_li_32bit(rd, upper as i32);
                if words.len() == 2 {
                    let first_opcode = words[0] & 0x7F;
                    let second_opcode = words[1] & 0x7F;
                    if first_opcode == OP_LUI && second_opcode == OP_OP_IMM {
                        words[1] = (words[1] & !0x7F) | OP_OP_IMM_32;
                    }
                }
                words.push(encode_i(OP_OP_IMM, rd, 0b001, rd, shift1));
                if lo12_a != 0 {
                    words.push(encode_i(OP_OP_IMM, rd, 0, rd, lo12_a as i32));
                }
                words.push(encode_i(OP_OP_IMM, rd, 0b001, rd, shift2));
                if lo12_b != 0 {
                    words.push(encode_i(OP_OP_IMM, rd, 0, rd, lo12_b as i32));
                }
                words.push(encode_i(OP_OP_IMM, rd, 0b001, rd, shift3));
                if lo12 != 0 {
                    words.push(encode_i(OP_OP_IMM, rd, 0, rd, lo12 as i32));
                }

                if best.is_none() || words.len() < best.as_ref().unwrap().len() {
                    best = Some(words);
                }
            }
        }
    }

    if let Some(words) = best {
        return words;
    }

    // Fallback: lui + addiw + slli 32, then add lower bits via addi chain
    eprintln!("warning: li fallback for 0x{:x}", imm as u64);
    let upper = (imm >> 32) as i32;
    let mut words = encode_li_32bit(rd, upper);
    if words.len() == 2 {
        let first_opcode = words[0] & 0x7F;
        let second_opcode = words[1] & 0x7F;
        if first_opcode == OP_LUI && second_opcode == OP_OP_IMM {
            words[1] = (words[1] & !0x7F) | OP_OP_IMM_32;
        }
    }
    words.push(encode_i(OP_OP_IMM, rd, 0b001, rd, 32)); // slli rd, rd, 32
    let mut remaining = imm as i32 as i64;
    while remaining != 0 {
        let chunk = remaining.clamp(-2048, 2047);
        words.push(encode_i(OP_OP_IMM, rd, 0, rd, chunk as i32));
        remaining -= chunk;
    }
    words
}

pub(crate) fn encode_mv(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let rs = get_reg(operands, 1)?;
    // Use `add rd, x0, rs` instead of `addi rd, rs, 0` so the instruction
    // is eligible for RV64C compression to C.MV (which requires the ADD form).
    Ok(EncodeResult::Word(encode_r(
        OP_OP, rd, 0b000, 0, rs, 0b0000000,
    ))) // add rd, x0, rs
}

pub(crate) fn encode_not(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let rs1 = get_reg(operands, 1)?;
    Ok(EncodeResult::Word(encode_i(OP_OP_IMM, rd, 0b100, rs1, -1))) // xori rd, rs1, -1
}

pub(crate) fn encode_neg(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let rs2 = get_reg(operands, 1)?;
    Ok(EncodeResult::Word(encode_r(
        OP_OP, rd, 0b000, 0, rs2, 0b0100000,
    ))) // sub rd, x0, rs2
}

pub(crate) fn encode_negw(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let rs2 = get_reg(operands, 1)?;
    Ok(EncodeResult::Word(encode_r(
        OP_OP_32, rd, 0b000, 0, rs2, 0b0100000,
    )))
}

pub(crate) fn encode_sext_w(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let rs1 = get_reg(operands, 1)?;
    Ok(EncodeResult::Word(encode_i(OP_OP_IMM_32, rd, 0, rs1, 0))) // addiw rd, rs1, 0
}

pub(crate) fn encode_seqz(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let rs1 = get_reg(operands, 1)?;
    Ok(EncodeResult::Word(encode_i(OP_OP_IMM, rd, 0b011, rs1, 1))) // sltiu rd, rs1, 1
}

pub(crate) fn encode_snez(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let rs2 = get_reg(operands, 1)?;
    Ok(EncodeResult::Word(encode_r(
        OP_OP, rd, 0b011, 0, rs2, 0b0000000,
    ))) // sltu rd, x0, rs2
}

pub(crate) fn encode_sltz(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let rs1 = get_reg(operands, 1)?;
    Ok(EncodeResult::Word(encode_r(
        OP_OP, rd, 0b010, rs1, 0, 0b0000000,
    ))) // slt rd, rs1, x0
}

pub(crate) fn encode_sgtz(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let rs2 = get_reg(operands, 1)?;
    Ok(EncodeResult::Word(encode_r(
        OP_OP, rd, 0b010, 0, rs2, 0b0000000,
    ))) // slt rd, x0, rs2
}

// Branch pseudo-instructions
pub(crate) fn encode_beqz(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rs1 = get_reg(operands, 0)?;
    let label = get_branch_target(operands, 1)?;
    Ok(EncodeResult::WordWithReloc {
        word: encode_b(OP_BRANCH, 0b000, rs1, 0, 0),
        reloc: Relocation {
            reloc_type: RelocType::Branch,
            symbol: label,
            addend: 0,
        },
    })
}

pub(crate) fn encode_bnez(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rs1 = get_reg(operands, 0)?;
    let label = get_branch_target(operands, 1)?;
    Ok(EncodeResult::WordWithReloc {
        word: encode_b(OP_BRANCH, 0b001, rs1, 0, 0),
        reloc: Relocation {
            reloc_type: RelocType::Branch,
            symbol: label,
            addend: 0,
        },
    })
}

pub(crate) fn encode_blez(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rs2 = get_reg(operands, 0)?;
    let label = get_branch_target(operands, 1)?;
    Ok(EncodeResult::WordWithReloc {
        word: encode_b(OP_BRANCH, 0b101, 0, rs2, 0), // bge x0, rs
        reloc: Relocation {
            reloc_type: RelocType::Branch,
            symbol: label,
            addend: 0,
        },
    })
}

pub(crate) fn encode_bgez(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rs1 = get_reg(operands, 0)?;
    let label = get_branch_target(operands, 1)?;
    Ok(EncodeResult::WordWithReloc {
        word: encode_b(OP_BRANCH, 0b101, rs1, 0, 0), // bge rs, x0
        reloc: Relocation {
            reloc_type: RelocType::Branch,
            symbol: label,
            addend: 0,
        },
    })
}

pub(crate) fn encode_bltz(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rs1 = get_reg(operands, 0)?;
    let label = get_branch_target(operands, 1)?;
    Ok(EncodeResult::WordWithReloc {
        word: encode_b(OP_BRANCH, 0b100, rs1, 0, 0), // blt rs, x0
        reloc: Relocation {
            reloc_type: RelocType::Branch,
            symbol: label,
            addend: 0,
        },
    })
}

pub(crate) fn encode_bgtz(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rs2 = get_reg(operands, 0)?;
    let label = get_branch_target(operands, 1)?;
    Ok(EncodeResult::WordWithReloc {
        word: encode_b(OP_BRANCH, 0b100, 0, rs2, 0), // blt x0, rs
        reloc: Relocation {
            reloc_type: RelocType::Branch,
            symbol: label,
            addend: 0,
        },
    })
}

pub(crate) fn encode_bgt(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rs1 = get_reg(operands, 0)?;
    let rs2 = get_reg(operands, 1)?;
    let label = get_branch_target(operands, 2)?;
    Ok(EncodeResult::WordWithReloc {
        word: encode_b(OP_BRANCH, 0b100, rs2, rs1, 0), // blt rs2, rs1
        reloc: Relocation {
            reloc_type: RelocType::Branch,
            symbol: label,
            addend: 0,
        },
    })
}

pub(crate) fn encode_ble(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rs1 = get_reg(operands, 0)?;
    let rs2 = get_reg(operands, 1)?;
    let label = get_branch_target(operands, 2)?;
    Ok(EncodeResult::WordWithReloc {
        word: encode_b(OP_BRANCH, 0b101, rs2, rs1, 0), // bge rs2, rs1
        reloc: Relocation {
            reloc_type: RelocType::Branch,
            symbol: label,
            addend: 0,
        },
    })
}

pub(crate) fn encode_bgtu(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rs1 = get_reg(operands, 0)?;
    let rs2 = get_reg(operands, 1)?;
    let label = get_branch_target(operands, 2)?;
    Ok(EncodeResult::WordWithReloc {
        word: encode_b(OP_BRANCH, 0b110, rs2, rs1, 0), // bltu rs2, rs1
        reloc: Relocation {
            reloc_type: RelocType::Branch,
            symbol: label,
            addend: 0,
        },
    })
}

pub(crate) fn encode_bleu(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rs1 = get_reg(operands, 0)?;
    let rs2 = get_reg(operands, 1)?;
    let label = get_branch_target(operands, 2)?;
    Ok(EncodeResult::WordWithReloc {
        word: encode_b(OP_BRANCH, 0b111, rs2, rs1, 0), // bgeu rs2, rs1
        reloc: Relocation {
            reloc_type: RelocType::Branch,
            symbol: label,
            addend: 0,
        },
    })
}

pub(crate) fn get_branch_target(operands: &[Operand], idx: usize) -> Result<String, String> {
    match operands.get(idx) {
        Some(Operand::Symbol(s)) | Some(Operand::Label(s)) => Ok(s.clone()),
        Some(Operand::Imm(v)) => Ok(format!("{}", v)),
        // A register name can also be a symbol/label name (e.g. `beqz a0, t1`
        // where t1 is a label). Treat Reg as symbol in branch target context.
        Some(Operand::Reg(s)) => Ok(s.clone()),
        _ => Err(format!("expected branch target at operand {}", idx)),
    }
}

pub(crate) fn encode_j_pseudo(operands: &[Operand]) -> Result<EncodeResult, String> {
    // j offset -> jal x0, offset
    match &operands[0] {
        Operand::Symbol(s) | Operand::Label(s) | Operand::Reg(s) => {
            Ok(EncodeResult::WordWithReloc {
                word: encode_j(OP_JAL, 0, 0),
                reloc: Relocation {
                    reloc_type: RelocType::Jal,
                    symbol: s.clone(),
                    addend: 0,
                },
            })
        }
        Operand::Imm(imm) => Ok(EncodeResult::Word(encode_j(OP_JAL, 0, *imm as i32))),
        _ => Err("j: expected offset or label".to_string()),
    }
}

pub(crate) fn encode_jr(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rs1 = get_reg(operands, 0)?;
    Ok(EncodeResult::Word(encode_i(OP_JALR, 0, 0, rs1, 0)))
}

pub(crate) fn encode_call(operands: &[Operand]) -> Result<EncodeResult, String> {
    // call symbol -> auipc ra, %pcrel_hi(symbol) ; jalr ra, %pcrel_lo(symbol)(ra)
    let (symbol, addend) = get_symbol(operands, 0)?;
    Ok(EncodeResult::WordsWithRelocs(vec![
        (
            encode_u(OP_AUIPC, 1, 0),
            Some(Relocation {
                reloc_type: RelocType::CallPlt,
                symbol: symbol.clone(),
                addend,
            }),
        ),
        (encode_i(OP_JALR, 1, 0, 1, 0), None), // jalr ra, 0(ra)
    ]))
}

pub(crate) fn encode_tail(operands: &[Operand]) -> Result<EncodeResult, String> {
    // tail symbol -> auipc t1, %pcrel_hi(symbol) ; jalr x0, %pcrel_lo(symbol)(t1)
    let (symbol, addend) = get_symbol(operands, 0)?;
    Ok(EncodeResult::WordsWithRelocs(vec![
        (
            encode_u(OP_AUIPC, 6, 0),
            Some(Relocation {
                // t1 = x6
                reloc_type: RelocType::CallPlt,
                symbol: symbol.clone(),
                addend,
            }),
        ),
        (encode_i(OP_JALR, 0, 0, 6, 0), None),
    ]))
}

pub(crate) fn encode_jump(operands: &[Operand]) -> Result<EncodeResult, String> {
    // jump label, temp_reg -> auipc temp, %pcrel_hi(label) ; jalr x0, %pcrel_lo(label)(temp)
    // Our codegen emits: jump .LBB42, t6
    let (symbol, addend) = get_symbol(operands, 0)?;
    let temp = if operands.len() > 1 {
        get_reg(operands, 1)?
    } else {
        31 // t6
    };

    Ok(EncodeResult::WordsWithRelocs(vec![
        (
            encode_u(OP_AUIPC, temp, 0),
            Some(Relocation {
                reloc_type: RelocType::CallPlt,
                symbol: symbol.clone(),
                addend,
            }),
        ),
        (encode_i(OP_JALR, 0, 0, temp, 0), None),
    ]))
}

pub(crate) fn encode_la(operands: &[Operand]) -> Result<EncodeResult, String> {
    // la rd, symbol -> auipc rd, %pcrel_hi(symbol) ; addi rd, rd, %pcrel_lo(symbol)
    // TODO: For PIC, this should use GOT
    encode_lla(operands) // for now, same as lla
}

pub(crate) fn encode_lla(operands: &[Operand]) -> Result<EncodeResult, String> {
    // lla rd, symbol -> auipc rd, %pcrel_hi(symbol) ; addi rd, rd, %pcrel_lo(symbol)
    let rd = get_reg(operands, 0)?;
    let (symbol, addend) = get_symbol(operands, 1)?;

    Ok(EncodeResult::WordsWithRelocs(vec![
        (
            encode_u(OP_AUIPC, rd, 0),
            Some(Relocation {
                reloc_type: RelocType::PcrelHi20,
                symbol: symbol.clone(),
                addend,
            }),
        ),
        (
            encode_i(OP_OP_IMM, rd, 0, rd, 0),
            Some(Relocation {
                reloc_type: RelocType::PcrelLo12I,
                symbol, // TODO: This should reference the auipc label, not the symbol directly
                addend,
            }),
        ),
    ]))
}

pub(crate) fn encode_rdcsr(mnemonic: &str, operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let csr = match mnemonic {
        "rdcycle" => 0xC00,
        "rdtime" => 0xC01,
        "rdinstret" => 0xC02,
        _ => return Err(format!("unknown CSR pseudo: {}", mnemonic)),
    };
    Ok(EncodeResult::Word(encode_i(OP_SYSTEM, rd, 0b010, 0, csr))) // csrrs rd, csr, x0
}

pub(crate) fn encode_csrr(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let csr = get_csr_num(operands, 1)?;
    Ok(EncodeResult::Word(encode_i(
        OP_SYSTEM, rd, 0b010, 0, csr as i32,
    )))
}

pub(crate) fn encode_csrw(operands: &[Operand]) -> Result<EncodeResult, String> {
    let csr = get_csr_num(operands, 0)?;
    if matches!(operands.get(1), Some(Operand::Imm(_))) {
        let zimm = get_imm(operands, 1)? as u32 & 0x1F;
        return Ok(EncodeResult::Word(encode_i(
            OP_SYSTEM, 0, 0b101, zimm, csr as i32,
        )));
    }
    let rs1 = get_reg(operands, 1)?;
    Ok(EncodeResult::Word(encode_i(
        OP_SYSTEM, 0, 0b001, rs1, csr as i32,
    )))
}

pub(crate) fn encode_csrs(operands: &[Operand]) -> Result<EncodeResult, String> {
    let csr = get_csr_num(operands, 0)?;
    if matches!(operands.get(1), Some(Operand::Imm(_))) {
        let zimm = get_imm(operands, 1)? as u32 & 0x1F;
        return Ok(EncodeResult::Word(encode_i(
            OP_SYSTEM, 0, 0b110, zimm, csr as i32,
        )));
    }
    let rs1 = get_reg(operands, 1)?;
    Ok(EncodeResult::Word(encode_i(
        OP_SYSTEM, 0, 0b010, rs1, csr as i32,
    )))
}

pub(crate) fn encode_csrc(operands: &[Operand]) -> Result<EncodeResult, String> {
    let csr = get_csr_num(operands, 0)?;
    if matches!(operands.get(1), Some(Operand::Imm(_))) {
        let zimm = get_imm(operands, 1)? as u32 & 0x1F;
        return Ok(EncodeResult::Word(encode_i(
            OP_SYSTEM, 0, 0b111, zimm, csr as i32,
        )));
    }
    let rs1 = get_reg(operands, 1)?;
    Ok(EncodeResult::Word(encode_i(
        OP_SYSTEM, 0, 0b011, rs1, csr as i32,
    )))
}

// Float pseudo-instructions
pub(crate) fn encode_fmv_s(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_freg(operands, 0)?;
    let rs1 = get_freg(operands, 1)?;
    // fsgnj.s rd, rs, rs
    Ok(EncodeResult::Word(encode_r(
        OP_OP_FP, rd, 0b000, rs1, rs1, 0b0010000,
    )))
}

pub(crate) fn encode_fmv_d(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_freg(operands, 0)?;
    let rs1 = get_freg(operands, 1)?;
    Ok(EncodeResult::Word(encode_r(
        OP_OP_FP, rd, 0b000, rs1, rs1, 0b0010001,
    )))
}

pub(crate) fn encode_fabs_s(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_freg(operands, 0)?;
    let rs1 = get_freg(operands, 1)?;
    // fsgnjx.s rd, rs, rs
    Ok(EncodeResult::Word(encode_r(
        OP_OP_FP, rd, 0b010, rs1, rs1, 0b0010000,
    )))
}

pub(crate) fn encode_fabs_d(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_freg(operands, 0)?;
    let rs1 = get_freg(operands, 1)?;
    Ok(EncodeResult::Word(encode_r(
        OP_OP_FP, rd, 0b010, rs1, rs1, 0b0010001,
    )))
}

pub(crate) fn encode_fneg_s(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_freg(operands, 0)?;
    let rs1 = get_freg(operands, 1)?;
    // fsgnjn.s rd, rs, rs
    Ok(EncodeResult::Word(encode_r(
        OP_OP_FP, rd, 0b001, rs1, rs1, 0b0010000,
    )))
}

pub(crate) fn encode_fneg_d(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_freg(operands, 0)?;
    let rs1 = get_freg(operands, 1)?;
    Ok(EncodeResult::Word(encode_r(
        OP_OP_FP, rd, 0b001, rs1, rs1, 0b0010001,
    )))
}

// ── Relocation modifier parsing ──────────────────────────────────────

/// Extract symbol name from %modifier(symbol) expressions
pub(crate) fn extract_modifier_symbol(s: &str) -> String {
    if let Some(start) = s.find('(') {
        if let Some(end) = s.rfind(')') {
            return s[start + 1..end].to_string();
        }
    }
    s.to_string()
}

/// Parse a relocation modifier like %pcrel_hi(symbol) and return (RelocType, symbol)
pub(crate) fn parse_reloc_modifier(s: &str) -> (RelocType, String) {
    if s.starts_with("%pcrel_hi(") {
        (RelocType::PcrelHi20, extract_modifier_symbol(s))
    } else if s.starts_with("%pcrel_lo(") {
        (RelocType::PcrelLo12I, extract_modifier_symbol(s))
    } else if s.starts_with("%hi(") {
        (RelocType::Hi20, extract_modifier_symbol(s))
    } else if s.starts_with("%lo(") {
        (RelocType::Lo12I, extract_modifier_symbol(s))
    } else if s.starts_with("%tprel_hi(") {
        (RelocType::TprelHi20, extract_modifier_symbol(s))
    } else if s.starts_with("%tprel_lo(") {
        (RelocType::TprelLo12I, extract_modifier_symbol(s))
    } else if s.starts_with("%tprel_add(") {
        (RelocType::TprelAdd, extract_modifier_symbol(s))
    } else if s.starts_with("%got_pcrel_hi(") {
        (RelocType::GotHi20, extract_modifier_symbol(s))
    } else if s.starts_with("%tls_ie_pcrel_hi(") {
        (RelocType::TlsGotHi20, extract_modifier_symbol(s))
    } else if s.starts_with("%tls_gd_pcrel_hi(") {
        (RelocType::TlsGdHi20, extract_modifier_symbol(s))
    } else {
        // Plain symbol - use as PC-relative
        (RelocType::PcrelHi20, s.to_string())
    }
}
