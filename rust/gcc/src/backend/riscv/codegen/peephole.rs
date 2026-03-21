//! RISC-V 64-bit peephole optimizer for assembly text.
//!
//! Operates on generated assembly text to eliminate redundant patterns from the
//! stack-based codegen. Lines are pre-parsed into `LineKind` enums so hot-path
//! pattern matching uses integer/enum comparisons instead of string parsing.
//!
//! ## Pass structure
//!
//! **Local passes** (iterative, up to 8 rounds): store/load elimination,
//! redundant jump removal, self-move elimination, move chain optimization,
//! and li-to-move chain elimination.
//!
//! **Global passes** (once): global store forwarding, register copy propagation,
//! and global dead store elimination.
//!
//! **Local cleanup** (up to 4 rounds): re-run local passes to clean up
//! opportunities exposed by global passes.
//!
//! ## Optimizations
//!
//! 1. **Adjacent store/load elimination**: `sd t0, off(s0)` followed by
//!    `ld t0, off(s0)` at the same offset — the load is redundant.
//!
//! 2. **Redundant jump elimination**: `jump .LBBN, t6` (or `j .LBBN`) when
//!    `.LBBN:` is the next non-empty line — the jump is redundant.
//!
//! 3. **Self-move elimination**: `mv tX, tX` is a no-op.
//!
//! 4. **Mv chain optimization**: `mv A, B; mv C, A` → redirect second to
//!    `mv C, B`, enabling the first mv to become dead if A is unused.
//!
//! 5. **Li-to-move chain**: `li rX, imm; mv rY, rX` → `li rY, imm` when
//!    rX is a temp register and the next instruction copies from it.
//!
//! 6. **Global store forwarding**: Tracks slot→register mappings within basic
//!    blocks. When a load reads a slot whose value is still in a register,
//!    the load is replaced with a register move (or eliminated if same reg).
//!
//! 7. **Register copy propagation**: After store forwarding converts loads to
//!    moves, propagate `mv dst, src` into the next instruction's source operands.
//!
//! 8. **Global dead store elimination**: Removes stores to stack slots that
//!    are never loaded anywhere in the function.

// ── Line classification types ────────────────────────────────────────────────

/// Compact classification of an assembly line.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum LineKind {
    /// Deleted / blank
    Nop,
    /// `sd reg, offset(s0)` or `sw reg, offset(s0)` — store to frame slot
    StoreS0 { reg: u8, offset: i32, is_word: bool },
    /// `ld reg, offset(s0)` or `lw reg, offset(s0)` — load from frame slot
    LoadS0 { reg: u8, offset: i32, is_word: bool },
    /// `mv rdst, rsrc` — register-to-register move
    Move { dst: u8, src: u8 },
    /// `li rdst, imm` — load immediate
    LoadImm { dst: u8 },
    /// `jump .label, t6` or `j .label` — unconditional jump
    Jump,
    /// Branch instruction (beq, bne, bge, blt, bgeu, bltu, bnez, beqz)
    Branch,
    /// Label (`.LBBx:` etc.)
    Label,
    /// `ret`
    Ret,
    /// `call` or `jal ra, ...`
    Call,
    /// Assembler directive (lines starting with `.`)
    Directive,
    /// ALU instruction (add, sub, and, or, xor, sll, srl, sra, mul, etc.)
    Alu,
    /// `sext.w` instruction — sign-extend word
    SextW { dst: u8, src: u8 },
    /// `lla reg, symbol` or `la reg, symbol` — load address of symbol
    /// Must not be modified by copy propagation (symbol names contain
    /// register-like substrings such as `main.s1.0`).
    LoadAddr { dst: u8 },
    /// Any other instruction
    Other,
}

/// RISC-V register IDs for pattern matching.
/// We only track the registers that matter for our patterns.
const REG_NONE: u8 = 255;
const REG_T0: u8 = 0;
const REG_T1: u8 = 1;
const REG_T2: u8 = 2;
const REG_T3: u8 = 3;
const REG_T4: u8 = 4;
const REG_T5: u8 = 5;
const REG_T6: u8 = 6;
const REG_S0: u8 = 10; // frame pointer
const REG_S1: u8 = 11;
const REG_S2: u8 = 12;
const REG_S3: u8 = 13;
const REG_S4: u8 = 14;
const REG_S5: u8 = 15;
const REG_S6: u8 = 16;
const REG_S7: u8 = 17;
const REG_S8: u8 = 18;
const REG_S9: u8 = 19;
const REG_S10: u8 = 20;
const REG_S11: u8 = 21;
const REG_A0: u8 = 30;
const REG_A1: u8 = 31;
const REG_A2: u8 = 32;
const REG_A3: u8 = 33;
const REG_A4: u8 = 34;
const REG_A5: u8 = 35;
const REG_A6: u8 = 36;
const REG_A7: u8 = 37;
const REG_RA: u8 = 40;
const REG_SP: u8 = 41;
/// Maximum register ID we track (exclusive upper bound for arrays)
const NUM_REGS: usize = 42;

/// Parse a register name to our internal ID.
fn parse_reg(name: &str) -> u8 {
    match name {
        "t0" => REG_T0,
        "t1" => REG_T1,
        "t2" => REG_T2,
        "t3" => REG_T3,
        "t4" => REG_T4,
        "t5" => REG_T5,
        "t6" => REG_T6,
        "s0" => REG_S0,
        "s1" => REG_S1,
        "s2" => REG_S2,
        "s3" => REG_S3,
        "s4" => REG_S4,
        "s5" => REG_S5,
        "s6" => REG_S6,
        "s7" => REG_S7,
        "s8" => REG_S8,
        "s9" => REG_S9,
        "s10" => REG_S10,
        "s11" => REG_S11,
        "a0" => REG_A0,
        "a1" => REG_A1,
        "a2" => REG_A2,
        "a3" => REG_A3,
        "a4" => REG_A4,
        "a5" => REG_A5,
        "a6" => REG_A6,
        "a7" => REG_A7,
        "ra" => REG_RA,
        "sp" => REG_SP,
        _ => REG_NONE,
    }
}

