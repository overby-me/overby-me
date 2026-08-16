//! AArch64 assembly parser.
//!
//! Parses the textual assembly format emitted by our AArch64 codegen into
//! structured `AsmStatement` values. The parser handles:
//! - Labels (global and local)
//! - Directives (.section, .globl, .type, .align, .byte, .long, .xword, etc.)
//! - AArch64 instructions (mov, add, sub, ldr, str, bl, ret, etc.)
//! - CFI directives (passed through as-is for DWARF unwind info)

// Some parser helper functions and enum variants are defined for completeness
// and used only by the encoder or ELF writer, not the parser entry point itself.
#![allow(dead_code)]

use crate::backend::asm_expr;
use crate::backend::asm_preprocess;
use crate::backend::elf;

/// A parsed assembly operand.
#[derive(Debug, Clone)]
pub enum Operand {
    /// Register: x0-x30, w0-w30, sp, xzr, wzr, d0-d31, s0-s31, q0-q31, v0-v31
    Reg(String),
    /// Immediate value: #42, #-1, #0x1000
    Imm(i64),
    /// Symbol reference: function name, label, etc.
    Symbol(String),
    /// Symbol with addend: symbol+offset or symbol-offset
    SymbolOffset(String, i64),
    /// Memory operand: [base] or [base, #offset]
    Mem { base: String, offset: i64 },
    /// Memory operand with symbolic offset expression: [base, #(sym_expr)] or [base, #(sym_expr)]!
    /// Used when the offset is a label/symbol expression that can't be resolved at parse time.
    MemExpr {
        base: String,
        expr: String,
        writeback: bool,
    },
    /// Memory operand with pre-index writeback: [base, #offset]!
    MemPreIndex { base: String, offset: i64 },
    /// Memory operand with post-index writeback: [base], #offset
    MemPostIndex { base: String, offset: i64 },
    /// Memory operand with register offset: [base, Xm]
    MemRegOffset {
        base: String,
        index: String,
        extend: Option<String>,
        shift: Option<u8>,
    },
    /// :lo12:symbol or :got_lo12:symbol modifier
    Modifier { kind: String, symbol: String },
    /// :lo12:symbol+offset
    ModifierOffset {
        kind: String,
        symbol: String,
        offset: i64,
    },
    /// Shift: lsl #N, lsr #N, asr #N
    Shift { kind: String, amount: u32 },
    /// Extend: sxtw, uxtw, sxtx, etc. with optional shift amount
    Extend { kind: String, amount: u32 },
    /// Condition code for csel etc.: eq, ne, lt, gt, ...
    Cond(String),
    /// Barrier option for dmb/dsb: ish, ishld, ishst, sy, etc.
    Barrier(String),
    /// Label reference for branches
    Label(String),
    /// Raw expression (for things we can't fully parse yet)
    Expr(String),
    /// NEON register with arrangement specifier: v0.8b, v0.16b, v0.4s, etc.
    RegArrangement { reg: String, arrangement: String },
    /// NEON register with lane index: v0.d[1], v0.b[0], v0.s[2], etc.
    RegLane {
        reg: String,
        elem_size: String,
        index: u32,
    },
    /// NEON register list: {v0.16b}, {v0.16b, v1.16b}, etc.
    RegList(Vec<Operand>),
    /// NEON register list with element index: {v0.s, v1.s}[0], {v0.d, v1.d}[1], etc.
    RegListIndexed { regs: Vec<Operand>, index: u32 },
}

/// Section directive with optional flags and type.
#[derive(Debug, Clone)]
pub struct SectionDirective {
    pub name: String,
    pub flags: Option<String>,
    pub section_type: Option<String>,
}

/// Symbol kind from `.type` directive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    Function,
    Object,
    TlsObject,
    NoType,
}

/// Size expression: either a constant or `.-name` (current position minus symbol).
#[derive(Debug, Clone)]
pub enum SizeExpr {
    Constant(u64),
    CurrentMinusSymbol(String),
}

/// A data value that can be a constant, a symbol, or a symbol expression.
#[derive(Debug, Clone)]
pub enum DataValue {
    Integer(i64),
    Symbol(String),
    /// symbol + offset (e.g., `.quad func+128`)
    SymbolOffset(String, i64),
    /// symbol - symbol (e.g., `.long .LBB3 - .Ljt_0`)
    SymbolDiff(String, String),
    /// symbol - symbol + addend
    SymbolDiffAddend(String, String, i64),
    /// Raw expression string for deferred evaluation (e.g., `(sym_a - sym_b) >> 5`)
    Expr(String),
}

/// A typed assembly directive, fully parsed at parse time.
#[derive(Debug, Clone)]
pub enum AsmDirective {
    /// Switch to a named section: `.section .text,"ax",@progbits`
    Section(SectionDirective),
    /// Global symbol: `.globl name`
    Global(String),
    /// Weak symbol: `.weak name`
    Weak(String),
    /// Hidden visibility: `.hidden name`
    Hidden(String),
    /// Protected visibility: `.protected name`
    Protected(String),
    /// Internal visibility: `.internal name`
    Internal(String),
    /// Symbol type: `.type name, %function`
    SymbolType(String, SymbolKind),
    /// Symbol size: `.size name, expr`
    Size(String, SizeExpr),
    /// Alignment: `.align N` or `.p2align N` (stored as byte count, already converted from 2^N)
    Align(u64),
    /// Byte-alignment: `.balign N` (stored as byte count directly)
    Balign(u64),
    /// Emit bytes: `.byte val, val, ...` (can be symbol differences for size computations)
    Byte(Vec<DataValue>),
    /// Emit 16-bit values: `.short val, ...`
    Short(Vec<i16>),
    /// Emit 32-bit values: `.long val, ...` (can be symbol references)
    Long(Vec<DataValue>),
    /// Emit 64-bit values: `.quad val, ...` (can be symbol references)
    Quad(Vec<DataValue>),
    /// Emit zero bytes: `.zero N[, fill]`
    Zero(usize, u8),
    /// NUL-terminated string: `.asciz "str"`
    Asciz(Vec<u8>),
    /// String without NUL: `.ascii "str"`
    Ascii(Vec<u8>),
    /// Common symbol: `.comm name, size, align`
    Comm(String, u64, u64),
    /// Local symbol: `.local name`
    Local(String),
    /// Symbol alias: `.set name, value`
    Set(String, String),
    /// Push current section and switch to a new one: `.pushsection name,"flags",@type`
    PushSection(SectionDirective),
    /// Pop section stack and restore previous section: `.popsection`
    PopSection,
    /// `.previous` — swap current and previous sections
    Previous,
    /// `.subsection N` — switch to numbered subsection within the current section
    Subsection(u64),
    /// CFI directive (ignored for code generation)
    Cfi,
    /// `.incbin "file"[, skip[, count]]` — include binary file contents
    Incbin {
        path: String,
        skip: u64,
        count: Option<u64>,
    },
    /// Raw bytes emitted from .float, .double, etc.
    RawBytes(Vec<u8>),
    /// Literal pool dump: `.ltorg` or `.pool`
    Ltorg,
    /// Other ignored directives (.file, .loc, .ident, etc.)
    Ignored,
}

/// A parsed assembly statement.
#[derive(Debug, Clone)]
pub enum AsmStatement {
    /// A label definition: "name:"
    Label(String),
    /// A typed directive, fully parsed
    Directive(AsmDirective),
    /// An AArch64 instruction with mnemonic and operands
    Instruction {
        mnemonic: String,
        operands: Vec<Operand>,
        /// The raw text of the operand string (for fallback encoding)
        raw_operands: String,
    },
    /// An empty line or comment
    Empty,
    /// Literal pool load pseudo-instruction: `ldr Rd, =symbol[+offset]`
    /// Will be expanded into `ldr Rd, .Llpool_N` + pool entries by a later pass.
    LdrLiteralPool {
        reg: String,
        symbol: String,
        addend: i64,
    },
}

// C-style /* ... */ comment stripping is handled by asm_preprocess::strip_c_comments
// (shared, string-aware version that correctly handles `/*` inside quoted strings).

/// Parse assembly text into a list of statements.
/// Expand .rept/.endr, .irp/.endr, and .irpc/.endr blocks by repeating contained lines.
// TODO: extract expand_rept_blocks to shared module (duplicated in ARM, RISC-V, x86 parsers)
fn is_rept_start(trimmed: &str) -> bool {
    trimmed.starts_with(".rept ")
        || trimmed.starts_with(".rept\t")
        || trimmed.starts_with(".rep ")
        || trimmed.starts_with(".rep\t")
}

fn is_irp_start(trimmed: &str) -> bool {
    trimmed.starts_with(".irp ") || trimmed.starts_with(".irp\t")
}

fn is_irpc_start(trimmed: &str) -> bool {
    trimmed.starts_with(".irpc ") || trimmed.starts_with(".irpc\t")
}

fn is_block_start(trimmed: &str) -> bool {
    is_rept_start(trimmed) || is_irp_start(trimmed) || is_irpc_start(trimmed)
}

/// Collect the body lines of a .rept/.irp block, handling nesting.
/// Returns the body lines and advances i past the closing .endr.
fn collect_block_body<'a>(lines: &[&'a str], i: &mut usize) -> Result<Vec<&'a str>, String> {
    let mut depth = 1;
    let mut body = Vec::new();
    *i += 1;
    while *i < lines.len() {
        let inner = strip_comment(lines[*i]).trim().to_string();
        if is_block_start(&inner) {
            depth += 1;
        } else if inner == ".endr" {
            depth -= 1;
            if depth == 0 {
                break;
            }
        }
        body.push(lines[*i]);
        *i += 1;
    }
    if depth != 0 {
        return Err(".rept/.irp/.irpc without matching .endr".to_string());
    }
    Ok(body)
}

/// Estimate the byte size of a single assembly line for label position tracking.
/// Used to resolve backward label references in .rept count expressions.
/// AArch64 instructions are always 4 bytes; directives have known sizes.
fn estimate_line_bytes(trimmed: &str) -> u64 {
    if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with('#') {
        return 0;
    }
    // Label definitions don't add bytes
    if trimmed.ends_with(':') && !trimmed.contains(' ') {
        return 0;
    }
    // Strip leading labels like "661:" from lines like "661: bl foo"
    let content = if let Some(pos) = trimmed.find(':') {
        let before = &trimmed[..pos];
        if before
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
        {
            trimmed[pos + 1..].trim()
        } else {
            trimmed
        }
    } else {
        trimmed
    };
    if content.is_empty() {
        return 0;
    }
    // Directives
    if content.starts_with('.') {
        let lower = content.to_lowercase();
        if lower.starts_with(".byte ") {
            return 1;
        }
        if lower.starts_with(".hword ")
            || lower.starts_with(".short ")
            || lower.starts_with(".2byte ")
        {
            return 2;
        }
        if lower.starts_with(".word ")
            || lower.starts_with(".long ")
            || lower.starts_with(".4byte ")
            || lower.starts_with(".inst ")
        {
            return 4;
        }
        if lower.starts_with(".quad ")
            || lower.starts_with(".xword ")
            || lower.starts_with(".8byte ")
        {
            return 8;
        }
        // .zero N, .space N, .skip N
        if lower.starts_with(".zero ")
            || lower.starts_with(".space ")
            || lower.starts_with(".skip ")
        {
            let arg = content.split_whitespace().nth(1).unwrap_or("0");
            if let Ok(n) = arg.trim_end_matches(',').parse::<u64>() {
                return n;
            }
        }
        // .ascii "str" / .asciz "str" — approximate
        if lower.starts_with(".ascii")
            || lower.starts_with(".asciz")
            || lower.starts_with(".string")
        {
            // Don't try to be exact; these are rarely used in .rept context
            return 0;
        }
        // Other directives (.align, .section, .globl, .type, etc.) — 0 bytes of code
        return 0;
    }
    // Everything else is an instruction = 4 bytes for AArch64
    4
}

