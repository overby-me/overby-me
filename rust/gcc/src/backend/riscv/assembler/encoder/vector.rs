use super::*;

// ── RVV (Vector) Extension Encoders ──────────────────────────────────

/// Parse a vtypei field from operands starting at `start_idx`.
/// The vtypei is specified as a sequence of operands: e.g., e8, m8, ta, ma
/// Returns the encoded vtypei value.
pub(crate) fn parse_vtypei(operands: &[Operand], start_idx: usize) -> Result<u32, String> {
    let mut sew: u32 = 0; // SEW encoding (3 bits): e8=000, e16=001, e32=010, e64=011
    let mut lmul: u32 = 0; // LMUL encoding (3 bits): m1=000, m2=001, m4=010, m8=011, mf2=111, mf4=110, mf8=101
    let mut ta: u32 = 0; // Tail agnostic
    let mut ma: u32 = 0; // Mask agnostic

    for i in start_idx..operands.len() {
        let name = match &operands[i] {
            Operand::Symbol(s) => s.to_lowercase(),
            Operand::Reg(s) => s.to_lowercase(),
            // Raw immediate: treat as pre-encoded vtypei value
            Operand::Imm(v) => return Ok(*v as u32 & 0x7FF),
            _ => continue,
        };
        match name.as_str() {
            "e8" => sew = 0b000,
            "e16" => sew = 0b001,
            "e32" => sew = 0b010,
            "e64" => sew = 0b011,
            "m1" => lmul = 0b000,
            "m2" => lmul = 0b001,
            "m4" => lmul = 0b010,
            "m8" => lmul = 0b011,
            "mf2" => lmul = 0b111,
            "mf4" => lmul = 0b110,
            "mf8" => lmul = 0b101,
            "ta" => ta = 1,
            "tu" => ta = 0,
            "ma" => ma = 1,
            "mu" => ma = 0,
            _ => return Err(format!("unknown vtypei field: {}", name)),
        }
    }

    // vtypei: [ma][ta][sew[2:0]][lmul[2:0]]
    Ok((ma << 7) | (ta << 6) | (sew << 3) | lmul)
}

/// Encode vsetvli rd, rs1, vtypei
/// Format: [0][vtypei[10:0]][rs1][111][rd][1010111]
pub(crate) fn encode_vsetvli(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let rs1 = get_reg(operands, 1)?;
    let vtypei = parse_vtypei(operands, 2)?;
    // bit 31 = 0 for vsetvli
    let word = ((vtypei & 0x7FF) << 20) | (rs1 << 15) | (0b111 << 12) | (rd << 7) | OP_V;
    Ok(EncodeResult::Word(word))
}

/// Encode vsetivli rd, uimm[4:0], vtypei
/// Format: [11][vtypei[9:0]][uimm[4:0]][111][rd][1010111]
pub(crate) fn encode_vsetivli(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let uimm = get_imm(operands, 1)? as u32 & 0x1F;
    let vtypei = parse_vtypei(operands, 2)?;
    // bits [31:30] = 11 for vsetivli
    let word = (0b11u32 << 30)
        | ((vtypei & 0x3FF) << 20)
        | (uimm << 15)
        | (0b111 << 12)
        | (rd << 7)
        | OP_V;
    Ok(EncodeResult::Word(word))
}

/// Encode vsetvl rd, rs1, rs2
/// Format: [1000000][rs2][rs1][111][rd][1010111]
pub(crate) fn encode_vsetvl(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let rs1 = get_reg(operands, 1)?;
    let rs2 = get_reg(operands, 2)?;
    let word = (0b1000000u32 << 25) | (rs2 << 20) | (rs1 << 15) | (0b111 << 12) | (rd << 7) | OP_V;
    Ok(EncodeResult::Word(word))
}

