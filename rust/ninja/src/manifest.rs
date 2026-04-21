//! Parser for `build.ninja` files.
//!
//! Implements the subset of the format needed by the upstream
//! `output_test.py` suite: `rule`, `build`, `default`, top-level `var = ...`
//! bindings, and indented per-edge bindings. `subninja` / `include` /
//! `pool` are stubbed and can be added in later phases.
//!
//! The parser is intentionally tolerant: unknown statements at the top
//! level are reported as errors with a line number.

use crate::graph::{Edge, Rule, State};
use std::collections::HashMap;

#[allow(dead_code)]
pub fn parse(src: &str) -> Result<State, String> {
    let mut state = State::default();
    parse_into(&mut state, src, std::path::Path::new("."))?;
    Ok(state)
}

/// Read and parse a manifest from disk. Recursively follows
/// `include` / `subninja` directives relative to the file’s parent
/// directory — enough for CMake-generated trees that split rules
/// across `CMakeFiles/rules.ninja`.
pub fn parse_file(path: &str) -> Result<State, String> {
    let mut state = State::default();
    let p = std::path::Path::new(path);
    let base = p.parent().unwrap_or_else(|| std::path::Path::new("."));
    let src = std::fs::read_to_string(path).map_err(|e| format!("loading '{path}': {e}"))?;
    parse_into(&mut state, &src, base)?;
    Ok(state)
}

/// Parse `src` (the contents of a manifest file located in `base_dir`)
/// into the existing `state`. Splits out from `parse` so that
/// `include` / `subninja` can recurse with the same accumulator and a
/// new `base_dir`.
fn parse_into(state: &mut State, src: &str, base_dir: &std::path::Path) -> Result<(), String> {
    let mut p = Parser::new(src);
    while !p.eof() {
        p.skip_blank_and_comments();
        if p.eof() {
            break;
        }
        let line_no = p.line;
        let word = p.word();
        match word.as_str() {
            "rule" => {
                let r = p.parse_rule().map_err(|e| with_line(line_no, &e))?;
                state.rules.insert(r.name.clone(), r);
            }
            "build" => {
                let edge = p.parse_build(&*state).map_err(|e| with_line(line_no, &e))?;
                let idx = state.edges.len();
                for o in &edge.outputs {
                    state.producers.insert(o.clone(), idx);
                }
                for o in &edge.implicit_outputs {
                    state.producers.insert(o.clone(), idx);
                }
                state.edges.push(edge);
            }
            "default" => {
                let mut targets = Vec::new();
                while let Some(t) = p.next_path()? {
                    targets.push(expand_simple(&t, &state.bindings, None, None));
                }
                p.expect_newline()?;
                state.defaults.extend(targets);
            }
            "pool" => {
                // Skip pool body for now — depth limits not enforced.
                let _ = p.word();
                p.expect_newline()?;
                while p.peek_indent() {
                    p.skip_line();
                }
            }
            kw @ ("include" | "subninja") => {
                // Resolve the include target against the *current*
                // file’s directory (gcc/CMake-generated trees rely on
                // this — `include CMakeFiles/rules.ninja` from a
                // `build.ninja` in `build/` must read
                // `build/CMakeFiles/rules.ninja`).
                let raw = p.read_value();
                let path_str = expand_simple(raw.trim(), &state.bindings, None, None);
                let nested_path = base_dir.join(&path_str);
                let nested_base = nested_path
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new("."))
                    .to_path_buf();
                let nested_src = std::fs::read_to_string(&nested_path).map_err(|e| {
                    with_line(line_no, &format!("{kw} '{}' - {e}", nested_path.display()))
                })?;
                parse_into(state, &nested_src, &nested_base)?;
            }
            "" => {
                // Stray blank line after skip_blank_and_comments shouldn't happen.
                break;
            }
            other => {
                // Top-level binding: `name = value`
                p.skip_spaces();
                if p.peek() == Some('=') {
                    p.bump();
                    let value = p.read_value();
                    let expanded = expand_simple(&value, &state.bindings, None, None);
                    state.bindings.insert(other.to_string(), expanded);
                } else {
                    return Err(with_line(
                        line_no,
                        &format!("expected statement, got '{other}'"),
                    ));
                }
            }
        }
    }
    Ok(())
}

fn with_line(line: usize, msg: &str) -> String {
    format!("line {line}: {msg}")
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
    line: usize,
}

impl<'a> Parser<'a> {
    fn new(s: &'a str) -> Self {
        Self {
            bytes: s.as_bytes(),
            pos: 0,
            line: 1,
        }
    }

    fn eof(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    fn peek(&self) -> Option<char> {
        self.bytes.get(self.pos).map(|&b| b as char)
    }

    fn peek_at(&self, off: usize) -> Option<char> {
        self.bytes.get(self.pos + off).map(|&b| b as char)
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += 1;
        if c == '\n' {
            self.line += 1;
        }
        Some(c)
    }

    fn skip_spaces(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t')) {
            self.pos += 1;
        }
    }