fn reg_name(id: u8) -> &'static str {
    match id {
        REG_T0 => "t0",
        REG_T1 => "t1",
        REG_T2 => "t2",
        REG_T3 => "t3",
        REG_T4 => "t4",
        REG_T5 => "t5",
        REG_T6 => "t6",
        REG_S0 => "s0",
        REG_S1 => "s1",
        REG_S2 => "s2",
        REG_S3 => "s3",
        REG_S4 => "s4",
        REG_S5 => "s5",
        REG_S6 => "s6",
        REG_S7 => "s7",
        REG_S8 => "s8",
        REG_S9 => "s9",
        REG_S10 => "s10",
        REG_S11 => "s11",
        REG_A0 => "a0",
        REG_A1 => "a1",
        REG_A2 => "a2",
        REG_A3 => "a3",
        REG_A4 => "a4",
        REG_A5 => "a5",
        REG_A6 => "a6",
        REG_A7 => "a7",
        REG_RA => "ra",
        REG_SP => "sp",
        _ => "??",
    }
}

// ── Line classification ──────────────────────────────────────────────────────

/// Classify a single assembly line into a LineKind.
fn classify_line(line: &str) -> LineKind {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return LineKind::Nop;
    }

    // Directives
    if trimmed.starts_with('.') {
        // Check if it's a label like `.LBB3:`
        if trimmed.ends_with(':') {
            return LineKind::Label;
        }
        return LineKind::Directive;
    }

    // Labels (e.g., `func_name:`)
    if trimmed.ends_with(':') && !trimmed.starts_with(' ') && !trimmed.starts_with('\t') {
        return LineKind::Label;
    }

    // Instructions (indented lines)
    // Store: sd/sw reg, offset(s0)
    if let Some(rest) = trimmed
        .strip_prefix("sd ")
        .or_else(|| trimmed.strip_prefix("sw "))
    {
        let is_word = trimmed.starts_with("sw");
        if let Some((reg_str, addr)) = rest.split_once(", ") {
            let reg = parse_reg(reg_str.trim());
            if reg != REG_NONE {
                if let Some(offset) = parse_s0_offset(addr.trim()) {
                    return LineKind::StoreS0 {
                        reg,
                        offset,
                        is_word,
                    };
                }
            }
        }
    }

    // Load: ld/lw reg, offset(s0)
    if let Some(rest) = trimmed
        .strip_prefix("ld ")
        .or_else(|| trimmed.strip_prefix("lw "))
    {
        let is_word = trimmed.starts_with("lw");
        if let Some((reg_str, addr)) = rest.split_once(", ") {
            let reg = parse_reg(reg_str.trim());
            if reg != REG_NONE {
                if let Some(offset) = parse_s0_offset(addr.trim()) {
                    return LineKind::LoadS0 {
                        reg,
                        offset,
                        is_word,
                    };
                }
            }
        }
    }

    // Move: mv dst, src
    if let Some(rest) = trimmed.strip_prefix("mv ") {
        if let Some((dst_str, src_str)) = rest.split_once(", ") {
            let dst = parse_reg(dst_str.trim());
            let src = parse_reg(src_str.trim());
            if dst != REG_NONE && src != REG_NONE {
                return LineKind::Move { dst, src };
            }
        }
    }

    // Load immediate: li dst, imm
    if let Some(rest) = trimmed.strip_prefix("li ") {
        if let Some((dst_str, _)) = rest.split_once(", ") {
            let dst = parse_reg(dst_str.trim());
            if dst != REG_NONE {
                return LineKind::LoadImm { dst };
            }
        }
    }

    // sext.w dst, src
    if let Some(rest) = trimmed.strip_prefix("sext.w ") {
        if let Some((dst_str, src_str)) = rest.split_once(", ") {
            let dst = parse_reg(dst_str.trim());
            let src = parse_reg(src_str.trim());
            if dst != REG_NONE && src != REG_NONE {
                return LineKind::SextW { dst, src };
            }
        }
    }

    // Unconditional jump: `jump .label, t6` or `j .label`
    if trimmed.starts_with("jump ") || trimmed.starts_with("j ") {
        // Don't match `jal`, `jalr`, etc.
        if trimmed.starts_with("j ") || trimmed.starts_with("jump ") {
            return LineKind::Jump;
        }
    }

    // Branch instructions
    if trimmed.starts_with("beq ")
        || trimmed.starts_with("bne ")
        || trimmed.starts_with("bge ")
        || trimmed.starts_with("blt ")
        || trimmed.starts_with("bgeu ")
        || trimmed.starts_with("bltu ")
        || trimmed.starts_with("bnez ")
        || trimmed.starts_with("beqz ")
    {
        return LineKind::Branch;
    }

    // ret
    if trimmed == "ret" {
        return LineKind::Ret;
    }

    // call
    if trimmed.starts_with("call ") || trimmed.starts_with("jal ra,") {
        return LineKind::Call;
    }

    // ALU instructions
    if trimmed.starts_with("add ")
        || trimmed.starts_with("addi ")
        || trimmed.starts_with("addw ")
        || trimmed.starts_with("addiw ")
        || trimmed.starts_with("sub ")
        || trimmed.starts_with("subw ")
        || trimmed.starts_with("and ")
        || trimmed.starts_with("andi ")
        || trimmed.starts_with("or ")
        || trimmed.starts_with("ori ")
        || trimmed.starts_with("xor ")
        || trimmed.starts_with("xori ")
        || trimmed.starts_with("sll ")
        || trimmed.starts_with("slli ")
        || trimmed.starts_with("srl ")
        || trimmed.starts_with("srli ")
        || trimmed.starts_with("sra ")
        || trimmed.starts_with("srai ")
        || trimmed.starts_with("sllw ")
        || trimmed.starts_with("slliw ")
        || trimmed.starts_with("srlw ")
        || trimmed.starts_with("srliw ")
        || trimmed.starts_with("sraw ")
        || trimmed.starts_with("sraiw ")
        || trimmed.starts_with("mul ")
        || trimmed.starts_with("mulw ")
        || trimmed.starts_with("div ")
        || trimmed.starts_with("divu ")
        || trimmed.starts_with("divw ")
        || trimmed.starts_with("divuw ")
        || trimmed.starts_with("rem ")
        || trimmed.starts_with("remu ")
        || trimmed.starts_with("remw ")
        || trimmed.starts_with("remuw ")
        || trimmed.starts_with("slt ")
        || trimmed.starts_with("sltu ")
        || trimmed.starts_with("slti ")
        || trimmed.starts_with("sltiu ")
        || trimmed.starts_with("neg ")
        || trimmed.starts_with("negw ")
        || trimmed.starts_with("not ")
        || trimmed.starts_with("snez ")
        || trimmed.starts_with("seqz ")
        || trimmed.starts_with("lui ")
    {
        return LineKind::Alu;
    }

    // Load address: lla/la reg, symbol
    // These must not have their symbol operands modified by copy propagation,
    // since symbol names can contain register-like substrings (e.g. `main.s1.0`).
    if let Some(rest) = trimmed
        .strip_prefix("lla ")
        .or_else(|| trimmed.strip_prefix("la "))
    {
        if let Some((reg_str, _)) = rest.split_once(", ") {
            let dst = parse_reg(reg_str.trim());
            if dst != REG_NONE {
                return LineKind::LoadAddr { dst };
            }
        }
    }

    LineKind::Other
}

