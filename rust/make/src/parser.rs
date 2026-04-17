//! Makefile parser.

use crate::ast::*;

/// Strip the recipe-prefix character from the start of `line`, returning the
/// remainder. Default prefix is tab (`\t`), but `.RECIPEPREFIX := X` can
/// override it.
fn strip_recipe_prefix(line: &str, prefix: char) -> Option<&str> {
    let mut chars = line.chars();
    if chars.next() == Some(prefix) {
        Some(&line[prefix.len_utf8()..])
    } else {
        None
    }
}

/// Strip a Makefile comment (# and everything after, unless preceded by \)
fn strip_makefile_comment(s: &str) -> &str {
    let bytes = s.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'#' && (i == 0 || bytes[i - 1] != b'\\') {
            return &s[..i];
        }
    }
    s
}

pub struct Parser {
    lines: Vec<String>,
    line_nos: Vec<usize>,
    pos: usize,
    source_name: String,
    /// Currently configured recipe prefix character (default: tab).
    /// Updated when `.RECIPEPREFIX := X` is seen.
    recipe_prefix: char,
}

impl Parser {
    pub fn new(input: &str) -> Self {
        Self::new_with_source(input, "-".to_string())
    }

    pub fn new_with_source(input: &str, source_name: String) -> Self {
        // Join backslash-continued lines, keeping track of the starting
        // physical line number for each logical line (1-based).
        let mut lines = Vec::new();
        let mut line_nos = Vec::new();
        let mut current = String::new();
        let mut start_line = 0usize;
        let mut in_continuation = false;
        let mut logical_is_recipe = false;
        for (i, line) in input.lines().enumerate() {
            if current.is_empty() {
                start_line = i + 1;
                logical_is_recipe = line.starts_with('\t');
            }
            // After a backslash-newline, leading whitespace on the
            // next physical line is collapsed — except inside a
            // recipe (logical line that started with the recipe
            // prefix), where whitespace is preserved because the
            // shell interprets continuations.
            let effective = if in_continuation && !logical_is_recipe {
                line.trim_start()
            } else {
                line
            };
            if let Some(stripped) = effective.strip_suffix('\\') {
                current.push_str(stripped);
                current.push(' ');
                in_continuation = true;
            } else {
                current.push_str(effective);
                lines.push(std::mem::take(&mut current));
                line_nos.push(start_line);
                in_continuation = false;
            }
        }
        if !current.is_empty() {
            lines.push(current);
            line_nos.push(start_line);
        }
        Self {
            lines,
            line_nos,
            pos: 0,
            source_name,
            recipe_prefix: '\t',
        }
    }

    fn current_line_no(&self) -> usize {
        self.line_nos.get(self.pos).copied().unwrap_or(0)
    }

    #[allow(dead_code)]
    pub fn source_name(&self) -> &str {
        &self.source_name
    }

    fn peek(&self) -> Option<&str> {
        self.lines.get(self.pos).map(|s| s.as_str())
    }

    fn advance(&mut self) -> Option<String> {
        if self.pos < self.lines.len() {
            let line = self.lines[self.pos].clone();
            self.pos += 1;
            Some(line)
        } else {
            None
        }
    }

    pub fn parse(&mut self) -> Result<Makefile, String> {
        self.parse_body(&[])
    }

    fn parse_body(&mut self, end_keywords: &[&str]) -> Result<Makefile, String> {
        let mut directives = Vec::new();

        while let Some(line) = self.peek() {
            let trimmed = line.trim();

            // Skip empty lines and comments
            if trimmed.is_empty() || trimmed.starts_with('#') {
                self.advance();
                continue;
            }

            // Check for end keywords (else, endif, endef)
            for &kw in end_keywords {
                if trimmed == kw || trimmed.starts_with(&format!("{kw} ")) {
                    return Ok(directives);
                }
            }

            // Recipe lines (start with tab)
            if line.starts_with('\t') {
                // Stray recipe line outside a rule — skip
                self.advance();
                continue;
            }

            if let Some(dir) = self.try_parse_directive()? {
                directives.push(dir);
            } else {
                self.advance();
            }
        }

        Ok(directives)
    }

