//! RV64C compressed instruction support.
//!
//! Implements the RISC-V C (compressed) extension for RV64. After the assembler
//! encodes all instructions as 32-bit words, this module attempts to compress
//! eligible instructions into 16-bit (2-byte) equivalents, matching GCC's
//! default behavior for RV64GC targets.
//!
//! The compression pass runs after instruction encoding but before local branch
//! resolution, so that branch offsets are computed against the final (compressed)
//! layout.

/// Try to compress a 32-bit RISC-V instruction into a 16-bit RV64C instruction.
///
/// Returns `Some(halfword)` if the instruction can be compressed, `None` otherwise.
/// This only handles instructions without relocations; instructions with pending
/// relocations are not candidates for compression.
// Binary literals use groupings matching RISC-V compressed instruction format fields.
#[allow(clippy::unusual_byte_groupings)]
pub fn try_compress_rv64(word: u32) -> Option<u16> {
    let opcode = word & 0x7F;
    let rd = (word >> 7) & 0x1F;
    let funct3 = (word >> 12) & 0x7;
    let rs1 = (word >> 15) & 0x1F;
    let rs2 = (word >> 20) & 0x1F;
    let funct7 = (word >> 25) & 0x7F;

    match opcode {
        // ── LUI (opcode 0110111) ──
        0b0110111 => {
            // C.LUI: lui rd, imm  where rd != {x0, x2}
            // imm[17:12] from word[31:12]
            if rd == 0 || rd == 2 {
                return None;
            }
            let imm20 = (word >> 12) as i32;
            // Sign-extend the 20-bit value
            let imm20 = (imm20 << 12) >> 12; // sign-extend from 20 bits
                                             // C.LUI uses imm[17:12], so the actual nzimm is bits 17:12
                                             // which is imm20[5:0] (since LUI loads imm into [31:12])
                                             // C.LUI constraint: nzimm != 0, and it's sign-extended from 6 bits
            let nzimm = imm20; // this is the full 20-bit value
                               // C.LUI stores bits [17:12] = nzimm[5:0], sign-extended from bit 17
                               // So nzimm must fit in signed 6-bit range: -32..31 (but not 0)
            if nzimm == 0 {
                return None;
            }
            if !(-32..=31).contains(&nzimm) {
                return None;
            }
            let nzimm = nzimm as u32;
            let bit17 = (nzimm >> 5) & 1;
            let bits16_12 = nzimm & 0x1F;
            Some((0b011_0_00000_00000_01 | (bit17 << 12) | (rd << 7) | (bits16_12 << 2)) as u16)
        }

        // ── ADDI (opcode 0010011, funct3=000) ──
        0b0010011 if funct3 == 0b000 => {
            let imm = (word as i32) >> 20; // sign-extended imm[11:0]

            if rd == 0 && rs1 == 0 && imm == 0 {
                // C.NOP
                Some(0b000_0_00000_00000_01)
            } else if rd == rs1 && rd != 0 && imm != 0 && (-32..=31).contains(&imm) {
                // C.ADDI: addi rd, rd, nzimm (including sp)
                // GCC prefers C.ADDI over C.ADDI16SP when both apply
                let nzimm = imm as u32;
                let bit5 = (nzimm >> 5) & 1;
                let bits4_0 = nzimm & 0x1F;
                Some((0b000_0_00000_00000_01 | (bit5 << 12) | (rd << 7) | (bits4_0 << 2)) as u16)
            } else if rd == 2
                && rs1 == 2
                && imm != 0
                && (imm % 16) == 0
                && (-512..=496).contains(&imm)
            {
                // C.ADDI16SP: addi x2, x2, imm (for larger imm not fitting C.ADDI)
                let uimm = imm as u32;
                let bit9 = (uimm >> 9) & 1;
                let bit4 = (uimm >> 4) & 1;
                let bit6 = (uimm >> 6) & 1;
                let bits8_7 = (uimm >> 7) & 0x3;
                let bit5 = (uimm >> 5) & 1;
                Some(
                    (0b011_0_00010_00000_01
                        | (bit9 << 12)
                        | (bit4 << 6)
                        | (bit6 << 5)
                        | (bits8_7 << 3)
                        | (bit5 << 2)) as u16,
                )
            } else if rs1 == 2
                && rd != 0
                && rd != 2
                && is_creg(rd)
                && imm > 0
                && (imm % 4) == 0
                && imm <= 1020
            {
                // C.ADDI4SPN: addi rd', x2, uimm
                let uimm = imm as u32;
                let rd_prime = creg_num(rd);
                let bits5_4 = (uimm >> 4) & 0x3;
                let bits9_6 = (uimm >> 6) & 0xF;
                let bit2 = (uimm >> 2) & 1;
                let bit3 = (uimm >> 3) & 1;
                Some(
                    ((bits5_4 << 11) | (bits9_6 << 7) | (bit2 << 6) | (bit3 << 5) | (rd_prime << 2))
                        as u16,
                )
            } else if rs1 == 0 && rd != 0 {
                // C.LI: addi rd, x0, imm  (li rd, imm)
                if !(-32..=31).contains(&imm) {
                    return None;
                }
                let imm_u = imm as u32;
                let bit5 = (imm_u >> 5) & 1;
                let bits4_0 = imm_u & 0x1F;
                Some((0b010_0_00000_00000_01 | (bit5 << 12) | (rd << 7) | (bits4_0 << 2)) as u16)
            } else {
                None
            }
        }

        // ── ADDIW (opcode 0011011, funct3=000) ──
        0b0011011 if funct3 == 0b000 => {
            let imm = (word as i32) >> 20;
            if rd == rs1 && rd != 0 {
                // C.ADDIW: addiw rd, rd, imm  (imm can be 0 for sext.w)
                if !(-32..=31).contains(&imm) {
                    return None;
                }
                let imm_u = imm as u32;
                let bit5 = (imm_u >> 5) & 1;
                let bits4_0 = imm_u & 0x1F;
                Some((0b001_0_00000_00000_01 | (bit5 << 12) | (rd << 7) | (bits4_0 << 2)) as u16)
            } else if rs1 == 0 && rd != 0 {
                // addiw rd, x0, imm => also C.ADDIW if we treat rd as rd/rs1
                // Actually this would be `sext.w rd` which is `addiw rd, rd, 0`
                // This pattern doesn't match C.ADDIW since rd != rs1
                None
            } else {
                None
            }
        }

        // ── SLLI (opcode 0010011, funct3=001) ──
        0b0010011 if funct3 == 0b001 => {
            // slli rd, rs1, shamt  (funct7 encodes high bit of shamt for RV64)
            let shamt = (word >> 20) & 0x3F; // 6-bit shift for RV64
            if rd == rs1 && rd != 0 && shamt != 0 {
                // C.SLLI: slli rd, rd, shamt
                let bit5 = (shamt >> 5) & 1;
                let bits4_0 = shamt & 0x1F;
                Some((0b000_0_00000_00000_10 | (bit5 << 12) | (rd << 7) | (bits4_0 << 2)) as u16)
            } else {
                None
            }
        }

        // ── SRLI/SRAI (opcode 0010011, funct3=101) ──
        0b0010011 if funct3 == 0b101 => {
            let shamt = (word >> 20) & 0x3F;
            let is_srai = (funct7 & 0x20) != 0;
            if rd == rs1 && is_creg(rd) && shamt != 0 {
                let rd_prime = creg_num(rd);
                let bit5 = (shamt >> 5) & 1;
                let bits4_0 = shamt & 0x1F;
                if is_srai {
                    // C.SRAI
                    Some(
                        (0b100_0_01_000_00000_01 | (bit5 << 12) | (rd_prime << 7) | (bits4_0 << 2))
                            as u16,
                    )
                } else {
                    // C.SRLI
                    Some(
                        (0b100_0_00_000_00000_01 | (bit5 << 12) | (rd_prime << 7) | (bits4_0 << 2))
                            as u16,
                    )
                }
            } else {
                None
            }
        }

        // ── ANDI (opcode 0010011, funct3=111) ──
        0b0010011 if funct3 == 0b111 => {
            let imm = (word as i32) >> 20;
            if rd == rs1 && is_creg(rd) {
                if !(-32..=31).contains(&imm) {
                    return None;
                }
                let rd_prime = creg_num(rd);
                let imm_u = imm as u32;
                let bit5 = (imm_u >> 5) & 1;
                let bits4_0 = imm_u & 0x1F;
                // C.ANDI
                Some(
                    (0b100_0_10_000_00000_01 | (bit5 << 12) | (rd_prime << 7) | (bits4_0 << 2))
                        as u16,
                )
            } else {
                None
            }
        }

        // ── ADD/SUB/AND/OR/XOR/ADDW/SUBW (opcode 0110011, R-type) ──
        0b0110011 => {
            match (funct7, funct3) {
                // ADD
                (0b0000000, 0b000) => {
                    if rd == rs1 && rd != 0 && rs2 != 0 {
                        // C.ADD: add rd, rd, rs2
                        Some((0b100_1_00000_00000_10 | (rd << 7) | (rs2 << 2)) as u16)
                    } else if rs1 == 0 && rd != 0 && rs2 != 0 {
                        // C.MV: add rd, x0, rs2  (mv rd, rs2)
                        Some((0b100_0_00000_00000_10 | (rd << 7) | (rs2 << 2)) as u16)
                    } else {
                        None
                    }
                }
                // SUB
                (0b0100000, 0b000) => {
                    if rd == rs1 && is_creg(rd) && is_creg(rs2) {
                        let rd_prime = creg_num(rd);
                        let rs2_prime = creg_num(rs2);
                        // C.SUB
                        Some((0b100_0_11_000_00_000_01 | (rd_prime << 7) | (rs2_prime << 2)) as u16)
                    } else {
                        None
                    }
                }
                // XOR
                (0b0000000, 0b100) => {
                    if rd == rs1 && is_creg(rd) && is_creg(rs2) {
                        let rd_prime = creg_num(rd);
                        let rs2_prime = creg_num(rs2);
                        // C.XOR
                        Some((0b100_0_11_000_01_000_01 | (rd_prime << 7) | (rs2_prime << 2)) as u16)
                    } else {
                        None
                    }
                }
                // OR
                (0b0000000, 0b110) => {
                    if rd == rs1 && is_creg(rd) && is_creg(rs2) {
                        let rd_prime = creg_num(rd);
                        let rs2_prime = creg_num(rs2);
                        // C.OR
                        Some((0b100_0_11_000_10_000_01 | (rd_prime << 7) | (rs2_prime << 2)) as u16)
                    } else {
                        None
                    }
                }
                // AND
                (0b0000000, 0b111) => {
                    if rd == rs1 && is_creg(rd) && is_creg(rs2) {
                        let rd_prime = creg_num(rd);
                        let rs2_prime = creg_num(rs2);
                        // C.AND
                        Some((0b100_0_11_000_11_000_01 | (rd_prime << 7) | (rs2_prime << 2)) as u16)
                    } else {
                        None
                    }
                }
                _ => None,
            }
        }

        // ── ADDW/SUBW (opcode 0111011, R-type word ops) ──
        0b0111011 => {
            match (funct7, funct3) {
                // ADDW
                (0b0000000, 0b000) => {
                    if rd == rs1 && is_creg(rd) && is_creg(rs2) {
                        let rd_prime = creg_num(rd);
                        let rs2_prime = creg_num(rs2);
                        // C.ADDW
                        Some((0b100_1_11_000_01_000_01 | (rd_prime << 7) | (rs2_prime << 2)) as u16)
                    } else {
                        None
                    }
                }
                // SUBW
                (0b0100000, 0b000) => {
                    if rd == rs1 && is_creg(rd) && is_creg(rs2) {
                        let rd_prime = creg_num(rd);
                        let rs2_prime = creg_num(rs2);
                        // C.SUBW
                        Some((0b100_1_11_000_00_000_01 | (rd_prime << 7) | (rs2_prime << 2)) as u16)
                    } else {
                        None
                    }
                }
                _ => None,
            }
        }

        // ── LD (opcode 0000011, funct3=011) ──
        0b0000011 if funct3 == 0b011 => {
            let imm = (word as i32) >> 20;
            if rs1 == 2 && rd != 0 {
                // C.LDSP: ld rd, offset(sp) - offset must be multiple of 8, 0..504
                let offset = imm;
                if (0..=504).contains(&offset) && (offset % 8) == 0 {
                    let uoff = offset as u32;
                    // Encoding: 011 | uimm[5] | rd | uimm[4:3|8:6] | 10
                    let bit5 = (uoff >> 5) & 1;
                    let bits4_3 = (uoff >> 3) & 0x3;
                    let bits8_6 = (uoff >> 6) & 0x7;
                    Some(
                        (0b011_0_00000_00000_10
                            | (bit5 << 12)
                            | (rd << 7)
                            | (bits4_3 << 5)
                            | (bits8_6 << 2)) as u16,
                    )
                } else {
                    None
                }
            } else if is_creg(rs1) && is_creg(rd) {
                // C.LD: ld rd', offset(rs1') - offset must be multiple of 8, 0..248
                let offset = imm;
                if (0..=248).contains(&offset) && (offset % 8) == 0 {
                    let uoff = offset as u32;
                    let rs1_prime = creg_num(rs1);
                    let rd_prime = creg_num(rd);
                    // Encoding: 011 | uimm[5:3] | rs1' | uimm[7:6] | rd' | 00
                    let bits5_3 = (uoff >> 3) & 0x7;
                    let bits7_6 = (uoff >> 6) & 0x3;
                    Some(
                        (0b011_000_000_00_000_00
                            | (bits5_3 << 10)
                            | (rs1_prime << 7)
                            | (bits7_6 << 5)
                            | (rd_prime << 2)) as u16,
                    )
                } else {
                    None
                }
            } else {
                None
            }
        }

        // ── LW (opcode 0000011, funct3=010) ──
        0b0000011 if funct3 == 0b010 => {
            let imm = (word as i32) >> 20;
            if rs1 == 2 && rd != 0 {
                // C.LWSP: lw rd, offset(sp) - offset must be multiple of 4, 0..252
                let offset = imm;
                if (0..=252).contains(&offset) && (offset % 4) == 0 {
                    let uoff = offset as u32;
                    // Encoding: 010 | uimm[5] | rd | uimm[4:2|7:6] | 10
                    let bit5 = (uoff >> 5) & 1;
                    let bits4_2 = (uoff >> 2) & 0x7;
                    let bits7_6 = (uoff >> 6) & 0x3;
                    Some(
                        (0b010_0_00000_00000_10
                            | (bit5 << 12)
                            | (rd << 7)
                            | (bits4_2 << 4)
                            | (bits7_6 << 2)) as u16,
                    )
                } else {
                    None
                }
            } else if is_creg(rs1) && is_creg(rd) {
                // C.LW: lw rd', offset(rs1') - offset must be multiple of 4, 0..124
                let offset = imm;
                if (0..=124).contains(&offset) && (offset % 4) == 0 {
                    let uoff = offset as u32;
                    let rs1_prime = creg_num(rs1);
                    let rd_prime = creg_num(rd);
                    // Encoding: 010 | uimm[5:3] | rs1' | uimm[2|6] | rd' | 00
                    let bits5_3 = (uoff >> 3) & 0x7;
                    let bit2 = (uoff >> 2) & 1;
                    let bit6 = (uoff >> 6) & 1;
                    Some(
                        (0b010_000_000_00_000_00
                            | (bits5_3 << 10)
                            | (rs1_prime << 7)
                            | (bit2 << 6)
                            | (bit6 << 5)
                            | (rd_prime << 2)) as u16,
                    )
                } else {
                    None
                }
            } else {
                None
            }
        }

        // ── SD (opcode 0100011, funct3=011) ──
        0b0100011 if funct3 == 0b011 => {
            // S-type: imm[11:5] = funct7, imm[4:0] = rd field
            let imm11_5 = (word >> 25) as i32;
            let imm4_0 = ((word >> 7) & 0x1F) as i32;
            let imm = (imm11_5 << 5) | imm4_0;
            let imm = (imm << 20) >> 20; // sign-extend from 12 bits

            if rs1 == 2 {
                // C.SDSP: sd rs2, offset(sp) - offset must be multiple of 8, 0..504
                if (0..=504).contains(&imm) && (imm % 8) == 0 {
                    let uoff = imm as u32;
                    // Encoding: 111 | uimm[5:3|8:6] | rs2 | 10
                    let bits5_3 = (uoff >> 3) & 0x7;
                    let bits8_6 = (uoff >> 6) & 0x7;
                    Some(
                        (0b111_000000_00000_10 | (bits5_3 << 10) | (bits8_6 << 7) | (rs2 << 2))
                            as u16,
                    )
                } else {
                    None
                }
            } else if is_creg(rs1) && is_creg(rs2) {
                // C.SD: sd rs2', offset(rs1') - offset must be multiple of 8, 0..248
                if (0..=248).contains(&imm) && (imm % 8) == 0 {
                    let uoff = imm as u32;
                    let rs1_prime = creg_num(rs1);
                    let rs2_prime = creg_num(rs2);
                    // Encoding: 111 | uimm[5:3] | rs1' | uimm[7:6] | rs2' | 00
                    let bits5_3 = (uoff >> 3) & 0x7;
                    let bits7_6 = (uoff >> 6) & 0x3;
                    Some(
                        (0b111_000_000_00_000_00
                            | (bits5_3 << 10)
                            | (rs1_prime << 7)
                            | (bits7_6 << 5)
                            | (rs2_prime << 2)) as u16,
                    )
                } else {
                    None
                }
            } else {
                None
            }
        }

        // ── SW (opcode 0100011, funct3=010) ──
        0b0100011 if funct3 == 0b010 => {
            let imm11_5 = (word >> 25) as i32;
            let imm4_0 = ((word >> 7) & 0x1F) as i32;
            let imm = (imm11_5 << 5) | imm4_0;
            let imm = (imm << 20) >> 20;

            if rs1 == 2 {
                // C.SWSP: sw rs2, offset(sp) - offset must be multiple of 4, 0..252
                if (0..=252).contains(&imm) && (imm % 4) == 0 {
                    let uoff = imm as u32;
                    // Encoding: 110 | uimm[5:2|7:6] | rs2 | 10
                    let bits5_2 = (uoff >> 2) & 0xF;
                    let bits7_6 = (uoff >> 6) & 0x3;
                    Some(
                        (0b110_000000_00000_10 | (bits5_2 << 9) | (bits7_6 << 7) | (rs2 << 2))
                            as u16,
                    )
                } else {
                    None
                }
            } else if is_creg(rs1) && is_creg(rs2) {
                // C.SW: sw rs2', offset(rs1') - offset must be multiple of 4, 0..124
                if (0..=124).contains(&imm) && (imm % 4) == 0 {
                    let uoff = imm as u32;
                    let rs1_prime = creg_num(rs1);
                    let rs2_prime = creg_num(rs2);
                    // Encoding: 110 | uimm[5:3] | rs1' | uimm[2|6] | rs2' | 00
                    let bits5_3 = (uoff >> 3) & 0x7;
                    let bit2 = (uoff >> 2) & 1;
                    let bit6 = (uoff >> 6) & 1;
                    Some(
                        (0b110_000_000_00_000_00
                            | (bits5_3 << 10)
                            | (rs1_prime << 7)
                            | (bit2 << 6)
                            | (bit6 << 5)
                            | (rs2_prime << 2)) as u16,
                    )
                } else {
                    None
                }
            } else {
                None
            }
        }

        // ── FLD (opcode 0000111, funct3=011) ──
        0b0000111 if funct3 == 0b011 => {
            let imm = (word as i32) >> 20;
            if rs1 == 2 {
                // C.FLDSP: fld rd, offset(sp) - offset must be multiple of 8, 0..504
                let offset = imm;
                if (0..=504).contains(&offset) && (offset % 8) == 0 {
                    let uoff = offset as u32;
                    let bit5 = (uoff >> 5) & 1;
                    let bits4_3 = (uoff >> 3) & 0x3;
                    let bits8_6 = (uoff >> 6) & 0x7;
                    Some(
                        (0b001_0_00000_00000_10
                            | (bit5 << 12)
                            | (rd << 7)
                            | (bits4_3 << 5)
                            | (bits8_6 << 2)) as u16,
                    )
                } else {
                    None
                }
            } else if is_creg(rs1) && is_creg(rd) {
                // C.FLD: fld rd', offset(rs1') - offset must be multiple of 8, 0..248
                let offset = imm;
                if (0..=248).contains(&offset) && (offset % 8) == 0 {
                    let uoff = offset as u32;
                    let rs1_prime = creg_num(rs1);
                    let rd_prime = creg_num(rd);
                    let bits5_3 = (uoff >> 3) & 0x7;
                    let bits7_6 = (uoff >> 6) & 0x3;
                    Some(
                        (0b001_000_000_00_000_00
                            | (bits5_3 << 10)
                            | (rs1_prime << 7)
                            | (bits7_6 << 5)
                            | (rd_prime << 2)) as u16,
                    )
                } else {
                    None
                }
            } else {
                None
            }
        }

        // ── FSD (opcode 0100111, funct3=011) ──
        0b0100111 if funct3 == 0b011 => {
            let imm11_5 = (word >> 25) as i32;
            let imm4_0 = ((word >> 7) & 0x1F) as i32;
            let imm = (imm11_5 << 5) | imm4_0;
            let imm = (imm << 20) >> 20;

            if rs1 == 2 {
                // C.FSDSP: fsd rs2, offset(sp) - offset must be multiple of 8, 0..504
                if (0..=504).contains(&imm) && (imm % 8) == 0 {
                    let uoff = imm as u32;
                    let bits5_3 = (uoff >> 3) & 0x7;
                    let bits8_6 = (uoff >> 6) & 0x7;
                    Some(
                        (0b101_000000_00000_10 | (bits5_3 << 10) | (bits8_6 << 7) | (rs2 << 2))
                            as u16,
                    )
                } else {
                    None
                }
            } else if is_creg(rs1) && is_creg(rs2) {
                // C.FSD: fsd rs2', offset(rs1') - offset must be multiple of 8, 0..248
                if (0..=248).contains(&imm) && (imm % 8) == 0 {
                    let uoff = imm as u32;
                    let rs1_prime = creg_num(rs1);
                    let rs2_prime = creg_num(rs2);
                    let bits5_3 = (uoff >> 3) & 0x7;
                    let bits7_6 = (uoff >> 6) & 0x3;
                    Some(
                        (0b101_000_000_00_000_00
                            | (bits5_3 << 10)
                            | (rs1_prime << 7)
                            | (bits7_6 << 5)
                            | (rs2_prime << 2)) as u16,
                    )
                } else {
                    None
                }
            } else {
                None
            }
        }

        // ── JALR (opcode 1100111) ──
        0b1100111 => {
            let imm = (word as i32) >> 20;
            if imm == 0 && rs2 == 0 {
                if rd == 0 && rs1 != 0 {
                    // C.JR: jalr x0, 0(rs1)
                    Some((0b100_0_00000_00000_10 | (rs1 << 7)) as u16)
                } else if rd == 1 && rs1 != 0 {
                    // C.JALR: jalr x1, 0(rs1)
                    Some((0b100_1_00000_00000_10 | (rs1 << 7)) as u16)
                } else {
                    None
                }
            } else {
                None
            }
        }

        // ── JAL (opcode 1101111) ──
        0b1101111 => {
            // C.J: jal x0, offset (only if rd=x0)
            // Not compressing JAL here because it has relocations typically
            // and the offset range is limited to +-2KiB for C.J
            None
        }

        // ── BEQ/BNE (opcode 1100011) ──
        0b1100011 => {
            // C.BEQZ/C.BNEZ: beq/bne rs1', x0, offset
            // Not compressing branches here because they typically have pending
            // relocations for local labels, and we'd need to change the relocation
            // type. This would be handled separately in a more advanced compression pass.
            None
        }

        // ── EBREAK (opcode 1110011) ──
        0b1110011 => {
            if word == 0x00100073 {
                // C.EBREAK
                Some(0b100_1_00000_00000_10)
            } else {
                None
            }
        }

        _ => None,
    }
}