/// Parse `offset(s0)` and return the offset if it references s0.
fn parse_s0_offset(addr: &str) -> Option<i32> {
    // Format: `-24(s0)` or `48(s0)` etc.
    if !addr.ends_with("(s0)") {
        return None;
    }
    let offset_str = &addr[..addr.len() - 4]; // strip "(s0)"
    offset_str.parse::<i32>().ok()
}

/// Extract the jump target label from a jump instruction line.
fn jump_target(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    if let Some(rest) = trimmed.strip_prefix("jump ") {
        // `jump .LBBN, t6` -> `.LBBN`
        if let Some((label, _)) = rest.split_once(", ") {
            return Some(label.trim());
        }
    }
    if let Some(rest) = trimmed.strip_prefix("j ") {
        return Some(rest.trim());
    }
    None
}

/// Extract the label name from a label line (strip trailing `:`)
fn label_name(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    trimmed.strip_suffix(':')
}

/// Parse the destination register from an ALU-style instruction.
/// RISC-V ALU instructions have the form: `mnemonic rd, rs1, rs2/imm`
/// The destination is the first operand.
fn parse_alu_dest(line: &str) -> Option<u8> {
    let trimmed = line.trim();
    let space_pos = trimmed.find(' ')?;
    let args = &trimmed[space_pos + 1..];
    let first_arg = if let Some(comma) = args.find(',') {
        args[..comma].trim()
    } else {
        args.trim()
    };
    let reg = parse_reg(first_arg);
    if reg != REG_NONE {
        Some(reg)
    } else {
        None
    }
}

// ── Main entry point ─────────────────────────────────────────────────────────

/// Run peephole optimization on RISC-V assembly text.
/// Returns the optimized assembly string.
pub fn peephole_optimize(asm: String) -> String {
    let mut lines: Vec<String> = asm.lines().map(String::from).collect();
    let mut kinds: Vec<LineKind> = lines.iter().map(|l| classify_line(l)).collect();
    let n = lines.len();

    if n == 0 {
        return asm;
    }

    // Phase 1: Iterative local passes (up to 8 rounds)
    let mut changed = true;
    let mut rounds = 0;
    while changed && rounds < 8 {
        changed = false;
        changed |= eliminate_adjacent_store_load(&mut lines, &mut kinds, n);
        changed |= eliminate_redundant_jumps(&lines, &mut kinds, n);
        changed |= eliminate_self_moves(&mut kinds, n);
        changed |= eliminate_redundant_mv_chain(&mut lines, &mut kinds, n);
        changed |= eliminate_li_mv_chain(&mut lines, &mut kinds, n);
        rounds += 1;
    }

    // NOTE: Branch-over-branch optimization is not safe on RISC-V because
    // B-type branches have ±4KB range while the `jump label, t6` instruction
    // (auipc+jalr) has unlimited range. Inverting a branch to directly target
    // a far label would cause R_RISCV_JAL relocation truncation errors in
    // large functions. The codegen already uses the near/far pattern correctly.

    // Phase 2: Global passes (run once)
    let mut global_changed = false;
    global_changed |= global_store_forwarding(&mut lines, &mut kinds, n);
    global_changed |= propagate_register_copies(&mut lines, &mut kinds, n);
    global_changed |= eliminate_dead_reg_moves(&lines, &mut kinds, n);
    global_changed |= global_dead_store_elimination(&lines, &mut kinds, n);

    // Phase 3: Local cleanup after global passes (up to 4 rounds)
    if global_changed {
        let mut changed2 = true;
        let mut rounds2 = 0;
        while changed2 && rounds2 < 4 {
            changed2 = false;
            changed2 |= eliminate_adjacent_store_load(&mut lines, &mut kinds, n);
            changed2 |= eliminate_redundant_jumps(&lines, &mut kinds, n);
            changed2 |= eliminate_self_moves(&mut kinds, n);
            changed2 |= eliminate_redundant_mv_chain(&mut lines, &mut kinds, n);
            changed2 |= eliminate_li_mv_chain(&mut lines, &mut kinds, n);
            changed2 |= eliminate_dead_reg_moves(&lines, &mut kinds, n);
            rounds2 += 1;
        }
    }

    // Build result
    let mut result = String::with_capacity(asm.len());
    for i in 0..n {
        if kinds[i] != LineKind::Nop {
            result.push_str(&lines[i]);
            result.push('\n');
        }
    }
    result
}

// ── Pass 1: Adjacent store/load elimination ──────────────────────────────────
//
// Pattern: sd/sw tX, off(s0)  ;  ld/lw tX, off(s0)  (same reg, same offset)
// The load is redundant because the value is already in the register.
// Also handles: sd rX, off(s0)  ;  ld rY, off(s0)  → mv rY, rX