/// Resolve backward numeric label references (like 662b, 661b) in a .rept count expression.
/// Substitutes each backward reference with its byte position, then evaluates the expression.
fn resolve_rept_label_expr(
    count_str: &str,
    label_positions: &std::collections::HashMap<String, Vec<u64>>,
) -> Result<i64, String> {
    // First try direct evaluation (handles simple integer expressions)
    if let Ok(val) = asm_expr::parse_integer_expr(count_str) {
        return Ok(val);
    }

    // Check if expression contains backward label references (e.g., 662b)
    let mut resolved = count_str.to_string();
    let mut found_label_ref = false;

    // Find and replace backward label references: digits followed by 'b' or 'B'
    // We need to scan for patterns like "662b" that are numeric label backward refs
    loop {
        let mut replaced = false;
        // Use a simple scan to find numeric label backward references
        let bytes = resolved.as_bytes();
        let len = bytes.len();
        let mut i = 0;
        while i < len {
            // Look for a digit sequence followed by 'b' or 'B'
            if bytes[i].is_ascii_digit() {
                let start = i;
                while i < len && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                if i < len && (bytes[i] == b'b' || bytes[i] == b'B') {
                    // Check that the next char (if any) is not alphanumeric
                    // (to avoid matching things like "0b1010" binary literals)
                    let after_ok = i + 1 >= len || !bytes[i + 1].is_ascii_alphanumeric();
                    // Also ensure it's not a binary literal starting with 0b
                    let is_binary = start + 1 == i && bytes[start] == b'0';
                    if after_ok && !is_binary {
                        let label_num = &resolved[start..i];
                        let ref_end = i + 1;
                        // Look up the most recent definition of this label
                        if let Some(positions) = label_positions.get(label_num) {
                            if let Some(&pos) = positions.last() {
                                let before = &resolved[..start];
                                let after = &resolved[ref_end..];
                                resolved = format!("{}{}{}", before, pos, after);
                                found_label_ref = true;
                                replaced = true;
                                break; // restart scan since string changed
                            }
                        }
                    }
                }
            }
            i += 1;
        }
        if !replaced {
            break;
        }
    }

    if found_label_ref {
        asm_expr::parse_integer_expr(&resolved)
    } else {
        Err(format!("cannot evaluate .rept count: {}", count_str))
    }
}

fn expand_rept_blocks(lines: &[&str]) -> Result<Vec<String>, String> {
    let mut result = Vec::new();
    let mut i = 0;
    // Track numeric label positions (byte offsets) for resolving backward refs
    let mut label_positions: std::collections::HashMap<String, Vec<u64>> =
        std::collections::HashMap::new();
    let mut current_byte_pos: u64 = 0;
    while i < lines.len() {
        let trimmed = strip_comment(lines[i]).trim().to_string();
        if is_rept_start(&trimmed) {
            let prefix_len = if trimmed.starts_with(".rept") { 5 } else { 4 };
            let count_str = trimmed[prefix_len..].trim();
            let count_val = resolve_rept_label_expr(count_str, &label_positions).unwrap_or(0);
            // Treat negative counts as 0 (matches GNU as behavior)
            let count = if count_val < 0 {
                0usize
            } else {
                count_val as usize
            };
            let body = collect_block_body(lines, &mut i)?;
            let expanded_body = expand_rept_blocks(&body)?;
            for _ in 0..count {
                result.extend(expanded_body.iter().cloned());
            }
        } else if is_irp_start(&trimmed) {
            // .irp var, val1, val2, ...
            let args_str = trimmed[".irp".len()..].trim();
            // Split on first comma to get variable name and values
            let (var, values_str) = match args_str.find(',') {
                Some(pos) => (args_str[..pos].trim(), args_str[pos + 1..].trim()),
                None => (args_str, ""),
            };
            let values: Vec<&str> = values_str.split(',').map(|s| s.trim()).collect();
            let body = collect_block_body(lines, &mut i)?;
            for val in &values {
                // Substitute \var with val in each body line
                let subst_body: Vec<String> = body
                    .iter()
                    .map(|line| {
                        let pattern = format!("\\{}", var);
                        asm_preprocess::replace_macro_param(line, &pattern, val)
                    })
                    .collect();
                let subst_refs: Vec<&str> = subst_body.iter().map(|s| s.as_str()).collect();
                let expanded = expand_rept_blocks(&subst_refs)?;
                result.extend(expanded);
            }
        } else if is_irpc_start(&trimmed) {
            // .irpc var, string — iterate over each character in the string
            let args_str = trimmed[".irpc".len()..].trim();
            // Split on first comma to get variable name and string
            let (var, char_str) = match args_str.find(',') {
                Some(pos) => (args_str[..pos].trim(), args_str[pos + 1..].trim()),
                None => (args_str, ""),
            };
            let body = collect_block_body(lines, &mut i)?;
            for ch in char_str.chars() {
                // Substitute \var with the current character in each body line
                let ch_str = ch.to_string();
                let subst_body: Vec<String> = body
                    .iter()
                    .map(|line| {
                        let pattern = format!("\\{}", var);
                        let substituted =
                            asm_preprocess::replace_macro_param(line, &pattern, &ch_str);
                        // Strip GAS macro argument delimiters: \() resolves to empty string
                        substituted.replace("\\()", "")
                    })
                    .collect();
                let subst_refs: Vec<&str> = subst_body.iter().map(|s| s.as_str()).collect();
                let expanded = expand_rept_blocks(&subst_refs)?;
                result.extend(expanded);
            }
        } else if trimmed == ".endr" {
            // stray .endr without .rept - skip
        } else {
            // Track numeric label definitions and byte positions
            // Check if this line defines a numeric label (e.g., "661:" or "661: instruction")
            if let Some(colon_pos) = trimmed.find(':') {
                let before = &trimmed[..colon_pos];
                if !before.is_empty() && before.chars().all(|c| c.is_ascii_digit()) {
                    label_positions
                        .entry(before.to_string())
                        .or_default()
                        .push(current_byte_pos);
                }
            }
            current_byte_pos += estimate_line_bytes(&trimmed);
            result.push(lines[i].to_string());
        }
        i += 1;
    }
    Ok(result)
}

/// Evaluate a `.if` condition expression using the shared implementation.
/// Supports: `==`, `!=`, `>`, `>=`, `<`, `<=`, `||`, `&&`, parentheses.
fn eval_if_condition(cond: &str) -> bool {
    asm_preprocess::eval_if_condition(cond)
}

/// Split macro invocation arguments, separating on commas (preferred) or whitespace.
/// If the argument string contains commas, only commas are used as separators
/// (allowing spaces within arguments like `20 - 8`). If no commas are present,
/// whitespace acts as separator for backwards compatibility.
/// Quoted strings are kept as a single argument with quotes stripped.
/// Parenthesized groups like `0(a1)` are kept together.
fn split_macro_args(s: &str) -> Vec<String> {
    if s.is_empty() {
        return Vec::new();
    }
    // Determine if commas are present (outside parens) — if so, use comma-only splitting
    let has_commas = {
        let mut depth = 0i32;
        s.bytes().any(|b| match b {
            b'(' => {
                depth += 1;
                false
            }
            b')' => {
                depth -= 1;
                false
            }
            b',' if depth == 0 => true,
            _ => false,
        })
    };
    let mut args = Vec::new();
    let mut current = String::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut paren_depth = 0i32;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => {
                paren_depth += 1;
                current.push('(');
            }
            b')' => {
                paren_depth -= 1;
                current.push(')');
            }
            b',' if paren_depth == 0 => {
                let trimmed = current.trim().to_string();
                if !trimmed.is_empty() {
                    args.push(trimmed);
                }
                current.clear();
            }
            b' ' | b'\t' if paren_depth == 0 && !has_commas => {
                // Whitespace acts as separator when no commas are present.
                // However, we need to be careful with expressions like "20 - 8".
                // Check if the whitespace is part of an arithmetic expression.
                let trimmed_so_far = current.trim();
                let last_is_num_or_op = trimmed_so_far.is_empty()
                    || trimmed_so_far
                        .chars()
                        .last()
                        .is_some_and(|c| c.is_ascii_digit() || c == ')');
                // Peek ahead: skip whitespace to see what follows
                let mut peek = i + 1;
                while peek < bytes.len() && (bytes[peek] == b' ' || bytes[peek] == b'\t') {
                    peek += 1;
                }
                let next_is_op = peek < bytes.len()
                    && matches!(
                        bytes[peek],
                        b'+' | b'-' | b'*' | b'/' | b'|' | b'&' | b'^' | b'~'
                    );
                let next_is_num_after_op = if !current.is_empty() {
                    let last_ch = current.as_bytes()[current.len() - 1];
                    matches!(
                        last_ch,
                        b'+' | b'-' | b'*' | b'/' | b'|' | b'&' | b'^' | b'~' | b'('
                    ) && peek < bytes.len()
                        && (bytes[peek].is_ascii_digit() || bytes[peek] == b'(')
                } else {
                    false
                };
                if (last_is_num_or_op && next_is_op) || next_is_num_after_op {
                    // Part of an arithmetic expression — keep as whitespace in current token
                    current.push(' ');
                } else {
                    let trimmed = current.trim().to_string();
                    if !trimmed.is_empty() {
                        args.push(trimmed);
                        current.clear();
                    }
                }
                // Skip remaining whitespace
                while i + 1 < bytes.len() && (bytes[i + 1] == b' ' || bytes[i + 1] == b'\t') {
                    i += 1;
                }
            }
            b'"' => {
                // Consume quoted string, stripping the outer quotes
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    if bytes[i] == b'\\' && i + 1 < bytes.len() {
                        current.push(bytes[i + 1] as char);
                        i += 2;
                        continue;
                    }
                    current.push(bytes[i] as char);
                    i += 1;
                }
                // Skip closing quote
            }
            _ => {
                current.push(bytes[i] as char);
            }
        }
        i += 1;
    }
    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        args.push(trimmed);
    }
    args
}

/// Macro definition: name, parameter list (with optional defaults), and body lines.
struct MacroDef {
    /// Parameter names (without default values).
    params: Vec<String>,
    /// Default values for parameters (indexed by param position). None = no default.
    defaults: Vec<Option<String>>,
    body: Vec<String>,
    /// Whether the last parameter is :vararg (receives all remaining args).
    has_vararg: bool,
}

/// Parse a .macro directive line, returning (name, params, defaults, has_vararg).
/// Handles GAS syntax: `.macro name param1, param2=default, param3`
fn parse_macro_directive(trimmed: &str) -> (String, Vec<String>, Vec<Option<String>>, bool) {
    let rest = trimmed[".macro".len()..].trim();
    let (name, params_str) = match rest.find([' ', '\t', ',']) {
        Some(pos) => (
            rest[..pos].trim(),
            rest[pos..].trim().trim_start_matches(','),
        ),
        None => (rest, ""),
    };
    let mut params = Vec::new();
    let mut defaults = Vec::new();
    let mut has_vararg = false;
    if !params_str.is_empty() {
        // GAS allows spaces around '=' in macro parameter defaults:
        //   .macro foo enable = 1    =>  param "enable", default "1"
        // First try comma-separated specs, then handle space-separated with '=' merging.
        let specs: Vec<&str> = if params_str.contains(',') {
            params_str
                .split(',')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .collect()
        } else {
            // Space-separated: merge "name = value" triples into single specs.
            let tokens: Vec<&str> = params_str.split_whitespace().collect();
            let mut merged: Vec<String> = Vec::new();
            let mut i = 0;
            while i < tokens.len() {
                if i + 2 < tokens.len() && tokens[i + 1] == "=" {
                    merged.push(format!("{}={}", tokens[i], tokens[i + 2]));
                    i += 3;
                } else if i + 1 < tokens.len() && tokens[i + 1].starts_with('=') {
                    merged.push(format!("{}{}", tokens[i], tokens[i + 1]));
                    i += 2;
                } else {
                    merged.push(tokens[i].to_string());
                    i += 1;
                }
            }
            // Convert to a form we can iterate
            // We'll handle this below using the merged vector directly
            let mut specs_out = Vec::new();
            for m in &merged {
                specs_out.push(m.as_str());
            }
            // Since we can't return borrowed refs to local merged vec,
            // process directly here
            for m in &merged {
                let s = m.trim();
                if s.is_empty() {
                    continue;
                }
                if let Some(eq_pos) = s.find('=') {
                    params.push(s[..eq_pos].to_string());
                    defaults.push(Some(s[eq_pos + 1..].to_string()));
                } else if let Some(colon_pos) = s.find(':') {
                    let qualifier = &s[colon_pos + 1..];
                    if qualifier.eq_ignore_ascii_case("vararg") {
                        has_vararg = true;
                    }
                    params.push(s[..colon_pos].to_string());
                    defaults.push(None);
                } else {
                    params.push(s.to_string());
                    defaults.push(None);
                }
            }
            return (name.to_string(), params, defaults, has_vararg);
        };
        for raw in &specs {
            let s = raw.trim();
            if s.is_empty() {
                continue;
            }
            // Handle param=default or param:req syntax
            if let Some(eq_pos) = s.find('=') {
                params.push(s[..eq_pos].trim().to_string());
                defaults.push(Some(s[eq_pos + 1..].trim().to_string()));
            } else if let Some(colon_pos) = s.find(':') {
                let qualifier = &s[colon_pos + 1..];
                if qualifier.eq_ignore_ascii_case("vararg") {
                    has_vararg = true;
                }
                params.push(s[..colon_pos].to_string());
                defaults.push(None);
            } else {
                params.push(s.to_string());
                defaults.push(None);
            }
        }
    }
    (name.to_string(), params, defaults, has_vararg)
}

