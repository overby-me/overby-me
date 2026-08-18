//! Shared peephole optimizer utilities used by multiple backends.
//!
//! Several peephole passes across ARM, RISC-V, x86, and i686 perform the same
//! text-level operations on assembly lines: whole-word matching, register
//! replacement in source operands, and a compact line store that avoids
//! per-line `String` allocation.  This module extracts those shared building
//! blocks so each backend can focus on arch-specific patterns.

// ── Word-boundary matching helpers ───────────────────────────────────────
//
// Assembly register names like "x1" must not match inside "x11", "x10", or
// symbol names like "main.x1.0".  These functions treat alphanumeric, `.`,
// and `_` as word characters (common in ELF symbol names and GAS labels).

/// Returns `true` if `b` is a "word character" for register/symbol matching:
/// ASCII alphanumeric, `.`, or `_`.
#[inline]
pub(crate) fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'.' || b == b'_'
}

/// Replace every whole-word occurrence of `old` with `new` in `text`.
///
/// A word boundary is a position where the adjacent character is not an
/// identifier char.  This prevents "x1" from matching inside "x11" or
/// "main.x1.0".
pub(crate) fn replace_whole_word(text: &str, old: &str, new: &str) -> String {
    let bytes = text.as_bytes();
    let old_bytes = old.as_bytes();
    let old_len = old_bytes.len();
    let text_len = bytes.len();
    let mut result = String::with_capacity(text.len());
    let mut i = 0;

    while i < text_len {
        if i + old_len <= text_len && &bytes[i..i + old_len] == old_bytes {
            let before_ok = i == 0 || !is_ident_char(bytes[i - 1]);
            let after_ok = i + old_len >= text_len || !is_ident_char(bytes[i + old_len]);
            if before_ok && after_ok {
                result.push_str(new);
                i += old_len;
                continue;
            }
        }
        result.push(bytes[i] as char);
        i += 1;
    }
    result
}

/// Returns `true` if `text` contains `word` at a word boundary.
pub(crate) fn has_whole_word(text: &str, word: &str) -> bool {
    let bytes = text.as_bytes();
    let word_bytes = word.as_bytes();
    let word_len = word_bytes.len();
    let text_len = bytes.len();

    let mut i = 0;
    while i + word_len <= text_len {
        if &bytes[i..i + word_len] == word_bytes {
            let before_ok = i == 0 || !is_ident_char(bytes[i - 1]);
            let after_ok = i + word_len >= text_len || !is_ident_char(bytes[i + word_len]);
            if before_ok && after_ok {
                return true;
            }
        }
        i += 1;
    }
    false
}

/// Replace a register name in the *source* operand positions of an instruction.
///
/// Given an assembly line like `"  add x0, x1, x2"`, this replaces `old_reg`
/// with `new_reg` only in the operands *after* the first comma (i.e. the
/// source operands, not the destination).  Returns `None` if no replacement
/// was made.  Preserves leading whitespace.
pub(crate) fn replace_source_reg_in_instruction(
    line: &str,
    old_reg: &str,
    new_reg: &str,
) -> Option<String> {
    let trimmed = line.trim();

    // Find the first space to separate mnemonic from operands
    let space_pos = trimmed.find(' ')?;
    let args_start = space_pos + 1;
    let args = &trimmed[args_start..];

    // Find first comma -- everything after it is source operands
    let comma_pos = args.find(',')?;
    let after_first_arg = &args[comma_pos..];

    // Only replace in the source part (after the first comma)
    let new_suffix = replace_whole_word(after_first_arg, old_reg, new_reg);
    if new_suffix == after_first_arg {
        return None;
    }

    // Build the new line
    let prefix = &trimmed[..args_start + comma_pos];
    let new_trimmed = format!("{}{}", prefix, new_suffix);

    // Preserve leading whitespace
    let leading = line.len() - line.trim_start().len();
    let leading_ws = &line[..leading];
    Some(format!("{}{}", leading_ws, new_trimmed))
}

// ── LineStore: compact assembly line storage ─────────────────────────────
//
// During peephole optimization we repeatedly access and occasionally replace
// individual assembly lines.  Storing each line as its own `String` is
// wasteful (24 bytes overhead per line on 64-bit).  `LineStore` keeps the
// original text as a single `String` and records byte-offset ranges for each
// line.  Replaced lines go into a small side buffer.  This typically reduces
// memory traffic significantly for large functions.

/// Compact (start, len) entry for one line.  8 bytes vs. 24 for String.
/// When `len == u32::MAX`, the line has been replaced and `start` is the
/// index into the `replacements` vector.
#[derive(Clone, Copy)]
pub(crate) struct LineEntry {
    start: u32,
    len: u32,
}

/// Compact storage for assembly lines that avoids per-line allocation.
pub(crate) struct LineStore {
    /// The original assembly text (kept alive for the duration of optimization).
    original: String,
    /// One entry per line.
    entries: Vec<LineEntry>,
    /// Side buffer for lines that have been replaced by optimization passes.
    replacements: Vec<String>,
}

impl LineStore {
    /// Build a `LineStore` from an assembly string.
    pub(crate) fn new(asm: String) -> Self {
        let bytes = asm.as_bytes();
        let total_len = bytes.len();
        let estimated_lines = total_len / 20 + 1;
        let mut entries = Vec::with_capacity(estimated_lines);

        let mut start = 0usize;
        let mut i = 0;
        while i < total_len {
            if bytes[i] == b'\n' {
                entries.push(LineEntry {
                    start: start as u32,
                    len: (i - start) as u32,
                });
                start = i + 1;
            }
            i += 1;
        }
        // Handle last line (no trailing newline)
        if start <= total_len {
            let remaining = total_len - start;
            if remaining > 0 || entries.is_empty() {
                entries.push(LineEntry {
                    start: start as u32,
                    len: remaining as u32,
                });
            }
        }

        LineStore {
            original: asm,
            entries,
            replacements: Vec::new(),
        }
    }

    /// Get the text of line `idx`.
    #[inline]
    pub(crate) fn get(&self, idx: usize) -> &str {
        let e = &self.entries[idx];
        if e.len == u32::MAX {
            &self.replacements[e.start as usize]
        } else {
            let start = e.start as usize;
            let end = start + e.len as usize;
            &self.original[start..end]
        }
    }

    /// Number of lines.
    #[inline]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// Replace a line with new text.
    pub(crate) fn replace(&mut self, idx: usize, new_text: String) {
        let rep_idx = self.replacements.len();
        self.replacements.push(new_text);
        self.entries[idx] = LineEntry {
            start: rep_idx as u32,
            len: u32::MAX,
        };
    }

    /// Build the final output string, skipping lines where `skip(i)` returns true.
    pub(crate) fn build_result(&self, skip: impl Fn(usize) -> bool) -> String {
        let mut result = String::with_capacity(self.original.len());
        for i in 0..self.entries.len() {
            if !skip(i) {
                result.push_str(self.get(i));
                result.push('\n');
            }
        }
        result
    }
}