    fn skip_line(&mut self) {
        while let Some(c) = self.peek() {
            self.bump();
            if c == '\n' {
                break;
            }
        }
    }

    fn skip_blank_and_comments(&mut self) {
        loop {
            self.skip_spaces();
            match self.peek() {
                Some('\n') => {
                    self.bump();
                }
                Some('#') => self.skip_line(),
                _ => break,
            }
        }
    }

    /// Reads the next bareword (identifier-ish). Stops at whitespace, `=`,
    /// `:`, `|`, or newline.
    fn word(&mut self) -> String {
        self.skip_spaces();
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' {
                self.pos += 1;
            } else {
                break;
            }
        }
        std::str::from_utf8(&self.bytes[start..self.pos])
            .unwrap_or("")
            .to_string()
    }

    /// Reads the value of a `name = value` line up to (but not including)
    /// the terminating newline. Handles `$\n` line continuations and
    /// trims trailing whitespace.
    fn read_value(&mut self) -> String {
        self.skip_spaces();
        let mut out = String::new();
        while let Some(c) = self.peek() {
            if c == '$' && self.peek_at(1) == Some('\n') {
                self.pos += 2;
                self.line += 1;
                self.skip_spaces();
                continue;
            }
            if c == '\n' {
                self.bump();
                break;
            }
            out.push(c);
            self.pos += 1;
        }
        // Trim trailing spaces but preserve internal spacing.
        while out.ends_with(' ') || out.ends_with('\t') {
            out.pop();
        }
        out
    }

    fn expect_newline(&mut self) -> Result<(), String> {
        self.skip_spaces();
        match self.peek() {
            Some('\n') => {
                self.bump();
                Ok(())
            }
            None => Ok(()),
            Some(c) => Err(format!("expected newline, got '{c}'")),
        }
    }

    /// Returns true if the current line begins with a leading space/tab,
    /// indicating an indented continuation of the previous block.
    fn peek_indent(&self) -> bool {
        matches!(self.bytes.get(self.pos), Some(b' ' | b'\t'))
    }

    /// Reads the next path-ish token on a build line. Returns None when
    /// it hits `:`, `|`, `||`, or end of line.
    fn next_path(&mut self) -> Result<Option<String>, String> {
        self.skip_spaces();
        match self.peek() {
            None | Some('\n' | ':' | '|') => return Ok(None),
            _ => {}
        }
        let mut out = String::new();
        while let Some(c) = self.peek() {
            match c {
                ' ' | '\t' | '\n' | ':' | '|' => break,
                '$' => {
                    self.pos += 1;
                    match self.peek() {
                        Some(' ') | Some(':') | Some('$') | Some('|') => {
                            out.push(self.peek().unwrap());
                            self.pos += 1;
                        }
                        Some('\n') => {
                            self.line += 1;
                            self.pos += 1;
                            self.skip_spaces();
                        }
                        _ => out.push('$'),
                    }
                }
                _ => {
                    out.push(c);
                    self.pos += 1;
                }
            }
        }
        Ok(Some(out))
    }

    fn parse_rule(&mut self) -> Result<Rule, String> {
        let name = self.word();
        if name.is_empty() {
            return Err("rule needs a name".into());
        }
        self.expect_newline()?;
        let mut bindings = HashMap::new();
        while self.peek_indent() {
            self.skip_spaces();
            let key = self.word();
            if key.is_empty() {
                self.skip_line();
                continue;
            }
            self.skip_spaces();
            if self.peek() != Some('=') {
                return Err(format!("expected '=' after '{key}'"));
            }
            self.bump();
            bindings.insert(key, self.read_value());
        }
        Ok(Rule { name, bindings })
    }

    fn parse_build(&mut self, state: &State) -> Result<Edge, String> {
        let mut outputs = Vec::new();
        let mut implicit_outputs = Vec::new();
        let mut implicit = false;
        while let Some(p) = self.next_path()? {
            if implicit {
                implicit_outputs.push(p);
            } else {
                outputs.push(p);
            }
            self.skip_spaces();
            if self.peek() == Some('|') && self.peek_at(1) != Some('|') {
                self.bump();
                implicit = true;
            }
        }
        self.skip_spaces();
        if self.peek() != Some(':') {
            return Err("expected ':' after build outputs".into());
        }
        self.bump();
        let rule = self.word();
        if rule.is_empty() {
            return Err("expected rule name in build".into());
        }
        let mut inputs = Vec::new();
        while let Some(p) = self.next_path()? {
            inputs.push(p);
        }
        let mut implicit_inputs = Vec::new();
        let mut order_only_inputs = Vec::new();
        loop {
            self.skip_spaces();
            match (self.peek(), self.peek_at(1)) {
                (Some('|'), Some('|')) => {
                    self.bump();
                    self.bump();
                    while let Some(p) = self.next_path()? {
                        order_only_inputs.push(p);
                    }
                }
                (Some('|'), _) => {
                    self.bump();
                    while let Some(p) = self.next_path()? {
                        implicit_inputs.push(p);
                    }
                }
                _ => break,
            }
        }
        self.expect_newline()?;
        let mut bindings = HashMap::new();
        while self.peek_indent() {
            self.skip_spaces();
            let key = self.word();
            if key.is_empty() {
                self.skip_line();
                continue;
            }
            self.skip_spaces();
            if self.peek() != Some('=') {
                return Err(format!("expected '=' after '{key}'"));
            }
            self.bump();
            bindings.insert(key, self.read_value());
        }
        // Expand paths against file-scope bindings.
        let expand_all = |v: Vec<String>| {
            v.into_iter()
                .map(|s| expand_simple(&s, &state.bindings, None, None))
                .collect()
        };
        // \`dyndep = ...\` may live on the edge OR be inherited from
        // the rule (the test_issue_2621 plan declares it on the rule).
        // Resolve via the standard layered lookup so per-edge bindings
        // shadow rule bindings.
        let rule_obj = state.rules.get(&rule);
        let dyndep_raw = bindings
            .get("dyndep")
            .cloned()
            .or_else(|| rule_obj.and_then(|r| r.bindings.get("dyndep").cloned()));
        let dyndep = dyndep_raw.map(|s| {
            expand_simple(
                &s,
                &state.bindings,
                Some(&bindings),
                rule_obj.map(|r| &r.bindings),
            )
        });
        Ok(Edge {
            rule,
            outputs: expand_all(outputs),
            implicit_outputs: expand_all(implicit_outputs),
            inputs: expand_all(inputs),
            implicit_inputs: expand_all(implicit_inputs),
            order_only_inputs: expand_all(order_only_inputs),
            bindings,
            dyndep,
        })
    }
}