/// Collect a macro body from lines[i..], tracking nested .macro/.endm depth.
/// Returns the body lines and the index after the closing .endm.
fn collect_macro_body(lines: &[&str], start: usize) -> (Vec<String>, usize) {
    let mut body = Vec::new();
    let mut depth = 1;
    let mut i = start;
    while i < lines.len() {
        let inner = strip_comment(lines[i]).trim().to_string();
        if inner.starts_with(".macro ") || inner.starts_with(".macro\t") {
            depth += 1;
        } else if inner == ".endm" || inner.starts_with(".endm ") || inner.starts_with(".endm\t") {
            depth -= 1;
            if depth == 0 {
                return (body, i);
            }
        }
        body.push(lines[i].to_string());
        i += 1;
    }
    (body, i)
}

/// Substitute macro parameters in body lines.
///
/// Handles both positional and named arguments (e.g., `shift=1`).
/// Falls back to default values when arguments are not provided.
fn substitute_params(
    body: &[String],
    params: &[String],
    defaults: &[Option<String>],
    args: &[String],
    _has_vararg: bool,
) -> Vec<String> {
    // Build a map of param_name -> value, considering named args and defaults.
    let mut param_values: Vec<String> = Vec::with_capacity(params.len());
    // Start with defaults or "0" for each param
    for (i, _param) in params.iter().enumerate() {
        param_values.push(defaults.get(i).and_then(|d| d.clone()).unwrap_or_default());
    }
    // Apply positional and named arguments
    let mut pos_idx = 0;
    for arg in args.iter() {
        if let Some(eq_pos) = arg.find('=') {
            // Named argument: "param=value"
            let name = &arg[..eq_pos];
            let value = &arg[eq_pos + 1..];
            if let Some(pi) = params.iter().position(|p| p == name) {
                param_values[pi] = value.to_string();
            }
        } else {
            // Positional argument
            if pos_idx < params.len() {
                param_values[pos_idx] = arg.clone();
            }
            pos_idx += 1;
        }
    }

    // Sort parameter indices by name length (longest first) to avoid
    // partial substitution: e.g., \x must not match before \xb.
    let mut sorted_indices: Vec<usize> = (0..params.len()).collect();
    sorted_indices.sort_by(|&a, &b| params[b].len().cmp(&params[a].len()));

    body.iter()
        .map(|body_line| {
            let mut expanded = body_line.clone();
            for &pi in &sorted_indices {
                let pattern = format!("\\{}", params[pi]);
                expanded =
                    asm_preprocess::replace_macro_param(&expanded, &pattern, &param_values[pi]);
            }
            // Strip GAS macro argument delimiters: \() resolves to empty string.
            // Used in GAS macros to separate parameter names from adjacent text,
            // e.g., \param\().suffix → value.suffix after substitution.
            expanded = expanded.replace("\\()", "");
            expanded
        })
        .collect()
}

/// Expand .macro/.endm definitions and macro invocations in a single pass.
///
/// Handles nested macros: when a macro body contains .macro/.endm definitions
/// (like FFmpeg's `function` macro which defines `endfunc` inside its body),
/// those definitions are registered in the macro table during expansion.
/// Also handles .purgem to remove macro definitions (used by FFmpeg to allow
/// each `function` invocation to redefine `endfunc`).
fn expand_macros(lines: &[&str]) -> Result<Vec<String>, String> {
    use std::collections::HashMap;
    let mut macros: HashMap<String, MacroDef> = HashMap::new();
    let mut counter = 0u64;
    expand_macros_impl(lines, &mut macros, 0, &mut counter)
}

/// Core macro expansion implementation with a shared, mutable macro table.
/// `depth` limits recursion to prevent infinite expansion.
/// `counter` is GAS's `\@` macro invocation counter.
fn expand_macros_impl(
    lines: &[&str],
    macros: &mut std::collections::HashMap<String, MacroDef>,
    depth: usize,
    counter: &mut u64,
) -> Result<Vec<String>, String> {
    if depth > 64 {
        return Err("Macro expansion depth limit exceeded (>64)".to_string());
    }
    let mut result = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let trimmed = strip_comment(lines[i]).trim().to_string();

        if trimmed.starts_with(".macro ") || trimmed.starts_with(".macro\t") {
            // Collect macro definition (with nested depth tracking)
            let (name, params, defaults, has_vararg) = parse_macro_directive(&trimmed);
            let (body, end_idx) = collect_macro_body(lines, i + 1);
            macros.insert(
                name,
                MacroDef {
                    params,
                    defaults,
                    body,
                    has_vararg,
                },
            );
            i = end_idx + 1;
            continue;
        } else if trimmed == ".endm"
            || trimmed.starts_with(".endm ")
            || trimmed.starts_with(".endm\t")
        {
            // Stray .endm — skip
            i += 1;
            continue;
        } else if trimmed.starts_with(".purgem ") || trimmed.starts_with(".purgem\t") {
            // Remove a macro definition
            let name = trimmed[".purgem".len()..].trim();
            macros.remove(name);
            i += 1;
            continue;
        } else if !trimmed.is_empty() && !trimmed.starts_with('.') && !trimmed.starts_with('#') {
            // Could be a macro invocation
            let first_word = trimmed.split([' ', '\t']).next().unwrap_or("");
            let potential_name = first_word.trim_end_matches(':');
            if potential_name != first_word {
                // It's a label (has trailing colon) — check if followed by a macro invocation
                let rest_after_label = trimmed[first_word.len()..].trim();
                let rest_first_word = rest_after_label.split([' ', '\t']).next().unwrap_or("");
                if !rest_first_word.is_empty() && macros.contains_key(rest_first_word) {
                    // Label followed by macro invocation: emit label, then expand macro
                    result.push(first_word.to_string());
                    let mac_params = macros[rest_first_word].params.clone();
                    let mac_defaults = macros[rest_first_word].defaults.clone();
                    let mac_body = macros[rest_first_word].body.clone();
                    let mac_vararg = macros[rest_first_word].has_vararg;
                    let macro_args_str =
                        rest_after_label[rest_first_word.len()..].trim().to_string();
                    let macro_args = split_macro_args(&macro_args_str);
                    let mut expanded_lines = substitute_params(
                        &mac_body,
                        &mac_params,
                        &mac_defaults,
                        &macro_args,
                        mac_vararg,
                    );
                    let ctr_str = counter.to_string();
                    for line in &mut expanded_lines {
                        *line = line.replace("\\@", &ctr_str);
                    }
                    *counter += 1;
                    let refs: Vec<&str> = expanded_lines.iter().map(|s| s.as_str()).collect();
                    let re_expanded = expand_macros_impl(&refs, macros, depth + 1, counter)?;
                    result.extend(re_expanded);
                } else {
                    result.push(lines[i].to_string());
                }
            } else if macros.contains_key(potential_name) {
                // Clone what we need before mutably borrowing macros again
                let mac_params = macros[potential_name].params.clone();
                let mac_defaults = macros[potential_name].defaults.clone();
                let mac_body = macros[potential_name].body.clone();
                let mac_vararg = macros[potential_name].has_vararg;
                let args_str = trimmed[first_word.len()..].trim().to_string();
                let args = if mac_vararg && !mac_params.is_empty() {
                    // For vararg macros, split only the non-vararg params,
                    // and pass the remaining raw text as the vararg value
                    let all_args = split_macro_args(&args_str);
                    let non_vararg_count = mac_params.len() - 1;
                    if all_args.len() > non_vararg_count {
                        // Find where the vararg portion starts in the raw string
                        let mut vararg_args = all_args[..non_vararg_count].to_vec();
                        // Reconstruct the raw vararg text by finding the position after
                        // the non-vararg args in the original string
                        let mut pos = 0;
                        for _ in 0..non_vararg_count {
                            // Skip whitespace/commas
                            while pos < args_str.len()
                                && (args_str.as_bytes()[pos] == b' '
                                    || args_str.as_bytes()[pos] == b'\t'
                                    || args_str.as_bytes()[pos] == b',')
                            {
                                pos += 1;
                            }
                            // Skip the arg token
                            while pos < args_str.len()
                                && args_str.as_bytes()[pos] != b','
                                && args_str.as_bytes()[pos] != b' '
                                && args_str.as_bytes()[pos] != b'\t'
                            {
                                pos += 1;
                            }
                        }
                        // Skip separator after last non-vararg arg
                        while pos < args_str.len()
                            && (args_str.as_bytes()[pos] == b' '
                                || args_str.as_bytes()[pos] == b'\t'
                                || args_str.as_bytes()[pos] == b',')
                        {
                            pos += 1;
                        }
                        let raw_vararg = args_str[pos..].to_string();
                        vararg_args.push(raw_vararg);
                        vararg_args
                    } else {
                        all_args
                    }
                } else {
                    split_macro_args(&args_str)
                };
                let mut expanded_lines =
                    substitute_params(&mac_body, &mac_params, &mac_defaults, &args, mac_vararg);
                let ctr_str = counter.to_string();
                for line in &mut expanded_lines {
                    *line = line.replace("\\@", &ctr_str);
                }
                *counter += 1;
                // Recursively expand the result (may define new macros, invoke others)
                let refs: Vec<&str> = expanded_lines.iter().map(|s| s.as_str()).collect();
                let re_expanded = expand_macros_impl(&refs, macros, depth + 1, counter)?;
                result.extend(re_expanded);
            } else {
                result.push(lines[i].to_string());
            }
        } else {
            result.push(lines[i].to_string());
        }
        i += 1;
    }
    Ok(result)
}

/// Resolve .set/.equ constants with simple integer values in instruction lines.
///
/// Sequential single-pass: as each `.set name, value` is encountered, update the
/// constant map. For non-.set lines, substitute whole-word occurrences of known
/// constant names with their current values. This correctly handles constants that
/// are reassigned (e.g., `.Lasm_alt_mode` in kernel ALTERNATIVE macros).
/// The `.set` directives themselves are preserved for the encoder to process.
fn resolve_set_constants(lines: &[String]) -> Vec<String> {
    use std::collections::HashMap;
    let mut constants: HashMap<String, String> = HashMap::new();
    let mut result = Vec::with_capacity(lines.len());

    // Single sequential pass: update constants as .set directives are encountered,
    // and substitute using the current map state at each line. This correctly handles
    // constants that are reassigned (e.g., .Lasm_alt_mode in kernel ALTERNATIVE macros).
    for line in lines {
        let trimmed = strip_comment(line).trim().to_lowercase();
        if trimmed.starts_with(".set ")
            || trimmed.starts_with(".set\t")
            || trimmed.starts_with(".equ ")
            || trimmed.starts_with(".equ\t")
        {
            let rest = strip_comment(line).trim();
            let directive_len = 4; // both .set and .equ are 4 chars
            let args = rest[directive_len..].trim();
            if let Some(comma_pos) = args.find(',') {
                let name = args[..comma_pos].trim().to_string();
                let value_str = args[comma_pos + 1..].trim().to_string();
                // Only resolve simple integer values
                if let Ok(_val) = asm_expr::parse_integer_expr(&value_str) {
                    constants.insert(name, value_str);
                }
            }
            result.push(line.clone());
        } else if constants.is_empty() {
            result.push(line.clone());
        } else {
            // Substitute current constant values in this line
            let mut substituted = line.clone();
            for (name, value) in &constants {
                // Replace whole-word occurrences of the constant name
                let mut new_result = String::with_capacity(substituted.len());
                let bytes = substituted.as_bytes();
                let name_bytes = name.as_bytes();
                let mut i = 0;
                while i < bytes.len() {
                    if i + name_bytes.len() <= bytes.len()
                        && &bytes[i..i + name_bytes.len()] == name_bytes
                    {
                        // Check word boundary before
                        let before_ok = i == 0 || !is_ident_char(bytes[i - 1]);
                        // Check word boundary after
                        let after_pos = i + name_bytes.len();
                        let after_ok = after_pos >= bytes.len() || !is_ident_char(bytes[after_pos]);
                        if before_ok && after_ok {
                            new_result.push_str(value);
                            i += name_bytes.len();
                            continue;
                        }
                    }
                    new_result.push(bytes[i] as char);
                    i += 1;
                }
                substituted = new_result;
            }
            result.push(substituted);
        }
    }

    result
}

