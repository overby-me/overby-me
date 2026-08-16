use super::*;

// ── Fence ──

pub(crate) fn encode_fence(operands: &[Operand]) -> Result<EncodeResult, String> {
    let (pred, succ) = if operands.is_empty() {
        (0xF, 0xF) // fence iorw, iorw
    } else if operands.len() >= 2 {
        let pred = match &operands[0] {
            Operand::FenceArg(s) => parse_fence_bits(s),
            _ => 0xF,
        };
        let succ = match &operands[1] {
            Operand::FenceArg(s) => parse_fence_bits(s),
            _ => 0xF,
        };
        (pred, succ)
    } else {
        (0xF, 0xF)
    };
    let imm = ((pred << 4) | succ) as i32;
    Ok(EncodeResult::Word(encode_i(OP_MISC_MEM, 0, 0, 0, imm)))
}

/// Encode sfence.vma rs1, rs2
/// Format: funct7=0001001 | rs2 | rs1 | funct3=000 | rd=00000 | opcode=1110011
/// If no operands: sfence.vma zero, zero
/// If 1 operand: sfence.vma rs1, zero
/// If 2 operands: sfence.vma rs1, rs2
pub(crate) fn encode_sfence_vma(operands: &[Operand]) -> Result<EncodeResult, String> {
    let rs1 = if operands.is_empty() {
        0
    } else {
        get_reg(operands, 0)?
    };
    let rs2 = if operands.len() < 2 {
        0
    } else {
        get_reg(operands, 1)?
    };
    // sfence.vma is encoded as: funct7=0001001(0x09) | rs2 | rs1 | 000 | 00000 | SYSTEM(1110011)
    let word = encode_r(OP_SYSTEM, 0, 0b000, rs1, rs2, 0b0001001);
    Ok(EncodeResult::Word(word))
}

// ── CSR ──

pub(crate) fn encode_csr(operands: &[Operand], funct3: u32) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let csr = get_csr_num(operands, 1)?;
    // If operand 2 is a bare immediate (not a register name), use the immediate
    // CSR encoding (csrrwi/csrrsi/csrrci) instead of the register form.
    // GNU as allows e.g. `csrrc t0, sstatus, 2` and auto-selects the immediate form.
    if matches!(operands.get(2), Some(Operand::Imm(_))) {
        let zimm = get_imm(operands, 2)? as u32;
        let rs1 = zimm & 0x1F;
        let imm_funct3 = funct3 | 0b100; // 001->101, 010->110, 011->111
        return Ok(EncodeResult::Word(encode_i(
            OP_SYSTEM, rd, imm_funct3, rs1, csr as i32,
        )));
    }
    let rs1 = get_reg(operands, 2)?;
    Ok(EncodeResult::Word(encode_i(
        OP_SYSTEM, rd, funct3, rs1, csr as i32,
    )))
}

pub(crate) fn encode_csri(operands: &[Operand], funct3: u32) -> Result<EncodeResult, String> {
    let rd = get_reg(operands, 0)?;
    let csr = get_csr_num(operands, 1)?;
    let zimm = get_imm(operands, 2)? as u32;
    let rs1 = zimm & 0x1F;
    Ok(EncodeResult::Word(encode_i(
        OP_SYSTEM, rd, funct3, rs1, csr as i32,
    )))
}

pub(crate) fn get_csr_num(operands: &[Operand], idx: usize) -> Result<u32, String> {
    match operands.get(idx) {
        Some(Operand::Imm(v)) => Ok(*v as u32),
        Some(Operand::Csr(name)) => csr_name_to_num(name),
        Some(Operand::Symbol(name)) => csr_name_to_num(name),
        Some(Operand::Reg(name)) => csr_name_to_num(name), // sometimes CSR names look like regs
        other => Err(format!("expected CSR at operand {}, got {:?}", idx, other)),
    }
}

pub(crate) fn csr_name_to_num(name: &str) -> Result<u32, String> {
    match name.to_lowercase().as_str() {
        "fflags" => Ok(0x001),
        "frm" => Ok(0x002),
        "fcsr" => Ok(0x003),
        "cycle" => Ok(0xC00),
        "time" => Ok(0xC01),
        "instret" => Ok(0xC02),
        "cycleh" => Ok(0xC80),
        "timeh" => Ok(0xC81),
        "instreth" => Ok(0xC82),
        "mstatus" => Ok(0x300),
        "misa" => Ok(0x301),
        "mie" => Ok(0x304),
        "mtvec" => Ok(0x305),
        "mscratch" => Ok(0x340),
        "mepc" => Ok(0x341),
        "mcause" => Ok(0x342),
        "mtval" => Ok(0x343),
        "mip" => Ok(0x344),
        "sstatus" => Ok(0x100),
        "sip" => Ok(0x144),
        "sie" => Ok(0x104),
        "stvec" => Ok(0x105),
        "sscratch" => Ok(0x140),
        "sepc" => Ok(0x141),
        "scause" => Ok(0x142),
        "stval" => Ok(0x143),
        "satp" => Ok(0x180),
        _ => {
            // Try parsing as a number
            if let Ok(v) = name.parse::<u32>() {
                Ok(v)
            } else if let Some(hex) = name.strip_prefix("0x") {
                u32::from_str_radix(hex, 16).map_err(|_| format!("invalid CSR: {}", name))
            } else {
                Err(format!("unknown CSR: {}", name))
            }
        }
    }
}
