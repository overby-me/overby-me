//! Minimal dyndep file parser.
//!
//! Dyndep files are emitted by build steps to declare additional
//! implicit inputs/outputs (and an optional `restat = 1`) for a target
//! build statement *after* the manifest has been parsed. The format is
//! a tiny subset of build.ninja:
//!
//! ```text
//! ninja_dyndep_version = 1
//! build OUT [| IMP_OUT...]: dyndep [| IMP_IN...]
//!   restat = 1
//! ```
//!
//! Only `dyndep` may appear as the rule name. We support exactly the
//! constructs the upstream `output_test.py::test_issue_2621` exercises:
//! a `build` line with an explicit output, a single implicit output
//! after `|`, and the literal `dyndep` rule.

use std::collections::HashMap;

#[derive(Debug, Default)]
pub struct DyndepFile {
    /// Map of explicit-output name → declarations for the matching edge.
    pub entries: HashMap<String, DyndepEntry>,
}

#[derive(Debug, Default)]
pub struct DyndepEntry {
    pub implicit_outputs: Vec<String>,
    pub implicit_inputs: Vec<String>,
    pub restat: bool,
}

pub fn parse(src: &str) -> Result<DyndepFile, String> {
    let mut ddf = DyndepFile::default();
    let mut lines = src.lines().peekable();
    let mut saw_version = false;
    while let Some(raw) = lines.next() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if !saw_version {
            // Expect `ninja_dyndep_version = N`.
            let (key, value) = split_kv(line)
                .ok_or_else(|| "expected 'ninja_dyndep_version = ...'".to_string())?;
            if key != "ninja_dyndep_version" {
                return Err("expected 'ninja_dyndep_version = ...'".into());
            }
            if value.trim() != "1" {
                return Err(format!("unsupported 'ninja_dyndep_version = {value}'"));
            }
            saw_version = true;
            continue;
        }
        if let Some(rest) = line.strip_prefix("build ") {
            let entry = parse_build_line(rest)?;
            // Indented `restat = 1` follow-up.
            let mut e = entry.1;
            while let Some(peek) = lines.peek() {
                if peek.starts_with(' ') || peek.starts_with('\t') {
                    let next = lines.next().unwrap().trim();
                    if let Some((k, v)) = split_kv(next)
                        && k == "restat"
                        && v.trim() == "1"
                    {
                        e.restat = true;
                    }
                } else {
                    break;
                }
            }
            ddf.entries.insert(entry.0, e);
        } else {
            return Err(format!("unexpected dyndep statement: {line}"));
        }
    }
    if !saw_version {
        return Err("expected 'ninja_dyndep_version = ...'".into());
    }
    Ok(ddf)
}

/// Parse `OUT [| IMP_OUT...]: dyndep [| IMP_IN...]` into the explicit
/// output (used as the lookup key) and the discovered declarations.
fn parse_build_line(s: &str) -> Result<(String, DyndepEntry), String> {
    let colon = s
        .find(':')
        .ok_or_else(|| format!("expected ':' in dyndep build line: {s}"))?;
    let lhs = &s[..colon];
    let rhs = s[colon + 1..].trim();
    // Split outputs at unescaped `|`.
    let (explicit, implicit_out) = match lhs.find('|') {
        Some(p) => (lhs[..p].trim(), lhs[p + 1..].trim()),
        None => (lhs.trim(), ""),
    };
    let mut tokens = explicit.split_whitespace();
    let out = tokens
        .next()
        .ok_or_else(|| "expected output before ':'".to_string())?;
    let implicit_outputs: Vec<String> = implicit_out
        .split_whitespace()
        .map(|s| s.to_string())
        .collect();
    // RHS: rule name then optional `| IMP_IN...`.
    let mut rhs_parts = rhs.splitn(2, '|');
    let rule_part = rhs_parts.next().unwrap_or("").trim();
    let implicit_in_part = rhs_parts.next().unwrap_or("").trim();
    let mut rule_tokens = rule_part.split_whitespace();
    let rule = rule_tokens.next().unwrap_or("");
    if rule != "dyndep" {
        return Err(format!(
            "dyndep build line must use 'dyndep' rule, got '{rule}'"
        ));
    }
    let implicit_inputs: Vec<String> = implicit_in_part
        .split_whitespace()
        .map(|s| s.to_string())
        .collect();
    Ok((
        out.to_string(),
        DyndepEntry {
            implicit_outputs,
            implicit_inputs,
            restat: false,
        },
    ))
}

fn split_kv(line: &str) -> Option<(&str, &str)> {
    let eq = line.find('=')?;
    Some((line[..eq].trim(), line[eq + 1..].trim()))
}