/// Check if a byte is a valid identifier character (alphanumeric, underscore, dot).
fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'.'
}

/// Resolve `.req` / `.unreq` register aliases.
///
/// First pass: collect `name .req register` definitions and `.unreq name` removals.
/// Second pass: substitute alias names with their register values in all non-directive lines.
/// The `.req` / `.unreq` lines themselves are preserved (they'll be parsed as Empty later).
fn resolve_register_aliases(lines: &[String]) -> Vec<String> {
    use std::collections::HashMap;
    let mut aliases: HashMap<String, String> = HashMap::new();

    // First pass: collect alias definitions and removals in order
    for line in lines {
        let trimmed = strip_comment(line).trim();
        // Match "name .req register"
        if let Some(req_pos) = trimmed.find(".req") {
            // Ensure it's " .req " or "\t.req " (not part of a longer directive like .reqXYZ)
            let after_req = req_pos + 4;
            if req_pos > 0
                && (trimmed.as_bytes()[req_pos - 1] == b' '
                    || trimmed.as_bytes()[req_pos - 1] == b'\t')
                && after_req < trimmed.len()
                && (trimmed.as_bytes()[after_req] == b' ' || trimmed.as_bytes()[after_req] == b'\t')
            {
                let name = trimmed[..req_pos].trim();
                let register = trimmed[after_req..].trim();
                if !name.is_empty() && !register.is_empty() {
                    aliases.insert(name.to_string(), register.to_string());
                }
                continue;
            }
        }
        // Note: We intentionally DON'T process .unreq here.
        // .unreq is used to allow re-definition of aliases in different scopes,
        // but for our two-pass approach, we want ALL aliases available for substitution.
        // The .unreq directive is handled by the parser as an ignored directive.
    }

    if aliases.is_empty() {
        return lines.to_vec();
    }

    // Sort aliases by name length (longest first) to avoid partial substitution
    let mut sorted_aliases: Vec<(&String, &String)> = aliases.iter().collect();
    sorted_aliases.sort_by(|a, b| b.0.len().cmp(&a.0.len()));

    // Second pass: substitute aliases in all lines
    lines
        .iter()
        .map(|line| {
            let trimmed = strip_comment(line).trim();
            // Skip .req/.unreq definition lines — leave them as-is
            if trimmed.contains(" .req ")
                || trimmed.contains("\t.req\t")
                || trimmed.contains("\t.req ")
                || trimmed.contains(" .unreq ")
                || trimmed.contains("\t.unreq\t")
                || trimmed.contains("\t.unreq ")
            {
                return line.clone();
            }
            // Skip directives (starting with .) — aliases are only used in instructions
            if trimmed.starts_with('.') {
                return line.clone();
            }
            // Skip empty lines and comments
            if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("//") {
                return line.clone();
            }
            let mut result = line.clone();
            for (name, register) in &sorted_aliases {
                let name_bytes = name.as_bytes();
                let mut new_result = String::with_capacity(result.len());
                let bytes = result.as_bytes();
                let mut i = 0;
                while i < bytes.len() {
                    if i + name_bytes.len() <= bytes.len()
                        && &bytes[i..i + name_bytes.len()] == name_bytes
                    {
                        // Check word boundary before
                        let before_ok = i == 0 || !is_alias_ident_char(bytes[i - 1]);
                        // Check word boundary after
                        let after_pos = i + name_bytes.len();
                        let after_ok =
                            after_pos >= bytes.len() || !is_alias_ident_char(bytes[after_pos]);
                        if before_ok && after_ok {
                            new_result.push_str(register);
                            i += name_bytes.len();
                            continue;
                        }
                    }
                    new_result.push(bytes[i] as char);
                    i += 1;
                }
                result = new_result;
            }
            result
        })
        .collect()
}

/// Check if a byte is a valid identifier character for register alias matching.
/// Similar to is_ident_char but does NOT include '.' since register names like
/// "v0.8h" should allow alias substitution where the alias is followed by '.'.
fn is_alias_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

pub fn parse_asm(text: &str) -> Result<Vec<AsmStatement>, String> {
    // Pre-process: strip C-style /* ... */ comments
    let text = asm_preprocess::strip_c_comments(text);

    // Split lines on ';' (GAS statement separator) before macro expansion,
    // so macro invocations after ';' on the same line get expanded correctly.
    // Strip // and @ line comments BEFORE splitting on semicolons, so that
    // semicolons inside comments (e.g. "// struct {int a;} *p;") don't cause
    // spurious splits.
    let presplit: Vec<String> = text
        .lines()
        .flat_map(|line| {
            let line = strip_comment(line);
            split_on_semicolons(line)
                .into_iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
        })
        .collect();
    let presplit_refs: Vec<&str> = presplit.iter().map(|s| s.as_str()).collect();

    // Expand .macro/.endm definitions and invocations
    let macro_expanded = expand_macros(&presplit_refs)?;
    let macro_refs: Vec<&str> = macro_expanded.iter().map(|s| s.as_str()).collect();

    // Expand .rept/.endr blocks
    let expanded_lines = expand_rept_blocks(&macro_refs)?;

    // Resolve .set/.equ constants in expressions
    let expanded_lines = resolve_set_constants(&expanded_lines);

    // Resolve .req/.unreq register aliases
    let expanded_lines = resolve_register_aliases(&expanded_lines);

    let mut statements = Vec::new();
    // Stack for .if/.else/.endif conditional assembly.
    // Each entry is (active, any_taken): active = current block emitting code,
    // any_taken = whether any branch in this if/elseif/else chain was taken.
    let mut if_stack: Vec<(bool, bool)> = Vec::new();
    // Track defined symbols for .ifdef/.ifndef
    let mut defined_symbols: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (line_num, line) in expanded_lines.iter().enumerate() {
        let line = line.trim();

        // Skip empty lines
        if line.is_empty() {
            statements.push(AsmStatement::Empty);
            continue;
        }

        // Strip comments (// style)
        let line = strip_comment(line);
        let line = line.trim();
        if line.is_empty() {
            statements.push(AsmStatement::Empty);
            continue;
        }

        // Handle .if/.else/.endif before anything else
        let lower = line.to_ascii_lowercase();
        if lower.starts_with(".endif") {
            if if_stack.pop().is_none() {
                return Err(format!(
                    "Line {}: .endif without matching .if",
                    line_num + 1
                ));
            }
            continue;
        }
        // .elseif / .elsif — else-if branch (must be checked BEFORE .else)
        if lower.starts_with(".elseif ")
            || lower.starts_with(".elseif\t")
            || lower.starts_with(".elsif ")
            || lower.starts_with(".elsif\t")
        {
            let len = if_stack.len();
            if len > 0 {
                let parent_active = if len >= 2 { if_stack[len - 2].0 } else { true };
                if if_stack[len - 1].1 || !parent_active {
                    if_stack[len - 1].0 = false;
                } else {
                    let keyword_len = if lower.starts_with(".elseif") { 7 } else { 6 };
                    let cond_str = line[keyword_len..].trim();
                    let result = eval_if_condition(cond_str);
                    if_stack[len - 1].0 = result;
                    if result {
                        if_stack[len - 1].1 = true;
                    }
                }
            }
            continue;
        }
        if lower == ".else"
            || (lower.starts_with(".else")
                && !lower.starts_with(".elseif")
                && !lower.starts_with(".elsif")
                && lower.len() >= 5
                && (lower.len() == 5 || lower.as_bytes()[5].is_ascii_whitespace()))
        {
            let len = if_stack.len();
            if len > 0 {
                let parent_active = if len >= 2 { if_stack[len - 2].0 } else { true };
                if if_stack[len - 1].1 || !parent_active {
                    if_stack[len - 1].0 = false;
                } else {
                    if_stack[len - 1].0 = true;
                    if_stack[len - 1].1 = true;
                }
            }
            continue;
        }
        if lower == ".if" || lower.starts_with(".if ") || lower.starts_with(".if\t") {
            let cond_str = if line.len() > 3 { line[3..].trim() } else { "" };
            // Evaluate the condition: if we're already in a false block, push false
            let active = if if_stack.last().map(|&(a, _)| a).unwrap_or(true) {
                if cond_str.is_empty() {
                    false
                } else {
                    eval_if_condition(cond_str)
                }
            } else {
                false
            };
            if_stack.push((active, active));
            continue;
        }
        // .ifc string1, string2  — conditional if strings are equal
        if lower.starts_with(".ifc ") || lower.starts_with(".ifc\t") {
            let args = line[4..].trim();
            let active = if if_stack.last().map(|&(a, _)| a).unwrap_or(true) {
                if let Some((a, b)) = args.split_once(',') {
                    a.trim() == b.trim()
                } else {
                    false
                }
            } else {
                false
            };
            if_stack.push((active, active));
            continue;
        }
        // .ifnc string1, string2  — conditional if strings are NOT equal
        if lower.starts_with(".ifnc ") || lower.starts_with(".ifnc\t") {
            let args = line[5..].trim();
            let active = if if_stack.last().map(|&(a, _)| a).unwrap_or(true) {
                if let Some((a, b)) = args.split_once(',') {
                    a.trim() != b.trim()
                } else {
                    true
                }
            } else {
                false
            };
            if_stack.push((active, active));
            continue;
        }
        // .ifb string — conditional if string is blank
        if lower == ".ifb" || lower.starts_with(".ifb ") || lower.starts_with(".ifb\t") {
            let arg = if line.len() > 4 { line[4..].trim() } else { "" };
            let active = if if_stack.last().map(|&(a, _)| a).unwrap_or(true) {
                arg.is_empty()
            } else {
                false
            };
            if_stack.push((active, active));
            continue;
        }
        // .ifnb string — conditional if string is NOT blank
        if lower == ".ifnb" || lower.starts_with(".ifnb ") || lower.starts_with(".ifnb\t") {
            let arg = if line.len() > 5 { line[5..].trim() } else { "" };
            let active = if if_stack.last().map(|&(a, _)| a).unwrap_or(true) {
                !arg.is_empty()
            } else {
                false
            };
            if_stack.push((active, active));
            continue;
        }
        // .ifdef symbol — conditional if symbol is defined
        if lower.starts_with(".ifdef ") || lower.starts_with(".ifdef\t") {
            let sym = line[6..].trim().to_string();
            let active = if if_stack.last().map(|&(a, _)| a).unwrap_or(true) {
                defined_symbols.contains(&sym)
            } else {
                false
            };
            if_stack.push((active, active));
            continue;
        }
        // .ifndef symbol — conditional if symbol is NOT defined
        if lower.starts_with(".ifndef ") || lower.starts_with(".ifndef\t") {
            let sym = line[7..].trim().to_string();
            let active = if if_stack.last().map(|&(a, _)| a).unwrap_or(true) {
                !defined_symbols.contains(&sym)
            } else {
                false
            };
            if_stack.push((active, active));
            continue;
        }
        // .ifeq expression — conditional if expression equals 0
        if lower.starts_with(".ifeq ") || lower.starts_with(".ifeq\t") {
            let expr_str = line[5..].trim();
            let active = if if_stack.last().map(|&(a, _)| a).unwrap_or(true) {
                asm_expr::parse_integer_expr(expr_str).unwrap_or(1) == 0
            } else {
                false
            };
            if_stack.push((active, active));
            continue;
        }
        // .ifne expression — conditional if expression is not equal to 0
        if lower.starts_with(".ifne ") || lower.starts_with(".ifne\t") {
            let expr_str = line[5..].trim();
            let active = if if_stack.last().map(|&(a, _)| a).unwrap_or(true) {
                asm_expr::parse_integer_expr(expr_str).unwrap_or(0) != 0
            } else {
                false
            };
            if_stack.push((active, active));
            continue;
        }
        // If we're inside a false .if block, skip this line
        if !if_stack.last().map(|&(a, _)| a).unwrap_or(true) {
            continue;
        }

        // In AArch64 GAS, '#' at the start of a line is a comment character.
        // This covers: C preprocessor line markers (# 123 "file"), and
        // comment-producing macros (e.g., FFmpeg's FUNC expanding to '#').
        if line.starts_with('#') {
            statements.push(AsmStatement::Empty);
            continue;
        }

        // Handle ';' as statement separator (GAS syntax).
        // Split the line on ';' and parse each part independently.
        let parts = split_on_semicolons(line);
        for part in parts {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            match parse_line(part) {
                Ok(stmts) => {
                    for stmt in &stmts {
                        match stmt {
                            AsmStatement::Label(name) => {
                                defined_symbols.insert(name.clone());
                            }
                            AsmStatement::Directive(AsmDirective::Set(name, _)) => {
                                defined_symbols.insert(name.clone());
                            }
                            _ => {}
                        }
                    }
                    statements.extend(stmts);
                }
                Err(e) => return Err(format!("Line {}: {}: '{}'", line_num + 1, e, part)),
            }
        }
    }
    Ok(statements)
}