fn eliminate_adjacent_store_load(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    let mut changed = false;
    let mut i = 0;
    while i + 1 < n {
        if let LineKind::StoreS0 {
            reg: store_reg,
            offset: store_off,
            is_word: store_word,
        } = kinds[i]
        {
            // Look ahead for the matching load (skip Nops)
            let mut j = i + 1;
            while j < n && kinds[j] == LineKind::Nop {
                j += 1;
            }
            if j < n {
                if let LineKind::LoadS0 {
                    reg: load_reg,
                    offset: load_off,
                    is_word: load_word,
                } = kinds[j]
                {
                    if store_off == load_off && store_word == load_word {
                        if store_reg == load_reg {
                            // Same register: eliminate the load
                            kinds[j] = LineKind::Nop;
                            changed = true;
                        } else {
                            // Different register: replace load with mv
                            lines[j] =
                                format!("    mv {}, {}", reg_name(load_reg), reg_name(store_reg));
                            kinds[j] = LineKind::Move {
                                dst: load_reg,
                                src: store_reg,
                            };
                            changed = true;
                        }
                    }
                }
            }
        }
        i += 1;
    }
    changed
}

// ── Pass 2: Redundant jump elimination ───────────────────────────────────────
//
// Pattern: jump .LBBN, t6  ;  .LBBN:  (jump to the immediately next label)
// The jump is redundant because execution falls through anyway.

fn eliminate_redundant_jumps(lines: &[String], kinds: &mut [LineKind], n: usize) -> bool {
    let mut changed = false;
    for i in 0..n {
        if kinds[i] == LineKind::Jump {
            // Get jump target
            if let Some(target) = jump_target(&lines[i]) {
                // Find next non-Nop line
                let mut j = i + 1;
                while j < n && kinds[j] == LineKind::Nop {
                    j += 1;
                }
                if j < n && kinds[j] == LineKind::Label {
                    if let Some(lbl) = label_name(&lines[j]) {
                        if target == lbl {
                            kinds[i] = LineKind::Nop;
                            changed = true;
                        }
                    }
                }
            }
        }
    }
    changed
}

// ── Pass 3: Self-move elimination ────────────────────────────────────────────
//
// Pattern: mv tX, tX  (move to itself is a no-op)

fn eliminate_self_moves(kinds: &mut [LineKind], n: usize) -> bool {
    let mut changed = false;
    for i in 0..n {
        if let LineKind::Move { dst, src } = kinds[i] {
            if dst == src {
                kinds[i] = LineKind::Nop;
                changed = true;
            }
        }
    }
    changed
}

// ── Pass 4: Redundant mv chain elimination ───────────────────────────────────
//
// Pattern: mv tX, sN  ;  mv tY, tX  → mv tY, sN  (when tX is a temp not used again)
// Also: mv sN, t0  ;  mv t1, t0  → keep both (t0 is still live)
// The key insight: if we see `mv A, B` followed by `mv C, A` and A is a temp
// register, we can redirect to `mv C, B` and eliminate the first mv if A
// is not used elsewhere.

fn eliminate_redundant_mv_chain(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    let mut changed = false;
    let mut i = 0;
    while i + 1 < n {
        if let LineKind::Move {
            dst: dst1,
            src: src1,
        } = kinds[i]
        {
            // Find next non-Nop instruction
            let mut j = i + 1;
            while j < n && kinds[j] == LineKind::Nop {
                j += 1;
            }
            if j < n {
                if let LineKind::Move {
                    dst: dst2,
                    src: src2,
                } = kinds[j]
                {
                    // Pattern: mv dst1, src1 ; mv dst2, dst1
                    // → redirect second: mv dst2, src1
                    // Keep the first mv (dst1 might be used later).
                    if src2 == dst1 && dst2 != src1 {
                        lines[j] = format!("    mv {}, {}", reg_name(dst2), reg_name(src1));
                        kinds[j] = LineKind::Move {
                            dst: dst2,
                            src: src1,
                        };
                        changed = true;
                        // The first mv may become dead. The self-move elimination or
                        // unused callee-save passes may clean it up later.
                    }
                }
            }
        }
        i += 1;
    }
    changed
}

// ── Pass 5: Li-to-move chain elimination ─────────────────────────────────────
//
// Pattern: li tX, imm ; mv tY, tX → li tY, imm
// This avoids the intermediate temp register for load-immediate sequences.

fn eliminate_li_mv_chain(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    let mut changed = false;
    let mut i = 0;
    while i + 1 < n {
        if let LineKind::LoadImm { dst: imm_dst } = kinds[i] {
            // Only optimize when the immediate dest is a temp register (t0-t6)
            if imm_dst <= REG_T6 {
                let mut j = i + 1;
                while j < n && kinds[j] == LineKind::Nop {
                    j += 1;
                }
                if j < n {
                    if let LineKind::Move {
                        dst: mv_dst,
                        src: mv_src,
                    } = kinds[j]
                    {
                        if mv_src == imm_dst {
                            // Retarget the li to the move destination
                            let trimmed = lines[i].trim();
                            if let Some(new_line) = retarget_li(trimmed, mv_dst) {
                                lines[j] = format!("    {}", new_line);
                                kinds[j] = LineKind::LoadImm { dst: mv_dst };
                                changed = true;
                            }
                        }
                    }
                }
            }
        }
        i += 1;
    }
    changed
}

/// Retarget a `li` instruction to a different destination register.
fn retarget_li(line: &str, new_dst: u8) -> Option<String> {
    if let Some(rest) = line.strip_prefix("li ") {
        if let Some((_, imm_part)) = rest.split_once(", ") {
            return Some(format!("li {}, {}", reg_name(new_dst), imm_part));
        }
    }
    None
}

// ── Global store forwarding ──────────────────────────────────────────────────
//
// Tracks slot→register mappings as we scan forward within each basic block.
// When we see a load from a stack slot that has a known register value,
// we replace the load with a register move (or eliminate it if same register).
//
// At labels (which may be branch targets), all mappings are invalidated.
// This ensures correctness: we only forward within straight-line basic blocks.

/// A tracked store mapping: stack slot at `offset` holds the value from `reg`.
#[derive(Clone, Copy)]
struct SlotMapping {
    reg: u8,
    is_word: bool,
    active: bool,
}