    fn try_parse_directive(&mut self) -> Result<Option<Directive>, String> {
        let line = match self.peek() {
            Some(l) => l.to_string(),
            None => return Ok(None),
        };
        let trimmed = line.trim();

        // Conditional directives
        if trimmed.starts_with("ifdef ")
            || trimmed.starts_with("ifndef ")
            || trimmed.starts_with("ifeq ")
            || trimmed.starts_with("ifeq(")
            || trimmed.starts_with("ifneq ")
            || trimmed.starts_with("ifneq(")
        {
            return self.parse_conditional().map(Some);
        }

        // Include
        if let Some(rest) = trimmed.strip_prefix("include ") {
            self.advance();
            // Strip comments (# and everything after)
            let rest = strip_makefile_comment(rest);
            let files: Vec<String> = rest.split_whitespace().map(|s| s.to_string()).collect();
            return Ok(Some(Directive::Include(files, false)));
        }
        if let Some(rest) = trimmed
            .strip_prefix("-include ")
            .or_else(|| trimmed.strip_prefix("sinclude "))
        {
            self.advance();
            let rest = strip_makefile_comment(rest);
            let files: Vec<String> = rest.split_whitespace().map(|s| s.to_string()).collect();
            return Ok(Some(Directive::Include(files, true)));
        }

        // Export / Unexport
        if trimmed == "export" {
            self.advance();
            return Ok(Some(Directive::Export(None)));
        }
        if let Some(rest) = trimmed.strip_prefix("export ") {
            self.advance();
            // `export VAR = value` both assigns and marks VAR exported.
            if let Some(assign) = try_parse_assignment(rest) {
                return Ok(Some(Directive::ExportAssign(Box::new(assign))));
            }
            let names: Vec<String> = rest.split_whitespace().map(|s| s.to_string()).collect();
            return Ok(Some(Directive::Export(Some(names))));
        }
        if trimmed == "unexport" {
            self.advance();
            return Ok(Some(Directive::Unexport(None)));
        }
        if let Some(rest) = trimmed.strip_prefix("unexport ") {
            self.advance();
            let names: Vec<String> = rest.split_whitespace().map(|s| s.to_string()).collect();
            return Ok(Some(Directive::Unexport(Some(names))));
        }

        // Override
        if let Some(rest) = trimmed.strip_prefix("override ") {
            if let Some(assign) = try_parse_assignment(rest) {
                self.advance();
                return Ok(Some(Directive::Override(Box::new(assign))));
            }
            // `override undefine VAR` — force-remove even command-line vars.
            if let Some(var) = rest.strip_prefix("undefine ") {
                self.advance();
                return Ok(Some(Directive::OverrideUndefine(var.trim().to_string())));
            }
            // `override define NAME …` — multi-line variable override.
            if rest.starts_with("define ") || rest == "define" {
                // Rewrite the current line to drop the `override ` prefix so
                // `parse_define` can consume it normally, then wrap the
                // resulting Define in override semantics.
                let rest_owned = rest.to_string();
                self.lines[self.pos] = rest_owned;
                let directive = self.parse_define()?;
                if let Directive::Define(name, op, body) = directive {
                    return Ok(Some(Directive::OverrideDefine(name, op, body)));
                }
                return Ok(Some(directive));
            }
        }

        // Undefine. Skip when the line also parses as an assignment —
        // `undefine = value` is a variable named `undefine`, not an
        // undefine directive.
        if let Some(rest) = trimmed.strip_prefix("undefine ")
            && try_parse_assignment(trimmed).is_none()
        {
            self.advance();
            return Ok(Some(Directive::Undefine(rest.trim().to_string())));
        }

        // Define (multi-line variable)
        if trimmed.starts_with("define ") {
            return self.parse_define().map(Some);
        }

        // Vpath
        if trimmed == "vpath" {
            self.advance();
            return Ok(Some(Directive::Vpath(None)));
        }
        if let Some(rest) = trimmed.strip_prefix("vpath ") {
            self.advance();
            let parts: Vec<&str> = rest.splitn(2, ' ').collect();
            if parts.len() == 2 {
                return Ok(Some(Directive::Vpath(Some((
                    parts[0].to_string(),
                    parts[1].to_string(),
                )))));
            }
            return Ok(Some(Directive::Vpath(None)));
        }

        // Try assignment
        if let Some(assign) = try_parse_assignment(trimmed) {
            self.advance();
            // Track `.RECIPEPREFIX := X` so subsequent rules use X as the
            // recipe-line marker. Empty resets to default tab.
            if assign.name.trim() == ".RECIPEPREFIX" {
                let val = assign.value.trim();
                self.recipe_prefix = val.chars().next().unwrap_or('\t');
            }
            return Ok(Some(Directive::Assignment(assign)));
        }

        // Before parsing as a rule, look for target-specific variable
        // assignments: `target[, target...]: [private|override|export] VAR OP value`.
        // Detecting this here avoids misparsing the assignment as a
        // prereq list.
        if let Some(colon) = find_rule_colon(trimmed) {
            let is_double = trimmed[colon..].starts_with("::");
            let after = if is_double {
                &trimmed[colon + 2..]
            } else {
                &trimmed[colon + 1..]
            };
            let after = strip_makefile_comment(after).trim();
            // Strip optional modifier prefixes on target-specific
            // assignments. GNU make accepts any combination of
            // `private`, `export`, and `override` in any order before
            // the assignment; we accept them but currently ignore the
            // distinction (the assignment itself is applied as a
            // target-specific override regardless).
            let mut after_mod = after;
            loop {
                let trimmed_mod = after_mod
                    .strip_prefix("private ")
                    .or_else(|| after_mod.strip_prefix("override "))
                    .or_else(|| after_mod.strip_prefix("export "));
                match trimmed_mod {
                    Some(s) => after_mod = s.trim_start(),
                    None => break,
                }
            }
            // Prefer the original (un-stripped) text if it itself
            // parses as an assignment — this handles the case where a
            // modifier word (e.g. `private`) is actually the variable
            // name, as in `a: private = a`.
            let assign = try_parse_assignment(after).or_else(|| {
                if after_mod.is_empty() {
                    None
                } else {
                    try_parse_assignment(after_mod)
                }
            });
            if let Some(assign) = assign {
                let targets_str = trimmed[..colon].to_string();
                self.advance();
                return Ok(Some(Directive::TargetVarAssign(
                    targets_str,
                    Box::new(assign),
                )));
            }
        }

        // Try rule
        if let Some(rule) = self.try_parse_rule()? {
            return Ok(Some(Directive::Rule(rule)));
        }

        // Bare `$(...)` expression line — expand for side effects (errors,
        // info/warning, eval). Must be checked after rule parsing since a
        // line like `$(x): foo` is a rule, not a bare expression.
        if trimmed.starts_with("$(") || trimmed.starts_with("${") {
            let expr = trimmed.to_string();
            let line_no = self.current_line_no();
            let source = self.source_name.clone();
            self.advance();
            return Ok(Some(Directive::Expand(expr, source, line_no)));
        }

        Ok(None)
    }