/// Split a line on ';' characters, respecting strings and comments.
/// In GAS syntax, ';' separates multiple statements on the same line.
/// Stops splitting once a `//` or `@` line comment is encountered (outside strings),
/// so semicolons inside comments are not treated as statement separators.
fn split_on_semicolons(line: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut in_string = false;
    let mut escape = false;
    let mut start = 0;
    let bytes = line.as_bytes();
    for (i, c) in line.char_indices() {
        if escape {
            escape = false;
            continue;
        }
        if c == '\\' && in_string {
            escape = true;
            continue;
        }
        if c == '"' {
            in_string = !in_string;
            continue;
        }
        if !in_string {
            // Stop splitting at // comment start
            if c == '/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                break;
            }
            // Stop splitting at @ comment start (but not @function, @object, etc.)
            if c == '@' {
                let after = &line[i + 1..];
                if !after.starts_with("object")
                    && !after.starts_with("function")
                    && !after.starts_with("progbits")
                    && !after.starts_with("nobits")
                    && !after.starts_with("tls_object")
                    && !after.starts_with("note")
                {
                    break;
                }
            }
            if c == ';' {
                parts.push(&line[start..i]);
                start = i + 1;
            }
        }
    }
    parts.push(&line[start..]);
    parts
}

fn strip_comment(line: &str) -> &str {
    // Scan character by character, tracking string state to find comments
    // outside of string literals. This correctly handles escaped quotes (\")
    // inside strings (e.g. .asciz "a\"b//c" should not strip at //).
    let bytes = line.as_bytes();
    let mut in_string = false;
    let mut i = 0;
    while i < bytes.len() {
        if in_string {
            if bytes[i] == b'\\' {
                i += 2; // skip escaped character
                continue;
            }
            if bytes[i] == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        // Not in string
        if bytes[i] == b'"' {
            in_string = true;
            i += 1;
            continue;
        }
        // Check for // comment
        if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            return &line[..i];
        }
        // Check for @ comment (GAS ARM comment character).
        // Skip \@ which is the GAS macro invocation counter, not a comment.
        if bytes[i] == b'@' && !(i > 0 && bytes[i - 1] == b'\\') {
            let after = &line[i + 1..];
            if !after.starts_with("object")
                && !after.starts_with("function")
                && !after.starts_with("progbits")
                && !after.starts_with("nobits")
                && !after.starts_with("tls_object")
                && !after.starts_with("note")
            {
                return &line[..i];
            }
        }
        i += 1;
    }
    line
}

/// Try to parse `ldr Rd, =symbol[+offset]` pseudo-instruction.
///
/// In GAS, `ldr Rd, =expr` loads a 64-bit value via a literal pool.
/// Returns a `LdrLiteralPool` statement that will be expanded into
/// `ldr Rd, .Llpool_N` + pool entries by the `expand_literal_pools` pass.
fn try_expand_ldr_literal(line: &str) -> Option<Result<Vec<AsmStatement>, String>> {
    let lower = line.to_ascii_lowercase();
    // Match: ldr xN, =symbol or ldr wN, =symbol (32-bit registers use adrp too on AArch64)
    if !lower.starts_with("ldr ") && !lower.starts_with("ldr\t") {
        return None;
    }
    let rest = line[3..].trim();
    // Find the comma separating register from operand
    let comma_pos = rest.find(',')?;
    let reg = rest[..comma_pos].trim();
    let operand = rest[comma_pos + 1..].trim();
    if !operand.starts_with('=') {
        return None;
    }
    let expr = operand[1..].trim();
    // Parse symbol and optional +offset, e.g. =coeffs+8
    let (symbol, addend) = if let Some(plus_pos) = expr.rfind('+') {
        let sym = expr[..plus_pos].trim();
        let off_str = expr[plus_pos + 1..].trim();
        if let Ok(off) = if off_str.starts_with("0x") || off_str.starts_with("0X") {
            i64::from_str_radix(&off_str[2..], 16)
        } else {
            off_str.parse::<i64>()
        } {
            (sym, off)
        } else {
            (expr, 0i64)
        }
    } else if let Some(minus_pos) = expr.rfind('-') {
        // Only treat as symbol-offset if there's a valid symbol before the minus
        let sym = expr[..minus_pos].trim();
        let off_str = expr[minus_pos + 1..].trim();
        if !sym.is_empty() && !sym.ends_with(['+', '-']) {
            if let Ok(off) = if off_str.starts_with("0x") || off_str.starts_with("0X") {
                i64::from_str_radix(&off_str[2..], 16)
            } else {
                off_str.parse::<i64>()
            } {
                (sym, -off)
            } else {
                (expr, 0i64)
            }
        } else {
            (expr, 0i64)
        }
    } else {
        (expr, 0i64)
    };
    Some(Ok(vec![AsmStatement::LdrLiteralPool {
        reg: reg.to_string(),
        symbol: symbol.to_string(),
        addend,
    }]))
}

fn parse_line(line: &str) -> Result<Vec<AsmStatement>, String> {
    // Check for label definition (name:)
    // Labels can be at the start of the line, possibly followed by an instruction
    if let Some(colon_pos) = line.find(':') {
        let potential_label = &line[..colon_pos].trim();
        // Verify it looks like a valid label (no spaces before colon, alphanumeric + _ + .)
        if !potential_label.is_empty()
            && !potential_label.contains(' ')
            && !potential_label.contains('\t')
            && !potential_label.starts_with('.')  // Could be a directive
            || potential_label.starts_with(".L") // Local labels start with .L
            || potential_label.starts_with(".Lstr") // String labels
            || potential_label.starts_with(".Lmemcpy")
            || potential_label.starts_with(".Lskip")
        {
            // Check if this is actually a directive like ".section .rodata"
            if potential_label.starts_with('.')
                && !potential_label.starts_with(".L")
                && !potential_label.starts_with(".l")
            {
                // This is a directive, not a label
            } else {
                let mut result = vec![AsmStatement::Label(potential_label.to_string())];
                // Check for instruction/directive after the label on the same line
                let rest = line[colon_pos + 1..].trim();
                if !rest.is_empty() {
                    result.extend(parse_line(rest)?);
                }
                return Ok(result);
            }
        }
    }

    let trimmed = line.trim();

    // Register alias: "name .req register" or "name .unreq register"
    // These define register aliases and can be safely ignored.
    if trimmed.contains(" .req ")
        || trimmed.contains("\t.req\t")
        || trimmed.contains("\t.req ")
        || trimmed.contains(" .unreq ")
        || trimmed.contains("\t.unreq\t")
        || trimmed.contains("\t.unreq ")
    {
        return Ok(vec![AsmStatement::Empty]);
    }

    // Directive: starts with .
    if trimmed.starts_with('.') {
        return Ok(vec![parse_directive(trimmed)?]);
    }

    // Handle ldr Rd, =symbol pseudo-instruction (creates LdrLiteralPool for later expansion)
    if let Some(expanded) = try_expand_ldr_literal(trimmed) {
        return expanded;
    }

    // Instruction
    Ok(vec![parse_instruction(trimmed)?])
}