/// Maximum number of tracked slot mappings.
const MAX_SLOT_ENTRIES: usize = 64;

fn global_store_forwarding(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    if n == 0 {
        return false;
    }

    // Slot tracking: Vec of (offset, SlotMapping)
    let mut slots: Vec<(i32, SlotMapping)> = Vec::new();
    // Reverse mapping: register -> list of offsets it backs
    let mut reg_slots: Vec<Vec<i32>> = vec![Vec::new(); NUM_REGS];
    let mut changed = false;

    for i in 0..n {
        if kinds[i] == LineKind::Nop {
            continue;
        }

        match kinds[i] {
            LineKind::Label => {
                // Conservatively invalidate all mappings at every label.
                gsf_invalidate_all(&mut slots, &mut reg_slots);
            }

            LineKind::StoreS0 {
                reg,
                offset,
                is_word,
            } => {
                // Invalidate any existing mapping that overlaps this store
                let store_size: i32 = if is_word { 4 } else { 8 };
                gsf_invalidate_overlapping(&mut slots, &mut reg_slots, offset, store_size);
                // Record new mapping
                if (reg as usize) < NUM_REGS {
                    slots.push((
                        offset,
                        SlotMapping {
                            reg,
                            is_word,
                            active: true,
                        },
                    ));
                    reg_slots[reg as usize].push(offset);
                }
                if slots.len() > MAX_SLOT_ENTRIES {
                    slots.retain(|&(_, m)| m.active);
                }
            }

            LineKind::LoadS0 {
                reg: load_reg,
                offset: load_off,
                is_word: load_word,
            } => {
                // Try to forward from a stored value
                let mapping = slots
                    .iter()
                    .rev()
                    .find(|(off, m)| m.active && *off == load_off && m.is_word == load_word)
                    .map(|(_, m)| *m);

                if let Some(mapping) = mapping {
                    if load_reg == mapping.reg {
                        // Same register: eliminate the redundant load
                        kinds[i] = LineKind::Nop;
                        changed = true;
                    } else {
                        // Different register: replace load with mv
                        lines[i] =
                            format!("    mv {}, {}", reg_name(load_reg), reg_name(mapping.reg));
                        kinds[i] = LineKind::Move {
                            dst: load_reg,
                            src: mapping.reg,
                        };
                        changed = true;
                    }
                }
                // The load overwrites load_reg, invalidate any slot backed by it
                if (load_reg as usize) < NUM_REGS {
                    gsf_invalidate_reg(&mut slots, &mut reg_slots, load_reg);
                }
            }

            LineKind::Jump | LineKind::Branch | LineKind::Ret => {
                gsf_invalidate_all(&mut slots, &mut reg_slots);
            }

            LineKind::Call => {
                // Calls invalidate everything: callee may access stack via pointers.
                gsf_invalidate_all(&mut slots, &mut reg_slots);
            }

            LineKind::Move { dst, .. } => {
                // Move writes to dst, invalidate any slot backed by it
                if (dst as usize) < NUM_REGS {
                    gsf_invalidate_reg(&mut slots, &mut reg_slots, dst);
                }
            }

            LineKind::LoadImm { dst }
            | LineKind::SextW { dst, .. }
            | LineKind::LoadAddr { dst } => {
                if (dst as usize) < NUM_REGS {
                    gsf_invalidate_reg(&mut slots, &mut reg_slots, dst);
                }
            }

            LineKind::Alu => {
                // ALU writes to destination register
                if let Some(dst) = parse_alu_dest(&lines[i]) {
                    if (dst as usize) < NUM_REGS {
                        gsf_invalidate_reg(&mut slots, &mut reg_slots, dst);
                    }
                }
            }

            LineKind::Directive | LineKind::Nop => {
                // No effect on register state.
            }

            LineKind::Other => {
                // For any unclassified instruction, conservatively invalidate all.
                gsf_invalidate_all(&mut slots, &mut reg_slots);
            }
        }
    }

    changed
}

/// Invalidate all slot mappings.
fn gsf_invalidate_all(slots: &mut Vec<(i32, SlotMapping)>, reg_slots: &mut [Vec<i32>]) {
    slots.clear();
    for rs in reg_slots.iter_mut() {
        rs.clear();
    }
}

/// Invalidate slot mappings whose byte range overlaps [store_off, store_off + store_size).
fn gsf_invalidate_overlapping(
    slots: &mut [(i32, SlotMapping)],
    reg_slots: &mut [Vec<i32>],
    store_off: i32,
    store_size: i32,
) {
    let store_end = store_off + store_size;
    for &mut (off, ref mut m) in slots.iter_mut() {
        if !m.active {
            continue;
        }
        let mapping_size: i32 = if m.is_word { 4 } else { 8 };
        let mapping_end = off + mapping_size;
        if off < store_end && store_off < mapping_end {
            let r = m.reg as usize;
            if r < reg_slots.len() {
                reg_slots[r].retain(|&o| o != off);
            }
            m.active = false;
        }
    }
}

/// Invalidate all slot mappings backed by a specific register.
fn gsf_invalidate_reg(slots: &mut [(i32, SlotMapping)], reg_slots: &mut [Vec<i32>], reg: u8) {
    let r = reg as usize;
    if r >= reg_slots.len() {
        return;
    }
    for &offset in &reg_slots[r] {
        for &mut (off, ref mut m) in slots.iter_mut().rev() {
            if off == offset && m.active && m.reg == reg {
                m.active = false;
                break;
            }
        }
    }
    reg_slots[r].clear();
}

// ── Register copy propagation ────────────────────────────────────────────────
//
// After store forwarding converts loads into register moves, propagate those
// copies into subsequent instructions. For `mv dst, src`, replace references
// to dst with src in the immediately following instruction (within the same
// basic block).