    fn parse_conditional(&mut self) -> Result<Directive, String> {
        let line = self.advance().unwrap();
        let trimmed = line.trim();

        let kind = parse_cond_kind(trimmed)?;

        let then_body = self.parse_body(&["else", "endif"])?;

        let else_body = if self.peek().map(|l| l.trim().starts_with("else")) == Some(true) {
            let else_line = self.advance().unwrap();
            let else_trimmed = else_line.trim();
            // Check for else ifeq / else ifdef etc.
            let rest = else_trimmed.strip_prefix("else").unwrap().trim();
            if rest.starts_with("ifdef ")
                || rest.starts_with("ifndef ")
                || rest.starts_with("ifeq ")
                || rest.starts_with("ifeq(")
                || rest.starts_with("ifneq ")
                || rest.starts_with("ifneq(")
            {
                // Nested conditional in else branch — re-parse as a conditional
                let nested_kind = parse_cond_kind(rest)?;
                let nested_then = self.parse_body(&["else", "endif"])?;
                let nested_else = if self.peek().map(|l| l.trim().starts_with("else")) == Some(true)
                {
                    self.advance();
                    Some(self.parse_body(&["endif"])?)
                } else {
                    None
                };
                self.expect_line("endif")?;
                Some(vec![Directive::Conditional(Conditional {
                    kind: nested_kind,
                    then_body: nested_then,
                    else_body: nested_else,
                })])
            } else {
                let body = self.parse_body(&["endif"])?;
                self.expect_line("endif")?;
                Some(body)
            }
        } else {
            self.expect_line("endif")?;
            None
        };

        Ok(Directive::Conditional(Conditional {
            kind,
            then_body,
            else_body,
        }))
    }

