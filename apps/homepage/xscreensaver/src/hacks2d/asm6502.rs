//! Port of `hacks/asm6502.c` and `hacks/asm6502.h`.
//!
//! ```text
//! Copyright (C) 2007 Jeremy English <jhe@jeremyenglish.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Created: 12-April-2007
//!
//!       This is a port of the javascript 6502 assembler, compiler and
//!       debugger. The orignal code was copyright 2006 by Stian Soreng -
//!       https://web.archive.org/web/20070516072609/http%3A//www.6502asm.com/
//!
//!       I changed the structure of the assembler in this version.
//! ```
//!
//! Not a hack: the processor [`super::m6502`] runs its programs on, and the
//! assembler that turns them into bytes. Upstream keeps it in its own file next
//! to the saver and so does this port.
//!
//! The memory map is the one the original 6502asm.com page used: the stack is
//! page `$100`, the display is 1024 bytes from `$200` (one byte a pixel, low
//! nibble is a colour index, 32 across), programs load at `$600`, `$fe` reads
//! random, and `$ff` would be the last key pressed except that input is
//! disabled. Storing anywhere in the display range calls the plotter, which is
//! how a program draws: there is no video hardware to scan the memory, the
//! write itself is the pixel. Upstream's plotter is a callback into the saver's
//! own array; here the array is [`Machine::pixels`], which comes to the same
//! thing and saves handing a closure through the interpreter.
//!
//! This is emphatically not a cycle-accurate 6502, and several instructions
//! are simply wrong: `RTI` pops one byte of return address instead of two,
//! `TXS` pushes X onto the stack rather than setting the stack pointer, `CMP`
//! sets carry from `A + M > 0xff` rather than `A >= M`, and `LSR` on the
//! accumulator tests the wrong register for zero. They are kept as they are.
//! The programs in `images/m6502/` were written against this interpreter, on
//! that web page, and are the only things that will ever run on it: an
//! instruction repaired here is a program broken there.
//!
//! `m6502_trace` and `m6502_hexDump` are not ported. They print the registers
//! and memory to a stream for someone debugging an assembly program, and there
//! is no stream to print to.

/// The number of unique instructions, not counting `DCB`.
const NUM_OPCODES: usize = 56;
/// We have 64k of memory to work with.
const MEM_64K: usize = 65536;
/// The number of values allowed behind `DCB`.
const MAX_PARAM_VALUE: usize = 25;
/// Each assembly command is 3 characters long. Upstream's buffer is four bytes
/// and its parser fills all four, so this is the count it will read, not the
/// length it will accept.
const MAX_CMD_LEN: usize = 4;
/// Upstream truncates every label with `sprintf("%.*s", MAX_LABEL_LEN - 1, ..)`.
const MAX_LABEL_LEN: usize = 79;
/// The stack works from the top down in page $100 to $1ff.
const STACK_TOP: u16 = 0x1ff;
const STACK_BOTTOM: u16 = 0x100;
/// The default entry point for the program.
const PROG_START: u16 = 0x600;

/* Bit Flags
    _  _  _  _  _  _  _  _
   |N||V||F||B||D||I||Z||C|
    -  -  -  -  -  -  -  -
    7  6  5  4  3  2  1  0
*/
const CARRY_FL: u8 = 0;
const ZERO_FL: u8 = 1;
const INTERRUPT_FL: u8 = 2;
const DECIMAL_FL: u8 = 3;
const FUTURE_FL: u8 = 5;
const OVERFLOW_FL: u8 = 6;
const NEGATIVE_FL: u8 = 7;

fn bit_on(value: u8, bit: u8) -> bool {
    value & (1 << bit) != 0
}

fn set_bit(value: u8, bit: u8, on: bool) -> u8 {
    if on {
        value | (1 << bit)
    } else {
        value & !(1 << bit)
    }
}

/// `nibble(value, LEFT)`: the top four bits, still in the top four bits.
fn nibble_left(value: u8) -> u8 {
    value & 0xf0
}

/// `nibble(value, RIGHT)`.
fn nibble_right(value: u8) -> u8 {
    value & 0x0f
}

/// How an instruction says where its operand is.
///
/// The assembler works these out from the shape of the text and the
/// interpreter gets them back from the opcode byte, so the same list serves
/// both halves. `AbsLabelX`/`AbsLabelY` differ from `AbsX`/`AbsY` only before
/// linking, when the address is still a name.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Adm {
    #[default]
    Single,
    ImmediateValue,
    ImmediateGreat,
    ImmediateLess,
    IndirectX,
    IndirectY,
    Zero,
    ZeroX,
    ZeroY,
    AbsValue,
    AbsOrBranch,
    AbsX,
    AbsY,
    AbsLabelX,
    AbsLabelY,
    DcbParam,
}

/// One row of upstream's opcode table: a mnemonic and the byte it assembles to
/// in each addressing mode, `0x00` where the mode does not exist.
struct OpDef {
    name: &'static str,
    imm: u8,
    zp: u8,
    zpx: u8,
    zpy: u8,
    abs: u8,
    absx: u8,
    absy: u8,
    indx: u8,
    indy: u8,
    sngl: u8,
    bra: u8,
    func: Option<fn(&mut Machine, Adm)>,
}

/// `SETOP`, so the table below can keep upstream's columns.
macro_rules! opcodes {
    ($($name:literal, $imm:literal, $zp:literal, $zpx:literal, $zpy:literal, $abs:literal,
       $absx:literal, $absy:literal, $indx:literal, $indy:literal, $sngl:literal, $bra:literal,
       $func:expr;)*) => {
        [$(OpDef {
            name: $name, imm: $imm, zp: $zp, zpx: $zpx, zpy: $zpy, abs: $abs, absx: $absx,
            absy: $absy, indx: $indx, indy: $indy, sngl: $sngl, bra: $bra, func: $func,
        }),*]
    };
}

