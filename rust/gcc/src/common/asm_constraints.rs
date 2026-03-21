//! Shared inline assembly constraint classification utilities.
//!
//! Used by the inline pass (symbol resolution), cfg_simplify (dead block detection),
//! and the backend (operand emission) to consistently identify immediate-only constraints.

/// Check whether a constraint string is purely immediate (no register or memory alternatives).
///
/// Returns true for constraints like "i", "n", "e" that ONLY accept compile-time constants.
/// Returns false for multi-alternative constraints like "ri", "Ir", "g" that also accept
/// registers or memory operands.
///
/// The constraint is first stripped of output/early-clobber modifiers ('=', '+', '&').
/// Named operand references like "[name]" are not immediate constraints.
pub fn constraint_is_immediate_only(constraint: &str) -> bool {
    let stripped = constraint.trim_start_matches(['=', '+', '&', '%']);
    if stripped.is_empty() {
        return false;
    }
    if stripped.starts_with('[') && stripped.ends_with(']') {
        return false;
    }
    // Must have at least one immediate letter
    let has_imm = stripped.chars().any(|c| {
        matches!(
            c,
            'i' | 'I' | 'n' | 'N' | 'e' | 'E' | 'K' | 'M' | 'G' | 'H' | 'J' | 'L' | 'O'
        )
    });
    if !has_imm {
        return false;
    }
    // Must NOT have any register or memory alternative
    let has_reg_or_mem = stripped.chars().any(|c| {
        matches!(
            c,
            'r' | 'q' | 'R' | 'l' |           // GP register
        'g' |                              // general (reg + mem + imm)
        'x' | 'v' | 'Y' |                 // FP register
        'a' | 'b' | 'c' | 'd' | 'S' | 'D' | // specific register
        'm' | 'o' | 'V' | 'p' | 'Q' // memory
        )
    });
    !has_reg_or_mem && !stripped.chars().any(|c| c.is_ascii_digit())
}