    fn expect_line(&mut self, keyword: &str) -> Result<(), String> {
        match self.peek() {
            Some(line)
                if line.trim() == keyword || line.trim().starts_with(&format!("{keyword} ")) =>
            {
                self.advance();
                Ok(())
            }
            Some(line) => Err(format!("expected '{}', got '{}'", keyword, line.trim())),
            None => Err(format!("expected '{}', got EOF", keyword)),
        }
    }

    fn parse_define(&mut self) -> Result<Directive, String> {
        let line = self.advance().unwrap();
        let trimmed = line.trim();
        let rest = trimmed.strip_prefix("define ").unwrap().trim();

        // Check for assignment operator: `define VAR OP` (optionally
        // followed by a `#comment`). Strip any trailing comment first,
        // then identify the operator suffix.
        let rest = strip_makefile_comment(rest).trim();
        let (name, op) = if let Some(n) = rest.strip_suffix(" ::=") {
            (n.trim().to_string(), AssignOp::Simple)
        } else if let Some(n) = rest.strip_suffix(" :=") {
            (n.trim().to_string(), AssignOp::Simple)
        } else if let Some(n) = rest.strip_suffix(" +=") {
            (n.trim().to_string(), AssignOp::Append)
        } else if let Some(n) = rest.strip_suffix(" ?=") {
            (n.trim().to_string(), AssignOp::Conditional)
        } else if let Some(n) = rest.strip_suffix(" !=") {
            (n.trim().to_string(), AssignOp::Shell)
        } else if let Some(n) = rest.strip_suffix(" =") {
            (n.trim().to_string(), AssignOp::Recursive)
        } else {
            (rest.to_string(), AssignOp::Recursive)
        };

        let mut body = Vec::new();
        loop {
            match self.peek() {
                Some(line)
                    if {
                        let t = line.trim();
                        t == "endef" || t.starts_with("endef ") || t.starts_with("endef\t")
                    } =>
                {
                    self.advance();
                    break;
                }
                Some(_) => {
                    body.push(self.advance().unwrap());
                }
                None => return Err("unterminated define".to_string()),
            }
        }

        Ok(Directive::Define(name, op, body))
    }