#[rustfmt::skip]
static OPCODES: [OpDef; NUM_OPCODES] = opcodes![
/*  OPCODE Imm   ZP    ZPX   ZPY   ABS   ABSX  ABSY  INDX  INDY  SGNL  BRA   Jump Function*/
    "ADC", 0x69, 0x65, 0x75, 0x00, 0x6d, 0x7d, 0x79, 0x61, 0x71, 0x00, 0x00, Some(Machine::adc);
    "AND", 0x29, 0x25, 0x35, 0x31, 0x2d, 0x3d, 0x39, 0x00, 0x00, 0x00, 0x00, Some(Machine::and);
    "ASL", 0x00, 0x06, 0x16, 0x00, 0x0e, 0x1e, 0x00, 0x00, 0x00, 0x0a, 0x00, Some(Machine::asl);
    "BIT", 0x00, 0x24, 0x00, 0x00, 0x2c, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, Some(Machine::bit);
    "BPL", 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, Some(Machine::bpl);
    "BMI", 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x30, Some(Machine::bmi);
    "BVC", 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x50, Some(Machine::bvc);
    "BVS", 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x70, Some(Machine::bvs);
    "BCC", 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x90, Some(Machine::bcc);
    "BCS", 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xb0, Some(Machine::bcs);
    "BNE", 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xd0, Some(Machine::bne);
    "BEQ", 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xf0, Some(Machine::beq);
    "CMP", 0xc9, 0xc5, 0xd5, 0x00, 0xcd, 0xdd, 0xd9, 0xc1, 0xd1, 0x00, 0x00, Some(Machine::cmp);
    "CPX", 0xe0, 0xe4, 0x00, 0x00, 0xec, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, Some(Machine::cpx);
    "CPY", 0xc0, 0xc4, 0x00, 0x00, 0xcc, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, Some(Machine::cpy);
    "DEC", 0x00, 0xc6, 0xd6, 0x00, 0xce, 0xde, 0x00, 0x00, 0x00, 0x00, 0x00, Some(Machine::dec);
    "EOR", 0x49, 0x45, 0x55, 0x00, 0x4d, 0x5d, 0x59, 0x41, 0x51, 0x00, 0x00, Some(Machine::eor);
    "CLC", 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x18, 0x00, Some(Machine::clc);
    "SEC", 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x38, 0x00, Some(Machine::sec);
    "CLI", 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x58, 0x00, Some(Machine::cli);
    "SEI", 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x78, 0x00, Some(Machine::sei);
    "CLV", 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xb8, 0x00, Some(Machine::clv);
    "CLD", 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xd8, 0x00, Some(Machine::cld);
    "SED", 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xf8, 0x00, Some(Machine::sed);
    "INC", 0x00, 0xe6, 0xf6, 0x00, 0xee, 0xfe, 0x00, 0x00, 0x00, 0x00, 0x00, Some(Machine::inc);
    "JMP", 0x00, 0x00, 0x00, 0x00, 0x4c, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, Some(Machine::jmp);
    "JSR", 0x00, 0x00, 0x00, 0x00, 0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, Some(Machine::jsr);
    "LDA", 0xa9, 0xa5, 0xb5, 0x00, 0xad, 0xbd, 0xb9, 0xa1, 0xb1, 0x00, 0x00, Some(Machine::lda);
    "LDX", 0xa2, 0xa6, 0x00, 0xb6, 0xae, 0x00, 0xbe, 0x00, 0x00, 0x00, 0x00, Some(Machine::ldx);
    "LDY", 0xa0, 0xa4, 0xb4, 0x00, 0xac, 0xbc, 0x00, 0x00, 0x00, 0x00, 0x00, Some(Machine::ldy);
    "LSR", 0x00, 0x46, 0x56, 0x00, 0x4e, 0x5e, 0x00, 0x00, 0x00, 0x4a, 0x00, Some(Machine::lsr);
    "NOP", 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xea, 0x00, Some(Machine::nop);
    "ORA", 0x09, 0x05, 0x15, 0x00, 0x0d, 0x1d, 0x19, 0x01, 0x11, 0x00, 0x00, Some(Machine::ora);
    "TAX", 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xaa, 0x00, Some(Machine::tax);
    "TXA", 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x8a, 0x00, Some(Machine::txa);
    "DEX", 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xca, 0x00, Some(Machine::dex);
    "INX", 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xe8, 0x00, Some(Machine::inx);
    "TAY", 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xa8, 0x00, Some(Machine::tay);
    "TYA", 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x98, 0x00, Some(Machine::tya);
    "DEY", 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x88, 0x00, Some(Machine::dey);
    "INY", 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xc8, 0x00, Some(Machine::iny);
    "ROR", 0x00, 0x66, 0x76, 0x00, 0x6e, 0x7e, 0x00, 0x00, 0x00, 0x6a, 0x00, Some(Machine::ror);
    "ROL", 0x00, 0x26, 0x36, 0x00, 0x2e, 0x3e, 0x00, 0x00, 0x00, 0x2a, 0x00, Some(Machine::rol);
    "RTI", 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, Some(Machine::rti);
    "RTS", 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x60, 0x00, Some(Machine::rts);
    "SBC", 0xe9, 0xe5, 0xf5, 0x00, 0xed, 0xfd, 0xf9, 0xe1, 0xf1, 0x00, 0x00, Some(Machine::sbc);
    "STA", 0x00, 0x85, 0x95, 0x00, 0x8d, 0x9d, 0x99, 0x81, 0x91, 0x00, 0x00, Some(Machine::sta);
    "TXS", 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x9a, 0x00, Some(Machine::txs);
    "TSX", 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xba, 0x00, Some(Machine::tsx);
    "PHA", 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x48, 0x00, Some(Machine::pha);
    "PLA", 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x68, 0x00, Some(Machine::pla);
    "PHP", 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x00, Some(Machine::php);
    "PLP", 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x28, 0x00, Some(Machine::plp);
    "STX", 0x00, 0x86, 0x00, 0x96, 0x8e, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, Some(Machine::stx);
    "STY", 0x00, 0x84, 0x94, 0x00, 0x8c, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, Some(Machine::sty);
    "---", 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, None;
];

/// Where an operand lives and what is there.
#[derive(Clone, Copy, Default)]
struct Pointer {
    addr: u16,
    value: u16,
}

/// The index into [`OPCODES`] an opcode byte means, and the mode it was
/// written in.
#[derive(Clone, Copy, Default)]
struct OpIndex {
    index: u8,
    adm: Adm,
}

pub(crate) struct Machine {
    reg_a: u8,
    reg_x: u8,
    reg_y: u8,
    reg_p: u8,
    /// A pair of 8 bit registers.
    reg_pc: u16,
    reg_sp: u16,
    default_code_pc: u16,
    memory: Vec<u8>,
    code_running: bool,
    code_len: u32,
    opcache: [OpIndex; 256],
    /// What the plotter has drawn, `[x][y]`, one colour index a pixel.
    pub(crate) pixels: [[u8; 32]; 32],
}

impl Machine {
    /// `m6502_build`.
    pub(crate) fn new() -> Self {
        let mut m = Machine {
            reg_a: 0,
            reg_x: 0,
            reg_y: 0,
            reg_p: 0,
            reg_pc: 0,
            reg_sp: 0,
            default_code_pc: 0,
            memory: vec![0; MEM_64K],
            code_running: false,
            code_len: 0,
            opcache: [OpIndex::default(); 256],
            pixels: [[0; 32]; 32],
        };
        m.build_index_cache();
        m.reset();
        m
    }

    /// `buildIndexCache`: the opcode table read backwards, so the interpreter
    /// can go from a byte to an instruction and a mode in one lookup.
    ///
    /// Bytes no instruction claims are left as the zero entry, which names
    /// `ADC` in `Single` mode: an undefined opcode adds the carry flag to the
    /// accumulator and moves on. That is what upstream does too, since its
    /// cache is calloced and never checked. (Its cache is also one entry short
    /// of 256, so `0xff` reads past the end of the array; here it reads the
    /// same zero entry as any other unclaimed byte.)
    fn build_index_cache(&mut self) {
        for (i, op) in OPCODES.iter().enumerate() {
            let modes = [
                (op.imm, Adm::ImmediateValue),
                (op.zp, Adm::Zero),
                (op.zpx, Adm::ZeroX),
                (op.zpy, Adm::ZeroY),
                (op.abs, Adm::AbsValue),
                (op.absx, Adm::AbsX),
                (op.absy, Adm::AbsY),
                (op.indx, Adm::IndirectX),
                (op.indy, Adm::IndirectY),
                (op.sngl, Adm::Single),
                (op.bra, Adm::AbsOrBranch),
            ];
            for (byte, adm) in modes {
                if byte != 0x00 {
                    self.opcache[byte as usize] = OpIndex {
                        index: i as u8,
                        adm,
                    };
                }
            }
        }
    }

