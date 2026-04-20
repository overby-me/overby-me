//! Makefile-style depfile parser.
//!
//! gcc emits depfiles via `-MMD -MF $out.d`. The format is a tiny
//! subset of make rules:
//!
//! ```text
//! target1 [target2...]: dep1 dep2 \
//!     dep3 dep4
//! ```
//!
//! Rules we handle, scoped to what real-world C/C++ toolchains emit:
//!   - line continuations via `\` at end of line
//!   - `\<space>` escapes a literal space inside a path
//!   - `$$` collapses to a literal `$` (rare, but make-syntax legal)
//!   - comments starting with `#`
//!
//! We deliberately accept multiple `target:` blocks and merge them so
//! callers don't need to care about how the toolchain split things up.

use std::collections::HashMap;

#[derive(Debug, Default)]
pub struct Depfile {
    /// Map of target path → its discovered prerequisites, in source
    /// order with duplicates removed.
    pub targets: HashMap<String, Vec<String>>,
}

/// Parse a depfile's contents. Returns the per-target dependency map.
/// Malformed input is best-effort: any rule we can't make sense of is
/// silently dropped so a stray byte never breaks the build.
pub fn parse(src: &str) -> Depfile {
    let mut out = Depfile::default();
    let logical = unfold(src);
    for raw_line in logical.lines() {
        let line = strip_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }
        let Some(colon) = find_colon(line) else {
            continue;
        };
        let lhs = &line[..colon];
        let rhs = &line[colon + 1..];
        let targets = tokenize(lhs);
        let deps = tokenize(rhs);
        for t in &targets {
            let entry = out.targets.entry(t.clone()).or_default();
            for d in &deps {
                if !entry.contains(d) {
                    entry.push(d.clone());
                }
            }
        }
    }
    out
}

/// Collapse `\<newline>` continuations into spaces so the rest of the
/// parser can work line-by-line.
fn unfold(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let bytes = src.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
            out.push(' ');
            i += 2;
        } else if bytes[i] == b'\\'
            && i + 2 < bytes.len()
            && bytes[i + 1] == b'\r'
            && bytes[i + 2] == b'\n'
        {
            out.push(' ');
            i += 3;
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

fn strip_comment(line: &str) -> &str {
    line.split_once('#').map(|(a, _)| a).unwrap_or(line)
}

/// Find the rule-separating colon, skipping ones that are part of a
/// Windows-style drive letter (`C:`) at the very start of a token.
fn find_colon(line: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b':' {
            // Windows path heuristic: a single drive letter followed
            // by `:` at position 1 (or right after whitespace) is part
            // of the path, not the rule separator.
            let prev = if i == 0 { None } else { Some(bytes[i - 1]) };
            let two_back = if i < 2 { None } else { Some(bytes[i - 2]) };
            let drive_letter = prev.is_some_and(|c| c.is_ascii_alphabetic())
                && two_back.is_none_or(|c| c == b' ' || c == b'\t');
            if !drive_letter {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

/// Split a side of the rule into path tokens, honoring `\ ` and `$$`
/// escapes that real depfile producers emit.
fn tokenize(side: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut cur = String::new();
    let bytes = side.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        match c {
            b' ' | b'\t' => {
                if !cur.is_empty() {
                    tokens.push(std::mem::take(&mut cur));
                }
                i += 1;
            }
            b'\\' if i + 1 < bytes.len() && (bytes[i + 1] == b' ' || bytes[i + 1] == b'\t') => {
                cur.push(bytes[i + 1] as char);
                i += 2;
            }
            b'$' if i + 1 < bytes.len() && bytes[i + 1] == b'$' => {
                cur.push('$');
                i += 2;
            }
            _ => {
                cur.push(c as char);
                i += 1;
            }
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple() {
        let d = parse("foo.o: foo.c bar.h\n");
        assert_eq!(d.targets["foo.o"], vec!["foo.c", "bar.h"]);
    }

    #[test]
    fn continuation() {
        let d = parse("foo.o: foo.c \\\n bar.h \\\n baz.h\n");
        assert_eq!(d.targets["foo.o"], vec!["foo.c", "bar.h", "baz.h"]);
    }

    #[test]
    fn escaped_space() {
        let d = parse("foo.o: with\\ space.h\n");
        assert_eq!(d.targets["foo.o"], vec!["with space.h"]);
    }

    #[test]
    fn dedup_across_blocks() {
        let d = parse("foo.o: a.h\nfoo.o: a.h b.h\n");
        assert_eq!(d.targets["foo.o"], vec!["a.h", "b.h"]);
    }
}