    fn try_parse_rule(&mut self) -> Result<Option<Rule>, String> {
        let line = match self.peek() {
            Some(l) => l.to_string(),
            None => return Ok(None),
        };
        let trimmed = line.trim();

        // Look for colon (but not inside variable references)
        let colon_pos = find_rule_colon(trimmed);
        let colon_pos = match colon_pos {
            Some(p) => p,
            None => return Ok(None),
        };

        self.advance();

        let is_double_colon = trimmed[colon_pos..].starts_with("::");
        let targets_str = &trimmed[..colon_pos];
        let after_colon = if is_double_colon {
            &trimmed[colon_pos + 2..]
        } else {
            &trimmed[colon_pos + 1..]
        };

        // Strip trailing comment from the prerequisites section (a `#` not
        // escaped starts a comment that runs to the end of the logical line,
        // which may include an inline `;` recipe separator).
        let after_colon = strip_makefile_comment(after_colon);

        // Split after-colon on semicolon for inline recipe
        let (prereqs_str, inline_recipe) = if let Some(semi_pos) = after_colon.find(';') {
            (
                &after_colon[..semi_pos],
                Some(after_colon[semi_pos + 1..].trim().to_string()),
            )
        } else {
            (after_colon, None)
        };

        // Split prerequisites on | for order-only
        let (normal_prereqs, order_only) = if let Some(pipe_pos) = prereqs_str.find('|') {
            (&prereqs_str[..pipe_pos], &prereqs_str[pipe_pos + 1..])
        } else {
            (prereqs_str, "")
        };

        let targets: Vec<String> = targets_str
            .split_whitespace()
            .map(|s| s.to_string())
            .collect();
        let prerequisites: Vec<String> = normal_prereqs
            .split_whitespace()
            .map(|s| s.to_string())
            .collect();
        let order_only: Vec<String> = order_only
            .split_whitespace()
            .map(|s| s.to_string())
            .collect();

        // Detect pattern rules
        let pattern = if targets.iter().any(|t| t.contains('%')) {
            Some(PatternRule {
                target_pattern: targets[0].clone(),
                prereq_patterns: prerequisites.clone(),
            })
        } else {
            None
        };

        // Read recipe lines.
        // Lines ending with \ are joined with the next line (continuation).
        // The backslash-newline is preserved in the recipe text since the
        // shell handles continuation, not make.
        let mut recipe = Vec::new();
        let mut recipe_lines = Vec::new();
        let rule_line = self
            .line_nos
            .get(self.pos.saturating_sub(1))
            .copied()
            .unwrap_or(0);
        // An inline recipe marker (`target: ; ...`) means the rule has a
        // recipe even when the inline text is empty — that distinguishes
        // `all:` ("Nothing to be done") from `all: ;` ("is up to date").
        if let Some(inline) = inline_recipe {
            recipe.push(inline);
            recipe_lines.push(rule_line);
        }
        let rp = self.recipe_prefix;
        while let Some(line) = self.peek() {
            if let Some(stripped) = strip_recipe_prefix(line, rp) {
                let line_no = self.current_line_no();
                let mut combined = stripped.to_string();
                self.advance();
                // Join continuation lines: if line ends with \, append next line
                while combined.ends_with('\\') {
                    if let Some(next) = self.peek() {
                        if let Some(next_stripped) = strip_recipe_prefix(next, rp) {
                            combined.push('\n');
                            combined.push_str(next_stripped);
                            self.advance();
                        } else if next.trim_start().starts_with('#') {
                            // Skip comment lines between continuation lines
                            self.advance();
                        } else {
                            break;
                        }
                    } else {
                        break;
                    }
                }
                recipe.push(combined);
                recipe_lines.push(line_no);
            } else if line.is_empty() {
                // Empty lines within a recipe are allowed
                self.advance();
            } else if line.trim_start().starts_with('#') {
                // Comment lines within a recipe block are skipped
                // (e.g., automake's #\t commented-out recipe alternatives)
                self.advance();
            } else {
                break;
            }
        }

        Ok(Some(Rule {
            targets,
            pattern,
            prerequisites,
            order_only,
            recipe,
            recipe_lines,
            source_name: self.source_name.clone(),
            is_double_colon,
        }))
    }
}