    /// `reset`: clear the screen and memory.
    ///
    /// The processor status register is *not* cleared, only its unused bit 5
    /// set, so a program starts with whatever flags the last one left behind.
    fn reset(&mut self) {
        self.pixels = [[0; 32]; 32];
        self.memory.fill(0);
        self.reg_a = 0;
        self.reg_x = 0;
        self.reg_y = 0;
        self.reg_p = set_bit(self.reg_p, FUTURE_FL, true);
        self.default_code_pc = PROG_START;
        self.reg_pc = PROG_START;
        self.reg_sp = STACK_TOP;
        self.code_running = false;
    }

    /* Memory */

    fn stack_push(&mut self, value: u8) {
        if self.reg_sp >= STACK_BOTTOM {
            self.memory[self.reg_sp as usize] = value;
            self.reg_sp = self.reg_sp.wrapping_sub(1);
        } else {
            /* The stack is full. */
            self.code_running = false;
        }
    }

    fn stack_pop(&mut self) -> u8 {
        if self.reg_sp < STACK_TOP {
            self.reg_sp += 1;
            self.memory[self.reg_sp as usize]
        } else {
            /* The stack is empty. */
            self.code_running = false;
            0
        }
    }

    /// Assemble one byte at the load address.
    fn push_byte(&mut self, value: u32) {
        self.memory[self.default_code_pc as usize] = (value & 0xff) as u8;
        self.code_len += 1;
        self.default_code_pc = self.default_code_pc.wrapping_add(1);
    }

    fn push_word(&mut self, value: u16) {
        self.push_byte(u32::from(value & 0xff));
        self.push_byte(u32::from(value >> 8));
    }

    /// Read the byte under the program counter and step over it.
    fn pop_byte(&mut self) -> u8 {
        let value = self.memory[self.reg_pc as usize];
        self.reg_pc = self.reg_pc.wrapping_add(1);
        value
    }

    fn pop_word(&mut self) -> u16 {
        let lo = u16::from(self.pop_byte());
        lo.wrapping_add(u16::from(self.pop_byte()) << 8)
    }

    /// Peek a byte, don't touch any registers. Address `$fe` is the random
    /// number generator, which is why so many of these programs look alive.
    fn read(&self, addr: usize) -> u8 {
        if addr == 0xfe {
            return (crate::runtime::random() % 255) as u8;
        }
        self.memory[addr]
    }

    /// Poke a byte, don't touch any registers. A write inside the display
    /// range is a pixel.
    fn store(&mut self, addr: u16, value: u8) {
        self.memory[addr as usize] = value;
        if (0x200..=0x5ff).contains(&addr) {
            let idx = self.read(addr as usize) & 0x0f;
            let rel = addr - 0x200;
            self.pixels[(rel & 0x1f) as usize][(rel >> 5) as usize] = idx;
        }
    }

    /* Emulation */

    /// Figure out how to get the value from the addrmode and get it.
    ///
    /// False means the instruction has no operand, which for the shift and
    /// rotate instructions is how they say they meant the accumulator.
    fn get_value(&mut self, adm: Adm) -> (bool, Pointer) {
        let mut p = Pointer::default();
        match adm {
            Adm::Single => return (false, p),
            Adm::ImmediateLess | Adm::ImmediateGreat | Adm::ImmediateValue => {
                p.value = u16::from(self.pop_byte());
            }
            Adm::IndirectX => {
                let zp = usize::from(self.pop_byte().wrapping_add(self.reg_x));
                p.addr = u16::from(self.read(zp)) + (u16::from(self.read(zp + 1)) << 8);
                p.value = u16::from(self.read(p.addr as usize));
            }
            Adm::IndirectY => {
                let zp = usize::from(self.pop_byte());
                p.addr = (u16::from(self.read(zp)) + (u16::from(self.read(zp + 1)) << 8))
                    .wrapping_add(u16::from(self.reg_y));
                p.value = u16::from(self.read(p.addr as usize));
            }
            Adm::Zero => {
                p.addr = u16::from(self.pop_byte());
                p.value = u16::from(self.read(p.addr as usize));
            }
            Adm::ZeroX => {
                p.addr = u16::from(self.pop_byte()) + u16::from(self.reg_x);
                p.value = u16::from(self.read(p.addr as usize));
            }
            Adm::ZeroY => {
                p.addr = u16::from(self.pop_byte()) + u16::from(self.reg_y);
                p.value = u16::from(self.read(p.addr as usize));
            }
            Adm::AbsOrBranch => {
                p.addr = u16::from(self.pop_byte());
            }
            Adm::AbsValue => {
                p.addr = self.pop_word();
                p.value = u16::from(self.read(p.addr as usize));
            }
            Adm::AbsLabelX | Adm::AbsX => {
                p.addr = self.pop_word().wrapping_add(u16::from(self.reg_x));
                p.value = u16::from(self.read(p.addr as usize));
            }
            Adm::AbsLabelY | Adm::AbsY => {
                p.addr = self.pop_word().wrapping_add(u16::from(self.reg_y));
                p.value = u16::from(self.read(p.addr as usize));
            }
            /* Handled elsewhere */
            Adm::DcbParam => return (false, p),
        }
        (true, p)
    }

    /// Manage the negative and zero flags.
    fn zero_neg(&mut self, value: u8) {
        self.reg_p = set_bit(self.reg_p, ZERO_FL, value == 0);
        self.reg_p = set_bit(self.reg_p, NEGATIVE_FL, bit_on(value, NEGATIVE_FL));
    }

    fn adc(&mut self, adm: Adm) {
        let c = u16::from(bit_on(self.reg_p, CARRY_FL));
        let (_, ptr) = self.get_value(adm);
        let value = ptr.value as u8;
        let tmp: u16;

        let both_negative = bit_on(self.reg_a, NEGATIVE_FL) && bit_on(value, NEGATIVE_FL);
        self.reg_p = set_bit(self.reg_p, OVERFLOW_FL, !both_negative);

        if bit_on(self.reg_p, DECIMAL_FL) {
            let mut t = u16::from(nibble_right(self.reg_a)) + u16::from(nibble_right(value)) + c;
            /* The decimal part is limited to 0 through 9 */
            if t >= 10 {
                t = 0x10 | ((t + 6) & 0xf);
            }
            t += u16::from(nibble_left(self.reg_a)) + u16::from(nibble_left(value));
            if t >= 160 {
                self.reg_p = set_bit(self.reg_p, CARRY_FL, true);
                if bit_on(self.reg_p, OVERFLOW_FL) && t >= 0x180 {
                    self.reg_p = set_bit(self.reg_p, OVERFLOW_FL, false);
                }
                t += 0x60;
            } else {
                self.reg_p = set_bit(self.reg_p, CARRY_FL, false);
                if bit_on(self.reg_p, OVERFLOW_FL) && t < 0x80 {
                    self.reg_p = set_bit(self.reg_p, OVERFLOW_FL, false);
                }
            }
            tmp = t;
        } else {
            let t = u16::from(self.reg_a) + ptr.value + c;
            if t >= 0x100 {
                self.reg_p = set_bit(self.reg_p, CARRY_FL, true);
                if bit_on(self.reg_p, OVERFLOW_FL) && t >= 0x180 {
                    self.reg_p = set_bit(self.reg_p, OVERFLOW_FL, false);
                }
            } else {
                self.reg_p = set_bit(self.reg_p, CARRY_FL, false);
                if bit_on(self.reg_p, OVERFLOW_FL) && t < 0x80 {
                    self.reg_p = set_bit(self.reg_p, OVERFLOW_FL, false);
                }
            }
            tmp = t;
        }

        self.reg_a = tmp as u8;
        self.zero_neg(self.reg_a);
    }

