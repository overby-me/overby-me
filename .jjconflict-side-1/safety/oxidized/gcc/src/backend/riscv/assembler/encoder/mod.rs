//! RISC-V instruction encoder.
//!
//! Encodes RISC-V instructions into 32-bit machine code words.
//! This covers the subset of instructions emitted by our codegen (RV64GC + Zbb).
//!
//! RISC-V base instructions are always 4 bytes (32 bits), little-endian.
//! The encoding uses six main formats: R, I, S, B, U, J.

// Encoding helpers for all RISC-V instruction formats; not all formats used yet.
#![allow(dead_code)]

mod atomics;
mod base;
mod compressed;
mod float;
mod pseudo;
mod system;
mod vector;

pub(crate) use atomics::*;
pub(crate) use base::*;
pub(crate) use compressed::*;
pub(crate) use float::*;
pub(crate) use pseudo::*;
pub(crate) use system::*;
pub(crate) use vector::*;

use super::parser::Operand;

/// Result of encoding an instruction.
#[derive(Debug, Clone)]
pub enum EncodeResult {
    /// Successfully encoded as a 4-byte instruction word
    Word(u32),
    /// Successfully encoded as a 2-byte compressed instruction
    Half(u16),
    /// Two 4-byte instruction words (e.g., pseudo-instructions like `call`, `li` with large imm)
    Words(Vec<u32>),
    /// Instruction needs a relocation to be applied later
    WordWithReloc { word: u32, reloc: Relocation },
    /// Multiple words with relocations (e.g., `call` = auipc + jalr)
    WordsWithRelocs(Vec<(u32, Option<Relocation>)>),
    /// Skip this instruction (e.g., pseudo handled elsewhere)
    Skip,
}

/// RISC-V ELF relocation types
#[derive(Debug, Clone)]
pub enum RelocType {
    /// R_RISCV_CALL_PLT (combined auipc+jalr, 8 bytes)
    CallPlt,
    /// R_RISCV_PCREL_HI20 - for AUIPC (high 20 bits of PC-relative)
    PcrelHi20,
    /// R_RISCV_PCREL_LO12_I - for ADDI/LW/LD (low 12 bits of PC-relative, I-type)
    PcrelLo12I,
    /// R_RISCV_PCREL_LO12_S - for SW/SD (low 12 bits of PC-relative, S-type)
    PcrelLo12S,
    /// R_RISCV_HI20 - for LUI (absolute high 20 bits)
    Hi20,
    /// R_RISCV_LO12_I - for ADDI/LW/LD (absolute low 12 bits, I-type)
    Lo12I,
    /// R_RISCV_LO12_S - for SW/SD (absolute low 12 bits, S-type)
    Lo12S,
    /// R_RISCV_BRANCH - 12-bit PC-relative branch (B-type)
    Branch,
    /// R_RISCV_JAL - 20-bit PC-relative jump (J-type)
    Jal,
    /// R_RISCV_64 - 64-bit absolute
    Abs64,
    /// R_RISCV_32 - 32-bit absolute
    Abs32,
    /// R_RISCV_GOT_HI20 - GOT-relative AUIPC
    GotHi20,
    /// R_RISCV_TLS_GD_HI20
    TlsGdHi20,
    /// R_RISCV_TLS_GOT_HI20
    TlsGotHi20,
    /// R_RISCV_TPREL_HI20
    TprelHi20,
    /// R_RISCV_TPREL_LO12_I
    TprelLo12I,
    /// R_RISCV_TPREL_LO12_S
    TprelLo12S,
    /// R_RISCV_TPREL_ADD
    TprelAdd,
    /// R_RISCV_ADD16 - 16-bit addition (for symbol differences)
    Add16,
    /// R_RISCV_SUB16 - 16-bit subtraction (for symbol differences)
    Sub16,
    /// R_RISCV_ADD32 - 32-bit addition (for symbol differences)
    Add32,
    /// R_RISCV_SUB32 - 32-bit subtraction (for symbol differences)
    Sub32,
    /// R_RISCV_ADD64 - 64-bit addition (for symbol differences)
    Add64,
    /// R_RISCV_SUB64 - 64-bit subtraction (for symbol differences)
    Sub64,
}