/// Check if a register number is in the "compressed" range (x8-x15).
/// These are the registers that can be encoded in 3-bit fields.
#[inline]
fn is_creg(reg: u32) -> bool {
    (8..=15).contains(&reg)
}

/// Convert a full register number (x8-x15) to its 3-bit compressed encoding (0-7).
#[inline]
fn creg_num(reg: u32) -> u32 {
    debug_assert!(is_creg(reg), "register x{} is not in compressed range", reg);
    reg - 8
}

/// Compress instructions in a section's data buffer.
///
/// Takes the section data (which contains 32-bit instructions) and produces
/// a compressed version where eligible instructions are replaced with 16-bit
/// equivalents. Returns the new data and an offset mapping (old_offset -> new_offset)
/// for each 4-byte boundary in the original data.
///
/// `reloc_offsets` is the set of offsets that have relocations; instructions at
/// those offsets should NOT be compressed because relocations assume 4-byte
/// instruction size.
pub fn compress_section(
    data: &[u8],
    reloc_offsets: &std::collections::HashSet<u64>,
) -> (Vec<u8>, Vec<(u64, u64)>) {
    let mut new_data = Vec::with_capacity(data.len());
    let mut offset_map = Vec::new(); // (old_offset, new_offset)

    let mut pos = 0;
    while pos < data.len() {
        let old_offset = pos as u64;
        let new_offset = new_data.len() as u64;
        offset_map.push((old_offset, new_offset));

        // Check if this is already a compressed (2-byte) instruction.
        // In RISC-V, bits [1:0] != 0b11 indicates a 16-bit instruction.
        if pos + 2 <= data.len() {
            let low_byte = data[pos];
            if (low_byte & 0x03) != 0x03 {
                // Already a 2-byte compressed instruction — pass through as-is
                new_data.extend_from_slice(&data[pos..pos + 2]);
                pos += 2;
                continue;
            }
        }

        // Need 4 bytes for a full-width instruction
        if pos + 4 > data.len() {
            // Trailing bytes — copy them directly
            new_data.extend_from_slice(&data[pos..]);
            pos = data.len();
            break;
        }

        // Don't compress instructions that have relocations
        if reloc_offsets.contains(&old_offset) {
            new_data.extend_from_slice(&data[pos..pos + 4]);
            pos += 4;
            continue;
        }

        let word = u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);

        if let Some(halfword) = try_compress_rv64(word) {
            new_data.extend_from_slice(&halfword.to_le_bytes());
        } else {
            new_data.extend_from_slice(&data[pos..pos + 4]);
        }

        pos += 4;
    }

    // Copy any trailing bytes (shouldn't happen for well-formed code)
    if pos < data.len() {
        let old_offset = pos as u64;
        let new_offset = new_data.len() as u64;
        offset_map.push((old_offset, new_offset));
        new_data.extend_from_slice(&data[pos..]);
    }

    (new_data, offset_map)
}