fn propagate_register_copies(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    let mut changed = false;

    for i in 0..n {
        // Only process register-to-register moves
        let (dst, src) = match kinds[i] {
            LineKind::Move { dst, src } => {
                // Don't propagate frame pointer or stack pointer moves
                if dst == REG_S0 || src == REG_S0 || dst == REG_SP || src == REG_SP {
                    continue;
                }
                (dst, src)
            }
            _ => continue,
        };

        // Find the next non-Nop instruction
        let mut j = i + 1;
        while j < n && kinds[j] == LineKind::Nop {
            j += 1;
        }
        if j >= n {
            continue;
        }

        // Don't propagate across control flow boundaries or into instructions
        // with symbol names (LoadAddr) that could be corrupted by register
        // name replacement (e.g. `lla t0, main.s1.0` contains "s1").
        match kinds[j] {
            LineKind::Label
            | LineKind::Jump
            | LineKind::Ret
            | LineKind::Directive
            | LineKind::Call
            | LineKind::LoadAddr { .. } => continue,
            _ => {}
        }

        let dst_name = reg_name(dst);
        if !lines[j].contains(dst_name) {
            continue;
        }

        let src_name = reg_name(src);

        // For move instructions, replace the source operand
        match kinds[j] {
            LineKind::Move {
                dst: dst2,
                src: src2,
            } if src2 == dst => {
                // mv X, dst → mv X, src
                if dst2 != src {
                    lines[j] = format!("    mv {}, {}", reg_name(dst2), src_name);
                    kinds[j] = LineKind::Move { dst: dst2, src };
                    changed = true;
                }
            }
            _ => {
                // General case: try to replace the register in source positions
                if let Some(new_line) =
                    replace_source_reg_in_instruction(&lines[j], dst_name, src_name)
                {
                    lines[j] = new_line;
                    kinds[j] = classify_line(&lines[j]);
                    changed = true;
                }
            }
        }
    }
    changed
}

// ── Dead register move elimination ──────────────────────────────────────────
//
// After copy propagation, some `mv` instructions may have dead destinations:
// the destination register is overwritten before being read. Scan forward
// from each `mv` instruction (within the same basic block, up to a window)
// to check if the destination is read before being overwritten.
//
// If the destination is overwritten without being read, the move is dead.

fn eliminate_dead_reg_moves(lines: &[String], kinds: &mut [LineKind], n: usize) -> bool {
    let mut changed = false;
    let window = 16; // look ahead up to 16 instructions

    for i in 0..n {
        let (dst, _src) = match kinds[i] {
            LineKind::Move { dst, src } => (dst, src),
            _ => continue,
        };

        // Only eliminate moves to temp registers (t0-t6).
        // Callee-saved registers might be live across basic blocks.
        if dst > REG_T6 {
            continue;
        }

        let dst_name = reg_name(dst);

        // Scan forward to check if dst is read before being overwritten
        let mut is_dead = false;
        let mut count = 0;
        let mut j = i + 1;

        while j < n && count < window {
            if kinds[j] == LineKind::Nop {
                j += 1;
                continue;
            }

            // Stop at control flow boundaries and any unclassified instruction
            match kinds[j] {
                LineKind::Label
                | LineKind::Jump
                | LineKind::Branch
                | LineKind::Ret
                | LineKind::Call
                | LineKind::Directive => break,
                LineKind::Other => {
                    // Conservatively assume Other instructions may read the register
                    break;
                }
                _ => {}
            }

            // For well-classified instructions, check reads precisely
            let reads_dst = match kinds[j] {
                LineKind::Move { src, .. } => src == dst,
                LineKind::SextW { src, .. } => src == dst,
                LineKind::StoreS0 { reg: store_reg, .. } => store_reg == dst,
                LineKind::LoadS0 { .. } => false, // only writes to dest
                LineKind::LoadImm { .. } => false, // only writes
                LineKind::LoadAddr { .. } => false, // only writes (lla/la)
                LineKind::Alu => {
                    // Check source positions (after first comma)
                    let trimmed = lines[j].trim();
                    if let Some(comma_pos) = trimmed.find(',') {
                        let after_dest = &trimmed[comma_pos + 1..];
                        has_whole_word(after_dest, dst_name)
                    } else {
                        // Single-operand: conservative
                        true
                    }
                }
                _ => true, // conservative for anything else
            };

            if reads_dst {
                break;
            }

            // Check if this instruction writes to dst (overwrites it)
            let dest_of_j = match kinds[j] {
                LineKind::Move { dst: d, .. } => Some(d),
                LineKind::LoadImm { dst: d } => Some(d),
                LineKind::SextW { dst: d, .. } => Some(d),
                LineKind::LoadS0 { reg: d, .. } => Some(d),
                LineKind::LoadAddr { dst: d } => Some(d),
                LineKind::Alu => parse_alu_dest(&lines[j]),
                _ => None,
            };

            if dest_of_j == Some(dst) {
                // dst is overwritten without being read: the move is dead!
                is_dead = true;
                break;
            }

            count += 1;
            j += 1;
        }

        if is_dead {
            kinds[i] = LineKind::Nop;
            changed = true;
        }
    }
    changed
}

// Shared peephole string utilities -- see backend/peephole_common.rs
#[cfg(test)]
use crate::backend::peephole_common::replace_whole_word;
use crate::backend::peephole_common::{has_whole_word, replace_source_reg_in_instruction};

// ── Global dead store elimination ────────────────────────────────────────────
//
// Scans the entire function to find stack slot offsets that are never loaded.
// Stores to such slots are dead and can be eliminated.
// This runs after global store forwarding, which may have converted loads
// to register moves, leaving the original stores dead.