/// Variable expansion. Supports `$var`, `${var}`, escapes `$$`, `$ `, `$:`,
/// `$|`, and `$\n` line continuations. Lookups walk:
///   1. per-edge bindings (if provided)
///   2. rule bindings (if provided)
///   3. file-scope bindings
///
/// Special names `in`, `out` are resolved by the caller via the rule path —
/// here they fall through to file-scope. The full edge-context expansion
/// lives in `build::expand_edge_command`.
pub fn expand_simple(
    s: &str,
    file_scope: &HashMap<String, String>,
    edge_bindings: Option<&HashMap<String, String>>,
    rule_bindings: Option<&HashMap<String, String>>,
) -> String {
    expand(s, &|name| {
        if let Some(b) = edge_bindings
            && let Some(v) = b.get(name)
        {
            return Some(v.clone());
        }
        if let Some(b) = rule_bindings
            && let Some(v) = b.get(name)
        {
            // Rule values may themselves reference file-scope vars, but
            // not other rule vars at this level. Recurse against file
            // scope only to avoid loops.
            return Some(expand_simple(v, file_scope, None, None));
        }
        file_scope.get(name).cloned()
    })
}

/// Generic expansion driver: replaces `$var` / `${var}` using `lookup`.
pub fn expand<F: Fn(&str) -> Option<String>>(s: &str, lookup: &F) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c != '$' {
            out.push(c);
            i += 1;
            continue;
        }
        i += 1;
        if i >= bytes.len() {
            out.push('$');
            break;
        }
        let n = bytes[i] as char;
        match n {
            '$' | ' ' | ':' | '|' => {
                out.push(n);
                i += 1;
            }
            '\n' => {
                i += 1;
                while matches!(bytes.get(i), Some(b' ' | b'\t')) {
                    i += 1;
                }
            }
            '{' => {
                i += 1;
                let start = i;
                while i < bytes.len() && bytes[i] != b'}' {
                    i += 1;
                }
                let name = std::str::from_utf8(&bytes[start..i]).unwrap_or("");
                if i < bytes.len() {
                    i += 1; // skip '}'
                }
                if let Some(v) = lookup(name) {
                    out.push_str(&v);
                }
            }
            c if c.is_ascii_alphanumeric() || c == '_' || c == '-' => {
                // Ninja variable names are restricted to
                // [A-Za-z0-9_-]. Crucially `.` is NOT part of a name,
                // so `$out.d` expands `$out` and leaves `.d` literal —
                // matching the behavior gcc-emitting depfile rules
                // depend on.
                let start = i;
                while i < bytes.len() {
                    let cc = bytes[i];
                    if cc.is_ascii_alphanumeric() || cc == b'_' || cc == b'-' {
                        i += 1;
                    } else {
                        break;
                    }
                }
                let name = std::str::from_utf8(&bytes[start..i]).unwrap_or("");
                if let Some(v) = lookup(name) {
                    out.push_str(&v);
                }
            }
            _ => {
                out.push('$');
                out.push(n);
                i += 1;
            }
        }
    }
    out
}
