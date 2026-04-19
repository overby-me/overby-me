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
    let mut paren_depth: i32 = 0;
    let mut brace_depth: i32 = 0;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        // Track `$(...)` / `${...}` nesting so `#` inside a function
        // call (e.g. `$(shell echo '#')`) is treated as literal.
        if c == b'$' && i + 1 < bytes.len() {
            match bytes[i + 1] {
                b'(' => {
                    paren_depth += 1;
                    i += 2;
                    continue;
                }
                b'{' => {
                    brace_depth += 1;
                    i += 2;
                    continue;
                }
                b'$' => {
                    i += 2;
                    continue;
                }
                _ => {}
            }
        }
        if c == b'(' && paren_depth > 0 {
            paren_depth += 1;
        } else if c == b')' && paren_depth > 0 {
            paren_depth -= 1;
        } else if c == b'{' && brace_depth > 0 {
            brace_depth += 1;
        } else if c == b'}' && brace_depth > 0 {
            brace_depth -= 1;
        } else if c == b'#'
            && paren_depth == 0
            && brace_depth == 0
            && (i == 0 || bytes[i - 1] != b'\\')
        {
            return &s[..i];
        }
        i += 1;
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

/// Check if a line appears to have an inline recipe (`:` followed by `;`
/// at the top level, not inside `$(...)` or `${...}`).
fn has_inline_recipe(line: &str) -> bool {
    let mut depth = 0usize;
    let mut found_colon = false;
    let mut prev = '\0';
    for c in line.chars() {
        match c {
            '(' | '{' if prev == '$' => depth += 1,
            ')' | '}' if depth > 0 => depth -= 1,
            ':' if depth == 0 && !found_colon => {
                found_colon = true;
            }
            '=' if depth == 0 => return false,
            ';' if depth == 0 && found_colon => return true,
            _ => {}
        }
        prev = c;
    }
    false
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
        let mut recipe_prefix = '\t';
        let mut posix_mode = false;
        for (i, line) in input.lines().enumerate() {
            if current.is_empty() {
                start_line = i + 1;
                logical_is_recipe = line.starts_with(recipe_prefix) || has_inline_recipe(line);
            }
            // After a backslash-newline, leading whitespace on the
            // next physical line is collapsed — except inside a
            // recipe (logical line that started with the recipe
            // prefix), where whitespace is preserved because the
            // shell interprets continuations.
            let effective = if in_continuation && !logical_is_recipe {
                line.trim_start()
            } else if in_continuation && logical_is_recipe {
                // Recipe continuation physical line: strip one leading
                // recipe-prefix (tab) to match GNU make's trace output,
                // where only the first physical line of a recipe is
                // tab-indented.
                line.strip_prefix(recipe_prefix).unwrap_or(line)
            } else {
                line
            };
            if let Some(stripped) = effective.strip_suffix('\\') {
                if logical_is_recipe {
                    // Inside a recipe: preserve `\<newline>` so the
                    // shell sees the continuation and concatenates the
                    // physical lines itself.
                    current.push_str(effective);
                    current.push('\n');
                } else {
                    // GNU make collapses `<ws>*\<newline><ws>*` into a
                    // single space in non-recipe context. Trim trailing
                    // whitespace from the portion before `\` (the next
                    // line's leading whitespace is stripped above).
                    let before_bs = if posix_mode {
                        stripped
                    } else {
                        stripped.trim_end()
                    };
                    current.push_str(before_bs);
                    // In POSIX mode, each backslash-newline contributes
                    // exactly one space (they are not collapsed). In non-POSIX
                    // mode, consecutive backslash-newlines collapse to a single space.
                    if posix_mode || !current.ends_with(' ') {
                        current.push(' ');
                    }
                }
                in_continuation = true;
            } else {
                current.push_str(effective);
                lines.push(std::mem::take(&mut current));
                line_nos.push(start_line);
                in_continuation = false;
                // Track .RECIPEPREFIX changes so subsequent recipe
                // lines use the correct prefix character.
                let pushed = lines.last().unwrap();
                let trimmed_pushed = pushed.trim();
                if let Some(rest) = trimmed_pushed.strip_prefix(".RECIPEPREFIX") {
                    let rest = rest.trim_start();
                    if let Some(val) = rest
                        .strip_prefix("=")
                        .or_else(|| rest.strip_prefix(":="))
                        .or_else(|| rest.strip_prefix("::="))
                    {
                        let val = val.trim();
                        recipe_prefix = val.chars().next().unwrap_or('\t');
                    }
                }
                if trimmed_pushed == ".POSIX:" {
                    posix_mode = true;
                }
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

            // Recipe-prefix lines (normally tab). At top level these
            // are stray; inside a conditional body they belong to the
            // rule that preceded the conditional, so emit a RecipeLine
            // directive. Collapse `\<nl>` continuations like the rule
            // recipe reader does.
            let rp = self.recipe_prefix;
            if let Some(stripped) = strip_recipe_prefix(line, rp) {
                let line_no = self.current_line_no();
                let mut combined = stripped.to_string();
                self.advance();
                while combined.ends_with('\\') {
                    if let Some(next) = self.peek() {
                        if let Some(next_stripped) = strip_recipe_prefix(next, rp) {
                            combined.push('\n');
                            combined.push_str(next_stripped);
                            self.advance();
                        } else if next.trim_start().starts_with('#') {
                            self.advance();
                        } else {
                            break;
                        }
                    } else {
                        break;
                    }
                }
                directives.push(Directive::RecipeLine(combined, line_no));
                continue;
            }

            if let Some(dir) = self.try_parse_directive()? {
                directives.push(dir);
            } else {
                // Line doesn't match any directive, assignment, or rule.
                // GNU make reports "missing separator" for such lines.
                let line_no = self.current_line_no();
                let line = self.peek().unwrap_or("");
                let trimmed_line = line.trim();
                let source = &self.source_name;
                if trimmed_line.starts_with("ifeq(") || trimmed_line.starts_with("ifneq(") {
                    eprintln!(
                        "{source}:{line_no}: *** missing separator (ifeq/ifneq must be followed by whitespace).  Stop."
                    );
                } else {
                    let hint = if self.recipe_prefix == '\t' && line.starts_with("        ") {
                        " (did you mean TAB instead of 8 spaces?)"
                    } else {
                        ""
                    };
                    eprintln!("{source}:{line_no}: *** missing separator{hint}.  Stop.");
                }
                std::process::exit(2);
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
            || trimmed.starts_with("ifneq ")
            || trimmed == "ifeq"
            || trimmed == "ifneq"
        {
            return self.parse_conditional().map(Some);
        }

        // Include
        if trimmed == "include" || trimmed.starts_with("include ") {
            let line_no = self.current_line_no();
            let source = self.source_name.clone();
            self.advance();
            let rest = trimmed.strip_prefix("include").unwrap().trim_start();
            // Strip comments (# and everything after)
            let rest = strip_makefile_comment(rest);
            let files: Vec<String> = rest.split_whitespace().map(|s| s.to_string()).collect();
            return Ok(Some(Directive::Include(files, false, source, line_no)));
        }
        if trimmed == "-include"
            || trimmed == "sinclude"
            || trimmed.starts_with("-include ")
            || trimmed.starts_with("sinclude ")
        {
            let line_no = self.current_line_no();
            let source = self.source_name.clone();
            self.advance();
            let rest = trimmed
                .strip_prefix("-include")
                .or_else(|| trimmed.strip_prefix("sinclude"))
                .unwrap()
                .trim_start();
            let rest = strip_makefile_comment(rest);
            let files: Vec<String> = rest.split_whitespace().map(|s| s.to_string()).collect();
            return Ok(Some(Directive::Include(files, true, source, line_no)));
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
            if let Some(assign) = try_parse_assignment(rest) {
                return Ok(Some(Directive::UnexportAssign(Box::new(assign))));
            }
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
                let line_no = self.current_line_no();
                let source = self.source_name.clone();
                self.advance();
                return Ok(Some(Directive::OverrideUndefine(
                    var.trim().to_string(),
                    source,
                    line_no,
                )));
            }
            // `override define NAME …` — multi-line variable override.
            if rest.starts_with("define ") || rest == "define" {
                // Rewrite the current line to drop the `override ` prefix so
                // `parse_define` can consume it normally, then wrap the
                // resulting Define in override semantics.
                let rest_owned = rest.to_string();
                self.lines[self.pos] = rest_owned;
                let directive = self.parse_define()?;
                if let Directive::Define(name, op, body, src, line) = directive {
                    return Ok(Some(Directive::OverrideDefine(name, op, body, src, line)));
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
            let line_no = self.current_line_no();
            let source = self.source_name.clone();
            self.advance();
            return Ok(Some(Directive::Undefine(
                rest.trim().to_string(),
                source,
                line_no,
            )));
        }

        // Define (multi-line variable). Skip when the line also parses
        // as an assignment — `define = value` is a variable named
        // `define`, not a define directive.
        if trimmed.starts_with("define ") && try_parse_assignment(trimmed).is_none() {
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

        // Global `private` variable directive:
        // `private VAR = val` or `private export VAR = val`
        if let Some(rest) = trimmed.strip_prefix("private ") {
            let rest = rest.trim_start();
            // `private export VAR = val`
            if let Some(export_rest) = rest.strip_prefix("export ") {
                let export_rest = export_rest.trim_start();
                if let Some(assign) = try_parse_assignment(export_rest) {
                    self.advance();
                    return Ok(Some(Directive::PrivateExportAssign(Box::new(assign))));
                }
            }
            // `private VAR = val`
            if let Some(assign) = try_parse_assignment(rest) {
                let line_no = self.current_line_no();
                let source = self.source_name.clone();
                self.advance();
                return Ok(Some(Directive::PrivateAssign(
                    Box::new(assign),
                    source,
                    line_no,
                )));
            }
        }

        // Try assignment. Strip comments first (GNU make: `VAR = val #comment`
        // stores just "val"), then pass through — keeping any trailing
        // whitespace preceding the `#`.
        let line_no_comment = strip_makefile_comment(&line);
        if let Some(assign) = try_parse_assignment(line_no_comment) {
            let assign_line_no = self.current_line_no();
            let assign_source = self.source_name.clone();
            self.advance();
            // Track `.RECIPEPREFIX := X` so subsequent rules use X as the
            // recipe-line marker. Empty resets to default tab.
            if assign.name.trim() == ".RECIPEPREFIX" {
                let val = assign.value.trim();
                self.recipe_prefix = val.chars().next().unwrap_or('\t');
            }
            return Ok(Some(Directive::Assignment(
                assign,
                assign_source,
                assign_line_no,
            )));
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
            // the assignment. When `export` is present AND the
            // assignment parses from the stripped text, mark the var
            // name in the target string with a leading `!` sentinel
            // so the engine adds it to the export set when the target
            // is built. (An ordinary variable name cannot begin with
            // `!`, so this is unambiguous.)
            let mut after_mod = after;
            let mut had_export = false;
            let mut had_unexport = false;
            let mut had_override = false;
            let mut had_private = false;
            loop {
                if let Some(rest) = after_mod.strip_prefix("export ") {
                    had_export = true;
                    after_mod = rest.trim_start();
                    continue;
                }
                if let Some(rest) = after_mod.strip_prefix("unexport ") {
                    had_unexport = true;
                    after_mod = rest.trim_start();
                    continue;
                }
                if let Some(rest) = after_mod.strip_prefix("override ") {
                    had_override = true;
                    after_mod = rest.trim_start();
                    continue;
                }
                let trimmed_mod = after_mod.strip_prefix("private ");
                match trimmed_mod {
                    Some(s) => {
                        had_private = true;
                        after_mod = s.trim_start();
                    }
                    None => break,
                }
            }
            let (assign, from_stripped) = match try_parse_assignment(after) {
                Some(a) => (Some(a), false),
                None if after_mod.is_empty() => (None, false),
                None => (try_parse_assignment(after_mod), true),
            };
            if let Some(mut assign) = assign {
                let targets_str = trimmed[..colon].to_string();
                self.advance();
                let mut prefix = String::new();
                if had_override && from_stripped {
                    prefix.push('^');
                }
                if had_export && from_stripped {
                    prefix.push('!');
                } else if had_unexport && from_stripped {
                    prefix.push('~');
                }
                if had_private && from_stripped {
                    prefix.push('@');
                }
                if !prefix.is_empty() {
                    assign.name = format!("{}{}", prefix, assign.name);
                }
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
        let cond_line_no = self.current_line_no();
        let line = self.advance().unwrap();
        let trimmed = line.trim();

        let kind = match parse_cond_kind(trimmed) {
            Ok(k) => k,
            Err(_) => {
                let source = &self.source_name;
                eprintln!("{source}:{cond_line_no}: *** invalid syntax in conditional.  Stop.");
                std::process::exit(2);
            }
        };

        let then_body = self.parse_body(&["else", "endif"])?;

        let else_body = if self.peek().map(|l| l.trim().starts_with("else")) == Some(true) {
            let else_line = self.advance().unwrap();
            let else_trimmed = else_line.trim();
            let rest = else_trimmed.strip_prefix("else").unwrap().trim();
            if rest.starts_with("ifdef ")
                || rest.starts_with("ifndef ")
                || rest.starts_with("ifeq ")
                || rest.starts_with("ifneq ")
                || rest == "ifeq"
                || rest == "ifneq"
            {
                // `else ifX …` chains to another conditional. Re-feed
                // the line (sans the `else ` prefix) into
                // `parse_conditional` which handles the remainder
                // including any further `else if…` / `else` branches
                // and the terminating `endif`.
                let reparsed = rest.to_string();
                // Replace the consumed line with the rewritten form so
                // parse_conditional sees it via peek/advance.
                self.lines.insert(self.pos, reparsed);
                self.line_nos.insert(
                    self.pos,
                    self.line_nos
                        .get(self.pos.saturating_sub(1))
                        .copied()
                        .unwrap_or(0),
                );
                let nested = self.parse_conditional()?;
                Some(vec![nested])
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
        let define_line_no = self.current_line_no();
        let source = self.source_name.clone();
        let line = self.advance().unwrap();
        let trimmed = line.trim();
        let rest = trimmed.strip_prefix("define ").unwrap().trim();

        // Strip trailing comment, then detect the assignment operator.
        // Use `find_assignment_op` directly (instead of
        // `try_parse_assignment`) because variable names in `define`
        // directives may contain spaces inside `$(...)` function
        // calls that `is_valid_varname` would reject.
        let rest = strip_makefile_comment(rest).trim();
        let (name, op) = {
            let mut found = None;
            for (suffix, assign_op) in [
                (":::=", AssignOp::ImmediateRecursive),
                ("::=", AssignOp::Simple),
                (":=", AssignOp::Simple),
                ("?=", AssignOp::Conditional),
                ("+=", AssignOp::Append),
                ("!=", AssignOp::Shell),
                ("=", AssignOp::Recursive),
            ] {
                if let Some(pos) = find_assignment_op(rest, suffix) {
                    let n = rest[..pos].trim().to_string();
                    let extra = rest[pos + suffix.len()..].trim();
                    if !extra.is_empty() {
                        eprintln!(
                            "{}:{}: extraneous text after 'define' directive",
                            source, define_line_no
                        );
                    }
                    found = Some((n, assign_op));
                    break;
                }
            }
            found.unwrap_or_else(|| (rest.to_string(), AssignOp::Recursive))
        };

        // Collect body lines, tracking nested define/endef pairs so
        // that inner `endef` tokens don't prematurely close the outer
        // define block.
        let mut depth: usize = 0;
        let mut body = Vec::new();
        loop {
            match self.peek() {
                Some(line) => {
                    let t = line.trim();
                    let is_endef =
                        t == "endef" || t.starts_with("endef ") || t.starts_with("endef\t");
                    let is_define = {
                        let d = t
                            .strip_prefix("override ")
                            .or_else(|| t.strip_prefix("override\t"))
                            .map(|r| r.trim_start())
                            .unwrap_or(t);
                        d == "define" || d.starts_with("define ") || d.starts_with("define\t")
                    };

                    if is_endef {
                        if depth == 0 {
                            // Warn about extraneous text after `endef`.
                            // Strip comments — `endef # comment` is fine.
                            let after =
                                strip_makefile_comment(t.strip_prefix("endef").unwrap()).trim();
                            if !after.is_empty() {
                                let endef_line = self.current_line_no();
                                eprintln!(
                                    "{}:{}: extraneous text after 'endef' directive",
                                    source, endef_line
                                );
                            }
                            self.advance();
                            break;
                        } else {
                            depth -= 1;
                            body.push(self.advance().unwrap());
                        }
                    } else if is_define {
                        depth += 1;
                        body.push(self.advance().unwrap());
                    } else {
                        body.push(self.advance().unwrap());
                    }
                }
                None => {
                    eprintln!(
                        "{}:{}: *** missing 'endef', unterminated 'define'.  Stop.",
                        source, define_line_no
                    );
                    std::process::exit(2);
                }
            }
        }

        Ok(Directive::Define(name, op, body, source, define_line_no))
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
        let mut targets_str = &trimmed[..colon_pos];
        // Grouped-target rule: `targets &:` (or `&: ` ungrouped). GNU
        // make treats all the listed targets as a single group — one
        // recipe invocation updates them all.
        let is_grouped = targets_str.ends_with('&') || targets_str.trim_end().ends_with('&');
        if is_grouped {
            let stripped = targets_str.trim_end();
            let stripped = stripped.strip_suffix('&').unwrap_or(stripped);
            targets_str = stripped;
        }
        let after_colon = if is_double_colon {
            &trimmed[colon_pos + 2..]
        } else {
            &trimmed[colon_pos + 1..]
        };

        // Find the first unescaped, top-level (outside `$(...)`) `;` or
        // `#`. A `;` first separates an inline recipe; a `#` first starts
        // a trailing comment on the prerequisites. The inline recipe text
        // itself is opaque to comment stripping (recipes may contain `#`).
        let (prereqs_str, inline_recipe) = {
            let bytes = after_colon.as_bytes();
            let mut paren = 0i32;
            let mut brace = 0i32;
            let mut i = 0usize;
            let mut split: Option<(usize, bool)> = None; // (pos, is_semi)
            while i < bytes.len() {
                let c = bytes[i];
                if c == b'$' && i + 1 < bytes.len() {
                    match bytes[i + 1] {
                        b'(' => {
                            paren += 1;
                            i += 2;
                            continue;
                        }
                        b'{' => {
                            brace += 1;
                            i += 2;
                            continue;
                        }
                        b'$' => {
                            // `$$` is two literal characters at this stage;
                            // GNU make does not treat `$$(` as opening a
                            // deferred function call when scanning for the
                            // inline-recipe `;` separator.
                            i += 2;
                            continue;
                        }
                        _ => {}
                    }
                }
                if c == b'(' && paren > 0 {
                    paren += 1;
                } else if c == b')' && paren > 0 {
                    paren -= 1;
                } else if c == b'{' && brace > 0 {
                    brace += 1;
                } else if c == b'}' && brace > 0 {
                    brace -= 1;
                } else if paren == 0 && brace == 0 {
                    if c == b';' {
                        split = Some((i, true));
                        break;
                    }
                    if c == b'#' && (i == 0 || bytes[i - 1] != b'\\') {
                        split = Some((i, false));
                        break;
                    }
                }
                i += 1;
            }
            match split {
                Some((pos, true)) => (
                    &after_colon[..pos],
                    Some(after_colon[pos + 1..].trim().to_string()),
                ),
                Some((pos, false)) => (&after_colon[..pos], None),
                None => (after_colon, None),
            }
        };

        // Detect static pattern rule: `targets : target-pattern : prereq-patterns`
        // — a second top-level colon (outside `$(...)`) in the prereqs
        // section means the middle field is the target pattern and
        // everything after the second colon is the prereq pattern list.
        let (static_pat, prereqs_str) = if let Some(second_colon) = find_rule_colon(prereqs_str) {
            let pat = prereqs_str[..second_colon].trim().to_string();
            let rest = &prereqs_str[second_colon + 1..];
            (Some(pat), rest)
        } else {
            (None, prereqs_str)
        };

        // Split prerequisites on | for order-only.
        // Skip `|` chars that are preceded by `$$` (literal `$|`
        // auto-var reference for second expansion) or that are
        // inside `$(...)` / `${...}` references.
        let find_pipe = |s: &str| -> Option<usize> {
            let bytes = s.as_bytes();
            let mut i = 0;
            let mut paren_depth: i32 = 0;
            let mut brace_depth: i32 = 0;
            while i < bytes.len() {
                let c = bytes[i];
                if c == b'$' && i + 1 < bytes.len() {
                    let nxt = bytes[i + 1];
                    if nxt == b'$' {
                        // `$$` at parse time becomes `$` after first
                        // expansion. If followed by `(` or `{`, treat
                        // as opening a (deferred) function call so an
                        // inner `|` is not mistaken for the order-only
                        // separator.
                        if i + 2 < bytes.len() && bytes[i + 2] == b'(' {
                            paren_depth += 1;
                            i += 3;
                            continue;
                        }
                        if i + 2 < bytes.len() && bytes[i + 2] == b'{' {
                            brace_depth += 1;
                            i += 3;
                            continue;
                        }
                        i += 2;
                        continue;
                    }
                    if nxt == b'(' {
                        paren_depth += 1;
                        i += 2;
                        continue;
                    }
                    if nxt == b'{' {
                        brace_depth += 1;
                        i += 2;
                        continue;
                    }
                    i += 2;
                    continue;
                }
                if c == b'(' && paren_depth > 0 {
                    paren_depth += 1;
                } else if c == b')' && paren_depth > 0 {
                    paren_depth -= 1;
                } else if c == b'{' && brace_depth > 0 {
                    brace_depth += 1;
                } else if c == b'}' && brace_depth > 0 {
                    brace_depth -= 1;
                } else if c == b'|' && paren_depth == 0 && brace_depth == 0 {
                    // Skip `|` immediately after `$` (i.e. `$|` or `$$|`)
                    // — it's an auto-var reference, not the order-only sep.
                    if i > 0 && bytes[i - 1] == b'$' {
                        i += 1;
                        continue;
                    }
                    return Some(i);
                }
                i += 1;
            }
            None
        };
        let (normal_prereqs, order_only) = if let Some(pipe_pos) = find_pipe(prereqs_str) {
            (&prereqs_str[..pipe_pos], &prereqs_str[pipe_pos + 1..])
        } else {
            (prereqs_str, "")
        };

        // GNU make strips backslash-escapes from `:` and `#` in
        // target/prerequisite names. A run of N backslashes before `:`
        // or `#` produces N/2 literal backslashes followed by the
        // character (literal if N is odd, separator-eligible if even —
        // separator-handling already happened during tokenization).
        fn unescape_one(t: &str) -> String {
            let bytes = t.as_bytes();
            let mut out = String::with_capacity(t.len());
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i] == b'\\' {
                    let mut j = i;
                    while j < bytes.len() && bytes[j] == b'\\' {
                        j += 1;
                    }
                    let n = j - i;
                    if j < bytes.len() && (bytes[j] == b':' || bytes[j] == b'#') {
                        for _ in 0..(n / 2) {
                            out.push('\\');
                        }
                        if n % 2 == 1 {
                            out.push(bytes[j] as char);
                            i = j + 1;
                        } else {
                            i = j;
                        }
                        continue;
                    } else {
                        for _ in 0..n {
                            out.push('\\');
                        }
                        i = j;
                        continue;
                    }
                }
                out.push(bytes[i] as char);
                i += 1;
            }
            out
        }
        fn unescape_target(toks: Vec<String>) -> Vec<String> {
            toks.into_iter().map(|t| unescape_one(&t)).collect()
        }
        // NOTE: do NOT unescape `\\:` / `\\#` here — the engine does
        // the unescape after variable expansion, so doing it twice would
        // halve the backslash count.
        let targets: Vec<String> = split_whitespace_respecting_refs(targets_str);
        let prerequisites: Vec<String> = split_whitespace_respecting_refs(normal_prereqs);
        let order_only: Vec<String> = split_whitespace_respecting_refs(order_only);
        let _ = unescape_target;

        // Detect pattern rules. A static pattern rule uses the middle
        // field (`target-pattern`) against the explicit target list,
        // while a conventional pattern rule has `%` in the target.
        let pattern = if let Some(pat) = static_pat {
            Some(PatternRule {
                target_pattern: pat,
                prereq_patterns: prerequisites.clone(),
            })
        } else if targets.iter().any(|t| t.contains('%')) {
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
            is_grouped,
            line_no: rule_line,
        }))
    }
}

/// Unescape `\\#` to `#` in a value string, but only outside of
/// `$(...)` / `${...}` references. GNU make removes the comment-escape
/// backslash from literal text but leaves it intact inside function
/// call arguments.
fn unescape_hash(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut paren = 0i32;
    let mut brace = 0i32;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'$' && i + 1 < bytes.len() {
            match bytes[i + 1] {
                b'(' => {
                    paren += 1;
                    out.push('$');
                    out.push('(');
                    i += 2;
                    continue;
                }
                b'{' => {
                    brace += 1;
                    out.push('$');
                    out.push('{');
                    i += 2;
                    continue;
                }
                _ => {}
            }
        }
        if paren > 0 || brace > 0 {
            if c == b'(' {
                paren += 1;
            } else if c == b')' && paren > 0 {
                paren -= 1;
            } else if c == b'{' {
                brace += 1;
            } else if c == b'}' && brace > 0 {
                brace -= 1;
            }
            out.push(c as char);
            i += 1;
            continue;
        }
        if c == b'\\' {
            let mut j = i;
            while j < bytes.len() && bytes[j] == b'\\' {
                j += 1;
            }
            let n = j - i;
            if j < bytes.len() && bytes[j] == b'#' {
                for _ in 0..(n - 1) {
                    out.push('\\');
                }
                out.push('#');
                i = j + 1;
                continue;
            } else {
                for _ in 0..n {
                    out.push('\\');
                }
                i = j;
                continue;
            }
        }
        out.push(c as char);
        i += 1;
    }
    out
}

pub fn try_parse_assignment(line: &str) -> Option<Assignment> {
    // Preserve trailing whitespace on the value (only strip leading).
    let line = line.trim_start();

    // Try each operator (longest first to avoid partial matches)
    for (suffix, op) in [
        (":::=", AssignOp::ImmediateRecursive),
        ("::=", AssignOp::Simple),
        (":=", AssignOp::Simple),
        ("?=", AssignOp::Conditional),
        ("+=", AssignOp::Append),
        ("!=", AssignOp::Shell),
        ("=", AssignOp::Recursive),
    ] {
        if let Some(eq_pos) = find_assignment_op(line, suffix) {
            let name = line[..eq_pos].trim().to_string();
            // GNU make: strip leading whitespace from the value, but
            // preserve trailing whitespace — `VAR := foo ` stores "foo ".
            let raw_value = &line[eq_pos + suffix.len()..];
            let trimmed = raw_value.trim_start();
            // Unescape `\#` to `#` in assignment values. The leading
            // backslash escapes the comment marker for parser purposes
            // and is removed in the stored value (GNU make semantics).
            let value = unescape_hash(trimmed);
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
        } else if depth == 0 && bytes[i] == b':' && (i == 0 || bytes[i - 1] != b'\\') {
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
        // Find the comma at paren-depth 0 so nested $(filter a,b)
        // calls don't split prematurely.
        let mut depth = 0i32;
        let mut comma_pos = None;
        for (i, ch) in inner.char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => depth -= 1,
                ',' if depth == 0 => {
                    comma_pos = Some(i);
                    break;
                }
                _ => {}
            }
        }
        if let Some(comma) = comma_pos {
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

/// Splits a string on whitespace, but keeps `$(...)` and `${...}` groups intact.
/// This is needed so that targets like `$(filter %.o,$(files))` are not broken apart.
/// `\\<space>` is treated as a literal space character (not a token boundary)
/// so target names with embedded spaces (`foo\\ bar:`) parse as one token.
pub fn split_whitespace_respecting_refs(s: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut depth = 0u32;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '$' {
            if let Some(&next) = chars.peek()
                && (next == '(' || next == '{')
            {
                depth += 1;
                current.push(c);
                current.push(chars.next().unwrap());
                continue;
            }
            // `$$(...)` / `${{...}}`: after first-pass expansion this
            // becomes `$(...)` etc. Treat the deferred reference as a
            // nested group so embedded whitespace doesn't split it.
            if let Some(&next) = chars.peek()
                && next == '$'
            {
                current.push(c);
                current.push(chars.next().unwrap());
                if let Some(&n2) = chars.peek()
                    && (n2 == '(' || n2 == '{')
                {
                    depth += 1;
                    current.push(chars.next().unwrap());
                }
                continue;
            }
            current.push(c);
        } else if depth > 0 {
            if c == '(' || c == '{' {
                depth += 1;
            } else if c == ')' || c == '}' {
                depth = depth.saturating_sub(1);
            }
            current.push(c);
        } else if c == '\\' {
            if let Some(&next) = chars.peek()
                && (next == ' ' || next == '\t')
            {
                // Escaped whitespace: consume the backslash, keep the
                // next char literally as part of the current token.
                current.push(chars.next().unwrap());
                continue;
            }
            current.push(c);
        } else if c.is_whitespace() {
            if !current.is_empty() {
                result.push(std::mem::take(&mut current));
            }
        } else {
            current.push(c);
        }
    }
    if !current.is_empty() {
        result.push(current);
    }
    result
}