/// Try to parse an assignment from a line.
pub fn try_parse_assignment(line: &str) -> Option<Assignment> {
    let line = line.trim();

    // Try each operator (longest first to avoid partial matches)
    for (suffix, op) in [
        ("::=", AssignOp::Simple),
        (":=", AssignOp::Simple),
        ("?=", AssignOp::Conditional),
        ("+=", AssignOp::Append),
        ("!=", AssignOp::Shell),
        ("=", AssignOp::Recursive),
    ] {
        if let Some(eq_pos) = find_assignment_op(line, suffix) {
            let name = line[..eq_pos].trim().to_string();
            let value = line[eq_pos + suffix.len()..].trim().to_string();
            if is_valid_varname(&name) {
                return Some(Assignment { name, op, value });
            }
        }
    }
    None
}

fn find_assignment_op(line: &str, op: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    let op_bytes = op.as_bytes();
    let mut depth = 0u32;

    for i in 0..bytes.len() {
        if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'(' {
            depth += 1;
        } else if bytes[i] == b')' && depth > 0 {
            depth -= 1;
        } else if depth == 0
            && i + op_bytes.len() <= bytes.len()
            && &bytes[i..i + op_bytes.len()] == op_bytes
        {
            // For '=', make sure we're not matching :=, +=, ?=, !=
            if op == "=" && i > 0 {
                let prev = bytes[i - 1];
                if matches!(prev, b':' | b'+' | b'?' | b'!') {
                    continue;
                }
            }
            return Some(i);
        }
    }
    None
}

fn is_valid_varname(name: &str) -> bool {
    !name.is_empty()
        && !name.contains(' ')
        && !name.contains('\t')
        && !name.contains(':')
        && !name.contains('#')
}

fn find_rule_colon(line: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    let mut depth = 0u32;

    for i in 0..bytes.len() {
        if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'(' {
            depth += 1;
        } else if bytes[i] == b')' && depth > 0 {
            depth -= 1;
        } else if depth == 0 && bytes[i] == b':' {
            // Make sure it's not := or ::= assignment
            if i + 1 < bytes.len() && bytes[i + 1] == b'=' {
                return None; // It's :=
            }
            if i + 2 < bytes.len() && bytes[i + 1] == b':' && bytes[i + 2] == b'=' {
                return None; // It's ::=
            }
            // Check it's not preceded by assignment-like content
            // Simple heuristic: if there's an '=' anywhere after, and no tab on the next line,
            // it might be an assignment. But for now, trust the colon.
            return Some(i);
        }
    }
    None
}

fn parse_cond_kind(trimmed: &str) -> Result<CondKind, String> {
    if let Some(rest) = trimmed.strip_prefix("ifdef ") {
        Ok(CondKind::Ifdef(rest.trim().to_string()))
    } else if let Some(rest) = trimmed.strip_prefix("ifndef ") {
        Ok(CondKind::Ifndef(rest.trim().to_string()))
    } else if let Some(rest) = trimmed.strip_prefix("ifeq") {
        let (a, b) = parse_cond_args(rest)?;
        Ok(CondKind::Ifeq(a, b))
    } else if let Some(rest) = trimmed.strip_prefix("ifneq") {
        let (a, b) = parse_cond_args(rest)?;
        Ok(CondKind::Ifneq(a, b))
    } else {
        Err(format!("unknown conditional: {trimmed}"))
    }
}

fn parse_cond_args(s: &str) -> Result<(String, String), String> {
    let s = s.trim();
    if s.starts_with('(') && s.ends_with(')') {
        let inner = &s[1..s.len() - 1];
        if let Some(comma) = inner.find(',') {
            let a = inner[..comma].trim();
            let b = inner[comma + 1..].trim();
            return Ok((strip_quotes(a), strip_quotes(b)));
        }
    }
    // Try quoted form: ifeq "a" "b" or ifeq 'a' 'b'
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() == 2 {
        return Ok((strip_quotes(parts[0]), strip_quotes(parts[1])));
    }
    Err(format!("cannot parse conditional args: {s}"))
}

fn strip_quotes(s: &str) -> String {
    let s = s.trim();
    if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}