    fn and(&mut self, adm: Adm) {
        let (_, ptr) = self.get_value(adm);
        self.reg_a &= ptr.value as u8;
        self.zero_neg(self.reg_a);
    }

    fn asl(&mut self, adm: Adm) {
        let (is_value, ptr) = self.get_value(adm);
        if is_value {
            self.reg_p = set_bit(self.reg_p, CARRY_FL, bit_on(ptr.value as u8, NEGATIVE_FL));
            let v = set_bit((ptr.value << 1) as u8, CARRY_FL, false);
            self.store(ptr.addr, v);
            self.zero_neg(v);
        } else {
            /* Accumulator */
            self.reg_p = set_bit(self.reg_p, CARRY_FL, bit_on(self.reg_a, NEGATIVE_FL));
            self.reg_a = set_bit(self.reg_a << 1, CARRY_FL, false);
            self.zero_neg(self.reg_a);
        }
    }

    fn bit(&mut self, adm: Adm) {
        let (_, ptr) = self.get_value(adm);
        let value = ptr.value as u8;
        self.reg_p = set_bit(self.reg_p, ZERO_FL, value & self.reg_a == 0);
        self.reg_p = set_bit(self.reg_p, OVERFLOW_FL, bit_on(value, OVERFLOW_FL));
        self.reg_p = set_bit(self.reg_p, NEGATIVE_FL, bit_on(value, NEGATIVE_FL));
    }

    /// A branch offset is one signed byte from the instruction after it.
    fn branch(&mut self, offset: u16) {
        if offset > 0x7f {
            self.reg_pc = self.reg_pc.wrapping_sub(0x100 - offset);
        } else {
            self.reg_pc = self.reg_pc.wrapping_add(offset);
        }
    }

    /// The eight conditional branches, which differ only in the flag they read.
    fn branch_if(&mut self, adm: Adm, taken: bool) {
        let (_, ptr) = self.get_value(adm);
        if taken {
            self.branch(ptr.addr);
        }
    }

    fn bpl(&mut self, adm: Adm) {
        self.branch_if(adm, !bit_on(self.reg_p, NEGATIVE_FL));
    }

    fn bmi(&mut self, adm: Adm) {
        self.branch_if(adm, bit_on(self.reg_p, NEGATIVE_FL));
    }

    fn bvc(&mut self, adm: Adm) {
        self.branch_if(adm, !bit_on(self.reg_p, OVERFLOW_FL));
    }

    fn bvs(&mut self, adm: Adm) {
        self.branch_if(adm, bit_on(self.reg_p, OVERFLOW_FL));
    }

    fn bcc(&mut self, adm: Adm) {
        self.branch_if(adm, !bit_on(self.reg_p, CARRY_FL));
    }

    fn bcs(&mut self, adm: Adm) {
        self.branch_if(adm, bit_on(self.reg_p, CARRY_FL));
    }

    fn bne(&mut self, adm: Adm) {
        self.branch_if(adm, !bit_on(self.reg_p, ZERO_FL));
    }

    fn beq(&mut self, adm: Adm) {
        self.branch_if(adm, bit_on(self.reg_p, ZERO_FL));
    }

    /// Carry comes out of `reg + M > 0xff`, which is not what a 6502 does
    /// (`reg >= M`), but is what these programs were written against.
    fn compare(&mut self, reg: u8, ptr: &Pointer) {
        self.reg_p = set_bit(self.reg_p, CARRY_FL, u16::from(reg) + ptr.value > 0xff);
        self.zero_neg(reg.wrapping_sub(ptr.value as u8));
    }

    fn cmp(&mut self, adm: Adm) {
        let (_, ptr) = self.get_value(adm);
        self.compare(self.reg_a, &ptr);
    }

    fn cpx(&mut self, adm: Adm) {
        let (_, ptr) = self.get_value(adm);
        self.compare(self.reg_x, &ptr);
    }

    fn cpy(&mut self, adm: Adm) {
        let (_, ptr) = self.get_value(adm);
        self.compare(self.reg_y, &ptr);
    }

    fn dec(&mut self, adm: Adm) {
        let (_, ptr) = self.get_value(adm);
        let v = if ptr.value > 0 {
            (ptr.value - 1) as u8
        } else {
            0xff
        };
        self.store(ptr.addr, v);
        self.zero_neg(v);
    }

    fn eor(&mut self, adm: Adm) {
        let (_, ptr) = self.get_value(adm);
        self.reg_a ^= ptr.value as u8;
        self.zero_neg(self.reg_a);
    }

    fn clc(&mut self, _adm: Adm) {
        self.reg_p = set_bit(self.reg_p, CARRY_FL, false);
    }

    fn sec(&mut self, _adm: Adm) {
        self.reg_p = set_bit(self.reg_p, CARRY_FL, true);
    }

    fn cli(&mut self, _adm: Adm) {
        self.reg_p = set_bit(self.reg_p, INTERRUPT_FL, false);
    }

    fn sei(&mut self, _adm: Adm) {
        self.reg_p = set_bit(self.reg_p, INTERRUPT_FL, true);
    }

    fn clv(&mut self, _adm: Adm) {
        self.reg_p = set_bit(self.reg_p, OVERFLOW_FL, false);
    }

    fn cld(&mut self, _adm: Adm) {
        self.reg_p = set_bit(self.reg_p, DECIMAL_FL, false);
    }

    fn sed(&mut self, _adm: Adm) {
        self.reg_p = set_bit(self.reg_p, DECIMAL_FL, true);
    }

    fn inc(&mut self, adm: Adm) {
        let (_, ptr) = self.get_value(adm);
        let v = (ptr.value + 1) as u8;
        self.store(ptr.addr, v);
        self.zero_neg(v);
    }

    fn jmp(&mut self, adm: Adm) {
        let (_, ptr) = self.get_value(adm);
        self.reg_pc = ptr.addr;
    }

    fn jsr(&mut self, adm: Adm) {
        /* Move past the 2 byte parameter. JSR is always followed by
        absolute address. */
        let curr = self.reg_pc.wrapping_add(2);
        let (_, ptr) = self.get_value(adm);
        self.stack_push((curr >> 8) as u8);
        self.stack_push(curr as u8);
        self.reg_pc = ptr.addr;
    }

    fn lda(&mut self, adm: Adm) {
        let (_, ptr) = self.get_value(adm);
        self.reg_a = ptr.value as u8;
        self.zero_neg(self.reg_a);
    }

    fn ldx(&mut self, adm: Adm) {
        let (_, ptr) = self.get_value(adm);
        self.reg_x = ptr.value as u8;
        self.zero_neg(self.reg_x);
    }

    fn ldy(&mut self, adm: Adm) {
        let (_, ptr) = self.get_value(adm);
        self.reg_y = ptr.value as u8;
        self.zero_neg(self.reg_y);
    }