/// Encode vector unit-stride load: vle{8,16,32,64}.v vd, (rs1)
/// Format: nf[31:29] | mew[28] | mop[27:26]=00 | vm[25] | lumop[24:20] | rs1[19:15] | width[14:12] | vd[11:7] | 0000111
/// For unit-stride: mop=00, lumop=00000 (or 01011 for whole-reg/mask)
pub(crate) fn encode_vload(
    operands: &[Operand],
    width: u32,
    lumop: u32,
) -> Result<EncodeResult, String> {
    let vd = get_vreg(operands, 0)?;
    // The second operand should be a memory operand (rs1) like (a1)
    let rs1 = match operands.get(1) {
        Some(Operand::Mem { base, offset: 0 }) => {
            reg_num(base).ok_or_else(|| format!("invalid base register: {}", base))?
        }
        Some(Operand::Reg(name)) => {
            // Parenthesized register may be parsed differently
            reg_num(name).ok_or_else(|| format!("invalid register: {}", name))?
        }
        other => return Err(format!("expected (rs1) at operand 1, got {:?}", other)),
    };
    // vm=1 means unmasked (no v0.t)
    let vm: u32 = 1;
    // nf=000 (single segment), mew=0, mop=00 (bits 31:26 = 0)
    let word = (vm << 25) | (lumop << 20) | (rs1 << 15) | (width << 12) | (vd << 7) | OP_LOAD_FP;
    Ok(EncodeResult::Word(word))
}

/// Encode vector unit-stride store: vse{8,16,32,64}.v vs3, (rs1)
/// Format: nf[31:29] | mew[28] | mop[27:26]=00 | vm[25] | sumop[24:20] | rs1[19:15] | width[14:12] | vs3[11:7] | 0100111
pub(crate) fn encode_vstore(
    operands: &[Operand],
    width: u32,
    sumop: u32,
) -> Result<EncodeResult, String> {
    let vs3 = get_vreg(operands, 0)?;
    let rs1 = match operands.get(1) {
        Some(Operand::Mem { base, offset: 0 }) => {
            reg_num(base).ok_or_else(|| format!("invalid base register: {}", base))?
        }
        Some(Operand::Reg(name)) => {
            reg_num(name).ok_or_else(|| format!("invalid register: {}", name))?
        }
        other => return Err(format!("expected (rs1) at operand 1, got {:?}", other)),
    };
    let vm: u32 = 1;
    // nf=000, mew=0, mop=00 (bits 31:26 = 0)
    let word = (vm << 25) | (sumop << 20) | (rs1 << 15) | (width << 12) | (vs3 << 7) | OP_STORE_FP;
    Ok(EncodeResult::Word(word))
}

/// Encode vector arithmetic VV (vector-vector): funct6[31:26] | vm[25] | vs2[24:20] | vs1[19:15] | funct3[14:12]=000 | vd[11:7] | OP_V
pub(crate) fn encode_v_arith_vv(operands: &[Operand], funct6: u32) -> Result<EncodeResult, String> {
    let vd = get_vreg(operands, 0)?;
    let vs2 = get_vreg(operands, 1)?;
    let vs1 = get_vreg(operands, 2)?;
    let vm: u32 = 1; // unmasked
                     // funct3=000 (OPIVV)
    let word = (funct6 << 26) | (vm << 25) | (vs2 << 20) | (vs1 << 15) | (vd << 7) | OP_V;
    Ok(EncodeResult::Word(word))
}

/// Encode vector arithmetic VX (vector-scalar): funct6[31:26] | vm[25] | vs2[24:20] | rs1[19:15] | funct3[14:12]=100 | vd[11:7] | OP_V
pub(crate) fn encode_v_arith_vx(operands: &[Operand], funct6: u32) -> Result<EncodeResult, String> {
    let vd = get_vreg(operands, 0)?;
    let vs2 = get_vreg(operands, 1)?;
    let rs1 = get_reg(operands, 2)?;
    let vm: u32 = 1;
    let word =
        (funct6 << 26) | (vm << 25) | (vs2 << 20) | (rs1 << 15) | (0b100 << 12) | (vd << 7) | OP_V;
    Ok(EncodeResult::Word(word))
}

/// Encode vector arithmetic VI (vector-immediate): funct6[31:26] | vm[25] | vs2[24:20] | simm5[19:15] | funct3[14:12]=011 | vd[11:7] | OP_V
pub(crate) fn encode_v_arith_vi(operands: &[Operand], funct6: u32) -> Result<EncodeResult, String> {
    let vd = get_vreg(operands, 0)?;
    let vs2 = get_vreg(operands, 1)?;
    let simm5 = get_imm(operands, 2)? as u32 & 0x1F;
    let vm: u32 = 1;
    let word = (funct6 << 26)
        | (vm << 25)
        | (vs2 << 20)
        | (simm5 << 15)
        | (0b011 << 12)
        | (vd << 7)
        | OP_V;
    Ok(EncodeResult::Word(word))
}