fn parse_directive(line: &str) -> Result<AsmStatement, String> {
    // Split directive name from arguments
    let (name, args) = if line.starts_with(".inst") && line.len() > 5 && line.as_bytes()[5] == b'('
    {
        // Handle .inst(expr) without space: ".inst(0x...)" -> name=".inst", args="(0x...)"
        (".inst", line[5..].trim())
    } else if let Some(space_pos) = line.find([' ', '\t']) {
        let name = &line[..space_pos];
        let args = line[space_pos..].trim();
        (name, args)
    } else {
        (line, "")
    };

    let dir = match name {
        ".section" => parse_section_directive(args)?,
        ".text" => AsmDirective::Section(SectionDirective {
            name: ".text".to_string(),
            flags: None,
            section_type: None,
        }),
        ".data" => AsmDirective::Section(SectionDirective {
            name: ".data".to_string(),
            flags: None,
            section_type: None,
        }),
        ".bss" => AsmDirective::Section(SectionDirective {
            name: ".bss".to_string(),
            flags: None,
            section_type: None,
        }),
        ".rodata" => AsmDirective::Section(SectionDirective {
            name: ".rodata".to_string(),
            flags: None,
            section_type: None,
        }),
        ".globl" | ".global" => AsmDirective::Global(args.trim().to_string()),
        ".weak" => AsmDirective::Weak(args.trim().to_string()),
        ".hidden" => AsmDirective::Hidden(args.trim().to_string()),
        ".protected" => AsmDirective::Protected(args.trim().to_string()),
        ".internal" => AsmDirective::Internal(args.trim().to_string()),
        ".type" => parse_type_directive(args)?,
        ".size" => parse_size_directive(args)?,
        ".align" | ".p2align" => {
            let align_val: u64 = args
                .trim()
                .split(',')
                .next()
                .and_then(|s| parse_int_literal(s.trim()).ok())
                .unwrap_or(0) as u64;
            // AArch64 .align N means 2^N bytes (same as .p2align)
            AsmDirective::Align(1u64 << align_val)
        }
        ".balign" => {
            let align_val: u64 = args
                .trim()
                .split(',')
                .next()
                .and_then(|s| parse_int_literal(s.trim()).ok())
                .unwrap_or(1) as u64;
            AsmDirective::Balign(align_val)
        }
        ".byte" => {
            let vals = parse_data_values(args)?;
            AsmDirective::Byte(vals)
        }
        ".short" | ".hword" | ".2byte" | ".half" => {
            let mut vals = Vec::new();
            for part in args.split(',') {
                let val = parse_data_value(part.trim())? as i16;
                vals.push(val);
            }
            AsmDirective::Short(vals)
        }
        ".long" | ".4byte" | ".word" | ".int" | ".inst" => {
            let vals = parse_data_values(args)?;
            AsmDirective::Long(vals)
        }
        ".quad" | ".8byte" | ".xword" | ".dword" => {
            let vals = parse_data_values(args)?;
            AsmDirective::Quad(vals)
        }
        ".zero" | ".space" => {
            let parts: Vec<&str> = args.trim().split(',').collect();
            let size: usize = parse_int_literal(parts[0].trim())
                .map_err(|_| format!("invalid .zero size: {}", args))?
                as usize;
            let fill: u8 = if parts.len() > 1 {
                parse_data_value(parts[1].trim())? as u8
            } else {
                0
            };
            AsmDirective::Zero(size, fill)
        }
        ".fill" => {
            // .fill repeat, size, value
            let parts: Vec<&str> = args.splitn(3, ',').collect();
            let repeat = parse_int_literal(parts[0].trim())
                .map_err(|_| format!("bad .fill repeat: {}", parts[0].trim()))?
                as u64;
            let size = if parts.len() > 1 {
                parse_int_literal(parts[1].trim())
                    .map_err(|_| format!("bad .fill size: {}", parts[1].trim()))?
                    as u64
            } else {
                1
            };
            let value = if parts.len() > 2 {
                parse_int_literal(parts[2].trim())
                    .map_err(|_| format!("bad .fill value: {}", parts[2].trim()))?
                    as u64
            } else {
                0
            };
            let total_bytes = (repeat * size.min(8)) as usize;
            if value == 0 {
                AsmDirective::Zero(total_bytes, 0)
            } else {
                let mut data = Vec::with_capacity(total_bytes);
                let value_bytes = value.to_le_bytes();
                for _ in 0..repeat {
                    for j in 0..size.min(8) as usize {
                        data.push(value_bytes[j]);
                    }
                }
                AsmDirective::Ascii(data)
            }
        }
        ".asciz" | ".string" => {
            let s = elf::parse_string_literal(args)?;
            let mut bytes = s;
            bytes.push(0); // null terminator
            AsmDirective::Asciz(bytes)
        }
        ".ascii" => {
            let s = elf::parse_string_literal(args)?;
            AsmDirective::Ascii(s)
        }
        ".comm" => parse_comm_directive(args)?,
        ".local" => AsmDirective::Local(args.trim().to_string()),
        ".set" | ".equ" => {
            let parts: Vec<&str> = args.splitn(2, ',').collect();
            if parts.len() == 2 {
                AsmDirective::Set(parts[0].trim().to_string(), parts[1].trim().to_string())
            } else {
                return Err(format!(
                    "malformed .set directive: expected 'name, value', got '{}'",
                    args
                ));
            }
        }
        ".symver" => {
            // .symver name, alias@@VERSION -> treat as alias for default version
            let parts: Vec<&str> = args.splitn(2, ',').collect();
            if parts.len() == 2 {
                let name = parts[0].trim();
                let ver_string = parts[1].trim();
                if let Some(at_pos) = ver_string.find('@') {
                    let alias = &ver_string[..at_pos];
                    if !alias.is_empty() {
                        AsmDirective::Set(alias.to_string(), name.to_string())
                    } else {
                        AsmDirective::Ignored
                    }
                } else {
                    AsmDirective::Ignored
                }
            } else {
                AsmDirective::Ignored
            }
        }
        // CFI directives
        ".cfi_startproc"
        | ".cfi_endproc"
        | ".cfi_def_cfa_offset"
        | ".cfi_offset"
        | ".cfi_def_cfa_register"
        | ".cfi_restore"
        | ".cfi_remember_state"
        | ".cfi_restore_state"
        | ".cfi_adjust_cfa_offset"
        | ".cfi_def_cfa"
        | ".cfi_sections"
        | ".cfi_personality"
        | ".cfi_lsda"
        | ".cfi_rel_offset"
        | ".cfi_register"
        | ".cfi_return_column"
        | ".cfi_undefined"
        | ".cfi_same_value"
        | ".cfi_escape" => AsmDirective::Cfi,
        ".pushsection" => {
            // .pushsection name,"flags",@type - same syntax as .section
            match parse_section_directive(args)? {
                AsmDirective::Section(dir) => AsmDirective::PushSection(dir),
                _ => AsmDirective::Ignored,
            }
        }
        ".popsection" => AsmDirective::PopSection,
        ".previous" => AsmDirective::Previous,
        ".subsection" => {
            let n: u64 = args.trim().parse().unwrap_or(0);
            AsmDirective::Subsection(n)
        }
        ".purgem" => {
            // .purgem name — remove a macro definition; ignore for now
            AsmDirective::Ignored
        }
        ".org" => {
            // .org expressions like ". - (X) + (Y)" are used as size assertions
            // in kernel alternative macros. Silently ignore them.
            AsmDirective::Ignored
        }
        ".incbin" => {
            let parts: Vec<&str> = args.splitn(3, ',').collect();
            let path = elf::parse_string_literal(parts[0].trim())
                .map_err(|e| format!(".incbin path: {}", e))?;
            let path = String::from_utf8(path)
                .map_err(|_| ".incbin: invalid UTF-8 in path".to_string())?;
            let skip = if parts.len() > 1 {
                parts[1].trim().parse::<u64>().unwrap_or(0)
            } else {
                0
            };
            let count = if parts.len() > 2 {
                Some(parts[2].trim().parse::<u64>().unwrap_or(0))
            } else {
                None
            };
            AsmDirective::Incbin { path, skip, count }
        }
        ".unreq" => {
            // .unreq name — remove register alias; ignore
            AsmDirective::Ignored
        }
        ".req" => {
            // .req register — register alias (standalone form); ignore
            AsmDirective::Ignored
        }
        ".float" | ".single" => {
            // .float val1, val2, ... — emit 32-bit IEEE floats
            let mut bytes = Vec::new();
            for part in args.split(',') {
                let val: f32 = part
                    .trim()
                    .parse()
                    .map_err(|_| format!("invalid .float value: {}", part.trim()))?;
                bytes.extend_from_slice(&val.to_le_bytes());
            }
            AsmDirective::RawBytes(bytes)
        }
        ".double" => {
            // .double val1, val2, ... — emit 64-bit IEEE floats
            let mut bytes = Vec::new();
            for part in args.split(',') {
                let val: f64 = part
                    .trim()
                    .parse()
                    .map_err(|_| format!("invalid .double value: {}", part.trim()))?;
                bytes.extend_from_slice(&val.to_le_bytes());
            }
            AsmDirective::RawBytes(bytes)
        }
        // Other directives we can safely ignore
        ".file" | ".loc" | ".ident" | ".addrsig" | ".addrsig_sym" | ".build_attributes"
        | ".eabi_attribute" | ".arch" | ".arch_extension" | ".cpu" | ".ltorg" | ".pool" => {
            AsmDirective::Ltorg
        }
        _ => {
            return Err(format!(
                "unsupported AArch64 assembler directive: {} {}",
                name, args
            ));
        }
    };

    Ok(AsmStatement::Directive(dir))
}

fn parse_instruction(line: &str) -> Result<AsmStatement, String> {
    // Split mnemonic from operands
    let (mnemonic, operands_str) = if let Some(space_pos) = line.find([' ', '\t']) {
        (&line[..space_pos], line[space_pos..].trim())
    } else {
        (line, "")
    };

    let mnemonic = mnemonic.to_lowercase();
    let operands = parse_operands(operands_str)?;

    Ok(AsmStatement::Instruction {
        mnemonic,
        operands,
        raw_operands: operands_str.to_string(),
    })
}

/// Parse an operand list separated by commas, handling brackets and nested expressions.
fn parse_operands(s: &str) -> Result<Vec<Operand>, String> {
    if s.is_empty() {
        return Ok(Vec::new());
    }

    let mut operands = Vec::new();
    let mut current = String::new();
    let mut bracket_depth = 0;
    let mut brace_depth = 0;

    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        match chars[i] {
            '{' => {
                brace_depth += 1;
                current.push('{');
            }
            '}' => {
                brace_depth -= 1;
                current.push('}');
            }
            '[' => {
                bracket_depth += 1;
                current.push('[');
            }
            ']' => {
                bracket_depth -= 1;
                current.push(']');
                // Check for '!' (pre-index writeback)
                if i + 1 < chars.len() && chars[i + 1] == '!' {
                    current.push('!');
                    i += 1;
                }
            }
            ',' if bracket_depth == 0 && brace_depth == 0 => {
                let op = parse_single_operand(current.trim())?;
                operands.push(op);
                current.clear();
            }
            _ => {
                current.push(chars[i]);
            }
        }
        i += 1;
    }

    // Last operand
    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        let op = parse_single_operand(&trimmed)?;
        operands.push(op);
    }

    // Handle memory operands with post-index: [base], #offset
    // This looks like two operands: Mem{base, 0} and Imm(offset)
    // We need to merge them into MemPostIndex
    let mut merged = Vec::new();
    let mut skip_next = false;
    for j in 0..operands.len() {
        if skip_next {
            skip_next = false;
            continue;
        }
        if j + 1 < operands.len() {
            if let (Operand::Mem { base, offset: 0 }, Operand::Imm(off)) =
                (&operands[j], &operands[j + 1])
            {
                merged.push(Operand::MemPostIndex {
                    base: base.clone(),
                    offset: *off,
                });
                skip_next = true;
                continue;
            }
        }
        merged.push(operands[j].clone());
    }

    Ok(merged)
}

fn parse_single_operand(s: &str) -> Result<Operand, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty operand".to_string());
    }

    // Register list: {v0.16b}, {v0.16b, v1.16b}, etc.
    // Register list with optional element index: {v0.s, v1.s}[0]
    if s.starts_with('{') {
        if s.ends_with('}') {
            return parse_register_list(s);
        }
        // Check for {regs}[index] form
        if let Some(close_brace) = s.find('}') {
            let rest = s[close_brace + 1..].trim();
            if rest.starts_with('[') && rest.ends_with(']') {
                let idx_str = &rest[1..rest.len() - 1];
                // Use expression evaluator to handle arithmetic like [1 - 1]
                let idx_result = idx_str.parse::<u32>().ok().or_else(|| {
                    asm_expr::parse_integer_expr(idx_str)
                        .ok()
                        .and_then(|v| u32::try_from(v).ok())
                });
                if let Some(idx) = idx_result {
                    let list_str = &s[..close_brace + 1];
                    let inner = &list_str[1..list_str.len() - 1];
                    let mut regs = Vec::new();
                    for part in inner.split(',') {
                        let part = part.trim();
                        if !part.is_empty() {
                            let op = parse_single_operand(part)?;
                            regs.push(op);
                        }
                    }
                    if regs.is_empty() {
                        return Err("empty register list".to_string());
                    }
                    return Ok(Operand::RegListIndexed { regs, index: idx });
                }
            }
        }
    }

    // Memory operand: [base, #offset]! (pre-index) or [base, #offset] or [base]
    if s.starts_with('[') {
        return parse_memory_operand(s);
    }

    // Immediate: #value
    if let Some(rest) = s.strip_prefix('#') {
        return parse_immediate(rest);
    }

    // :modifier:symbol
    if s.starts_with(':') {
        return parse_modifier(s);
    }

    // Shift: lsl, lsr, asr, ror
    let lower = s.to_lowercase();
    if lower.starts_with("lsl ")
        || lower.starts_with("lsr ")
        || lower.starts_with("asr ")
        || lower.starts_with("ror ")
    {
        let kind = &lower[..3];
        let amount_str = s[4..].trim();
        let amount = if let Some(stripped) = amount_str.strip_prefix('#') {
            parse_int_literal(stripped)?
        } else {
            parse_int_literal(amount_str)?
        };
        return Ok(Operand::Shift {
            kind: kind.to_string(),
            amount: amount as u32,
        });
    }

    // Extend specifiers: sxtw, uxtw, sxtx, uxtx, sxth, uxth, sxtb, uxtb
    // May appear alone (sxtw) or with shift (sxtw #2)
    {
        let extend_prefixes = [
            "sxtw", "sxtx", "sxth", "sxtb", "uxtw", "uxtx", "uxth", "uxtb",
        ];
        for prefix in &extend_prefixes {
            if lower == *prefix {
                return Ok(Operand::Extend {
                    kind: prefix.to_string(),
                    amount: 0,
                });
            }
            if lower.starts_with(prefix) && lower.as_bytes().get(prefix.len()) == Some(&b' ') {
                let amount_str = s[prefix.len()..].trim();
                let amount = if let Some(stripped) = amount_str.strip_prefix('#') {
                    parse_int_literal(stripped)?
                } else {
                    parse_int_literal(amount_str)?
                };
                return Ok(Operand::Extend {
                    kind: prefix.to_string(),
                    amount: amount as u32,
                });
            }
        }
    }

    // Barrier options
    match lower.as_str() {
        "ish" | "ishld" | "ishst" | "sy" | "ld" | "st" | "osh" | "oshld" | "oshst" | "nsh"
        | "nshld" | "nshst" => {
            // Store original case: this name may be a C symbol colliding with an ARM keyword
            return Ok(Operand::Barrier(s.to_string()));
        }
        _ => {}
    }

    // Condition codes (for csel, csinc, etc.)
    match lower.as_str() {
        "eq" | "ne" | "cs" | "hs" | "cc" | "lo" | "mi" | "pl" | "vs" | "vc" | "hi" | "ls"
        | "ge" | "lt" | "gt" | "le" | "al" | "nv" => {
            // Store original case: this name may be a C symbol colliding with an ARM keyword
            return Ok(Operand::Cond(s.to_string()));
        }
        _ => {}
    }

    // NEON register with lane index: v0.d[1], v0.b[0], v0.s[2], etc.
    if let Some(dot_pos) = s.find('.') {
        let reg_part = &s[..dot_pos];
        let arr_part = &s[dot_pos + 1..];
        if is_register(reg_part) {
            if let Some(bracket_pos) = arr_part.find('[') {
                if arr_part.ends_with(']') {
                    let elem_size = arr_part[..bracket_pos].to_lowercase();
                    let idx_str = &arr_part[bracket_pos + 1..arr_part.len() - 1];
                    // Use expression evaluator to handle arithmetic like [1 - 1]
                    let idx_result = idx_str.parse::<u32>().ok().or_else(|| {
                        asm_expr::parse_integer_expr(idx_str)
                            .ok()
                            .and_then(|v| u32::try_from(v).ok())
                    });
                    if let Some(idx) = idx_result {
                        if matches!(elem_size.as_str(), "b" | "h" | "s" | "d") {
                            return Ok(Operand::RegLane {
                                reg: reg_part.to_string(),
                                elem_size,
                                index: idx,
                            });
                        }
                    }
                }
            }
        }
    }

    // NEON register with arrangement: v0.8b, v0.16b, v0.4s, v0.2d, etc.
    if let Some(dot_pos) = s.find('.') {
        let reg_part = &s[..dot_pos];
        let arr_part = &s[dot_pos + 1..];
        if is_register(reg_part) {
            let arr_lower = arr_part.to_lowercase();
            if matches!(
                arr_lower.as_str(),
                "8b" | "16b"
                    | "4h"
                    | "8h"
                    | "2s"
                    | "4s"
                    | "1d"
                    | "2d"
                    | "1q"
                    | "b"
                    | "h"
                    | "s"
                    | "d"
            ) {
                return Ok(Operand::RegArrangement {
                    reg: reg_part.to_string(),
                    arrangement: arr_lower,
                });
            }
        }
    }

    // Register
    if is_register(s) {
        return Ok(Operand::Reg(s.to_string()));
    }

    // Bare integer (without # prefix) - some inline asm constraints emit these
    // e.g., "eor w9, w10, 255" or "ccmp x10, x13, 0, eq"
    // Also handles negative like -1, -2, etc.
    if s.chars()
        .next()
        .is_some_and(|c| c.is_ascii_digit() || c == '-')
    {
        if let Ok(val) = parse_int_literal(s) {
            return Ok(Operand::Imm(val));
        }
    }

    // Label/symbol reference (for branches, adrp, etc.)
    // Could be: .LBB42, func_name, symbol+offset
    if let Some(plus_pos) = s.find('+') {
        let sym = s[..plus_pos].trim();
        let off_str = s[plus_pos + 1..].trim();
        if !sym.is_empty() {
            if let Ok(off) = parse_int_literal(off_str) {
                return Ok(Operand::SymbolOffset(sym.to_string(), off));
            }
        }
    }
    if let Some(minus_pos) = s.find('-') {
        // Careful: don't confuse with label names containing '-' in label diff expressions
        if minus_pos > 0 {
            let sym = s[..minus_pos].trim();
            let off_str = &s[minus_pos..]; // includes the '-'
            if let Ok(off) = parse_int_literal(off_str) {
                return Ok(Operand::SymbolOffset(sym.to_string(), off));
            }
        }
    }

    // Try evaluating as a constant expression (handles parenthesized expressions like (0 + ...))
    if s.starts_with('(') || s.starts_with('~') {
        if let Ok(val) = parse_int_literal(s) {
            return Ok(Operand::Imm(val));
        }
    }

    // Plain symbol/label
    Ok(Operand::Symbol(s.to_string()))
}