    /// The accumulator form ends by testing the operand it did not have rather
    /// than the accumulator, so it always reports zero. Upstream's bug, kept.
    fn lsr(&mut self, adm: Adm) {
        let (is_value, ptr) = self.get_value(adm);
        if is_value {
            let value = ptr.value as u8;
            self.reg_p = set_bit(self.reg_p, CARRY_FL, bit_on(value, CARRY_FL));
            let v = set_bit(value >> 1, NEGATIVE_FL, false);
            self.store(ptr.addr, v);
            self.zero_neg(v);
        } else {
            /* Accumulator */
            self.reg_p = set_bit(self.reg_p, CARRY_FL, bit_on(self.reg_a, CARRY_FL));
            self.reg_a = set_bit(self.reg_a >> 1, NEGATIVE_FL, false);
            self.zero_neg(ptr.value as u8);
        }
    }

    fn nop(&mut self, _adm: Adm) {
        /* no operation */
    }

    fn ora(&mut self, adm: Adm) {
        let (_, ptr) = self.get_value(adm);
        self.reg_a |= ptr.value as u8;
        self.zero_neg(self.reg_a);
    }

    fn tax(&mut self, _adm: Adm) {
        self.reg_x = self.reg_a;
        self.zero_neg(self.reg_x);
    }

    fn txa(&mut self, _adm: Adm) {
        self.reg_a = self.reg_x;
        self.zero_neg(self.reg_a);
    }

    fn dex(&mut self, _adm: Adm) {
        self.reg_x = if self.reg_x > 0 { self.reg_x - 1 } else { 0xff };
        self.zero_neg(self.reg_x);
    }

    fn inx(&mut self, _adm: Adm) {
        self.reg_x = self.reg_x.wrapping_add(1);
        self.zero_neg(self.reg_x);
    }

    fn tay(&mut self, _adm: Adm) {
        self.reg_y = self.reg_a;
        self.zero_neg(self.reg_y);
    }

    fn tya(&mut self, _adm: Adm) {
        self.reg_a = self.reg_y;
        self.zero_neg(self.reg_a);
    }

    fn dey(&mut self, _adm: Adm) {
        self.reg_y = if self.reg_y > 0 { self.reg_y - 1 } else { 0xff };
        self.zero_neg(self.reg_y);
    }

    fn iny(&mut self, _adm: Adm) {
        self.reg_y = self.reg_y.wrapping_add(1);
        self.zero_neg(self.reg_y);
    }

    fn ror(&mut self, adm: Adm) {
        let (is_value, ptr) = self.get_value(adm);
        let cf = bit_on(self.reg_p, CARRY_FL);
        if is_value {
            let value = ptr.value as u8;
            self.reg_p = set_bit(self.reg_p, CARRY_FL, bit_on(value, CARRY_FL));
            let v = set_bit(value >> 1, NEGATIVE_FL, cf);
            self.store(ptr.addr, v);
            self.zero_neg(v);
        } else {
            /* Implied */
            self.reg_p = set_bit(self.reg_p, CARRY_FL, bit_on(self.reg_a, CARRY_FL));
            self.reg_a = set_bit(self.reg_a >> 1, NEGATIVE_FL, cf);
            self.zero_neg(self.reg_a);
        }
    }

    fn rol(&mut self, adm: Adm) {
        let (is_value, ptr) = self.get_value(adm);
        let cf = bit_on(self.reg_p, CARRY_FL);
        if is_value {
            self.reg_p = set_bit(self.reg_p, CARRY_FL, bit_on(ptr.value as u8, NEGATIVE_FL));
            let v = set_bit((ptr.value << 1) as u8, CARRY_FL, cf);
            self.store(ptr.addr, v);
            self.zero_neg(v);
        } else {
            /* Implied */
            self.reg_p = set_bit(self.reg_p, CARRY_FL, bit_on(self.reg_a, NEGATIVE_FL));
            self.reg_a = set_bit(self.reg_a << 1, CARRY_FL, cf);
            self.zero_neg(self.reg_a);
        }
    }

    /// Pops one byte of return address where a 6502 pops two. Upstream's, kept.
    fn rti(&mut self, _adm: Adm) {
        self.reg_p = self.stack_pop();
        self.reg_pc = u16::from(self.stack_pop());
    }

    fn rts(&mut self, adm: Adm) {
        let _ = self.get_value(adm);
        let nr = u16::from(self.stack_pop());
        let nl = u16::from(self.stack_pop());
        self.reg_pc = (nl << 8) | nr;
    }

    fn sbc(&mut self, adm: Adm) {
        let c = u16::from(bit_on(self.reg_p, CARRY_FL));
        let (_, ptr) = self.get_value(adm);
        let value = ptr.value as u8;
        let mut w: u16;

        if bit_on(self.reg_p, DECIMAL_FL) {
            let ar = u16::from(nibble_right(self.reg_a));
            let br = u16::from(nibble_right(value));
            let al = u16::from(nibble_left(self.reg_a));
            let bl = u16::from(nibble_left(value));

            let mut tmp = (0xf + ar).wrapping_sub(br).wrapping_add(c);
            if tmp < 0x10 {
                w = 0;
                tmp = tmp.wrapping_sub(6);
            } else {
                w = 0x10;
                tmp -= 0x10;
            }
            w = w.wrapping_add(0xf0 + al).wrapping_sub(bl);
            if w < 0x100 {
                self.reg_p = set_bit(self.reg_p, CARRY_FL, false);
                if bit_on(self.reg_p, OVERFLOW_FL) && w < 0x80 {
                    self.reg_p = set_bit(self.reg_p, OVERFLOW_FL, false);
                }
                w = w.wrapping_sub(0x60);
            } else {
                self.reg_p = set_bit(self.reg_p, CARRY_FL, true);
                if bit_on(self.reg_p, OVERFLOW_FL) && w >= 0x180 {
                    self.reg_p = set_bit(self.reg_p, OVERFLOW_FL, false);
                }
            }
            w = w.wrapping_add(tmp);
        } else {
            w = (0xff + u16::from(self.reg_a)) - ptr.value + c;
            if w < 0x100 {
                self.reg_p = set_bit(self.reg_p, CARRY_FL, false);
                if bit_on(self.reg_p, OVERFLOW_FL) && w < 0x80 {
                    self.reg_p = set_bit(self.reg_p, OVERFLOW_FL, false);
                }
            } else {
                self.reg_p = set_bit(self.reg_p, CARRY_FL, true);
                if bit_on(self.reg_p, OVERFLOW_FL) && w >= 0x180 {
                    self.reg_p = set_bit(self.reg_p, OVERFLOW_FL, false);
                }
            }
        }
        self.reg_a = w as u8;
        self.zero_neg(self.reg_a);
    }

    fn sta(&mut self, adm: Adm) {
        let (_, ptr) = self.get_value(adm);
        self.store(ptr.addr, self.reg_a);
    }

    /// Pushes X rather than setting the stack pointer from it. Upstream's, kept.
    fn txs(&mut self, _adm: Adm) {
        self.stack_push(self.reg_x);
    }

    fn tsx(&mut self, _adm: Adm) {
        self.reg_x = self.stack_pop();
        self.zero_neg(self.reg_x);
    }

    fn pha(&mut self, _adm: Adm) {
        self.stack_push(self.reg_a);
    }

    fn pla(&mut self, _adm: Adm) {
        self.reg_a = self.stack_pop();
        self.zero_neg(self.reg_a);
    }

    fn php(&mut self, _adm: Adm) {
        self.stack_push(self.reg_p);
    }

    fn plp(&mut self, _adm: Adm) {
        self.reg_p = self.stack_pop();
        self.reg_p = set_bit(self.reg_p, FUTURE_FL, true);
    }