/// vmv.v.v vd, vs1: OPIVV, funct6=010111, vm=1, vs2=0
pub(crate) fn encode_vmv_v_v(operands: &[Operand]) -> Result<EncodeResult, String> {
    let vd = get_vreg(operands, 0)?;
    let vs1 = get_vreg(operands, 1)?;
    // funct6=010111, vm=1, vs2=0, funct3=000 (OPIVV)
    let word = (0b010111u32 << 26) | (1u32 << 25) | (vs1 << 15) | (vd << 7) | OP_V;
    Ok(EncodeResult::Word(word))
}

/// vmv.v.x vd, rs1: OPIVX, funct6=010111, vm=1, vs2=0
pub(crate) fn encode_vmv_v_x(operands: &[Operand]) -> Result<EncodeResult, String> {
    let vd = get_vreg(operands, 0)?;
    let rs1 = get_reg(operands, 1)?;
    // funct6=010111, vm=1, vs2=0
    let word = (0b010111u32 << 26) | (1u32 << 25) | (rs1 << 15) | (0b100 << 12) | (vd << 7) | OP_V;
    Ok(EncodeResult::Word(word))
}

/// vmv.v.i vd, simm5: OPIVI, funct6=010111, vm=1, vs2=0
pub(crate) fn encode_vmv_v_i(operands: &[Operand]) -> Result<EncodeResult, String> {
    let vd = get_vreg(operands, 0)?;
    let simm5 = get_imm(operands, 1)? as u32 & 0x1F;
    // funct6=010111, vm=1, vs2=0
    let word =
        (0b010111u32 << 26) | (1u32 << 25) | (simm5 << 15) | (0b011 << 12) | (vd << 7) | OP_V;
    Ok(EncodeResult::Word(word))
}

/// vid.v vd: OPMVV, funct6=010100, vm=1, vs2=00000, rs1=10001
/// Encoding: funct6=010100 | vm=1 | vs2=00000 | 10001 | 010 | vd | OP_V
pub(crate) fn encode_vid_v(operands: &[Operand]) -> Result<EncodeResult, String> {
    let vd = get_vreg(operands, 0)?;
    // vs2=0 (bits 24:20), funct6=010100, vm=1
    let word =
        (0b010100u32 << 26) | (1u32 << 25) | (0b10001u32 << 15) | (0b010 << 12) | (vd << 7) | OP_V;
    Ok(EncodeResult::Word(word))
}

/// Encode Zvksh/Zvksed crypto instructions with VI format
/// vsm3c.vi, vsm4k.vi: funct6 | vm=1 | vs2 | uimm5 | 010 | vd | OP_V_CRYPTO
pub(crate) fn encode_v_crypto_vi(
    operands: &[Operand],
    funct6: u32,
) -> Result<EncodeResult, String> {
    let vd = get_vreg(operands, 0)?;
    let vs2 = get_vreg(operands, 1)?;
    let uimm5 = get_imm(operands, 2)? as u32 & 0x1F;
    let word = (funct6 << 26)
        | (1u32 << 25)
        | (vs2 << 20)
        | (uimm5 << 15)
        | (0b010 << 12)
        | (vd << 7)
        | OP_V_CRYPTO;
    Ok(EncodeResult::Word(word))
}

/// Encode Zvksh crypto instructions with VV format
/// vsm3me.vv: funct6 | vm=1 | vs2 | vs1 | 010 | vd | OP_V_CRYPTO
pub(crate) fn encode_v_crypto_vv(
    operands: &[Operand],
    funct6: u32,
) -> Result<EncodeResult, String> {
    let vd = get_vreg(operands, 0)?;
    let vs2 = get_vreg(operands, 1)?;
    let vs1 = get_vreg(operands, 2)?;
    let word = (funct6 << 26)
        | (1u32 << 25)
        | (vs2 << 20)
        | (vs1 << 15)
        | (0b010 << 12)
        | (vd << 7)
        | OP_V_CRYPTO;
    Ok(EncodeResult::Word(word))
}

/// Encode Zvksed crypto instructions with VS format
/// vsm4r.vs: funct6 | vm=1 | vs2 | 10000 | 010 | vd | OP_V_CRYPTO
pub(crate) fn encode_v_crypto_vs(
    operands: &[Operand],
    funct6: u32,
) -> Result<EncodeResult, String> {
    let vd = get_vreg(operands, 0)?;
    let vs2 = get_vreg(operands, 1)?;
    let word = (funct6 << 26)
        | (1u32 << 25)
        | (vs2 << 20)
        | (0b10000u32 << 15)
        | (0b010 << 12)
        | (vd << 7)
        | OP_V_CRYPTO;
    Ok(EncodeResult::Word(word))
}