fn global_dead_store_elimination(lines: &[String], kinds: &mut [LineKind], n: usize) -> bool {
    // Safety: if any instruction takes the address of s0 or uses s0 in a way
    // that could create pointers to stack slots (e.g., `addi xN, s0, off`
    // where xN is not s0 itself), we bail out to avoid eliminating stores
    // that might be accessed through pointers.
    for i in 0..n {
        if kinds[i] == LineKind::Nop {
            continue;
        }
        let trimmed = lines[i].trim();
        // Check for address-of-frame-pointer patterns
        if trimmed.starts_with("addi ") && trimmed.contains(", s0,") {
            // `addi s0, sp, N` is frame pointer setup — that's fine.
            // But `addi tX, s0, N` takes address of a stack slot.
            if let Some(dest) = parse_alu_dest(trimmed) {
                if dest != REG_S0 {
                    return false;
                }
            }
        }
        if trimmed.starts_with("mv ") && trimmed.contains(", s0") {
            if let Some(rest) = trimmed.strip_prefix("mv ") {
                if let Some((_, src)) = rest.split_once(", ") {
                    if src.trim() == "s0" {
                        // mv xN, s0 — copying frame pointer
                        return false;
                    }
                }
            }
        }
    }

    // Phase 1: Collect all (offset, size) byte ranges that are loaded from.
    // We must use byte-range overlap (not exact offset match) because a wide
    // store (e.g. `sd` at offset -24, 8 bytes) can be partially read by a
    // narrower load at a different offset (e.g. `lwu` at offset -20, 4 bytes).
    let mut loaded_ranges: Vec<(i32, i32)> = Vec::new(); // (offset, size)
    for i in 0..n {
        match kinds[i] {
            LineKind::LoadS0 {
                offset, is_word, ..
            } => {
                let size = if is_word { 4 } else { 8 };
                loaded_ranges.push((offset, size));
            }
            _ => {
                // Check for loads in Other/Alu instructions that reference s0 offsets
                let trimmed = lines[i].trim();
                let load_size = if trimmed.starts_with("ld ") {
                    Some(8)
                } else if trimmed.starts_with("lw ") || trimmed.starts_with("lwu ") {
                    Some(4)
                } else if trimmed.starts_with("lh ") || trimmed.starts_with("lhu ") {
                    Some(2)
                } else if trimmed.starts_with("lb ") || trimmed.starts_with("lbu ") {
                    Some(1)
                } else {
                    None
                };
                if let Some(sz) = load_size {
                    if trimmed.contains("(s0)") {
                        if let Some(off) = extract_s0_offset_from_line(trimmed) {
                            loaded_ranges.push((off, sz));
                        }
                    }
                }
            }
        }
    }

    // Phase 2: Remove stores whose byte range does not overlap any load range
    let mut changed = false;
    for i in 0..n {
        if let LineKind::StoreS0 {
            offset, is_word, ..
        } = kinds[i]
        {
            let store_size = if is_word { 4 } else { 8 };
            let overlaps_any_load = loaded_ranges.iter().any(|&(load_off, load_sz)| {
                // Two ranges [a, a+as) and [b, b+bs) overlap iff a < b+bs && b < a+as
                offset < load_off + load_sz && load_off < offset + store_size
            });
            if !overlaps_any_load {
                kinds[i] = LineKind::Nop;
                changed = true;
            }
        }
    }
    changed
}