    fn stx(&mut self, adm: Adm) {
        let (_, ptr) = self.get_value(adm);
        self.store(ptr.addr, self.reg_x);
    }

    fn sty(&mut self, adm: Adm) {
        let (_, ptr) = self.get_value(adm);
        self.store(ptr.addr, self.reg_y);
    }

    /* Assembler */

    /// `translate`: one parsed line to the bytes it means.
    fn translate(&mut self, op: &OpDef, param: &Param) -> bool {
        let (byte, operand) = match param.kind {
            Adm::Single => (op.sngl, Operand::None),
            Adm::ImmediateValue => (op.imm, Operand::Byte(param.value[0])),
            Adm::ImmediateGreat => (op.imm, Operand::Byte(param.lbladdr >> 8)),
            Adm::ImmediateLess => (op.imm, Operand::Byte(param.lbladdr & 0xff)),
            Adm::IndirectX => (op.indx, Operand::Byte(param.value[0])),
            Adm::IndirectY => (op.indy, Operand::Byte(param.value[0])),
            Adm::Zero => (op.zp, Operand::Byte(param.value[0])),
            Adm::ZeroX => (op.zpx, Operand::Byte(param.value[0])),
            Adm::ZeroY => (op.zpy, Operand::Byte(param.value[0])),
            Adm::AbsValue => (op.abs, Operand::Word(param.value[0] as u16)),
            Adm::AbsX => (op.absx, Operand::Word(param.value[0] as u16)),
            Adm::AbsY => (op.absy, Operand::Word(param.value[0] as u16)),
            Adm::AbsLabelX => (op.absx, Operand::Word(param.lbladdr as u16)),
            Adm::AbsLabelY => (op.absy, Operand::Word(param.lbladdr as u16)),
            Adm::AbsOrBranch => {
                if op.abs > 0 {
                    (op.abs, Operand::Word(param.lbladdr as u16))
                } else {
                    (op.bra, Operand::Branch(param.lbladdr))
                }
            }
            /* Handled elsewhere */
            Adm::DcbParam => return true,
        };
        if byte == 0 {
            return false;
        }
        self.push_byte(u32::from(byte));
        match operand {
            Operand::None => {}
            Operand::Byte(v) => self.push_byte(v),
            Operand::Word(v) => self.push_word(v),
            Operand::Branch(target) => {
                let here = i32::from(self.default_code_pc);
                let diff = (target as i32 - here).abs();
                let backward = target < u32::from(self.default_code_pc);
                let offset = if backward { 0xff - diff } else { diff - 1 };
                self.push_byte(offset as u32);
            }
        }
        true
    }

    /// `compileLine`.
    fn compile_line(&mut self, line: &AsmLine) -> bool {
        if line.command.is_empty() {
            return true;
        }
        if line.command == "*=" {
            self.default_code_pc = line.param.value[0] as u16;
            return true;
        }
        if line.command == "DCB" {
            for i in 0..line.param.vp {
                self.push_byte(line.param.value[i]);
            }
            return true;
        }
        match OPCODES.iter().find(|o| o.name == line.command) {
            Some(op) => self.translate(op, &line.param),
            None => false, /* unknown opcode */
        }
    }

    /// First pass: work out the address of every label by assembling each line
    /// and counting the bytes it took.
    fn index_labels(&mut self, lines: &mut [AsmLine]) -> bool {
        for line in lines.iter_mut() {
            let old_default = self.default_code_pc;
            let this_pc = self.reg_pc;
            /* Figure out how many bytes this instruction takes */
            self.code_len = 0;
            if !self.compile_line(line) {
                return false;
            }
            /* If the load address changed we hit a *=, so the code counter
            starts again from there. */
            if old_default == self.default_code_pc {
                self.reg_pc = self.reg_pc.wrapping_add(self.code_len as u16);
            } else {
                self.reg_pc = self.default_code_pc;
            }
            if line.label_decl {
                line.label_addr = u32::from(this_pc);
            }
        }
        true
    }

    /// `compileCode`: assemble the source into memory at [`PROG_START`].
    fn compile_code(&mut self, code: &str) -> bool {
        self.reset();
        self.default_code_pc = PROG_START;
        self.reg_pc = PROG_START;

        let Some(mut lines) = parse_assembly(code) else {
            /* An error occurred while parsing the file. */
            return false;
        };

        if !self.index_labels(&mut lines) {
            return false;
        }
        link_labels(&mut lines);

        /* Second pass: translate the instructions. Indexing the labels called
        push_byte, which moved the load address, so put it back. */
        self.code_len = 0;
        self.default_code_pc = PROG_START;
        for line in &lines {
            if !self.compile_line(line) {
                return false;
            }
        }

        if self.default_code_pc > PROG_START {
            self.memory[self.default_code_pc as usize] = 0x00;
            true
        } else {
            /* No Code to run. */
            false
        }
    }

    /// Executes one instruction. This is the main part of the CPU emulator.
    fn execute(&mut self) {
        if !self.code_running {
            return;
        }
        let opcode = self.pop_byte();
        if opcode == 0x00 {
            self.code_running = false;
        } else {
            let entry = self.opcache[opcode as usize];
            if let Some(func) = OPCODES[entry.index as usize].func {
                func(self, entry.adm);
            }
        }
        if self.reg_pc == 0 || !self.code_running {
            self.code_running = false;
        }
    }

    /// `m6502_start_eval_string`: assemble a program and run its first
    /// instruction. A program that will not assemble still gets run, on
    /// whatever is in memory, exactly as upstream does.
    pub(crate) fn start_eval_string(&mut self, code: &str) {
        self.reset();
        self.compile_code(code);
        self.default_code_pc = PROG_START;
        self.reg_pc = PROG_START;
        self.code_running = true;
        self.execute();
    }

    /// Execute the next `insno` machine instructions.
    pub(crate) fn next_eval(&mut self, insno: i32) {
        for _ in 1..insno {
            if !self.code_running {
                break;
            }
            self.execute();
        }
    }
}

/// What follows an opcode byte, if anything.
enum Operand {
    None,
    Byte(u32),
    Word(u16),
    /// A branch, whose byte is worked out from where it lands.
    Branch(u32),
}

/* Assembly parser */

#[derive(Clone)]
struct Param {
    kind: Adm,
    value: [u32; MAX_PARAM_VALUE],
    /// Value pointer, index into the value table.
    vp: usize,
    label: String,
    lbladdr: u32,
}

impl Default for Param {
    fn default() -> Self {
        Param {
            kind: Adm::Single,
            value: [0; MAX_PARAM_VALUE],
            vp: 0,
            label: String::new(),
            lbladdr: 0,
        }
    }
}

struct AsmLine {
    /// Does the line have a label declaration?
    label_decl: bool,
    label: String,
    label_addr: u32,
    command: String,
    param: Param,
}

/// The source text under a cursor. Reading past the end gives `\0`, which is
/// what the C reads at the end of its string and what every loop here stops on.
struct Src<'a> {
    s: &'a [u8],
    at: usize,
}