/// Parse a register list like {v0.16b} or {v0.16b, v1.16b, v2.16b, v3.16b}
/// Also handles range syntax: {v0.8b-v3.8b} which means {v0.8b, v1.8b, v2.8b, v3.8b}
fn parse_register_list(s: &str) -> Result<Operand, String> {
    let inner = &s[1..s.len() - 1].trim(); // strip { and }
    let mut regs = Vec::new();
    for part in inner.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        // Check for range syntax: v0.8b-v2.8b
        if let Some(dash_pos) = part.find('-') {
            let left = part[..dash_pos].trim();
            let right = part[dash_pos + 1..].trim();
            // Both must be register arrangements
            if let (Some(ldot), Some(rdot)) = (left.find('.'), right.find('.')) {
                let lreg = &left[..ldot];
                let larr = &left[ldot + 1..];
                let rreg = &right[..rdot];
                let rarr = &right[rdot + 1..];
                if is_register(lreg) && is_register(rreg) && larr.eq_ignore_ascii_case(rarr) {
                    let start = parse_reg_num_simple(lreg);
                    let end = parse_reg_num_simple(rreg);
                    if let (Some(s_num), Some(e_num)) = (start, end) {
                        let prefix = if lreg.starts_with('v') || lreg.starts_with('V') {
                            "v".to_string()
                        } else {
                            lreg.chars().next().unwrap().to_string()
                        };
                        let count = if e_num >= s_num {
                            e_num - s_num + 1
                        } else {
                            (32 - s_num) + e_num + 1
                        };
                        for i in 0..count {
                            let reg_num = (s_num + i) % 32;
                            regs.push(Operand::RegArrangement {
                                reg: format!("{}{}", prefix, reg_num),
                                arrangement: larr.to_lowercase(),
                            });
                        }
                        continue;
                    }
                }
            }
        }
        let op = parse_single_operand(part)?;
        regs.push(op);
    }
    if regs.is_empty() {
        return Err("empty register list".to_string());
    }
    Ok(Operand::RegList(regs))
}

fn parse_reg_num_simple(reg: &str) -> Option<u32> {
    let s = reg.trim();
    if s.len() < 2 {
        return None;
    }
    let first = s.chars().next()?;
    if matches!(
        first,
        'v' | 'V'
            | 'x'
            | 'X'
            | 'w'
            | 'W'
            | 'q'
            | 'Q'
            | 'd'
            | 'D'
            | 's'
            | 'S'
            | 'h'
            | 'H'
            | 'b'
            | 'B'
    ) {
        s[1..].parse::<u32>().ok()
    } else {
        None
    }
}

fn parse_memory_operand(s: &str) -> Result<Operand, String> {
    let has_writeback = s.ends_with('!');
    let inner = if has_writeback {
        &s[1..s.len() - 2] // strip [ and ]!
    } else {
        // Find the matching ]
        let end = s.find(']').ok_or("missing ] in memory operand")?;
        &s[1..end]
    };

    // Split on comma
    let parts: Vec<&str> = inner.splitn(2, ',').collect();
    let base = parts[0].trim().to_string();

    if parts.len() == 1 {
        // [base]
        if has_writeback {
            return Ok(Operand::MemPreIndex { base, offset: 0 });
        }
        return Ok(Operand::Mem { base, offset: 0 });
    }

    let second = parts[1].trim();

    // [base, #imm] or [base, imm] (bare immediate without # prefix)
    if let Some(imm_str) = second.strip_prefix('#') {
        match parse_int_literal(imm_str) {
            Ok(offset) => {
                if has_writeback {
                    return Ok(Operand::MemPreIndex { base, offset });
                }
                return Ok(Operand::Mem { base, offset });
            }
            Err(_) => {
                // Expression contains symbols/labels — defer resolution
                return Ok(Operand::MemExpr {
                    base,
                    expr: imm_str.to_string(),
                    writeback: has_writeback,
                });
            }
        }
    }

    // Handle bare immediate without # prefix (e.g., [sp, -16]! or [x0, 8])
    // Check if the second operand starts with a digit or minus sign followed by a digit
    if second.starts_with('-')
        || second.starts_with('+')
        || second.bytes().next().is_some_and(|b| b.is_ascii_digit())
    {
        if let Ok(offset) = parse_int_literal(second) {
            if has_writeback {
                return Ok(Operand::MemPreIndex { base, offset });
            }
            return Ok(Operand::Mem { base, offset });
        }
    }

    // [base, :lo12:symbol]
    if second.starts_with(':') {
        // Parse the modifier embedded in memory operand
        // The ] is already stripped, so just parse the modifier
        let mod_op = parse_modifier(second)?;
        // Return a special memory operand - we'll handle this in the encoder
        // For now, return it as a reg+symbol form
        match mod_op {
            Operand::Modifier { kind, symbol } => {
                return Ok(Operand::MemRegOffset {
                    base,
                    index: format!(":{}:{}", kind, symbol),
                    extend: None,
                    shift: None,
                });
            }
            Operand::ModifierOffset {
                kind,
                symbol,
                offset,
            } => {
                return Ok(Operand::MemRegOffset {
                    base,
                    index: format!(":{}:{}+{}", kind, symbol, offset),
                    extend: None,
                    shift: None,
                });
            }
            _ => {}
        }
    }

    // [base, Xm] or [base, Xm, extend #shift]
    // second may be "x0" or "x0, lsl #2" or "w0, sxtw" or "w0, sxtw #2"
    let sub_parts: Vec<&str> = second.splitn(2, ',').collect();
    let index_str = sub_parts[0].trim();
    if is_register(index_str) {
        let (extend, shift) = if sub_parts.len() > 1 {
            parse_extend_shift(sub_parts[1].trim())
        } else {
            (None, None)
        };
        return Ok(Operand::MemRegOffset {
            base,
            index: index_str.to_string(),
            extend,
            shift,
        });
    }

    // Fallback: treat as register offset
    Ok(Operand::MemRegOffset {
        base,
        index: second.to_string(),
        extend: None,
        shift: None,
    })
}

/// Parse an extend/shift specifier like "lsl #2", "sxtw", "sxtw #0", "uxtx #3"
fn parse_extend_shift(s: &str) -> (Option<String>, Option<u8>) {
    let s = s.trim().to_lowercase();
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.is_empty() {
        return (None, None);
    }
    let kind = parts[0];
    let shift = if parts.len() > 1 {
        let shift_str = parts[1].trim_start_matches('#');
        shift_str.parse::<u8>().ok()
    } else {
        None
    };
    match kind {
        "lsl" | "lsr" | "asr" | "ror" | "sxtw" | "sxtx" | "sxth" | "sxtb" | "uxtw" | "uxtx"
        | "uxth" | "uxtb" => (Some(kind.to_string()), shift),
        _ => (None, None),
    }
}

fn parse_modifier(s: &str) -> Result<Operand, String> {
    // :kind:symbol or :kind:symbol+offset
    let s = s.trim_start_matches(':');
    let colon_pos = s
        .find(':')
        .ok_or("malformed modifier, expected :kind:symbol")?;
    let kind = s[..colon_pos].to_string();
    let rest = &s[colon_pos + 1..];

    // Check for symbol+offset or symbol-offset
    if let Some(plus_pos) = rest.find('+') {
        let symbol = rest[..plus_pos].trim().to_string();
        let offset_str = rest[plus_pos + 1..].trim();
        if let Ok(offset) = parse_int_literal(offset_str) {
            return Ok(Operand::ModifierOffset {
                kind,
                symbol,
                offset,
            });
        }
    }
    if let Some(minus_pos) = rest.rfind('-') {
        if minus_pos > 0 {
            let symbol = rest[..minus_pos].trim().to_string();
            let offset_str = &rest[minus_pos..]; // includes the '-'
            if let Ok(offset) = parse_int_literal(offset_str) {
                return Ok(Operand::ModifierOffset {
                    kind,
                    symbol,
                    offset,
                });
            }
        }
    }

    Ok(Operand::Modifier {
        kind,
        symbol: rest.trim().to_string(),
    })
}

fn parse_immediate(s: &str) -> Result<Operand, String> {
    // Handle :modifier:symbol as immediate (e.g., #:lo12:symbol)
    if s.starts_with(':') {
        return parse_modifier(s);
    }

    match parse_int_literal(s) {
        Ok(val) => Ok(Operand::Imm(val)),
        Err(_) => {
            // Expression contains symbols/labels — store as raw expression for
            // deferred resolution (e.g., #(1b - .Lvector_start + 4))
            Ok(Operand::Expr(s.to_string()))
        }
    }
}

fn parse_int_literal(s: &str) -> Result<i64, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty integer literal".to_string());
    }

    // Use the shared expression evaluator which handles parentheses,
    // operator precedence, bitwise ops, and arithmetic expressions.
    asm_expr::parse_integer_expr(s)
}

fn is_register(s: &str) -> bool {
    let s = s.to_lowercase();
    // General purpose: x0-x30, w0-w30
    if (s.starts_with('x') || s.starts_with('w')) && s.len() >= 2 {
        let num = &s[1..];
        if let Ok(n) = num.parse::<u32>() {
            return n <= 30;
        }
    }
    // Special registers
    matches!(s.as_str(),
        "sp" | "wsp" | "xzr" | "wzr" | "lr"
    )
    ||
    // FP/SIMD: d0-d31, s0-s31, q0-q31, v0-v31, h0-h31, b0-b31
    {
        if (s.starts_with('d') || s.starts_with('s') || s.starts_with('q')
            || s.starts_with('v') || s.starts_with('h') || s.starts_with('b'))
            && s.len() >= 2
        {
            let num = &s[1..];
            if let Ok(n) = num.parse::<u32>() {
                return n <= 31;
            }
        }
        false
    }
}