/// Remap an offset from old to new using the offset map.
/// If the exact offset is in the map, returns the new offset.
/// Otherwise, interpolates based on the nearest lower entry.
pub fn remap_offset(offset: u64, offset_map: &[(u64, u64)]) -> u64 {
    // Binary search for the largest old_offset <= offset
    match offset_map.binary_search_by_key(&offset, |&(old, _)| old) {
        Ok(idx) => offset_map[idx].1,
        Err(0) => offset, // Before any mapped offset
        Err(idx) => {
            // offset is between offset_map[idx-1] and offset_map[idx]
            let (prev_old, prev_new) = offset_map[idx - 1];
            let delta = offset - prev_old;
            prev_new + delta
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compress_addi_sp() {
        // addi sp, sp, -48 = 0xfd010113
        let result = try_compress_rv64(0xfd010113);
        assert!(result.is_some());
        let hw = result.unwrap();
        assert_eq!(
            hw, 0x7179,
            "c.addi sp, -48 should be 0x7179, got 0x{:04x}",
            hw
        );
    }

    #[test]
    fn test_compress_sd_ra_sp() {
        // sd ra, 40(sp) = 0x02113423
        let result = try_compress_rv64(0x02113423);
        assert!(result.is_some());
        let hw = result.unwrap();
        assert_eq!(
            hw, 0xf406,
            "c.sdsp ra, 40(sp) should be 0xf406, got 0x{:04x}",
            hw
        );
    }

    #[test]
    fn test_compress_sd_s0_sp() {
        // sd s0, 32(sp) = 0x02813023
        let result = try_compress_rv64(0x02813023);
        assert!(result.is_some());
        let hw = result.unwrap();
        assert_eq!(
            hw, 0xf022,
            "c.sdsp s0, 32(sp) should be 0xf022, got 0x{:04x}",
            hw
        );
    }

    #[test]
    fn test_compress_addi_s0_sp_48() {
        // addi s0, sp, 48 = 0x03010413
        // This is addi rd=s0(x8), rs1=sp(x2), imm=48
        // rd != rs1, and rs1=x2, rd=x8 which is in creg range
        // This should be C.ADDI4SPN: addi rd', x2, uimm (scaled x4)
        let result = try_compress_rv64(0x03010413);
        assert!(result.is_some());
        let hw = result.unwrap();
        assert_eq!(
            hw, 0x1800,
            "c.addi4spn s0, sp, 48 should be 0x1800, got 0x{:04x}",
            hw
        );
    }

    #[test]
    fn test_compress_mv_t1_t0() {
        // mv t1, t0 is now encoded as add t1, x0, t0 (0x00500333) for C.MV compatibility.
        // C.MV t1, t0: 100_0_00110_00101_10 = 0x8316
        // add x6, x0, x5 = 0x00500333
        let result = try_compress_rv64(0x00500333);
        assert!(result.is_some());
        assert_eq!(result.unwrap(), 0x8316, "c.mv t1, t0 should be 0x8316");
    }

    #[test]
    fn test_compress_ld_ra_sp() {
        // ld ra, 40(sp) = 0x02813083
        let result = try_compress_rv64(0x02813083);
        assert!(result.is_some());
        let hw = result.unwrap();
        assert_eq!(
            hw, 0x70a2,
            "c.ldsp ra, 40(sp) should be 0x70a2, got 0x{:04x}",
            hw
        );
    }

    #[test]
    fn test_compress_ld_s0_sp() {
        // ld s0, 32(sp) = 0x02013403
        let result = try_compress_rv64(0x02013403);
        assert!(result.is_some());
        let hw = result.unwrap();
        assert_eq!(
            hw, 0x7402,
            "c.ldsp s0, 32(sp) should be 0x7402, got 0x{:04x}",
            hw
        );
    }

    #[test]
    fn test_compress_addi_sp_48() {
        // addi sp, sp, 48 = 0x03010113
        let result = try_compress_rv64(0x03010113);
        assert!(result.is_some());
        let hw = result.unwrap();
        assert_eq!(
            hw, 0x6145,
            "c.addi16sp 48 should be 0x6145, got 0x{:04x}",
            hw
        );
    }

    #[test]
    fn test_compress_ret() {
        // ret = jalr x0, 0(x1) = 0x00008067
        let result = try_compress_rv64(0x00008067);
        assert!(result.is_some());
        let hw = result.unwrap();
        assert_eq!(
            hw, 0x8082,
            "c.jr ra (ret) should be 0x8082, got 0x{:04x}",
            hw
        );
    }

    #[test]
    fn test_no_compress_with_reloc() {
        // Instructions with relocations shouldn't be compressed
        // (This is handled by compress_section, not try_compress_rv64)
    }

    #[test]
    fn test_compress_ebreak() {
        let result = try_compress_rv64(0x00100073);
        assert_eq!(result, Some(0x9002));
    }

    #[test]
    fn test_compress_nop() {
        // nop = addi x0, x0, 0
        let result = try_compress_rv64(0x00000013);
        assert_eq!(result, Some(0x0001));
    }

    #[test]
    fn test_compress_li() {
        // li a0, 5 = addi a0, x0, 5 = 0x00500513
        let result = try_compress_rv64(0x00500513);
        assert!(result.is_some());
        // C.LI a0, 5: 010_0_01010_00101_01 = 0x4515 (approximation, verify below)
        let hw = result.unwrap();
        // a0 = x10, imm = 5
        // 010 | 0 | 01010 | 00101 | 01
        let expected: u16 = 0b0100_0101_0001_0101;
        assert_eq!(
            hw, expected,
            "c.li a0, 5 = 0x{:04x}, expected 0x{:04x}",
            hw, expected
        );
    }

    #[test]
    fn test_compress_slli() {
        // slli a0, a0, 3 = 0x00351513
        // a0 = x10, shamt = 3
        let word = 0x00351513;
        let result = try_compress_rv64(word);
        assert!(result.is_some());
    }
}