impl Src<'_> {
    fn cur(&self) -> u8 {
        self.s.get(self.at).copied().unwrap_or(0)
    }

    fn bump(&mut self) {
        self.at += 1;
    }

    fn skip_space(&mut self) {
        while is_white(self.cur()) {
            self.bump();
        }
    }

    /// Skip a comment: from a semicolon to the end of the line.
    fn comment(&mut self) {
        self.skip_space();
        if self.cur() == b';' {
            while self.cur() != b'\n' && self.cur() != 0 {
                self.bump();
            }
        }
    }

    /// Does the rest of this line contain a certain character?
    fn has_char(&self, c: u8) -> bool {
        let mut i = self.at;
        while let Some(&ch) = self.s.get(i) {
            if ch == 0 || ch == b'\n' {
                break;
            }
            if ch == c {
                return true;
            }
            i += 1;
        }
        false
    }
}

fn is_white(c: u8) -> bool {
    c == b'\r' || c == b'\t' || c == b' '
}

fn is_hex_digit(c: u8) -> bool {
    c.is_ascii_hexdigit()
}

/// Is this a valid character for a command? All of the commands are alpha
/// except for the entry point code, which is `*=`.
fn is_cmd_char(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'*' || c == b'='
}

fn is_command(token: &str) -> bool {
    OPCODES.iter().any(|o| o.name == token) || token == "DCB"
}

fn add_value(param: &mut Param, value: u32) -> bool {
    if param.vp < MAX_PARAM_VALUE {
        param.value[param.vp] = value;
        param.vp += 1;
        true
    } else {
        /* Wrong number of parameters. */
        false
    }
}

/// Parse a command from the source code. An empty result is a blank line,
/// which is allowed; a word that is not an instruction is not.
fn command(src: &mut Src) -> Option<String> {
    let mut cmd = String::new();
    src.skip_space();
    while is_cmd_char(src.cur()) && cmd.len() < MAX_CMD_LEN {
        cmd.push(char::from(src.cur()));
        src.bump();
    }
    if cmd.is_empty() || cmd == "*=" || is_command(&cmd) {
        Some(cmd)
    } else {
        None
    }
}

fn declare_label(src: &mut Src, label: &mut String) -> bool {
    src.skip_space();
    while src.cur() != b':' && src.cur() != b'\n' && src.cur() != 0 {
        if !is_white(src.cur()) && label.len() < MAX_LABEL_LEN {
            label.push(char::from(src.cur()));
        }
        src.bump();
    }
    if label.is_empty() {
        /* Current line has to have a label */
        return false;
    }
    if src.cur() == b':' {
        src.bump(); /* Skip colon */
        true
    } else {
        false
    }
}

fn parse_hex(src: &mut Src, value: &mut u32) -> bool {
    /// Upstream's buffer, which is one longer than any address it can hold.
    const MAX_HEX_LEN: usize = 5;
    if src.cur() != b'$' {
        return false;
    }
    src.bump(); /* move pass $ */
    let mut v = 0;
    let mut n = 0;
    while is_hex_digit(src.cur()) && n < MAX_HEX_LEN {
        v = v * 16 + char::from(src.cur()).to_digit(16).unwrap_or(0);
        n += 1;
        src.bump();
    }
    *value = v;
    true
}

fn parse_dec(src: &mut Src, value: &mut u32) -> bool {
    const MAX_DEC_LEN: usize = 4;
    let mut v = 0;
    let mut n = 0;
    while src.cur().is_ascii_digit() && n < MAX_DEC_LEN {
        v = v * 10 + u32::from(src.cur() - b'0');
        n += 1;
        src.bump();
    }
    if n > 0 {
        *value = v;
        true
    } else {
        false
    }
}

fn parse_value(src: &mut Src, value: &mut u32) -> bool {
    src.skip_space();
    if src.cur() == b'$' {
        parse_hex(src, value)
    } else {
        parse_dec(src, value)
    }
}

fn param_label(src: &mut Src, label: &mut String) -> bool {
    while src.cur().is_ascii_alphanumeric() || src.cur() == b'_' {
        if label.len() < MAX_LABEL_LEN {
            label.push(char::from(src.cur()));
        }
        src.bump();
    }
    !label.is_empty()
}

fn immediate(src: &mut Src, param: &mut Param) -> bool {
    if src.cur() != b'#' {
        return false;
    }
    src.bump(); /* Move past hash */
    if src.cur() == b'<' || src.cur() == b'>' {
        param.kind = if src.cur() == b'<' {
            Adm::ImmediateLess
        } else {
            Adm::ImmediateGreat
        };
        src.bump(); /* move past < or > */
        let mut label = String::new();
        if param_label(src, &mut label) {
            param.label = label;
            return true;
        }
    } else {
        let mut value = 0;
        if parse_value(src, &mut value) {
            if value > 0xff {
                /* Immediate value is too large. */
                return false;
            }
            param.kind = Adm::ImmediateValue;
            return add_value(param, value);
        }
    }
    false
}

/// An index register named after a comma: the `,X` of `LDA $10,X`.
fn direction(src: &mut Src) -> Option<u8> {
    src.skip_space();
    if src.cur() == b',' {
        src.bump();
        src.skip_space();
        if src.cur() == b'X' || src.cur() == b'Y' {
            let c = src.cur();
            src.bump();
            return Some(c);
        }
    }
    None
}

fn indirect(src: &mut Src, param: &mut Param) -> bool {
    if src.cur() != b'(' {
        return false;
    }
    src.bump();

    let mut value = 0;
    if !parse_hex(src, &mut value) {
        return false;
    }
    if value > 0xff {
        /* Indirect value is too large. */
        return false;
    }
    if !add_value(param, value) {
        return false;
    }
    src.skip_space();
    if src.cur() == b')' {
        src.bump();
        if direction(src) == Some(b'Y') {
            param.kind = Adm::IndirectY;
            return true;
        }
    } else if direction(src) == Some(b'X') {
        src.skip_space();
        if src.cur() == b')' {
            src.bump();
            param.kind = Adm::IndirectX;
            return true;
        }
    }
    false
}

/// The comma-separated bytes behind `DCB`.
fn dcb_value(src: &mut Src, param: &mut Param) -> bool {
    loop {
        let mut val = 0;
        if !parse_value(src, &mut val) || val > 0xff || !add_value(param, val) {
            return false;
        }
        param.kind = Adm::DcbParam;
        src.skip_space();
        if src.cur() != b',' {
            return true;
        }
        src.bump();
    }
}

/// A literal operand, whose addressing mode follows from how big it is and
/// whether an index register comes after it.
fn value(src: &mut Src, param: &mut Param) -> bool {
    let mut val = 0;
    if !parse_value(src, &mut val) {
        return false;
    }
    let abs = val > 0xff;
    let dir = direction(src);
    if !add_value(param, val) {
        return false;
    }
    param.kind = match (abs, dir) {
        (true, Some(b'X')) => Adm::AbsX,
        (true, Some(b'Y')) => Adm::AbsY,
        (true, _) => Adm::AbsValue,
        (false, Some(b'X')) => Adm::ZeroX,
        (false, Some(b'Y')) => Adm::ZeroY,
        (false, None) => Adm::Zero,
        (false, Some(_)) => return false,
    };
    true
}

/// A named operand, resolved to an address after the first pass.
fn label(src: &mut Src, param: &mut Param) -> bool {
    let mut name = String::new();
    if !param_label(src, &mut name) {
        return false;
    }
    param.kind = Adm::AbsOrBranch;
    let ok = match direction(src) {
        None => true,
        Some(b'X') => {
            param.kind = Adm::AbsLabelX;
            true
        }
        Some(b'Y') => {
            param.kind = Adm::AbsLabelY;
            true
        }
        Some(_) => false,
    };
    param.label = name;
    ok
}