// ── Directive parsing helpers ──────────────────────────────────────────

/// Parse a `.section name,"flags",@type` directive.
fn parse_section_directive(args: &str) -> Result<AsmDirective, String> {
    let parts = split_section_args(args);
    let name = parts
        .first()
        .map(|s| s.trim().trim_matches('"').to_string())
        .unwrap_or_else(|| ".text".to_string());
    let flags = parts.get(1).map(|s| s.trim().trim_matches('"').to_string());
    let section_type = parts.get(2).map(|s| s.trim().to_string());
    Ok(AsmDirective::Section(SectionDirective {
        name,
        flags,
        section_type,
    }))
}

/// Split section directive args, respecting quoted strings.
fn split_section_args(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    for c in s.chars() {
        if c == '"' {
            in_quotes = !in_quotes;
            current.push(c);
        } else if c == ',' && !in_quotes {
            parts.push(current.clone());
            current.clear();
        } else {
            current.push(c);
        }
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

/// Parse `.type name, %function` or `@object` etc.
/// Also accepts space-separated form: `.type name STT_NOTYPE`.
fn parse_type_directive(args: &str) -> Result<AsmDirective, String> {
    let (sym, kind_str) = if let Some(comma_pos) = args.find(',') {
        (args[..comma_pos].trim(), args[comma_pos + 1..].trim())
    } else {
        // Space-separated fallback: ".type sym STT_NOTYPE"
        let parts: Vec<&str> = args.split_whitespace().collect();
        if parts.len() >= 2 {
            (parts[0], parts[1])
        } else {
            (args.trim(), "")
        }
    };
    let kind = match kind_str {
        "%function" | "@function" | "STT_FUNC" => SymbolKind::Function,
        "%object" | "@object" | "STT_OBJECT" => SymbolKind::Object,
        "@tls_object" => SymbolKind::TlsObject,
        _ => SymbolKind::NoType,
    };
    Ok(AsmDirective::SymbolType(sym.to_string(), kind))
}

/// Parse `.size name, expr`.
fn parse_size_directive(args: &str) -> Result<AsmDirective, String> {
    let parts: Vec<&str> = args.splitn(2, ',').collect();
    if parts.len() != 2 {
        return Err(format!(
            "malformed .size directive: expected 'name, expr', got '{}'",
            args
        ));
    }
    let sym = parts[0].trim().to_string();
    let expr_str = parts[1].trim();
    if let Some(rest) = expr_str.strip_prefix(".-") {
        let label = rest.trim().to_string();
        Ok(AsmDirective::Size(sym, SizeExpr::CurrentMinusSymbol(label)))
    } else if let Ok(size) = expr_str.parse::<u64>() {
        Ok(AsmDirective::Size(sym, SizeExpr::Constant(size)))
    } else {
        // Size expressions we can't evaluate (e.g. complex expressions) are non-fatal;
        // the symbol size is not critical for code correctness in static linking
        Ok(AsmDirective::Ignored)
    }
}

/// Parse `.comm name, size[, align]`.
fn parse_comm_directive(args: &str) -> Result<AsmDirective, String> {
    let parts: Vec<&str> = args.split(',').collect();
    if parts.len() < 2 {
        return Err(format!(
            "malformed .comm directive: expected 'name, size[, align]', got '{}'",
            args
        ));
    }
    let sym = parts[0].trim().to_string();
    let size: u64 = parts[1].trim().parse().unwrap_or(0);
    let align: u64 = if parts.len() > 2 {
        parts[2].trim().parse().unwrap_or(1)
    } else {
        1
    };
    Ok(AsmDirective::Comm(sym, size, align))
}

/// Parse comma-separated data values that may be integers, symbols, or symbol expressions.
fn parse_data_values(s: &str) -> Result<Vec<DataValue>, String> {
    let mut vals = Vec::new();
    for part in s.split(',') {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Check for symbol difference: A - B or A - B + C
        if let Some(dv) = try_parse_symbol_diff(trimmed) {
            vals.push(dv);
            continue;
        }
        // Try integer
        if let Ok(val) = parse_data_value(trimmed) {
            vals.push(DataValue::Integer(val));
            continue;
        }
        // Check for symbol+offset or symbol-offset
        if let Some(dv) = try_parse_symbol_offset(trimmed) {
            vals.push(dv);
            continue;
        }
        // Check if it looks like an expression with operators (for deferred evaluation)
        if trimmed.contains('-')
            || trimmed.contains('+')
            || trimmed.contains(">>")
            || trimmed.contains("<<")
            || trimmed.starts_with('(')
        {
            vals.push(DataValue::Expr(trimmed.to_string()));
        } else {
            // Symbol reference
            vals.push(DataValue::Symbol(trimmed.to_string()));
        }
    }
    Ok(vals)
}

/// Check if a string looks like a GNU numeric label reference (e.g. "2f", "1b", "42f").
fn is_numeric_label_ref(s: &str) -> bool {
    if s.len() < 2 {
        return false;
    }
    let last = s.as_bytes()[s.len() - 1];
    if last != b'f' && last != b'F' && last != b'b' && last != b'B' {
        return false;
    }
    s[..s.len() - 1].bytes().all(|b| b.is_ascii_digit())
}

/// Strip balanced outer parentheses from an expression, recursively.
/// e.g. "((foo) - .)" => "(foo) - ." => "foo - ." (inner parens on individual terms
/// are handled by callers).
fn strip_outer_parens(s: &str) -> &str {
    let s = s.trim();
    if !s.starts_with('(') || !s.ends_with(')') {
        return s;
    }
    // Check if the outer parens are actually matched (not "(a)-(b)")
    let inner = &s[1..s.len() - 1];
    let mut depth = 0i32;
    for ch in inner.chars() {
        match ch {
            '(' => depth += 1,
            ')' => {
                if depth == 0 {
                    return s; // closing paren in middle means outer parens aren't a simple wrapper
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    if depth == 0 {
        strip_outer_parens(inner) // recursively strip more layers
    } else {
        s
    }
}

/// Strip parentheses from a symbol name: "(9997f)" => "9997f", "(__label)" => "__label"
fn strip_sym_parens(s: &str) -> &str {
    let s = s.trim();
    if s.starts_with('(') && s.ends_with(')') {
        let inner = &s[1..s.len() - 1];
        // Make sure there are no unbalanced parens inside
        let mut depth = 0i32;
        for ch in inner.chars() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    if depth == 0 {
                        return s;
                    }
                    depth -= 1;
                }
                _ => {}
            }
        }
        if depth == 0 {
            strip_sym_parens(inner)
        } else {
            s
        }
    } else {
        s
    }
}

/// Try to parse a symbol difference expression like "A - B" or "A - B + C".
/// Also handles numeric label references like "662b-661b".
/// Handles parenthesized expressions like "((9997f) - .)".
fn try_parse_symbol_diff(expr: &str) -> Option<DataValue> {
    let expr = strip_outer_parens(expr);
    if expr.is_empty() {
        return None;
    }
    let first_char = expr.chars().next()?;
    let is_sym_start = first_char.is_ascii_alphabetic() || first_char == '_' || first_char == '.';
    let could_be_numeric_ref = first_char.is_ascii_digit();
    let could_be_paren_sym = first_char == '(';
    if !is_sym_start && !could_be_numeric_ref && !could_be_paren_sym {
        return None;
    }
    let minus_pos = find_symbol_diff_minus(expr)?;
    let sym_a_raw = strip_sym_parens(expr[..minus_pos].trim()).to_string();
    let rest = expr[minus_pos + 1..].trim();
    // rest might be "B" or "B + offset"
    let (sym_b, extra_addend) = if let Some(plus_pos) = rest.find('+') {
        let b = strip_sym_parens(rest[..plus_pos].trim()).to_string();
        let add_str = rest[plus_pos + 1..].trim();
        let add_val: i64 = add_str.parse().unwrap_or(0);
        (b, add_val)
    } else {
        (strip_sym_parens(rest).to_string(), 0i64)
    };
    // Decompose sym_a into symbol+offset if it contains a '+' or '-' with a numeric suffix.
    // ELF relocations require separate symbol name and numeric addend, so composite
    // names like "cgroup_bpf_enabled_key+48" must be split into ("cgroup_bpf_enabled_key", 48).
    let (sym_a, extra_addend) = {
        let mut sym = sym_a_raw.clone();
        let mut addend = extra_addend;
        if let Some(plus_idx) = sym_a_raw.rfind('+') {
            let left = sym_a_raw[..plus_idx].trim();
            let right = sym_a_raw[plus_idx + 1..].trim();
            if let Ok(val) = right.parse::<i64>() {
                if !left.is_empty() {
                    addend += val;
                    sym = left.to_string();
                }
            }
        } else if let Some(minus_idx) = sym_a_raw.rfind('-') {
            if minus_idx > 0 {
                let left = sym_a_raw[..minus_idx].trim();
                let right = sym_a_raw[minus_idx + 1..].trim();
                if let Ok(val) = right.parse::<i64>() {
                    if !left.is_empty() {
                        addend -= val;
                        sym = left.to_string();
                    }
                }
            }
        }
        (sym, addend)
    };
    if sym_b.is_empty() {
        return None;
    }
    let a_first = sym_a.chars().next()?;
    let a_is_sym = a_first.is_ascii_alphabetic() || a_first == '_' || a_first == '.';
    let b_first = sym_b.chars().next().unwrap();
    let b_is_sym = b_first.is_ascii_alphabetic() || b_first == '_' || b_first == '.';
    if !b_is_sym && !is_numeric_label_ref(&sym_b) {
        return None;
    }
    // Also verify sym_a is valid (symbol or numeric label ref)
    if !a_is_sym && !is_numeric_label_ref(&sym_a) {
        return None;
    }
    if extra_addend != 0 {
        Some(DataValue::SymbolDiffAddend(sym_a, sym_b, extra_addend))
    } else {
        Some(DataValue::SymbolDiff(sym_a, sym_b))
    }
}

/// Try to parse symbol+offset or symbol-offset.
/// Also handles offset+symbol (e.g., 0x9b000000 + some_symbol).
fn try_parse_symbol_offset(s: &str) -> Option<DataValue> {
    for (i, c) in s.char_indices().skip(1) {
        if c == '+' || c == '-' {
            let left = s[..i].trim();
            let right_with_sign = &s[i..]; // includes the sign

            // Case 1: symbol+offset or symbol-offset
            if let Ok(offset) = parse_int_literal(right_with_sign) {
                if !left.is_empty() && !left.contains(' ') {
                    return Some(DataValue::SymbolOffset(left.to_string(), offset));
                }
            }

            // Case 2: offset+symbol (e.g., "0x9b000000 + some_symbol") - only for '+'
            if c == '+' {
                if let Ok(offset) = parse_int_literal(left) {
                    let sym = right_with_sign[1..].trim(); // skip the '+'
                    if !sym.is_empty()
                        && !sym.contains(' ')
                        && sym
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.')
                    {
                        return Some(DataValue::SymbolOffset(sym.to_string(), offset));
                    }
                }
            }
        }
    }
    None
}

/// Find the position of the '-' operator in a symbol difference expression.
/// Skips over parenthesized sub-expressions.
fn find_symbol_diff_minus(expr: &str) -> Option<usize> {
    let bytes = expr.as_bytes();
    let len = bytes.len();
    let mut i = 1;
    let mut depth = 0i32;
    while i < len {
        match bytes[i] {
            b'(' => {
                depth += 1;
                i += 1;
                continue;
            }
            b')' => {
                depth -= 1;
                i += 1;
                continue;
            }
            _ => {}
        }
        if depth > 0 {
            i += 1;
            continue;
        }
        if bytes[i] == b'-' {
            let left_char = bytes[i - 1];
            let left_ok = left_char.is_ascii_alphanumeric()
                || left_char == b'_'
                || left_char == b'.'
                || left_char == b' '
                || left_char == b')';
            let right_start = expr[i + 1..].trim_start();
            if !right_start.is_empty() {
                let right_char = right_start.as_bytes()[0];
                let right_ok = right_char.is_ascii_alphabetic()
                    || right_char == b'_'
                    || right_char == b'.'
                    || right_char.is_ascii_digit()
                    || right_char == b'(';
                if left_ok && right_ok {
                    return Some(i);
                }
            }
        }
        i += 1;
    }
    None
}

/// Parse a data value (integer literal, possibly negative).
fn parse_data_value(s: &str) -> Result<i64, String> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(0);
    }
    asm_expr::parse_integer_expr(s)
}