impl RelocType {
    /// Get the ELF relocation type number.
    pub fn elf_type(&self) -> u32 {
        match self {
            RelocType::Branch => 16,     // R_RISCV_BRANCH
            RelocType::Jal => 17,        // R_RISCV_JAL
            RelocType::CallPlt => 19,    // R_RISCV_CALL_PLT
            RelocType::GotHi20 => 20,    // R_RISCV_GOT_HI20
            RelocType::TlsGdHi20 => 22,  // R_RISCV_TLS_GD_HI20
            RelocType::TlsGotHi20 => 21, // R_RISCV_TLS_GOT_HI20
            RelocType::PcrelHi20 => 23,  // R_RISCV_PCREL_HI20 = 23
            RelocType::PcrelLo12I => 24, // R_RISCV_PCREL_LO12_I = 24
            RelocType::PcrelLo12S => 25, // R_RISCV_PCREL_LO12_S = 25
            RelocType::Hi20 => 26,       // R_RISCV_HI20
            RelocType::Lo12I => 27,      // R_RISCV_LO12_I
            RelocType::Lo12S => 28,      // R_RISCV_LO12_S
            RelocType::TprelHi20 => 29,  // R_RISCV_TPREL_HI20
            RelocType::TprelLo12I => 30, // R_RISCV_TPREL_LO12_I
            RelocType::TprelLo12S => 31, // R_RISCV_TPREL_LO12_S
            RelocType::TprelAdd => 32,   // R_RISCV_TPREL_ADD
            RelocType::Abs32 => 1,       // R_RISCV_32
            RelocType::Abs64 => 2,       // R_RISCV_64
            RelocType::Add16 => 34,      // R_RISCV_ADD16
            RelocType::Sub16 => 38,      // R_RISCV_SUB16
            RelocType::Add32 => 35,      // R_RISCV_ADD32
            RelocType::Sub32 => 39,      // R_RISCV_SUB32
            RelocType::Add64 => 36,      // R_RISCV_ADD64
            RelocType::Sub64 => 40,      // R_RISCV_SUB64
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

// ── Register encoding ──────────────────────────────────────────────────

/// Parse a register name to its 5-bit encoding number (0-31).
pub fn reg_num(name: &str) -> Option<u32> {
    let name = name.to_lowercase();
    match name.as_str() {
        // ABI names for integer registers
        "zero" => Some(0),
        "ra" => Some(1),
        "sp" => Some(2),
        "gp" => Some(3),
        "tp" => Some(4),
        "t0" => Some(5),
        "t1" => Some(6),
        "t2" => Some(7),
        "s0" | "fp" => Some(8),
        "s1" => Some(9),
        "a0" => Some(10),
        "a1" => Some(11),
        "a2" => Some(12),
        "a3" => Some(13),
        "a4" => Some(14),
        "a5" => Some(15),
        "a6" => Some(16),
        "a7" => Some(17),
        "s2" => Some(18),
        "s3" => Some(19),
        "s4" => Some(20),
        "s5" => Some(21),
        "s6" => Some(22),
        "s7" => Some(23),
        "s8" => Some(24),
        "s9" => Some(25),
        "s10" => Some(26),
        "s11" => Some(27),
        "t3" => Some(28),
        "t4" => Some(29),
        "t5" => Some(30),
        "t6" => Some(31),
        _ => {
            // x0-x31
            if let Some(rest) = name.strip_prefix('x') {
                let n: u32 = rest.parse().ok()?;
                if n <= 31 {
                    Some(n)
                } else {
                    None
                }
            } else {
                None
            }
        }
    }
}

/// Parse a floating-point register name to its 5-bit encoding (0-31).
pub fn freg_num(name: &str) -> Option<u32> {
    let name = name.to_lowercase();
    match name.as_str() {
        "ft0" => Some(0),
        "ft1" => Some(1),
        "ft2" => Some(2),
        "ft3" => Some(3),
        "ft4" => Some(4),
        "ft5" => Some(5),
        "ft6" => Some(6),
        "ft7" => Some(7),
        "fs0" => Some(8),
        "fs1" => Some(9),
        "fa0" => Some(10),
        "fa1" => Some(11),
        "fa2" => Some(12),
        "fa3" => Some(13),
        "fa4" => Some(14),
        "fa5" => Some(15),
        "fa6" => Some(16),
        "fa7" => Some(17),
        "fs2" => Some(18),
        "fs3" => Some(19),
        "fs4" => Some(20),
        "fs5" => Some(21),
        "fs6" => Some(22),
        "fs7" => Some(23),
        "fs8" => Some(24),
        "fs9" => Some(25),
        "fs10" => Some(26),
        "fs11" => Some(27),
        "ft8" => Some(28),
        "ft9" => Some(29),
        "ft10" => Some(30),
        "ft11" => Some(31),
        _ => {
            // f0-f31
            if name.starts_with('f')
                && !name.starts_with("ft")
                && !name.starts_with("fs")
                && !name.starts_with("fa")
            {
                let n: u32 = name[1..].parse().ok()?;
                if n <= 31 {
                    Some(n)
                } else {
                    None
                }
            } else {
                None
            }
        }
    }
}

/// Parse a vector register name to its 5-bit encoding (0-31).
/// Handles v0-v31.
pub fn vreg_num(name: &str) -> Option<u32> {
    let name = name.to_lowercase();
    if let Some(rest) = name.strip_prefix('v') {
        // Avoid matching "vector" etc. - must be just digits after 'v'
        let n: u32 = rest.parse().ok()?;
        if n <= 31 {
            Some(n)
        } else {
            None
        }
    } else {
        None
    }
}

/// Try integer register first, then float register
fn any_reg_num(name: &str) -> Option<u32> {
    reg_num(name).or_else(|| freg_num(name))
}

/// Check if a register name is an integer register
fn is_int_reg(name: &str) -> bool {
    reg_num(name).is_some()
}

/// Check if a register name is a floating-point register
fn is_fp_reg(name: &str) -> bool {
    freg_num(name).is_some()
}

// ── Instruction format encoders ──────────────────────────────────────

/// R-type: funct7[31:25] | rs2[24:20] | rs1[19:15] | funct3[14:12] | rd[11:7] | opcode[6:0]
fn encode_r(opcode: u32, rd: u32, funct3: u32, rs1: u32, rs2: u32, funct7: u32) -> u32 {
    (funct7 << 25) | (rs2 << 20) | (rs1 << 15) | (funct3 << 12) | (rd << 7) | opcode
}

/// I-type: imm[31:20] | rs1[19:15] | funct3[14:12] | rd[11:7] | opcode[6:0]
fn encode_i(opcode: u32, rd: u32, funct3: u32, rs1: u32, imm: i32) -> u32 {
    let imm = (imm as u32) & 0xFFF;
    (imm << 20) | (rs1 << 15) | (funct3 << 12) | (rd << 7) | opcode
}

/// S-type: imm[11:5] | rs2[24:20] | rs1[19:15] | funct3[14:12] | imm[4:0] | opcode[6:0]
fn encode_s(opcode: u32, funct3: u32, rs1: u32, rs2: u32, imm: i32) -> u32 {
    let imm = imm as u32;
    let imm_11_5 = (imm >> 5) & 0x7F;
    let imm_4_0 = imm & 0x1F;
    (imm_11_5 << 25) | (rs2 << 20) | (rs1 << 15) | (funct3 << 12) | (imm_4_0 << 7) | opcode
}

/// B-type: imm[12|10:5] | rs2 | rs1 | funct3 | imm[4:1|11] | opcode
fn encode_b(opcode: u32, funct3: u32, rs1: u32, rs2: u32, imm: i32) -> u32 {
    let imm = imm as u32;
    let bit12 = (imm >> 12) & 1;
    let bit11 = (imm >> 11) & 1;
    let bits10_5 = (imm >> 5) & 0x3F;
    let bits4_1 = (imm >> 1) & 0xF;
    (bit12 << 31)
        | (bits10_5 << 25)
        | (rs2 << 20)
        | (rs1 << 15)
        | (funct3 << 12)
        | (bits4_1 << 8)
        | (bit11 << 7)
        | opcode
}

/// U-type: imm[31:12] | rd[11:7] | opcode[6:0]
fn encode_u(opcode: u32, rd: u32, imm: u32) -> u32 {
    (imm & 0xFFFFF000) | (rd << 7) | opcode
}

/// J-type: imm[20|10:1|11|19:12] | rd[11:7] | opcode[6:0]
fn encode_j(opcode: u32, rd: u32, imm: i32) -> u32 {
    let imm = imm as u32;
    let bit20 = (imm >> 20) & 1;
    let bits10_1 = (imm >> 1) & 0x3FF;
    let bit11 = (imm >> 11) & 1;
    let bits19_12 = (imm >> 12) & 0xFF;
    (bit20 << 31) | (bits10_1 << 21) | (bit11 << 20) | (bits19_12 << 12) | (rd << 7) | opcode
}

// ── Opcode constants ──────────────────────────────────────────────────

const OP_LUI: u32 = 0b0110111;
const OP_AUIPC: u32 = 0b0010111;
const OP_JAL: u32 = 0b1101111;
const OP_JALR: u32 = 0b1100111;
const OP_BRANCH: u32 = 0b1100011;
const OP_LOAD: u32 = 0b0000011;
const OP_STORE: u32 = 0b0100011;
const OP_OP_IMM: u32 = 0b0010011;
const OP_OP: u32 = 0b0110011;
const OP_OP_IMM_32: u32 = 0b0011011;
const OP_OP_32: u32 = 0b0111011;
const OP_SYSTEM: u32 = 0b1110011;
const OP_MISC_MEM: u32 = 0b0001111;
const OP_AMO: u32 = 0b0101111;
const OP_LOAD_FP: u32 = 0b0000111;
const OP_STORE_FP: u32 = 0b0100111;
const OP_OP_FP: u32 = 0b1010011;
const OP_FMADD: u32 = 0b1000011;
const OP_FMSUB: u32 = 0b1000111;
const OP_FNMSUB: u32 = 0b1001011;
const OP_FNMADD: u32 = 0b1001111;
const OP_V: u32 = 0b1010111; // Vector arithmetic/config (RVV 1.0)
const OP_V_CRYPTO: u32 = 0b1110111; // Vector crypto (Zvk*) — uses OP-P encoding space per RVV Crypto spec

// ── Helper functions ──────────────────────────────────────────────────

fn get_reg(operands: &[Operand], idx: usize) -> Result<u32, String> {
    match operands.get(idx) {
        Some(Operand::Reg(name)) => {
            reg_num(name).ok_or_else(|| format!("invalid integer register: {}", name))
        }
        // GCC sometimes emits bare register numbers (0-31) in inline asm
        Some(Operand::Imm(n)) if *n >= 0 && *n <= 31 => Ok(*n as u32),
        other => Err(format!(
            "expected register at operand {}, got {:?}",
            idx, other
        )),
    }
}

fn get_freg(operands: &[Operand], idx: usize) -> Result<u32, String> {
    match operands.get(idx) {
        Some(Operand::Reg(name)) => {
            freg_num(name).ok_or_else(|| format!("invalid float register: {}", name))
        }
        other => Err(format!(
            "expected float register at operand {}, got {:?}",
            idx, other
        )),
    }
}

fn get_any_reg(operands: &[Operand], idx: usize) -> Result<u32, String> {
    match operands.get(idx) {
        Some(Operand::Reg(name)) => {
            any_reg_num(name).ok_or_else(|| format!("invalid register: {}", name))
        }
        // GCC sometimes emits bare register numbers (0-31) in inline asm
        Some(Operand::Imm(n)) if *n >= 0 && *n <= 31 => Ok(*n as u32),
        other => Err(format!(
            "expected register at operand {}, got {:?}",
            idx, other
        )),
    }
}

fn get_vreg(operands: &[Operand], idx: usize) -> Result<u32, String> {
    match operands.get(idx) {
        Some(Operand::Reg(name)) => {
            vreg_num(name).ok_or_else(|| format!("invalid vector register: {}", name))
        }
        other => Err(format!(
            "expected vector register at operand {}, got {:?}",
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
        // Register names like "f1", "a0", "ra", "zero", "s1" etc. can also be
        // symbol names (e.g. `call f1` where f1 is a function). When an encoder
        // expects a symbol operand, treat a Reg as a symbol name.
        Some(Operand::Reg(s)) => Ok((s.clone(), 0)),
        other => Err(format!(
            "expected symbol at operand {}, got {:?}",
            idx, other
        )),
    }
}

fn get_mem(operands: &[Operand], idx: usize) -> Result<(u32, i64), String> {
    match operands.get(idx) {
        Some(Operand::Mem { base, offset }) => {
            let base_reg =
                reg_num(base).ok_or_else(|| format!("invalid base register: {}", base))?;
            Ok((base_reg, *offset))
        }
        other => Err(format!(
            "expected memory operand at operand {}, got {:?}",
            idx, other
        )),
    }
}

/// Parse a fence ordering string (e.g., "iorw") into a 4-bit mask.
fn parse_fence_bits(s: &str) -> u32 {
    let s = s.to_lowercase();
    let mut bits = 0u32;
    if s.contains('i') {
        bits |= 8;
    }
    if s.contains('o') {
        bits |= 4;
    }
    if s.contains('r') {
        bits |= 2;
    }
    if s.contains('w') {
        bits |= 1;
    }
    bits
}

/// Parse a rounding mode to 3-bit encoding.
fn parse_rm(s: &str) -> u32 {
    match s.to_lowercase().as_str() {
        "rne" => 0b000,
        "rtz" => 0b001,
        "rdn" => 0b010,
        "rup" => 0b011,
        "rmm" => 0b100,
        "dyn" => 0b111,
        _ => 0b111, // default to dynamic
    }
}

// ── Main encode function ──────────────────────────────────────────────

/// Encode a RISC-V instruction from its mnemonic and parsed operands.
pub fn encode_instruction(
    mnemonic: &str,
    operands: &[Operand],
    raw_operands: &str,
) -> Result<EncodeResult, String> {
    let mn = mnemonic.to_lowercase();

    match mn.as_str() {
        // ── RV64I Base Instructions ──

        // U-type
        "lui" => encode_lui(operands),
        "auipc" => encode_auipc(operands),

        // J-type
        "jal" => encode_jal(operands),
        "jalr" => encode_jalr(operands),

        // B-type branches
        "beq" => encode_branch_instr(operands, 0b000),
        "bne" => encode_branch_instr(operands, 0b001),
        "blt" => encode_branch_instr(operands, 0b100),
        "bge" => encode_branch_instr(operands, 0b101),
        "bltu" => encode_branch_instr(operands, 0b110),
        "bgeu" => encode_branch_instr(operands, 0b111),

        // Loads (I-type)
        "lb" => encode_load(operands, 0b000),
        "lh" => encode_load(operands, 0b001),
        "lw" => encode_load(operands, 0b010),
        "ld" => encode_load(operands, 0b011),
        "lbu" => encode_load(operands, 0b100),
        "lhu" => encode_load(operands, 0b101),
        "lwu" => encode_load(operands, 0b110),

        // Stores (S-type)
        "sb" => encode_store(operands, 0b000),
        "sh" => encode_store(operands, 0b001),
        "sw" => encode_store(operands, 0b010),
        "sd" => encode_store(operands, 0b011),

        // Immediate arithmetic (I-type)
        "addi" => encode_alu_imm(operands, 0b000),
        "slti" => encode_alu_imm(operands, 0b010),
        "sltiu" => encode_alu_imm(operands, 0b011),
        "xori" => encode_alu_imm(operands, 0b100),
        "ori" => encode_alu_imm(operands, 0b110),
        "andi" => encode_alu_imm(operands, 0b111),

        // Shifts immediate
        "slli" => encode_shift_imm(operands, 0b001, 0b000000),
        "srli" => encode_shift_imm(operands, 0b101, 0b000000),
        "srai" => encode_shift_imm(operands, 0b101, 0b010000),

        // Register-register arithmetic (R-type)
        // Auto-convert to immediate variants when 3rd operand is an immediate
        "add" => {
            if operands.len() >= 3 && matches!(operands[2], Operand::Imm(_)) {
                encode_alu_imm(operands, 0b000) // -> addi
            } else {
                encode_alu_reg(operands, 0b000, 0b0000000)
            }
        }
        "sub" => encode_alu_reg(operands, 0b000, 0b0100000),
        "sll" => {
            if operands.len() >= 3 && matches!(operands[2], Operand::Imm(_)) {
                encode_shift_imm(operands, 0b001, 0b000000)
            } else {
                encode_alu_reg(operands, 0b001, 0b0000000)
            }
        }
        "slt" => {
            if operands.len() >= 3 && matches!(operands[2], Operand::Imm(_)) {
                encode_alu_imm(operands, 0b010) // -> slti
            } else {
                encode_alu_reg(operands, 0b010, 0b0000000)
            }
        }
        "sltu" => {
            if operands.len() >= 3 && matches!(operands[2], Operand::Imm(_)) {
                encode_alu_imm(operands, 0b011) // -> sltiu
            } else {
                encode_alu_reg(operands, 0b011, 0b0000000)
            }
        }
        "xor" => {
            if operands.len() >= 3 && matches!(operands[2], Operand::Imm(_)) {
                encode_alu_imm(operands, 0b100) // -> xori
            } else {
                encode_alu_reg(operands, 0b100, 0b0000000)
            }
        }
        "srl" => {
            if operands.len() >= 3 && matches!(operands[2], Operand::Imm(_)) {
                encode_shift_imm(operands, 0b101, 0b000000)
            } else {
                encode_alu_reg(operands, 0b101, 0b0000000)
            }
        }
        "sra" => {
            if operands.len() >= 3 && matches!(operands[2], Operand::Imm(_)) {
                encode_shift_imm(operands, 0b101, 0b010000)
            } else {
                encode_alu_reg(operands, 0b101, 0b0100000)
            }
        }
        "or" => {
            if operands.len() >= 3 && matches!(operands[2], Operand::Imm(_)) {
                encode_alu_imm(operands, 0b110) // -> ori
            } else {
                encode_alu_reg(operands, 0b110, 0b0000000)
            }
        }
        "and" => {
            if operands.len() >= 3 && matches!(operands[2], Operand::Imm(_)) {
                encode_alu_imm(operands, 0b111) // -> andi
            } else {
                encode_alu_reg(operands, 0b111, 0b0000000)
            }
        }

        // RV64I word (32-bit) operations
        "addiw" => encode_alu_imm_w(operands, 0b000),
        "slliw" => encode_shift_imm_w(operands, 0b001, 0b0000000),
        "srliw" => encode_shift_imm_w(operands, 0b101, 0b0000000),
        "sraiw" => encode_shift_imm_w(operands, 0b101, 0b0100000),
        "addw" => {
            if operands.len() >= 3 && matches!(operands[2], Operand::Imm(_)) {
                encode_alu_imm_w(operands, 0b000) // -> addiw
            } else {
                encode_alu_reg_w(operands, 0b000, 0b0000000)
            }
        }
        "subw" => encode_alu_reg_w(operands, 0b000, 0b0100000),
        // sllw/srlw/sraw: auto-convert to slliw/srliw/sraiw when 3rd operand is immediate
        "sllw" => {
            if operands.len() >= 3 && matches!(operands[2], Operand::Imm(_)) {
                encode_shift_imm_w(operands, 0b001, 0b0000000)
            } else {
                encode_alu_reg_w(operands, 0b001, 0b0000000)
            }
        }
        "srlw" => {
            if operands.len() >= 3 && matches!(operands[2], Operand::Imm(_)) {
                encode_shift_imm_w(operands, 0b101, 0b0000000)
            } else {
                encode_alu_reg_w(operands, 0b101, 0b0000000)
            }
        }
        "sraw" => {
            if operands.len() >= 3 && matches!(operands[2], Operand::Imm(_)) {
                encode_shift_imm_w(operands, 0b101, 0b0100000)
            } else {
                encode_alu_reg_w(operands, 0b101, 0b0100000)
            }
        }

        // ── M Extension (multiply/divide) ──
        "mul" => encode_alu_reg(operands, 0b000, 0b0000001),
        "mulh" => encode_alu_reg(operands, 0b001, 0b0000001),
        "mulhsu" => encode_alu_reg(operands, 0b010, 0b0000001),
        "mulhu" => encode_alu_reg(operands, 0b011, 0b0000001),
        "div" => encode_alu_reg(operands, 0b100, 0b0000001),
        "divu" => encode_alu_reg(operands, 0b101, 0b0000001),
        "rem" => encode_alu_reg(operands, 0b110, 0b0000001),
        "remu" => encode_alu_reg(operands, 0b111, 0b0000001),
        "mulw" => encode_alu_reg_w(operands, 0b000, 0b0000001),
        "divw" => encode_alu_reg_w(operands, 0b100, 0b0000001),
        "divuw" => encode_alu_reg_w(operands, 0b101, 0b0000001),
        "remw" => encode_alu_reg_w(operands, 0b110, 0b0000001),
        "remuw" => encode_alu_reg_w(operands, 0b111, 0b0000001),

        // ── Zbb Extension (basic bit manipulation) ──
        // Unary operations (encoded as I-type with rs2 field in immediate)
        "clz" => encode_zbb_unary(operands, 0x600), // clz rd, rs1
        "ctz" => encode_zbb_unary(operands, 0x601), // ctz rd, rs1
        "cpop" => encode_zbb_unary(operands, 0x602), // cpop rd, rs1
        "sext.b" => encode_zbb_unary(operands, 0x604), // sext.b rd, rs1
        "sext.h" => encode_zbb_unary(operands, 0x605), // sext.h rd, rs1
        "rev8" => encode_zbb_unary_f5(operands, 0x6B8), // rev8 rd, rs1 (RV64, funct3=101)
        "orc.b" => encode_zbb_unary_f5(operands, 0x287), // orc.b rd, rs1 (funct3=101)
        // Unary word operations
        "clzw" => encode_zbb_unary_w(operands, 0x600), // clzw rd, rs1
        "ctzw" => encode_zbb_unary_w(operands, 0x601), // ctzw rd, rs1
        "cpopw" => encode_zbb_unary_w(operands, 0x602), // cpopw rd, rs1
        // Register-register operations
        "andn" => encode_alu_reg(operands, 0b111, 0b0100000),
        "orn" => encode_alu_reg(operands, 0b110, 0b0100000),
        "xnor" => encode_alu_reg(operands, 0b100, 0b0100000),
        "max" => encode_alu_reg(operands, 0b110, 0b0000101),
        "maxu" => encode_alu_reg(operands, 0b111, 0b0000101),
        "min" => encode_alu_reg(operands, 0b100, 0b0000101),
        "minu" => encode_alu_reg(operands, 0b101, 0b0000101),
        "rol" => encode_alu_reg(operands, 0b001, 0b0110000),
        "ror" => {
            if operands.len() >= 3 && matches!(operands[2], Operand::Imm(_)) {
                encode_shift_imm(operands, 0b101, 0b011000) // -> rori
            } else {
                encode_alu_reg(operands, 0b101, 0b0110000)
            }
        }
        "rolw" => encode_alu_reg_w(operands, 0b001, 0b0110000),
        "rorw" => {
            if operands.len() >= 3 && matches!(operands[2], Operand::Imm(_)) {
                encode_shift_imm_w(operands, 0b101, 0b0110000) // -> roriw
            } else {
                encode_alu_reg_w(operands, 0b101, 0b0110000)
            }
        }
        // Shift-immediate rotate
        "rori" => encode_shift_imm(operands, 0b101, 0b011000),
        "roriw" => encode_shift_imm_w(operands, 0b101, 0b0110000),
        // zext.h is R-type with rs2=0 on OP-32
        "zext.h" => encode_zbb_zexth(operands),

        // ── A Extension (atomics) ──
        "lr.w" => encode_lr(operands, 0b010),
        "lr.d" => encode_lr(operands, 0b011),
        "sc.w" => encode_sc(operands, 0b010),
        "sc.d" => encode_sc(operands, 0b011),
        "amoswap.w" => encode_amo(operands, 0b010, 0b00001),
        "amoadd.w" => encode_amo(operands, 0b010, 0b00000),
        "amoxor.w" => encode_amo(operands, 0b010, 0b00100),
        "amoand.w" => encode_amo(operands, 0b010, 0b01100),
        "amoor.w" => encode_amo(operands, 0b010, 0b01000),
        "amomin.w" => encode_amo(operands, 0b010, 0b10000),
        "amomax.w" => encode_amo(operands, 0b010, 0b10100),
        "amominu.w" => encode_amo(operands, 0b010, 0b11000),
        "amomaxu.w" => encode_amo(operands, 0b010, 0b11100),
        "amoswap.d" => encode_amo(operands, 0b011, 0b00001),
        "amoadd.d" => encode_amo(operands, 0b011, 0b00000),
        "amoxor.d" => encode_amo(operands, 0b011, 0b00100),
        "amoand.d" => encode_amo(operands, 0b011, 0b01100),
        "amoor.d" => encode_amo(operands, 0b011, 0b01000),
        "amomin.d" => encode_amo(operands, 0b011, 0b10000),
        "amomax.d" => encode_amo(operands, 0b011, 0b10100),
        "amominu.d" => encode_amo(operands, 0b011, 0b11000),
        "amomaxu.d" => encode_amo(operands, 0b011, 0b11100),

        // Handle .aq, .rl, .aqrl suffixes for atomics
        s if s.starts_with("lr.") => encode_lr_suffixed(s, operands),
        s if s.starts_with("sc.") => encode_sc_suffixed(s, operands),
        s if s.starts_with("amo") => encode_amo_suffixed(s, operands),

        // ── System ──
        "ecall" => Ok(EncodeResult::Word(0x00000073)),
        "ebreak" => Ok(EncodeResult::Word(0x00100073)),
        "fence" => encode_fence(operands),
        "fence.i" => Ok(EncodeResult::Word(0x0000100F)),
        "fence.tso" => Ok(EncodeResult::Word(0x8330000F)),
        "pause" => Ok(EncodeResult::Word(0x0100000F)),

        // ── Privileged instructions ──
        "wfi" => Ok(EncodeResult::Word(0x10500073)),
        "mret" => Ok(EncodeResult::Word(0x30200073)),
        "sret" => Ok(EncodeResult::Word(0x10200073)),
        "sfence.vma" => encode_sfence_vma(operands),

        "csrrw" => encode_csr(operands, 0b001),
        "csrrs" => encode_csr(operands, 0b010),
        "csrrc" => encode_csr(operands, 0b011),
        "csrrwi" => encode_csri(operands, 0b101),
        "csrrsi" => encode_csri(operands, 0b110),
        "csrrci" => encode_csri(operands, 0b111),

        // ── F Extension (single-precision float) ──
        "flw" => encode_float_load(operands, 0b010),
        "fsw" => encode_float_store(operands, 0b010),
        "fadd.s" => encode_fp_arith(operands, 0b0000000),
        "fsub.s" => encode_fp_arith(operands, 0b0000100),
        "fmul.s" => encode_fp_arith(operands, 0b0001000),
        "fdiv.s" => encode_fp_arith(operands, 0b0001100),
        "fsqrt.s" => encode_fp_unary(operands, 0b0101100, 0b00000),
        "fsgnj.s" => encode_fp_sgnj(operands, 0b0010000, 0b000),
        "fsgnjn.s" => encode_fp_sgnj(operands, 0b0010000, 0b001),
        "fsgnjx.s" => encode_fp_sgnj(operands, 0b0010000, 0b010),
        "fmin.s" => encode_fp_sgnj(operands, 0b0010100, 0b000),
        "fmax.s" => encode_fp_sgnj(operands, 0b0010100, 0b001),
        "feq.s" => encode_fp_cmp(operands, 0b1010000, 0b010),
        "flt.s" => encode_fp_cmp(operands, 0b1010000, 0b001),
        "fle.s" => encode_fp_cmp(operands, 0b1010000, 0b000),
        "fclass.s" => encode_fclass(operands, 0b1110000),
        "fcvt.w.s" => encode_fcvt_int(operands, 0b1100000, 0b00000),
        "fcvt.wu.s" => encode_fcvt_int(operands, 0b1100000, 0b00001),
        "fcvt.l.s" => encode_fcvt_int(operands, 0b1100000, 0b00010),
        "fcvt.lu.s" => encode_fcvt_int(operands, 0b1100000, 0b00011),
        "fcvt.s.w" => encode_fcvt_from_int(operands, 0b1101000, 0b00000),
        "fcvt.s.wu" => encode_fcvt_from_int(operands, 0b1101000, 0b00001),
        "fcvt.s.l" => encode_fcvt_from_int(operands, 0b1101000, 0b00010),
        "fcvt.s.lu" => encode_fcvt_from_int(operands, 0b1101000, 0b00011),
        "fmv.x.w" | "fmv.x.s" => encode_fmv_x_f(operands, 0b1110000, 0b00),
        "fmv.w.x" | "fmv.s.x" => encode_fmv_f_x(operands, 0b1111000, 0b00),

        // ── D Extension (double-precision float) ──
        "fld" => encode_float_load(operands, 0b011),
        "fsd" => encode_float_store(operands, 0b011),
        "fadd.d" => encode_fp_arith_d(operands, 0b0000001),
        "fsub.d" => encode_fp_arith_d(operands, 0b0000101),
        "fmul.d" => encode_fp_arith_d(operands, 0b0001001),
        "fdiv.d" => encode_fp_arith_d(operands, 0b0001101),
        "fsqrt.d" => encode_fp_unary(operands, 0b0101101, 0b00000),
        "fsgnj.d" => encode_fp_sgnj(operands, 0b0010001, 0b000),
        "fsgnjn.d" => encode_fp_sgnj(operands, 0b0010001, 0b001),
        "fsgnjx.d" => encode_fp_sgnj(operands, 0b0010001, 0b010),
        "fmin.d" => encode_fp_sgnj(operands, 0b0010101, 0b000),
        "fmax.d" => encode_fp_sgnj(operands, 0b0010101, 0b001),
        "feq.d" => encode_fp_cmp(operands, 0b1010001, 0b010),
        "flt.d" => encode_fp_cmp(operands, 0b1010001, 0b001),
        "fle.d" => encode_fp_cmp(operands, 0b1010001, 0b000),
        "fclass.d" => encode_fclass(operands, 0b1110001),
        "fcvt.w.d" => encode_fcvt_int(operands, 0b1100001, 0b00000),
        "fcvt.wu.d" => encode_fcvt_int(operands, 0b1100001, 0b00001),
        "fcvt.l.d" => encode_fcvt_int(operands, 0b1100001, 0b00010),
        "fcvt.lu.d" => encode_fcvt_int(operands, 0b1100001, 0b00011),
        "fcvt.d.w" => encode_fcvt_from_int(operands, 0b1101001, 0b00000),
        "fcvt.d.wu" => encode_fcvt_from_int(operands, 0b1101001, 0b00001),
        "fcvt.d.l" => encode_fcvt_from_int(operands, 0b1101001, 0b00010),
        "fcvt.d.lu" => encode_fcvt_from_int(operands, 0b1101001, 0b00011),
        "fcvt.s.d" => encode_fcvt_fp(operands, 0b0100000, 0b00001),
        "fcvt.d.s" => encode_fcvt_fp(operands, 0b0100001, 0b00000),
        "fmv.x.d" => encode_fmv_x_f(operands, 0b1110001, 0b00),
        "fmv.d.x" => encode_fmv_f_x(operands, 0b1111001, 0b00),

        // ── Fused multiply-add ──
        "fmadd.s" => encode_fma(operands, OP_FMADD, 0b00),
        "fmsub.s" => encode_fma(operands, OP_FMSUB, 0b00),
        "fnmsub.s" => encode_fma(operands, OP_FNMSUB, 0b00),
        "fnmadd.s" => encode_fma(operands, OP_FNMADD, 0b00),
        "fmadd.d" => encode_fma(operands, OP_FMADD, 0b01),
        "fmsub.d" => encode_fma(operands, OP_FMSUB, 0b01),
        "fnmsub.d" => encode_fma(operands, OP_FNMSUB, 0b01),
        "fnmadd.d" => encode_fma(operands, OP_FNMADD, 0b01),

        // ── Pseudo-instructions ──
        "nop" => Ok(EncodeResult::Word(encode_i(OP_OP_IMM, 0, 0, 0, 0))), // addi x0, x0, 0
        "li" => encode_li(operands),
        "mv" | "move" => encode_mv(operands),
        "not" => encode_not(operands),
        "neg" => encode_neg(operands),
        "negw" => encode_negw(operands),
        "sext.w" => encode_sext_w(operands),
        "seqz" => encode_seqz(operands),
        "snez" => encode_snez(operands),
        "sltz" => encode_sltz(operands),
        "sgtz" => encode_sgtz(operands),

        // Branch pseudo-instructions
        "beqz" => encode_beqz(operands),
        "bnez" => encode_bnez(operands),
        "blez" => encode_blez(operands),
        "bgez" => encode_bgez(operands),
        "bltz" => encode_bltz(operands),
        "bgtz" => encode_bgtz(operands),
        "bgt" => encode_bgt(operands),
        "ble" => encode_ble(operands),
        "bgtu" => encode_bgtu(operands),
        "bleu" => encode_bleu(operands),

        // Jump pseudo-instructions
        "j" => encode_j_pseudo(operands),
        "jr" => encode_jr(operands),
        "ret" => Ok(EncodeResult::Word(encode_i(OP_JALR, 0, 0, 1, 0))), // jalr x0, x1, 0
        "call" => encode_call(operands),
        "tail" => encode_tail(operands),

        // Address pseudo-instructions
        "la" => encode_la(operands),
        "lla" => encode_lla(operands),

        // CSR pseudo-instructions
        "rdcycle" | "rdtime" | "rdinstret" => encode_rdcsr(mnemonic, operands),
        "csrr" => encode_csrr(operands),
        "csrw" => encode_csrw(operands),
        "csrs" => encode_csrs(operands),
        "csrc" => encode_csrc(operands),

        // Misc pseudo-instructions
        "fmv.s" => encode_fmv_s(operands),
        "fmv.d" => encode_fmv_d(operands),
        "fabs.s" => encode_fabs_s(operands),
        "fabs.d" => encode_fabs_d(operands),
        "fneg.s" => encode_fneg_s(operands),
        "fneg.d" => encode_fneg_d(operands),

        // `jump` pseudo-instruction (our codegen emits this)
        "jump" => encode_jump(operands),

        // F/D CSR pseudo-instructions
        // frcsr rd -> csrrs rd, fcsr, x0
        "frcsr" => {
            let rd = get_reg(operands, 0)?;
            Ok(EncodeResult::Word(encode_i(OP_SYSTEM, rd, 0b010, 0, 0x003)))
        }
        // fscsr rs -> csrrw x0, fcsr, rs   (or fscsr rd, rs -> csrrw rd, fcsr, rs)
        "fscsr" => {
            if operands.len() >= 2 {
                let rd = get_reg(operands, 0)?;
                let rs1 = get_reg(operands, 1)?;
                Ok(EncodeResult::Word(encode_i(
                    OP_SYSTEM, rd, 0b001, rs1, 0x003,
                )))
            } else {
                let rs1 = get_reg(operands, 0)?;
                Ok(EncodeResult::Word(encode_i(
                    OP_SYSTEM, 0, 0b001, rs1, 0x003,
                )))
            }
        }
        // frrm rd -> csrrs rd, frm, x0
        "frrm" => {
            let rd = get_reg(operands, 0)?;
            Ok(EncodeResult::Word(encode_i(OP_SYSTEM, rd, 0b010, 0, 0x002)))
        }
        // fsrm rs -> csrrw x0, frm, rs   (or fsrm rd, rs -> csrrw rd, frm, rs)
        "fsrm" => {
            if operands.len() >= 2 {
                let rd = get_reg(operands, 0)?;
                let rs1 = get_reg(operands, 1)?;
                Ok(EncodeResult::Word(encode_i(
                    OP_SYSTEM, rd, 0b001, rs1, 0x002,
                )))
            } else {
                let rs1 = get_reg(operands, 0)?;
                Ok(EncodeResult::Word(encode_i(
                    OP_SYSTEM, 0, 0b001, rs1, 0x002,
                )))
            }
        }
        // frflags rd -> csrrs rd, fflags, x0
        "frflags" => {
            let rd = get_reg(operands, 0)?;
            Ok(EncodeResult::Word(encode_i(OP_SYSTEM, rd, 0b010, 0, 0x001)))
        }
        // fsflags rs -> csrrw x0, fflags, rs
        "fsflags" => {
            if operands.len() >= 2 {
                let rd = get_reg(operands, 0)?;
                let rs1 = get_reg(operands, 1)?;
                Ok(EncodeResult::Word(encode_i(
                    OP_SYSTEM, rd, 0b001, rs1, 0x001,
                )))
            } else {
                let rs1 = get_reg(operands, 0)?;
                Ok(EncodeResult::Word(encode_i(
                    OP_SYSTEM, 0, 0b001, rs1, 0x001,
                )))
            }
        }

        // Explicit compressed instructions
        "c.nop" => Ok(EncodeResult::Half(0x0001)),
        "c.ebreak" => Ok(EncodeResult::Half(0x9002)),
        "c.lui" => encode_c_lui(operands),
        "c.li" => encode_c_li(operands),
        "c.addi" => encode_c_addi(operands),
        "c.mv" => encode_c_mv(operands),
        "c.add" => encode_c_add(operands),
        "c.jr" => encode_c_jr(operands),
        "c.jalr" => encode_c_jalr(operands),

        // ── RVV (Vector) Extension ──
        // TODO: masked variants (v0.t) are not yet supported; vm is hardcoded to 1 (unmasked).

        // Vector configuration: vsetvli rd, rs1, vtypei
        "vsetvli" => encode_vsetvli(operands),
        // Vector configuration: vsetivli rd, uimm, vtypei
        "vsetivli" => encode_vsetivli(operands),
        // Vector configuration: vsetvl rd, rs1, rs2
        "vsetvl" => encode_vsetvl(operands),

        // Vector loads
        "vle8.v" => encode_vload(operands, 0b000, 0), // EEW=8
        "vle16.v" => encode_vload(operands, 0b101, 0), // EEW=16
        "vle32.v" => encode_vload(operands, 0b110, 0), // EEW=32
        "vle64.v" => encode_vload(operands, 0b111, 0), // EEW=64

        // Vector stores
        "vse8.v" => encode_vstore(operands, 0b000, 0), // EEW=8
        "vse16.v" => encode_vstore(operands, 0b101, 0), // EEW=16
        "vse32.v" => encode_vstore(operands, 0b110, 0), // EEW=32
        "vse64.v" => encode_vstore(operands, 0b111, 0), // EEW=64

        // Vector load/store mask
        "vlm.v" => encode_vload(operands, 0b000, 0x0B), // lumop=0b01011
        "vsm.v" => encode_vstore(operands, 0b000, 0x0B), // sumop=0b01011

        // Vector integer arithmetic - OPIVV (funct3=000), OPIVX (funct3=100), OPIVI (funct3=011)
        "vadd.vv" => encode_v_arith_vv(operands, 0b000000),
        "vadd.vx" => encode_v_arith_vx(operands, 0b000000),
        "vadd.vi" => encode_v_arith_vi(operands, 0b000000),
        "vsub.vv" => encode_v_arith_vv(operands, 0b000010),
        "vsub.vx" => encode_v_arith_vx(operands, 0b000010),
        "vand.vv" => encode_v_arith_vv(operands, 0b001001),
        "vand.vx" => encode_v_arith_vx(operands, 0b001001),
        "vand.vi" => encode_v_arith_vi(operands, 0b001001),
        "vor.vv" => encode_v_arith_vv(operands, 0b001010),
        "vor.vx" => encode_v_arith_vx(operands, 0b001010),
        "vor.vi" => encode_v_arith_vi(operands, 0b001010),
        "vxor.vv" => encode_v_arith_vv(operands, 0b001011),
        "vxor.vx" => encode_v_arith_vx(operands, 0b001011),
        "vxor.vi" => encode_v_arith_vi(operands, 0b001011),

        // Vector slide instructions
        "vslideup.vx" => encode_v_arith_vx(operands, 0b001110),
        "vslideup.vi" => encode_v_arith_vi(operands, 0b001110),
        "vslidedown.vx" => encode_v_arith_vx(operands, 0b001111),
        "vslidedown.vi" => encode_v_arith_vi(operands, 0b001111),

        // Vector merge/move
        "vmv.v.v" => encode_vmv_v_v(operands),
        "vmv.v.x" => encode_vmv_v_x(operands),
        "vmv.v.i" => encode_vmv_v_i(operands),

        // Vector misc
        "vid.v" => encode_vid_v(operands),

        // Zvksh (SM3 crypto)
        "vsm3c.vi" => encode_v_crypto_vi(operands, 0b101011),
        "vsm3me.vv" => encode_v_crypto_vv(operands, 0b100000),
        // Zvksed (SM4 crypto)
        "vsm4k.vi" => encode_v_crypto_vi(operands, 0b100001),
        "vsm4r.vs" => encode_v_crypto_vs(operands, 0b101001),

        _ => Err(format!(
            "unsupported instruction: {} {}",
            mnemonic, raw_operands
        )),
    }
}