fn parameter(cmd: &str, src: &mut Src, param: &mut Param) -> bool {
    src.skip_space();
    let c = src.cur();
    if c == 0 || c == b'\n' {
        true
    } else if c == b'#' {
        immediate(src, param)
    } else if c == b'(' {
        indirect(src, param)
    } else if c == b'$' || c.is_ascii_digit() {
        if cmd == "DCB" {
            dcb_value(src, param)
        } else {
            value(src, param)
        }
    } else if c.is_ascii_alphabetic() {
        label(src, param)
    } else {
        false /* Invalid Parameter */
    }
}

/// Read the source into a list of lines, or fail on the first one that will
/// not parse. The whole text is upper-cased first, so the assembler is
/// case-insensitive and every label matches in upper case.
fn parse_assembly(code: &str) -> Option<Vec<AsmLine>> {
    let upper: Vec<u8> = code.bytes().map(|c| c.to_ascii_uppercase()).collect();
    let mut src = Src { s: &upper, at: 0 };
    let mut lines: Vec<AsmLine> = Vec::new();

    while src.cur() != 0 {
        let mut param = Param::default();
        let mut label = String::new();
        let mut decl = false;

        src.skip_space();
        src.comment();
        if src.cur() == b'\n' {
            src.bump();
            continue; /* blank line */
        }
        if src.cur() == 0 {
            continue; /* no newline at the end of the code */
        }
        if src.has_char(b':') {
            decl = true;
            if !declare_label(&mut src, &mut label) {
                return None;
            }
            src.skip_space();
        }
        let cmd = command(&mut src)?;
        src.skip_space();
        src.comment();
        if !parameter(&cmd, &mut src, &mut param) {
            return None;
        }
        src.skip_space();
        src.comment();
        if src.cur() != b'\n' && src.cur() != 0 {
            return None;
        }
        lines.push(AsmLine {
            label_decl: decl,
            label,
            label_addr: 0,
            command: cmd,
            param,
        });
    }
    Some(lines)
}

/// Make sure all of the references to the labels contain the right address.
fn link_labels(lines: &mut [AsmLine]) {
    for i in 0..lines.len() {
        let name = std::mem::take(&mut lines[i].label);
        let addr = lines[i].label_addr;
        for line in lines.iter_mut() {
            if line.param.label == name {
                line.param.lbladdr = addr;
            }
        }
        lines[i].label = name;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run a program to a halt, or until it has clearly gone forever.
    ///
    /// There is no `BRK`: the assembler has no such mnemonic, and a program
    /// ends by running off its last instruction into the zero byte
    /// [`Machine::compile_code`] leaves there, which is opcode `$00`.
    fn run(code: &str) -> Machine {
        crate::runtime::ya_rand_init(20260811);
        let mut m = Machine::new();
        m.start_eval_string(code);
        m.next_eval(100_000);
        m
    }

    #[test]
    fn a_program_assembles_and_draws() {
        // Fill the top-left pixel with white, then stop.
        let m = run("  LDA #$01\n  STA $200\n");
        assert_eq!(m.pixels[0][0], 1);
        assert!(
            !m.code_running,
            "running off the end should have stopped it"
        );
    }

    /// The display is 32 wide and starts at $200, so $220 is the second row.
    #[test]
    fn the_display_wraps_every_thirty_two_bytes() {
        let m = run("  LDA #$05\n  STA $220\n  STA $23f\n");
        assert_eq!(m.pixels[0][1], 5);
        assert_eq!(m.pixels[31][1], 5);
    }

    /// A backwards branch is the interesting one: its offset is a two's
    /// complement byte counted from the instruction after it.
    #[test]
    fn a_loop_branches_backwards() {
        let m = run(concat!(
            "  LDX #$00\n",
            "loop:\n",
            "  LDA #$02\n",
            "  STA $200,X\n",
            "  INX\n",
            "  CPX #$20\n",
            "  BNE loop\n",
        ));
        for x in 0..32 {
            assert_eq!(m.pixels[x][0], 2, "column {x} was not painted");
        }
    }

    /// `JSR` leaves a return address on the stack and `RTS` finds it, which is
    /// the one place the stack has to be right.
    #[test]
    fn a_subroutine_returns() {
        let m = run(concat!(
            "  JSR sub\n",
            "  LDA #$0f\n",
            "  STA $201\n",
            "  DCB $00\n",
            "sub:\n",
            "  LDA #$07\n",
            "  STA $200\n",
            "  RTS\n",
        ));
        assert_eq!(m.pixels[0][0], 7);
        assert_eq!(m.pixels[1][0], 0x0f);
    }

    /// `*=` moves the load address, and a label declared after it takes an
    /// address in the new place.
    #[test]
    fn the_load_address_can_move() {
        let m = run(concat!(
            "  JMP there\n",
            "  *=$1000\n",
            "there:\n",
            "  LDA #$03\n",
            "  STA $202\n",
        ));
        assert_eq!(m.pixels[2][0], 3);
    }

    /// Indexed indirect addressing, which the Sierpinski demo leans on.
    #[test]
    fn indirect_addressing_reaches_the_screen() {
        let m = run(concat!(
            "  LDA #$00\n",
            "  STA $10\n",
            "  LDA #$02\n",
            "  STA $11\n",
            "  LDA #$0d\n",
            "  LDY #$03\n",
            "  STA ($10),Y\n",
        ));
        assert_eq!(m.pixels[3][0], 0x0d);
    }

    /// `DCB` is data, not code, and lands in memory as written.
    #[test]
    fn dcb_lays_down_bytes() {
        let mut m = Machine::new();
        m.start_eval_string("  DCB $01, $02, 3\n");
        assert_eq!(&m.memory[0x600..0x603], &[1, 2, 3]);
    }

    /// Nonsense must not assemble, and must not panic trying.
    #[test]
    fn a_bad_program_is_refused() {
        let mut m = Machine::new();
        assert!(!m.compile_code("  FLY #$01\n"));
        assert!(!m.compile_code("  LDA #$100\n"));
        assert!(!m.compile_code(""));
    }

    /// Every demo the saver ships has to assemble, or it is 30 seconds of a
    /// blank screen. Upstream only finds out by watching.
    #[test]
    fn every_demo_assembles() {
        for (name, code) in super::super::m6502::DEMOS {
            let mut m = Machine::new();
            assert!(m.compile_code(code), "{name} did not assemble");
        }
    }

    /// And has to do something once it runs: paint a pixel, in the first few
    /// thousand instructions.
    ///
    /// Seeded, because most of these read `$fe` and an unseeded generator is a
    /// degenerate one: `i1` and `i2` start equal, so it doubles one word of
    /// its state per call and, after a few thousand calls, returns nothing but
    /// zero. Upstream's is the same and upstream seeds it at startup too.
    #[test]
    fn every_demo_paints() {
        crate::runtime::ya_rand_init(20260811);
        for (name, code) in super::super::m6502::DEMOS {
            let mut m = Machine::new();
            m.start_eval_string(code);
            let mut painted = false;
            for _ in 0..40 {
                m.next_eval(5_000);
                painted = m.pixels.iter().flatten().any(|p| *p != 0);
                if painted {
                    break;
                }
            }
            assert!(painted, "{name} painted nothing");
        }
    }
}