/// Extract s0 offset from any instruction line containing `off(s0)`.
fn extract_s0_offset_from_line(line: &str) -> Option<i32> {
    if let Some(paren_pos) = line.find("(s0)") {
        // Walk backwards from paren_pos to find the start of the offset number
        let before = &line[..paren_pos];
        // Find the last comma or space before the offset
        let start = before.rfind([',', ' ']).map(|p| p + 1).unwrap_or(0);
        let offset_str = before[start..].trim();
        return offset_str.parse::<i32>().ok();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_store() {
        assert!(matches!(
            classify_line("    sd t0, -24(s0)"),
            LineKind::StoreS0 {
                reg: REG_T0,
                offset: -24,
                is_word: false
            }
        ));
        assert!(matches!(
            classify_line("    sw a0, -32(s0)"),
            LineKind::StoreS0 {
                reg: REG_A0,
                offset: -32,
                is_word: true
            }
        ));
    }

    #[test]
    fn test_classify_load() {
        assert!(matches!(
            classify_line("    ld t0, -24(s0)"),
            LineKind::LoadS0 {
                reg: REG_T0,
                offset: -24,
                is_word: false
            }
        ));
    }

    #[test]
    fn test_classify_move() {
        assert!(matches!(
            classify_line("    mv t1, t0"),
            LineKind::Move {
                dst: REG_T1,
                src: REG_T0
            }
        ));
    }

    #[test]
    fn test_classify_li() {
        assert!(matches!(
            classify_line("    li t0, 0"),
            LineKind::LoadImm { dst: REG_T0 }
        ));
        assert!(matches!(
            classify_line("    li a0, 42"),
            LineKind::LoadImm { dst: REG_A0 }
        ));
    }

    #[test]
    fn test_classify_sext_w() {
        assert!(matches!(
            classify_line("    sext.w t0, t0"),
            LineKind::SextW {
                dst: REG_T0,
                src: REG_T0
            }
        ));
    }

    #[test]
    fn test_classify_jump() {
        assert_eq!(classify_line("    jump .LBB3, t6"), LineKind::Jump);
        assert_eq!(classify_line("    j .LBB3"), LineKind::Jump);
    }

    #[test]
    fn test_classify_label() {
        assert_eq!(classify_line(".LBB3:"), LineKind::Label);
        assert_eq!(classify_line("main:"), LineKind::Label);
    }

    #[test]
    fn test_classify_ret() {
        assert_eq!(classify_line("    ret"), LineKind::Ret);
    }

    #[test]
    fn test_classify_branch() {
        assert_eq!(classify_line("    beq t1, t2, .LBB4"), LineKind::Branch);
        assert_eq!(classify_line("    bge t1, t2, .Lskip_0"), LineKind::Branch);
    }

    #[test]
    fn test_classify_alu() {
        assert_eq!(classify_line("    addw t0, t1, t2"), LineKind::Alu);
        assert_eq!(classify_line("    slli t0, t1, 2"), LineKind::Alu);
        assert_eq!(classify_line("    addi t0, t0, 4"), LineKind::Alu);
    }

    #[test]
    fn test_adjacent_store_load_elimination() {
        let input = "    sd t0, -24(s0)\n    ld t0, -24(s0)\n    ret\n";
        let result = peephole_optimize(input.to_string());
        assert!(result.contains("sd t0, -24(s0)"));
        assert!(!result.contains("ld t0, -24(s0)"));
    }

    #[test]
    fn test_redundant_jump_elimination() {
        let input = "    jump .LBB3, t6\n.LBB3:\n    ret\n";
        let result = peephole_optimize(input.to_string());
        assert!(!result.contains("jump .LBB3"));
        assert!(result.contains(".LBB3:"));
    }

    #[test]
    fn test_self_move_elimination() {
        let input = "    mv t0, t0\n    ret\n";
        let result = peephole_optimize(input.to_string());
        assert!(!result.contains("mv t0, t0"));
    }

    #[test]
    fn test_li_mv_chain() {
        let input = "    li t0, 42\n    mv s1, t0\n    ret\n";
        let result = peephole_optimize(input.to_string());
        assert!(result.contains("li s1, 42"));
    }

    #[test]
    fn test_gsf_same_reg_elimination() {
        // Store t0 then load t0 from same slot (non-adjacent) — load is dead
        let input = "\
    sd t0, -24(s0)\n\
    addi t1, t1, 4\n\
    ld t0, -24(s0)\n\
    ret\n";
        let result = peephole_optimize(input.to_string());
        assert!(result.contains("sd t0, -24(s0)"));
        assert!(!result.contains("ld t0, -24(s0)"));
    }

    #[test]
    fn test_gsf_different_reg_forwarding() {
        // Store t0 then load t1 from same slot — replace load with mv
        let input = "\
    sd t0, -24(s0)\n\
    addi t2, t2, 4\n\
    ld t1, -24(s0)\n\
    ret\n";
        let result = peephole_optimize(input.to_string());
        assert!(!result.contains("ld t1, -24(s0)"));
        assert!(result.contains("mv t1, t0"));
    }

    #[test]
    fn test_gsf_invalidation_on_reg_overwrite() {
        // After t0 is overwritten, the mapping slot → t0 is stale
        let input = "\
    sd t0, -24(s0)\n\
    li t0, 42\n\
    ld t1, -24(s0)\n\
    ret\n";
        let result = peephole_optimize(input.to_string());
        // The load should NOT be forwarded since t0 was overwritten
        assert!(result.contains("ld t1, -24(s0)"));
    }

    #[test]
    fn test_gsf_invalidation_at_label() {
        // Mappings are invalidated at labels
        let input = "\
    sd t0, -24(s0)\n\
    jump .LBB1, t6\n\
.LBB1:\n\
    ld t0, -24(s0)\n\
    ret\n";
        let result = peephole_optimize(input.to_string());
        // Label invalidates mappings, so load should remain
        assert!(result.contains("ld t0, -24(s0)"));
    }

    #[test]
    fn test_global_dse() {
        // Store to a slot that is never loaded — dead store
        let input = "\
    sd t0, -24(s0)\n\
    li t0, 0\n\
    ret\n";
        let result = peephole_optimize(input.to_string());
        assert!(!result.contains("sd t0, -24(s0)"));
    }

    #[test]
    fn test_global_dse_with_addr_taken() {
        // When frame pointer address is taken, no stores eliminated
        let input = "\
    sd t0, -24(s0)\n\
    addi t1, s0, -24\n\
    ret\n";
        let result = peephole_optimize(input.to_string());
        // Store must be preserved
        assert!(result.contains("sd t0, -24(s0)"));
    }

    #[test]
    fn test_copy_prop_word_boundary() {
        assert_eq!(replace_whole_word("t10, t1", "t1", "s1"), "t10, s1");
    }

    #[test]
    fn test_copy_prop_no_false_match() {
        assert_eq!(replace_whole_word("s10", "s1", "s5"), "s10");
    }

    #[test]
    fn test_copy_prop_no_symbol_corruption() {
        // Register name inside symbol names must not be replaced
        assert_eq!(replace_whole_word(" main.s1.0", "s1", "t0"), " main.s1.0");
        assert_eq!(replace_whole_word(" _s1_var", "s1", "t0"), " _s1_var");
    }

    #[test]
    fn test_lla_not_corrupted_by_copy_prop() {
        // Regression test: copy propagation must not corrupt symbol names in lla
        let input = "\
    mv t0, s1\n\
    lla t0, main.s1.0\n\
    ret\n";
        let result = peephole_optimize(input.to_string());
        assert!(
            result.contains("main.s1.0"),
            "Symbol name corrupted, got:\n{}",
            result
        );
        assert!(
            !result.contains("main.t0.0"),
            "Symbol name corrupted to main.t0.0"
        );
    }

    #[test]
    fn test_copy_prop_basic() {
        // mv t0, s1 ; addw t2, t0, t1 → addw t2, s1, t1
        let input = "\
    mv t0, s1\n\
    addw t2, t0, t1\n\
    ret\n";
        let result = peephole_optimize(input.to_string());
        assert!(result.contains("addw t2, s1, t1"));
    }

    #[test]
    fn test_dead_reg_move() {
        // mv t1, t0 ; li t1, 42 → (t1 overwritten, first mv is dead)
        let input = "\
    mv t1, t0\n\
    li t1, 42\n\
    mv a0, t1\n\
    ret\n";
        let result = peephole_optimize(input.to_string());
        assert!(
            !result.contains("mv t1, t0"),
            "Expected dead move eliminated, got:\n{}",
            result
        );
        assert!(result.contains("li t1, 42"));
    }

    #[test]
    fn test_full_bb_copy_prop() {
        // mv t0, s1 ; mv t1, t0 ; add t2, t1, t0
        // Copy propagation is single-step lookahead:
        // 1. mv t0, s1 → propagates into next: mv t1, t0 → mv t1, s1
        // 2. mv t1, s1 → propagates into next: add t2, t1, t0 → add t2, s1, t0
        // t0 is not further propagated since mv t0, s1 is no longer adjacent to the add.
        let input = "\
    mv t0, s1\n\
    mv t1, t0\n\
    add t2, t1, t0\n\
    ret\n";
        let result = peephole_optimize(input.to_string());
        assert!(
            result.contains("mv t1, s1"),
            "Expected t1 = s1 via chain, got:\n{}",
            result
        );
        assert!(
            result.contains("add t2, s1,"),
            "Expected t1 propagated in add, got:\n{}",
            result
        );
    }
}
