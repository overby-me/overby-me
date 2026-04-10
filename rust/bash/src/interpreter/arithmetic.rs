use super::*;

/// Represents a resolved array subscript — either an associative string key
/// or an indexed numeric position.
enum ArithSubscript {
    Assoc(String, String),
    Indexed(String, usize),
}

impl Shell {
    // ── Associative-array-aware helpers for arithmetic evaluation ──────

    /// Check if `name` is declared as an associative array.
    fn is_assoc_array(&self, name: &str) -> bool {
        self.assoc_arrays.contains_key(name)
    }

    /// Expand the subscript for arithmetic array access.
    /// For associative arrays the subscript is used as a string key
    /// (with variable expansion but no arithmetic evaluation).
    /// For indexed arrays the subscript is evaluated as an arithmetic
    /// expression and converted to `usize`.
    fn arith_subscript_key(&mut self, base: &str, idx_str: &str) -> ArithSubscript {
        let resolved = self.resolve_nameref(base);
        if self.is_assoc_array(&resolved) {
            // Expand $var references in the key but don't evaluate as arith
            let key = self.expand_arith_subscript_key(idx_str);
            ArithSubscript::Assoc(resolved, key)
        } else {
            let saved_in_subscript = self.arith_in_subscript;
            self.arith_in_subscript = true;
            let idx = self.eval_arith_expr_impl(idx_str) as usize;
            self.arith_in_subscript = saved_in_subscript;
            ArithSubscript::Indexed(resolved, idx)
        }
    }

    /// Check whether an arithmetic subscript is empty after quote removal.
    /// Returns true if the subscript is empty (e.g. `a[""]=N` or `a[]=N`),
    /// which bash rejects as "not a valid identifier".
    /// In `let` context, empty subscripts (from `""` quote stripping) evaluate
    /// to 0 rather than erroring, because `let "a[\"\"]"=22` is valid bash.
    fn is_empty_arith_subscript(idx_str: &str) -> bool {
        // Only reject subscripts that are literally empty (no characters)
        // or consist solely of a pair of quotes with nothing inside.
        // Whitespace-only subscripts like `a[" "]` are valid — the space
        // evaluates to 0 in arithmetic, so `a[ ]=N` sets a[0].
        if idx_str.is_empty() {
            return true;
        }
        // Handle "" (just a pair of double quotes)
        if idx_str.trim() == "\"\"" {
            return true;
        }
        // Handle '' (just a pair of single quotes)
        if idx_str.trim() == "''" {
            return true;
        }
        false
    }

    /// Check if the subscript was `""` or `''` BEFORE quote stripping occurred.
    /// Used in `let` context where `let "a[\"\"]"=22` is valid (assigns to a[0])
    /// — but only when `assoc_expand_once` is **unset**.  When that shopt is on,
    /// bash still rejects the empty subscript even from `let`.
    /// Check whether the pre-strip subscript was a quoted-empty form.
    /// Returns (bare_quoted, escaped_quoted):
    ///   bare_quoted: `""` or `''` — valid in `let` context, invalid in `(( ))`
    ///   escaped_quoted: `\"\"` or `\" \"` — valid in both `let` and `(( ))` contexts
    fn had_quoted_empty_subscript(pre_strip_expr: &str, bracket_pos: usize) -> (bool, bool) {
        // Find the `[` in the pre-stripped expression and extract the subscript
        if let Some(close) = pre_strip_expr[bracket_pos + 1..].find(']') {
            let sub = &pre_strip_expr[bracket_pos + 1..bracket_pos + 1 + close];
            let trimmed = sub.trim();
            // Exact `""` or `''` pair (bare quotes)
            if trimmed == "\"\"" || trimmed == "''" {
                return (true, false);
            }
            // Backslash-escaped quote pairs: `\"\"` (two escaped quotes)
            // After strip_arith_quotes these become empty, but they should
            // evaluate to 0 for indexed arrays, not error.
            if trimmed == "\\\"\\\"" {
                return (false, true);
            }
            // Also handle `\" \"` (escaped quote, space, escaped quote) —
            // the space between quotes is significant but the result after
            // stripping is just a space which evaluates to 0.
            if trimmed == "\\\" \\\"" {
                return (false, true);
            }
            (false, false)
        } else {
            (false, false)
        }
    }

    /// Expand variable references in an associative array subscript key
    /// without evaluating it as arithmetic.  Handles `$var`, `${var}`,
    /// and strips outer single/double quotes from the key.
    fn expand_arith_subscript_key(&self, key: &str) -> String {
        let trimmed = key.trim();
        // Strip matching outer single quotes (literal)
        if trimmed.len() >= 2 && trimmed.starts_with('\'') && trimmed.ends_with('\'') {
            return trimmed[1..trimmed.len() - 1].to_string();
        }
        // Strip matching outer double quotes (expand $vars inside)
        let inner = if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
            &trimmed[1..trimmed.len() - 1]
        } else {
            // Preserve original whitespace for assoc array keys — spaces
            // are significant (e.g. `a[ ]=val` stores key " ").  Only
            // trim for the outer-quote detection above.
            key
        };
        // Simple variable expansion: $name and ${name}
        let mut result = String::new();
        let chars: Vec<char> = inner.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '\\' && i + 1 < chars.len() {
                // Escaped char — keep the next char literally
                result.push(chars[i + 1]);
                i += 2;
                continue;
            }
            if chars[i] == '$' && i + 1 < chars.len() {
                if chars[i + 1] == '{' {
                    // ${name} form
                    if let Some(close) = chars[i + 2..].iter().position(|&c| c == '}') {
                        let var_name: String = chars[i + 2..i + 2 + close].iter().collect();
                        result.push_str(
                            self.vars
                                .get(var_name.trim())
                                .map(|s| s.as_str())
                                .unwrap_or(""),
                        );
                        i += 2 + close + 1;
                        continue;
                    }
                } else if chars[i + 1].is_alphabetic() || chars[i + 1] == '_' {
                    // $name form
                    let start = i + 1;
                    let mut end = start;
                    while end < chars.len() && (chars[end].is_alphanumeric() || chars[end] == '_') {
                        end += 1;
                    }
                    let var_name: String = chars[start..end].iter().collect();
                    result.push_str(self.vars.get(&var_name).map(|s| s.as_str()).unwrap_or(""));
                    i = end;
                    continue;
                }
            }
            result.push(chars[i]);
            i += 1;
        }
        result
    }

    /// Read the raw string value of an array element (empty string if unset).
    fn arith_array_get_str(&self, sub: &ArithSubscript) -> String {
        match sub {
            ArithSubscript::Assoc(resolved, key) => self
                .assoc_arrays
                .get(resolved)
                .and_then(|m| m.get(key))
                .cloned()
                .unwrap_or_default(),
            ArithSubscript::Indexed(resolved, idx) => self
                .arrays
                .get(resolved)
                .and_then(|a| a.get(*idx))
                .and_then(|v| v.as_deref())
                .map(|s| s.to_string())
                .unwrap_or_default(),
        }
    }

    /// Read the current integer value of an array element.
    /// Non-numeric values are recursively evaluated as arithmetic expressions
    /// (matching bash behavior where `a[0]="1+2"; echo $((a[0]))` yields 3).
    fn arith_array_get(&mut self, sub: &ArithSubscript) -> i64 {
        let val = self.arith_array_get_str(sub);
        if val.is_empty() {
            return 0;
        }
        if let Ok(n) = val.parse::<i64>() {
            return n;
        }
        // Recursively evaluate non-numeric values as arithmetic expressions
        self.eval_arith_expr_impl(&val)
    }

    /// Write an integer value to an array element.
    fn arith_array_set(&mut self, sub: &ArithSubscript, val: i64) {
        match sub {
            ArithSubscript::Assoc(resolved, key) => {
                self.assoc_arrays
                    .entry(resolved.clone())
                    .or_default()
                    .insert(key.clone(), val.to_string());
            }
            ArithSubscript::Indexed(resolved, idx) => {
                let arr = self.arrays.entry(resolved.clone()).or_default();
                while arr.len() <= *idx {
                    arr.push(None);
                }
                arr[*idx] = Some(val.to_string());
            }
        }
    }

    /// Evaluate an arithmetic expression and return the integer result.
    ///
    /// Find an operator in the expression at top-level (outside parentheses).
    /// Strip shell-level double-quote characters from arithmetic expressions.
    ///
    /// Bracket-depth-aware:
    /// - Outside `[...]` subscripts: `\"` → literal `"` (preserved as invalid
    ///   arithmetic char, matching bash where `$(( \"\" ))` errors).
    ///   Standalone `"` → removed (shell quoting) unless `keep_standalone_outside`
    ///   is true (used for `let` context where `"` is literal).
    /// - Inside `[...]` subscripts: `\"` → removed entirely (both `\` and `"`),
    ///   matching bash where `(( a[\" \"]=16 ))` strips to subscript ` ` → 0.
    ///   Standalone `"` → removed (subscript quoting).
    ///   UNLESS `keep_inside_brackets` is true — when `assoc_expand_once` is
    ///   set in `let` context, bash keeps `"` inside subscripts as literal
    ///   chars, causing errors like `" ": arithmetic syntax error`.
    fn strip_arith_quotes_impl(
        s: &str,
        keep_standalone_outside: bool,
        keep_inside_brackets: bool,
    ) -> String {
        let bytes = s.as_bytes();
        let mut result = String::with_capacity(s.len());
        let mut i = 0;
        let mut bracket_depth = 0i32;
        while i < bytes.len() {
            if bytes[i] == b'[' {
                bracket_depth += 1;
                result.push('[');
                i += 1;
            } else if bytes[i] == b']' && bracket_depth > 0 {
                bracket_depth -= 1;
                result.push(']');
                i += 1;
            } else if bytes[i] == b'\\' && i + 1 < bytes.len() && bytes[i + 1] == b'"' {
                if bracket_depth > 0 && !keep_inside_brackets {
                    // Inside subscript: `\"` → consume both (strip escapes)
                    i += 2;
                } else {
                    // Outside subscript (or keep_inside_brackets mode):
                    // `\"` → literal `"` (invalid in arith)
                    result.push('"');
                    i += 2;
                }
            } else if bytes[i] == b'"' {
                if bracket_depth > 0 && !keep_inside_brackets {
                    // Strip standalone `"` inside subscript
                    i += 1;
                } else if bracket_depth > 0 && keep_inside_brackets {
                    // assoc_expand_once + let: keep `"` inside brackets as literal
                    result.push('"');
                    i += 1;
                } else if keep_standalone_outside {
                    // In let context, keep standalone `"` outside brackets
                    // as literal chars — they'll be caught as invalid later
                    result.push('"');
                    i += 1;
                } else {
                    // Strip standalone `"` (shell quoting)
                    i += 1;
                }
            } else {
                result.push(bytes[i] as char);
                i += 1;
            }
        }
        result
    }

    fn strip_arith_quotes(s: &str) -> String {
        Self::strip_arith_quotes_impl(s, false, false)
    }

    fn strip_arith_quotes_for_let(s: &str, assoc_expand_once: bool) -> String {
        Self::strip_arith_quotes_impl(s, true, assoc_expand_once)
    }

    /// Unescape `\"` → `"` for error display purposes, but keep standalone `"`.
    /// Bash error messages show the expression with backslash-escapes removed
    /// but the quote characters themselves preserved.  For example,
    /// `a[\" \"]=15` displays as `a[" "]=15` in error messages.
    fn unescape_arith_quotes(s: &str) -> String {
        let bytes = s.as_bytes();
        let mut result = String::with_capacity(s.len());
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'\\' && i + 1 < bytes.len() && bytes[i + 1] == b'"' {
                // `\"` → `"` (remove backslash, keep quote)
                result.push('"');
                i += 2;
            } else {
                result.push(bytes[i] as char);
                i += 1;
            }
        }
        result
    }

    /// Remove only single-quote shell-level quoting from an array subscript
    /// string that comes from the AST (assign.name).  The parser has already
    /// converted `\"` → `"` during lexing, so bare `"` in the AST subscript
    /// are literal characters (not quoting delimiters).  Only `'...'` regions
    /// remain as shell-level quoting that the arithmetic evaluator cannot
    /// handle.
    ///
    /// After dequoting, the caller must set `arith_skip_quote_strip` so that
    /// the arithmetic evaluator does NOT re-strip the resulting `"` characters
    /// (they are literal arithmetic-invalid chars, matching bash behavior
    /// where `a[\" \"]=15` errors with `" ": arithmetic syntax error`).
    ///
    /// Example: AST subscript `'"' "` (bytes: `'`, `"`, `'`, ` `, `"`)
    /// → dequoted `" "` (the single-quote region `'"'` emits `"`, then
    /// literal ` ` and `"` pass through).
    pub(super) fn dequote_subscript(s: &str) -> String {
        let bytes = s.as_bytes();
        let mut result = String::with_capacity(s.len());
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'\'' {
                // Single-quoted region: content is literal until next `'`.
                // Emit the content without the surrounding quotes.
                i += 1; // skip opening `'`
                while i < bytes.len() && bytes[i] != b'\'' {
                    result.push(bytes[i] as char);
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1; // skip closing `'`
                }
            } else {
                // Everything else passes through unchanged — bare `"` are
                // already literal (parser converted `\"` → `"`), `\` may
                // still appear from single-quote content, etc.
                result.push(bytes[i] as char);
                i += 1;
            }
        }
        result
    }

    fn find_top_level_arith_op(expr: &str, op: &str) -> Option<usize> {
        let mut paren_depth = 0i32;
        let mut bracket_depth = 0i32;
        let bytes = expr.as_bytes();
        let op_bytes = op.as_bytes();
        for i in 0..bytes.len() {
            match bytes[i] {
                b'(' => paren_depth += 1,
                b')' => paren_depth -= 1,
                b'[' => bracket_depth += 1,
                b']' => bracket_depth -= 1,
                _ => {}
            }
            if paren_depth == 0
                && bracket_depth == 0
                && i + op_bytes.len() <= bytes.len()
                && &bytes[i..i + op_bytes.len()] == op_bytes
            {
                return Some(i);
            }
        }
        None
    }

    pub fn eval_arith_expr(&mut self, expr: &str) -> i64 {
        let is_top_level = self.arith_top_expr.is_none();
        if is_top_level {
            // Trim leading whitespace but preserve trailing (bash includes trailing space)
            // Also strip backslash-dollar (\$) → $ for error display
            self.arith_top_expr = Some(expr.trim_start().replace("\\$", "$"));
        }
        let result = self.eval_arith_expr_impl(expr);
        if is_top_level {
            self.arith_top_expr = None;
        }
        result
    }

    fn eval_arith_expr_impl(&mut self, expr: &str) -> i64 {
        self.arith_depth += 1;
        let result = self.eval_arith_expr_inner(expr);
        self.arith_depth -= 1;
        result
    }

    fn eval_arith_expr_inner(&mut self, expr: &str) -> i64 {
        let expr = expr.trim_start();

        // Check for unmatched parentheses at top level
        if self.arith_depth == 1 {
            let mut paren_depth = 0i32;
            let mut bracket_depth = 0i32;
            let mut unmatched_paren_inside_bracket = false;
            for ch in expr.chars() {
                match ch {
                    '[' => bracket_depth += 1,
                    ']' => {
                        if bracket_depth > 0 {
                            bracket_depth -= 1;
                        }
                    }
                    '(' => {
                        paren_depth += 1;
                        if bracket_depth > 0 {
                            unmatched_paren_inside_bracket = true;
                        }
                    }
                    ')' => paren_depth -= 1,
                    _ => {}
                }
            }
            if paren_depth > 0 {
                // If the unmatched `(` is inside `[...]` that itself has no
                // matching `]` (bracket_depth > 0), this is a bad array
                // subscript, not a missing `)`.  Matches bash's error for
                // e.g. `b[$(echo` where `$key` expanded into the subscript.
                if bracket_depth > 0 && unmatched_paren_inside_bracket {
                    let top_expr = self.arith_top_expr.as_deref().unwrap_or(expr);
                    // Error token: the portion starting from the identifier
                    // before the unclosed `[`.  Find the last comma-separated
                    // segment that contains `[` without `]`.
                    // For `b[$(echo`, the error token is `b[$(echo`.
                    let error_token = if let Some(comma_pos) = expr.rfind(',') {
                        let after_comma = expr[comma_pos + 1..].trim();
                        if after_comma.contains('[') && !after_comma.contains(']') {
                            after_comma
                        } else {
                            expr.trim()
                        }
                    } else {
                        expr.trim()
                    };
                    eprintln!(
                        "{}: {}{}: bad array subscript (error token is \"{}\")",
                        self.arith_error_prefix(),
                        self.arith_cmd_prefix(),
                        top_expr,
                        error_token
                    );
                    crate::expand::set_arith_error();
                    return 0;
                }
                let top_expr = self.arith_top_expr.as_deref().unwrap_or(expr);
                // Find the last token for the error
                let error_token = expr
                    .trim()
                    .rsplit(|c: char| c.is_whitespace() || c == '(')
                    .next()
                    .unwrap_or(expr.trim());
                eprintln!(
                    "{}: {}{}: missing `)' (error token is \"{}\")",
                    self.arith_error_prefix(),
                    self.arith_cmd_prefix(),
                    top_expr,
                    error_token.trim_end_matches(')')
                );
                crate::expand::set_arith_error();
                return 0;
            }
        }

        // Check recursion depth limit (bash uses 1024, but each level uses
        // significant stack space so we use a lower limit)
        if self.arith_depth > 64 {
            let var_name = expr.trim();
            eprintln!(
                "{}: {}: expression recursion level exceeded (error token is \"{}\")",
                self.arith_error_prefix(),
                var_name,
                var_name
            );
            crate::expand::set_arith_error();
            return 0;
        }

        // Expand command substitutions $(...), parameter expansions ${...},
        // and backtick comsubs `...`
        // BEFORE stripping quotes, since commands inside $() need their quotes preserved
        // BUT: when arith_skip_comsub_expand is set (array_expand_once mode for
        // subscript evaluation), do NOT expand $(...) — pass the raw text through
        // so the arithmetic parser errors on it (preventing injection).
        let expanded_cs: String;
        let expr = if !self.arith_skip_comsub_expand && (expr.contains('$') || expr.contains('`')) {
            expanded_cs = self.expand_comsubs_in_arith(expr);
            // Update top expression with expanded version for error messages
            if self.arith_depth == 1
                && let Some(ref mut top) = self.arith_top_expr
                && top.contains('$')
            {
                // When the expression contains single quotes (literal in
                // arithmetic), keep \$ escaping in the display form — bash
                // shows the backslash-escaped expansion in error messages.
                if expr.contains('\'') {
                    *top = expanded_cs.trim_start().to_string();
                } else {
                    *top = expanded_cs.trim_start().replace("\\$", "$");
                }
            }
            &expanded_cs
        } else {
            expr
        };

        // Strip double quotes from arith expressions (bash behavior).
        // When `arith_skip_quote_strip` is set, the caller already dequoted
        // the expression (e.g. assignment subscripts go through
        // `dequote_subscript`), so `"` characters are literal
        // arithmetic-invalid chars that must NOT be silently stripped.
        // Save the pre-stripped expression for `let` empty-subscript detection.
        let pre_strip_expr: String;
        let unquoted: String;
        let expr = if expr.contains('"') && !self.arith_skip_quote_strip {
            pre_strip_expr = expr.to_string();
            // Strip double quotes from arithmetic expressions.
            // Outside brackets: `\"` → literal `"` (invalid in arith).
            // Inside brackets: `\"` → removed (subscript quoting).
            // Standalone `"` → removed (shell quoting) unless in `let` context.
            let assoc_expand_once_on = self
                .shopt_options
                .get("assoc_expand_once")
                .copied()
                .unwrap_or(false);
            unquoted = if self.arith_is_let {
                Self::strip_arith_quotes_for_let(expr, assoc_expand_once_on)
            } else {
                Self::strip_arith_quotes(expr)
            };
            // Update top expression for error messages.
            // If the original had `\"` → `unescape_arith_quotes` converts
            // `\"` → `"` for display (bash shows literal quotes in errors).
            // If the original had only standalone `""` and we're NOT in `let`
            // context → strip them from the display too (bash shows the
            // quote-stripped expression for `(( ))` and `$(( ))`).
            // In `let` context, keep `"` in the display — bash shows them
            // in the error message (e.g. `let: 0 - "": operand expected`).
            if self.arith_depth == 1
                && let Some(ref mut top) = self.arith_top_expr
                && top.contains('"')
            {
                if top.contains("\\\"") {
                    *top = Self::unescape_arith_quotes(top);
                } else if !self.arith_is_let {
                    // Only standalone `"` in non-let context — strip from display
                    *top = Self::strip_arith_quotes(top);
                }
                // In let context with only standalone `"`, keep them for display
            }
            &unquoted
        } else {
            pre_strip_expr = String::new();
            expr
        };

        // Check for leading operators that can't start an expression (/, *, %, ')
        // These indicate "operand expected" — report once for the full expression.
        // Single quotes are literal chars in arithmetic (not quoting) and are
        // invalid operands, matching bash's behavior.
        if self.arith_depth == 1 {
            let trimmed = expr.trim();
            if !trimmed.is_empty() {
                let first = trimmed.as_bytes()[0];
                if matches!(first, b'/' | b'%' | b'\'') {
                    let top_expr = self.arith_top_expr.as_deref().unwrap_or(expr);
                    eprintln!(
                        "{}: {}{}: arithmetic syntax error: operand expected (error token is \"{}\")",
                        self.arith_error_prefix(),
                        self.arith_cmd_prefix(),
                        top_expr,
                        top_expr
                    );
                    crate::expand::set_arith_error();
                    return 0;
                }
            }
        }

        // Check for literal `"` at any depth — `"` is never valid in arithmetic.
        // This catches `$(( \"\" ))` (depth 1), `let '0 - ""'` right operand
        // (depth 2+), and subscript `" "` errors (depth 2+ in subscript context).
        {
            let trimmed = expr.trim();
            if !trimmed.is_empty() && trimmed.as_bytes()[0] == b'"' {
                if self.arith_depth == 1 {
                    // Top-level: use arith_top_expr for display and error token
                    let top_expr = self.arith_top_expr.as_deref().unwrap_or(trimmed);
                    eprintln!(
                        "{}: {}{}: arithmetic syntax error: operand expected (error token is \"{}\")",
                        self.arith_error_prefix(),
                        self.arith_cmd_prefix(),
                        top_expr,
                        top_expr
                    );
                } else if self.arith_in_subscript {
                    // Inside subscript evaluation (e.g. `a[" "]=18`): show
                    // just the sub-expression without cmd_prefix (no `let:`
                    // or `((:`).  This matches bash where subscript errors
                    // like `" ": arithmetic syntax error` don't include the
                    // command context.
                    eprintln!(
                        "{}: {}: arithmetic syntax error: operand expected (error token is \"{}\")",
                        self.arith_error_prefix(),
                        trimmed,
                        trimmed
                    );
                } else {
                    // Depth > 1 in non-subscript context (e.g. right operand
                    // of `let '0 - ""'` or `(( 1 - "" ))`): use
                    // arith_top_expr and cmd_prefix, matching bash which
                    // shows `let: 0 - "": ...`.
                    let top_expr = self.arith_top_expr.as_deref().unwrap_or(trimmed);
                    eprintln!(
                        "{}: {}{}: arithmetic syntax error: operand expected (error token is \"{}\")",
                        self.arith_error_prefix(),
                        self.arith_cmd_prefix(),
                        top_expr,
                        trimmed
                    );
                }
                crate::expand::set_arith_error();
                return 0;
            }
        }

        // Check for trailing operators (e.g., "4+" → syntax error)
        {
            let trimmed = expr.trim();
            if !trimmed.is_empty() {
                let last = trimmed.as_bytes()[trimmed.len() - 1];
                if matches!(last, b'+' | b'-' | b'*' | b'/' | b'%' | b'^' | b'~')
                    && !trimmed.ends_with("++")
                    && !trimmed.ends_with("--")
                {
                    let display_expr = if self.arith_depth == 1 {
                        self.arith_top_expr.as_deref().unwrap_or(trimmed)
                    } else {
                        expr // preserve original spacing for recursive evals
                    };
                    // Find the trailing operator in the display expression
                    let error_token = if let Some(pos) = display_expr.rfind(last as char) {
                        &display_expr[pos..]
                    } else {
                        &expr[expr.len() - 1..]
                    };
                    eprintln!(
                        "{}: {}{}: arithmetic syntax error: operand expected (error token is \"{}\")",
                        self.arith_error_prefix(),
                        self.arith_cmd_prefix(),
                        display_expr,
                        error_token
                    );
                    crate::expand::set_arith_error();
                    return 0;
                }
            }
        }

        // Check for "invalid arithmetic operator" BEFORE comma/assignment/binary
        // operator scanning.  When a valid identifier or number is followed by a
        // character that is not a valid arithmetic operator (], #, @, {, }, ., ;,
        // \, '), report the error immediately.  This must run before the comma
        // check because e.g. `x],b` would otherwise be split at `,` into `x]`
        // and `b`, masking the real error at `]`.
        {
            let trimmed = expr.trim();
            let ident_end = trimmed
                .find(|c: char| !c.is_alphanumeric() && c != '_')
                .unwrap_or(trimmed.len());
            if ident_end > 0 && ident_end < trimmed.len() {
                let next_ch = trimmed.as_bytes()[ident_end];
                if matches!(
                    next_ch,
                    b']' | b'@' | b'{' | b'}' | b'.' | b';' | b'\\' | b'\''
                ) {
                    let invalid_part = &trimmed[ident_end..];
                    let display_expr = trimmed;
                    let error_token = invalid_part;
                    eprintln!(
                        "{}: {}: arithmetic syntax error: invalid arithmetic operator (error token is \"{}\")",
                        self.arith_error_prefix(),
                        display_expr,
                        error_token
                    );
                    crate::expand::set_arith_error();
                    return 0;
                }
            }
            // Also check when the expression starts with an invalid operator character
            if !trimmed.is_empty() {
                let first_ch = trimmed.as_bytes()[0];
                if matches!(first_ch, b']' | b'@' | b'{' | b'}' | b'.' | b';') {
                    let display_expr = trimmed;
                    eprintln!(
                        "{}: {}: arithmetic syntax error: invalid arithmetic operator (error token is \"{}\")",
                        self.arith_error_prefix(),
                        display_expr,
                        display_expr
                    );
                    crate::expand::set_arith_error();
                    return 0;
                }
            }
        }

        // Handle comma operator (only at top level, not inside parens or brackets)
        {
            let mut paren_depth = 0i32;
            let mut bracket_depth = 0i32;
            let mut last_comma = None;
            for (i, ch) in expr.char_indices() {
                match ch {
                    '(' => paren_depth += 1,
                    ')' => paren_depth -= 1,
                    '[' => bracket_depth += 1,
                    ']' => {
                        if bracket_depth > 0 {
                            bracket_depth -= 1;
                        }
                    }
                    ',' if paren_depth == 0 && bracket_depth == 0 => last_comma = Some(i),
                    _ => {}
                }
            }
            if let Some(pos) = last_comma {
                self.eval_arith_expr_impl(&expr[..pos]);
                return self.eval_arith_expr_impl(&expr[pos + 1..]);
            }
        }

        // Handle assignment operators: var=, var+=, var-=, var*=, var/=, var%=,
        // var<<=, var>>=, var&=, var|=, var^=
        #[allow(clippy::type_complexity)]
        let assign_ops: &[(&str, fn(i64, i64) -> i64)] = &[
            ("<<=", |a, b| a.wrapping_shl(b as u32)),
            (">>=", |a, b| a.wrapping_shr(b as u32)),
            ("+=", |a, b| a.wrapping_add(b)),
            ("-=", |a, b| a.wrapping_sub(b)),
            ("*=", |a, b| a.wrapping_mul(b)),
            ("/=", |a, b| if b == 0 { 0 } else { a.wrapping_div(b) }),
            ("%=", |a, b| {
                if b == 0 || (a == i64::MIN && b == -1) {
                    0
                } else {
                    a.wrapping_rem(b)
                }
            }),
            ("&=", |a, b| a & b),
            ("|=", |a, b| a | b),
            ("^=", |a, b| a ^ b),
        ];

        for &(op, func) in assign_ops {
            if let Some(pos) = Self::find_top_level_arith_op(expr, op) {
                // Skip `+=` when preceded by `+` (i.e. `x++=7` is `x++ = 7`, not `x += =7`)
                // Similarly skip `-=` when preceded by `-` (i.e. `x--=7` is `x-- = 7`)
                if pos > 0 {
                    let prev = expr.as_bytes()[pos - 1];
                    if (op == "+=" && prev == b'+') || (op == "-=" && prev == b'-') {
                        continue;
                    }
                }
                let name = expr[..pos].trim();
                // Check if the assignment op is inside a ternary then-branch:
                // if the LHS contains an unmatched `?` (no corresponding `:`),
                // then the `+=` etc. is between `?` and `:` — skip and let the
                // ternary handler deal with it.
                let has_unmatched_question = {
                    let mut q = 0i32;
                    for ch in name.chars() {
                        if ch == '?' {
                            q += 1;
                        }
                        if ch == ':' {
                            q -= 1;
                        }
                    }
                    q > 0
                };
                if has_unmatched_question {
                    // Skip — this assignment op is inside a ternary branch
                    continue;
                }
                // Check if LHS is a valid variable name (or array element)
                let is_valid_lhs = !name.is_empty()
                    && name.chars().next().is_some_and(|c| !c.is_ascii_digit())
                    && (name.chars().all(|c| c.is_alphanumeric() || c == '_')
                        || name.contains('['));
                if !is_valid_lhs && !name.is_empty() {
                    // LHS is not a valid variable — "attempted assignment to non-variable"
                    let top_expr = self.arith_top_expr.as_deref().unwrap_or(expr);
                    eprintln!(
                        "{}: {}{}: attempted assignment to non-variable (error token is \"{}\")",
                        self.arith_error_prefix(),
                        self.arith_cmd_prefix(),
                        top_expr,
                        &expr[pos..]
                    );
                    crate::expand::set_arith_error();
                    return 0;
                }
                if is_valid_lhs {
                    let rhs = self.eval_arith_expr_impl(&expr[pos + op.len()..]);
                    // Check for division by zero in /= and %=
                    if (op == "/=" || op == "%=") && rhs == 0 {
                        let top_expr = self.arith_top_expr.as_deref().unwrap_or(expr);
                        let error_token = expr[pos + op.len()..].trim_start();
                        eprintln!(
                            "{}: {}{}: division by 0 (error token is \"{}\")",
                            self.arith_error_prefix(),
                            self.arith_cmd_prefix(),
                            top_expr,
                            error_token
                        );
                        crate::expand::set_arith_error();
                        return 0;
                    }
                    // Handle array element: name[subscript]
                    if let Some(bracket) = name.find('[') {
                        let base = &name[..bracket];
                        let idx_str = &name[bracket + 1..name.len() - 1];
                        // When assoc_expand_once is set in let context, the
                        // subscript may contain literal `"` chars (kept by
                        // strip_arith_quotes_for_let).  Don't treat `""` as
                        // an empty subscript — let it flow to recursive eval
                        // where the `"` detection will produce the proper
                        // arithmetic syntax error.
                        let aeo_let_has_quotes = self.arith_is_let
                            && self
                                .shopt_options
                                .get("assoc_expand_once")
                                .copied()
                                .unwrap_or(false)
                            && idx_str.contains('"');
                        if !aeo_let_has_quotes && Self::is_empty_arith_subscript(idx_str) {
                            // In `let` context, `a[""]=22` has quotes stripped to `a[]=22`
                            // but is valid — the empty subscript evaluates to 0.
                            // Only when `assoc_expand_once` is unset though; when set,
                            // bash still rejects the empty subscript.
                            // In (( )) and $(( )) contexts, `a[\"\"]=20` (escaped quotes)
                            // also strips to `a[]=20` and should evaluate to a[0]=20.
                            // But bare `a[""]=24` in (( )) DOES error — only escaped forms pass.
                            let assoc_expand_once = self
                                .shopt_options
                                .get("assoc_expand_once")
                                .copied()
                                .unwrap_or(false);
                            let (bare_quoted, escaped_quoted) = if !pre_strip_expr.is_empty() {
                                Self::had_quoted_empty_subscript(&pre_strip_expr, bracket)
                            } else {
                                (false, false)
                            };
                            let is_let_quoted_empty = self.arith_is_let
                                && !assoc_expand_once
                                && (bare_quoted || escaped_quoted);
                            // In (( )) and $(( )), only escaped-quote forms like `\"\"` are allowed
                            let is_arith_quoted_empty = !self.arith_is_let && escaped_quoted;
                            if !is_let_quoted_empty && !is_arith_quoted_empty {
                                eprintln!(
                                    "{}: {}`{}[]': not a valid identifier",
                                    self.arith_error_prefix(),
                                    self.arith_cmd_prefix(),
                                    base
                                );
                                crate::expand::set_arith_nonfatal_error();
                                return 0;
                            }
                        }
                        let sub = self.arith_subscript_key(base, idx_str);
                        if crate::expand::take_arith_error() {
                            crate::expand::set_arith_error();
                            return 0;
                        }
                        let lhs: i64 = self.arith_array_get(&sub);
                        let result = func(lhs, rhs);
                        self.arith_array_set(&sub, result);
                        return result;
                    }
                    let lhs: i64 = self
                        .vars
                        .get(name)
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0);
                    let result = func(lhs, rhs);
                    self.set_var(name, result.to_string());
                    return result;
                }
            }
        }

        // Handle simple assignment: var=expr (but not ==)
        if let Some(pos) = Self::find_top_level_arith_op(expr, "=")
            && pos > 0
            && !expr[..pos].ends_with('!')
            && !expr[..pos].ends_with('<')
            && !expr[..pos].ends_with('>')
            && !expr[pos + 1..].starts_with('=')
        {
            let name = expr[..pos].trim();
            // Check for ++/-- prefix/suffix on the LHS → attempted assignment to non-variable
            if !name.is_empty()
                && (name.starts_with("++")
                    || name.starts_with("--")
                    || name.ends_with("++")
                    || name.ends_with("--"))
            {
                let top_expr = self.arith_top_expr.as_deref().unwrap_or(expr);
                eprintln!(
                    "{}: {}{}: attempted assignment to non-variable (error token is \"{}\")",
                    self.arith_error_prefix(),
                    self.arith_cmd_prefix(),
                    top_expr,
                    &expr[pos..]
                );
                crate::expand::set_arith_error();
                return 0;
            }
            if !name.is_empty()
                && name.chars().next().is_some_and(|c| !c.is_ascii_digit())
                && (name.chars().all(|c| c.is_alphanumeric() || c == '_') || name.contains('['))
            {
                let rhs = &expr[pos + 1..];
                if rhs.trim().is_empty() {
                    // Empty RHS: e.g. "j=" → syntax error
                    eprintln!(
                        "{}: {}{}: arithmetic syntax error: operand expected (error token is \"{}\")",
                        self.arith_error_prefix(),
                        self.arith_cmd_prefix(),
                        expr,
                        &expr[pos..]
                    );
                    crate::expand::set_arith_error();
                    return 0;
                }
                // Check for readonly variable
                let base_name = if let Some(bracket) = name.find('[') {
                    &name[..bracket]
                } else {
                    name
                };
                let resolved_name = self.resolve_nameref(base_name).to_string();
                if self.readonly_vars.contains(&resolved_name) {
                    eprintln!(
                        "{}: {}: readonly variable",
                        self.error_prefix(),
                        resolved_name
                    );
                    crate::expand::set_arith_error();
                    return 0;
                }
                let val = self.eval_arith_expr_impl(rhs);
                if let Some(bracket) = name.find('[') {
                    let base = &name[..bracket];
                    let idx_str = &name[bracket + 1..name.len() - 1];
                    // Empty subscript after quote removal: a[""]=N → error (but not in let context)
                    let aeo_let_has_quotes = self.arith_is_let
                        && self
                            .shopt_options
                            .get("assoc_expand_once")
                            .copied()
                            .unwrap_or(false)
                        && idx_str.contains('"');
                    if !aeo_let_has_quotes && Self::is_empty_arith_subscript(idx_str) {
                        let assoc_expand_once = self
                            .shopt_options
                            .get("assoc_expand_once")
                            .copied()
                            .unwrap_or(false);
                        let (bare_quoted, escaped_quoted) = if !pre_strip_expr.is_empty() {
                            Self::had_quoted_empty_subscript(&pre_strip_expr, bracket)
                        } else {
                            (false, false)
                        };
                        let is_let_quoted_empty = self.arith_is_let
                            && !assoc_expand_once
                            && (bare_quoted || escaped_quoted);
                        let is_arith_quoted_empty = !self.arith_is_let && escaped_quoted;
                        if !is_let_quoted_empty && !is_arith_quoted_empty {
                            eprintln!(
                                "{}: {}`{}[]': not a valid identifier",
                                self.arith_error_prefix(),
                                self.arith_cmd_prefix(),
                                base
                            );
                            crate::expand::set_arith_nonfatal_error();
                            return 0;
                        }
                    }
                    let sub = self.arith_subscript_key(base, idx_str);
                    if crate::expand::take_arith_error() {
                        crate::expand::set_arith_error();
                        return 0;
                    }
                    self.arith_array_set(&sub, val);
                } else {
                    self.set_var(name, val.to_string());
                }
                return val;
            }
            // Assignment to non-variable (e.g., 7=4, or 0 && B=42)
            // But skip if LHS has unmatched `?` (inside ternary then-branch)
            if !name.is_empty() {
                let has_unmatched_question = {
                    let mut q = 0i32;
                    for ch in name.chars() {
                        if ch == '?' {
                            q += 1;
                        }
                        if ch == ':' {
                            q -= 1;
                        }
                    }
                    q > 0
                };
                if !has_unmatched_question {
                    // Check if this looks like a space-separated expression
                    // (e.g. "x=9 y=41") — bash reports "syntax error in expression"
                    // rather than "attempted assignment to non-variable" for those
                    if name.contains(' ') && !name.contains('&') && !name.contains('|') {
                        // Fall through to expression syntax error at end of function
                    } else {
                        let top_expr = self.arith_top_expr.as_deref().unwrap_or(expr);
                        eprintln!(
                            "{}: {}{}: attempted assignment to non-variable (error token is \"{}\")",
                            self.arith_error_prefix(),
                            self.arith_cmd_prefix(),
                            top_expr,
                            &expr[pos..]
                        );
                        crate::expand::set_arith_error();
                        return 0;
                    }
                }
            }
        }

        // Handle post-increment/decrement: var++, var--, arr[idx]++, arr[idx]--
        for (suffix, delta) in &[("++", 1i64), ("--", -1i64)] {
            if let Some(stripped) = expr.trim_end().strip_suffix(suffix) {
                let name = stripped.trim();
                if name.is_empty() {
                    continue;
                }
                // Check for array subscript: name[expr]
                if let Some(bracket) = name.find('[')
                    && name.ends_with(']')
                {
                    let base = &name[..bracket];
                    let idx_str = &name[bracket + 1..name.len() - 1];
                    let aeo_let_has_quotes = self.arith_is_let
                        && self
                            .shopt_options
                            .get("assoc_expand_once")
                            .copied()
                            .unwrap_or(false)
                        && idx_str.contains('"');
                    if !aeo_let_has_quotes && Self::is_empty_arith_subscript(idx_str) {
                        let assoc_expand_once = self
                            .shopt_options
                            .get("assoc_expand_once")
                            .copied()
                            .unwrap_or(false);
                        let (bare_quoted, escaped_quoted) = if !pre_strip_expr.is_empty() {
                            Self::had_quoted_empty_subscript(&pre_strip_expr, bracket)
                        } else {
                            (false, false)
                        };
                        let is_let_quoted_empty = self.arith_is_let
                            && !assoc_expand_once
                            && (bare_quoted || escaped_quoted);
                        let is_arith_quoted_empty = !self.arith_is_let && escaped_quoted;
                        if !is_let_quoted_empty && !is_arith_quoted_empty {
                            eprintln!(
                                "{}: {}`{}[]': not a valid identifier",
                                self.arith_error_prefix(),
                                self.arith_cmd_prefix(),
                                base
                            );
                            crate::expand::set_arith_nonfatal_error();
                            return 0;
                        }
                    }
                    let sub = self.arith_subscript_key(base, idx_str);
                    if crate::expand::take_arith_error() {
                        crate::expand::set_arith_error();
                        return 0;
                    }
                    let val: i64 = self.arith_array_get(&sub);
                    self.arith_array_set(&sub, val + delta);
                    return val;
                }
                if name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                    if name.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                        let op_char = if *delta > 0 { "+" } else { "-" };
                        eprintln!(
                            "{}: {}{}: arithmetic syntax error: operand expected (error token is \"{} \")",
                            self.arith_error_prefix(),
                            self.arith_cmd_prefix(),
                            expr,
                            op_char,
                        );
                        crate::expand::set_arith_error();
                        return 0;
                    }
                    let val: i64 = self
                        .vars
                        .get(name)
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0);
                    self.set_var(name, (val + delta).to_string());
                    return val;
                }
            }
        }

        // Handle pre-increment/decrement: ++var, --var, ++arr[idx], --arr[idx]
        if let Some(stripped) = expr.strip_prefix("++") {
            let name = stripped.trim();
            if name.is_empty() {
                eprintln!(
                    "{}: ((: ++ : arithmetic syntax error: operand expected (error token is \"+ \")",
                    self.arith_error_prefix()
                );
                crate::expand::set_arith_error();
                return 0;
            }
            // Check for ++x++ or ++x-- (post-increment/decrement on result of pre-increment)
            if name.ends_with("++") || name.ends_with("--") {
                let suffix_op = if name.ends_with("++") { "++" } else { "--" };
                let top_expr = self.arith_top_expr.as_deref().unwrap_or(expr);
                eprintln!(
                    "{}: {}{}: {}: assignment requires lvalue (error token is \"{} \")",
                    self.arith_error_prefix(),
                    self.arith_cmd_prefix(),
                    top_expr,
                    suffix_op,
                    suffix_op,
                );
                crate::expand::set_arith_error();
                return 0;
            }
            // Check for simple var or array element (name[...])
            // First char must be letter or _ (not digit — ++7 is not a pre-increment)
            let first_ok = name
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
            let is_var = first_ok
                && if let Some(bracket) = name.find('[') {
                    let base = &name[..bracket];
                    !base.is_empty()
                        && base.chars().all(|c| c.is_alphanumeric() || c == '_')
                        && name.ends_with(']')
                } else {
                    name.chars().all(|c| c.is_alphanumeric() || c == '_')
                };
            if is_var {
                if name.contains('[') {
                    let bracket = name.find('[').unwrap();
                    let base = &name[..bracket];
                    let idx_expr = &name[bracket + 1..name.len() - 1];
                    let aeo_let_has_quotes = self.arith_is_let
                        && self
                            .shopt_options
                            .get("assoc_expand_once")
                            .copied()
                            .unwrap_or(false)
                        && idx_expr.contains('"');
                    if !aeo_let_has_quotes && Self::is_empty_arith_subscript(idx_expr) {
                        let assoc_expand_once = self
                            .shopt_options
                            .get("assoc_expand_once")
                            .copied()
                            .unwrap_or(false);
                        let (bare_quoted, escaped_quoted) = if !pre_strip_expr.is_empty() {
                            Self::had_quoted_empty_subscript(&pre_strip_expr, bracket)
                        } else {
                            (false, false)
                        };
                        let is_let_quoted_empty = self.arith_is_let
                            && !assoc_expand_once
                            && (bare_quoted || escaped_quoted);
                        let is_arith_quoted_empty = !self.arith_is_let && escaped_quoted;
                        if !is_let_quoted_empty && !is_arith_quoted_empty {
                            eprintln!(
                                "{}: {}`{}[]': not a valid identifier",
                                self.arith_error_prefix(),
                                self.arith_cmd_prefix(),
                                base
                            );
                            crate::expand::set_arith_nonfatal_error();
                            return 0;
                        }
                    }
                    let sub = self.arith_subscript_key(base, idx_expr);
                    if crate::expand::take_arith_error() {
                        crate::expand::set_arith_error();
                        return 0;
                    }
                    let val: i64 = self.arith_array_get(&sub);
                    let new_val = val + 1;
                    self.arith_array_set(&sub, new_val);
                    return new_val;
                } else {
                    let val: i64 = self
                        .vars
                        .get(name)
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0);
                    let new_val = val + 1;
                    self.set_var(name, new_val.to_string());
                    return new_val;
                }
            }
        }
        if let Some(stripped) = expr.strip_prefix("--") {
            let name = stripped.trim();
            if name.is_empty() {
                eprintln!(
                    "{}: ((: -- : arithmetic syntax error: operand expected (error token is \"- \")",
                    self.arith_error_prefix()
                );
                crate::expand::set_arith_error();
                return 0;
            }
            // Check for --x++ or --x-- (post-increment/decrement on result of pre-decrement)
            if name.ends_with("++") || name.ends_with("--") {
                let suffix_op = if name.ends_with("++") { "++" } else { "--" };
                let top_expr = self.arith_top_expr.as_deref().unwrap_or(expr);
                eprintln!(
                    "{}: {}{}: {}: assignment requires lvalue (error token is \"{} \")",
                    self.arith_error_prefix(),
                    self.arith_cmd_prefix(),
                    top_expr,
                    suffix_op,
                    suffix_op,
                );
                crate::expand::set_arith_error();
                return 0;
            }
            let first_ok = name
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
            let is_var = first_ok
                && if let Some(bracket) = name.find('[') {
                    let base = &name[..bracket];
                    !base.is_empty()
                        && base.chars().all(|c| c.is_alphanumeric() || c == '_')
                        && name.ends_with(']')
                } else {
                    !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_')
                };
            if is_var {
                if name.contains('[') {
                    let bracket = name.find('[').unwrap();
                    let base = &name[..bracket];
                    let idx_expr = &name[bracket + 1..name.len() - 1];
                    let aeo_let_has_quotes = self.arith_is_let
                        && self
                            .shopt_options
                            .get("assoc_expand_once")
                            .copied()
                            .unwrap_or(false)
                        && idx_expr.contains('"');
                    if !aeo_let_has_quotes && Self::is_empty_arith_subscript(idx_expr) {
                        let assoc_expand_once = self
                            .shopt_options
                            .get("assoc_expand_once")
                            .copied()
                            .unwrap_or(false);
                        let (bare_quoted, escaped_quoted) = if !pre_strip_expr.is_empty() {
                            Self::had_quoted_empty_subscript(&pre_strip_expr, bracket)
                        } else {
                            (false, false)
                        };
                        let is_let_quoted_empty = self.arith_is_let
                            && !assoc_expand_once
                            && (bare_quoted || escaped_quoted);
                        let is_arith_quoted_empty = !self.arith_is_let && escaped_quoted;
                        if !is_let_quoted_empty && !is_arith_quoted_empty {
                            eprintln!(
                                "{}: {}`{}[]': not a valid identifier",
                                self.arith_error_prefix(),
                                self.arith_cmd_prefix(),
                                base
                            );
                            crate::expand::set_arith_nonfatal_error();
                            return 0;
                        }
                    }
                    let sub = self.arith_subscript_key(base, idx_expr);
                    if crate::expand::take_arith_error() {
                        crate::expand::set_arith_error();
                        return 0;
                    }
                    let val: i64 = self.arith_array_get(&sub);
                    let new_val = val - 1;
                    self.arith_array_set(&sub, new_val);
                    return new_val;
                } else {
                    let val: i64 = self
                        .vars
                        .get(name)
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0);
                    let new_val = val - 1;
                    self.set_var(name, new_val.to_string());
                    return new_val;
                }
            }
        }

        // Handle ternary operator: expr ? expr : expr
        // Only evaluate the taken branch (short-circuit)
        if let Some(q_pos) = Self::find_top_level_arith_op(expr, "?") {
            let cond = self.eval_arith_expr_impl(&expr[..q_pos]);
            let rest = &expr[q_pos + 1..];
            // Find the matching ':' at top level in the rest
            if let Some(c_pos) = Self::find_top_level_arith_op(rest, ":") {
                let then_part = &rest[..c_pos];
                let else_part = &rest[c_pos + 1..];
                // Check for empty then/else parts
                if then_part.trim().is_empty() || else_part.trim().is_empty() {
                    let top_expr = self.arith_top_expr.as_deref().unwrap_or(expr);
                    let error_token = &rest[c_pos..]; // from ':' onwards
                    eprintln!(
                        "{}: {}{}: expression expected (error token is \"{}\")",
                        self.arith_error_prefix(),
                        self.arith_cmd_prefix(),
                        top_expr,
                        error_token
                    );
                    crate::expand::set_arith_error();
                    return 0;
                }
                return if cond != 0 {
                    self.eval_arith_expr_impl(then_part)
                } else {
                    self.eval_arith_expr_impl(else_part)
                };
            }
            // Missing ':' in ternary — error
            let top_expr = self.arith_top_expr.as_deref().unwrap_or(expr);
            eprintln!(
                "{}: {}{}: `:' expected for conditional expression (error token is \"{}\")",
                self.arith_error_prefix(),
                self.arith_cmd_prefix(),
                top_expr,
                rest.trim_start()
            );
            crate::expand::set_arith_error();
            return 0;
        }

        // Handle || at top level (preserves assignments in subexprs)
        {
            let mut depth = 0i32;
            let bytes = expr.as_bytes();
            let mut i = bytes.len();
            while i > 1 {
                i -= 1;
                match bytes[i] {
                    b')' | b']' => depth += 1,
                    b'(' | b'[' => depth -= 1,
                    b'|' if depth == 0 && i > 0 && bytes[i - 1] == b'|' => {
                        let left = self.eval_arith_expr_impl(&expr[..i - 1]);
                        if left != 0 {
                            return 1;
                        }
                        let right = self.eval_arith_expr_impl(&expr[i + 1..]);
                        return if right != 0 { 1 } else { 0 };
                    }
                    _ => {}
                }
            }
        }

        // Handle && at top level
        {
            let mut depth = 0i32;
            let bytes = expr.as_bytes();
            let mut i = bytes.len();
            while i > 1 {
                i -= 1;
                match bytes[i] {
                    b')' | b']' => depth += 1,
                    b'(' | b'[' => depth -= 1,
                    b'&' if depth == 0 && i > 0 && bytes[i - 1] == b'&' => {
                        let left = self.eval_arith_expr_impl(&expr[..i - 1]);
                        if left == 0 {
                            return 0;
                        }
                        let right = self.eval_arith_expr_impl(&expr[i + 1..]);
                        return if right != 0 { 1 } else { 0 };
                    }
                    _ => {}
                }
            }
        }

        // Handle bitwise OR |  (not ||)
        {
            let bytes = expr.as_bytes();
            let mut depth = 0i32;
            let mut i = bytes.len();
            while i > 0 {
                i -= 1;
                match bytes[i] {
                    b')' | b']' => depth += 1,
                    b'(' | b'[' => depth -= 1,
                    b'|' if depth == 0
                        && !(i > 0 && bytes[i - 1] == b'|')
                        && (i + 1 >= bytes.len() || bytes[i + 1] != b'|')
                        && !(i > 0 && bytes[i - 1] == b'=') =>
                    {
                        let left = self.eval_arith_expr_impl(&expr[..i]);
                        let right = self.eval_arith_expr_impl(&expr[i + 1..]);
                        return left | right;
                    }
                    _ => {}
                }
            }
        }

        // Handle bitwise XOR ^
        {
            let bytes = expr.as_bytes();
            let mut depth = 0i32;
            let mut i = bytes.len();
            while i > 0 {
                i -= 1;
                match bytes[i] {
                    b')' | b']' => depth += 1,
                    b'(' | b'[' => depth -= 1,
                    b'^' if depth == 0 && !(i > 0 && bytes[i - 1] == b'=') => {
                        let left = self.eval_arith_expr_impl(&expr[..i]);
                        let right = self.eval_arith_expr_impl(&expr[i + 1..]);
                        return left ^ right;
                    }
                    _ => {}
                }
            }
        }

        // Handle bitwise AND & (not &&)
        {
            let bytes = expr.as_bytes();
            let mut depth = 0i32;
            let mut i = bytes.len();
            while i > 0 {
                i -= 1;
                match bytes[i] {
                    b')' | b']' => depth += 1,
                    b'(' | b'[' => depth -= 1,
                    b'&' if depth == 0
                        && !(i > 0 && bytes[i - 1] == b'&')
                        && (i + 1 >= bytes.len() || bytes[i + 1] != b'&')
                        && !(i > 0 && bytes[i - 1] == b'=') =>
                    {
                        let left = self.eval_arith_expr_impl(&expr[..i]);
                        let right = self.eval_arith_expr_impl(&expr[i + 1..]);
                        return left & right;
                    }
                    _ => {}
                }
            }
        }

        // Handle parenthesized expressions at top level
        let trimmed_for_paren = expr.trim_end();
        if trimmed_for_paren.starts_with('(') && trimmed_for_paren.ends_with(')') {
            let mut depth = 0i32;
            let mut all_matched = true;
            for (i, ch) in trimmed_for_paren.chars().enumerate() {
                match ch {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 && i < trimmed_for_paren.len() - 1 {
                            all_matched = false;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            if all_matched {
                return self
                    .eval_arith_expr_impl(&trimmed_for_paren[1..trimmed_for_paren.len() - 1]);
            }
        }

        // Handle comparison operators at top level (right-to-left scan)
        {
            let bytes = expr.as_bytes();
            let mut depth = 0i32;
            let mut i = bytes.len();
            while i > 0 {
                i -= 1;
                match bytes[i] {
                    b')' | b']' => depth += 1,
                    b'(' | b'[' => depth -= 1,
                    b'=' if depth == 0 && i > 0 && bytes[i - 1] == b'<' => {
                        let left = self.eval_arith_expr_impl(&expr[..i - 1]);
                        let right = self.eval_arith_expr_impl(&expr[i + 1..]);
                        return if left <= right { 1 } else { 0 };
                    }
                    b'=' if depth == 0 && i > 0 && bytes[i - 1] == b'>' => {
                        let left = self.eval_arith_expr_impl(&expr[..i - 1]);
                        let right = self.eval_arith_expr_impl(&expr[i + 1..]);
                        return if left >= right { 1 } else { 0 };
                    }
                    b'=' if depth == 0
                        && i > 0
                        && bytes[i - 1] == b'='
                        && !(i >= 2 && bytes[i - 2] == b'!') =>
                    {
                        let left = self.eval_arith_expr_impl(&expr[..i - 1]);
                        let right = self.eval_arith_expr_impl(&expr[i + 1..]);
                        return if left == right { 1 } else { 0 };
                    }
                    b'=' if depth == 0 && i > 0 && bytes[i - 1] == b'!' => {
                        let left = self.eval_arith_expr_impl(&expr[..i - 1]);
                        let right = self.eval_arith_expr_impl(&expr[i + 1..]);
                        return if left != right { 1 } else { 0 };
                    }
                    b'<' if depth == 0
                        && !(i > 0 && bytes[i - 1] == b'<')
                        && (i + 1 >= bytes.len()
                            || (bytes[i + 1] != b'=' && bytes[i + 1] != b'<')) =>
                    {
                        let left = self.eval_arith_expr_impl(&expr[..i]);
                        let right = self.eval_arith_expr_impl(&expr[i + 1..]);
                        return if left < right { 1 } else { 0 };
                    }
                    b'>' if depth == 0
                        && !(i > 0 && bytes[i - 1] == b'>')
                        && (i + 1 >= bytes.len()
                            || (bytes[i + 1] != b'=' && bytes[i + 1] != b'>')) =>
                    {
                        let left = self.eval_arith_expr_impl(&expr[..i]);
                        let right = self.eval_arith_expr_impl(&expr[i + 1..]);
                        return if left > right { 1 } else { 0 };
                    }
                    _ => {}
                }
            }
        }

        // Handle bitwise shift << and >>
        {
            let bytes = expr.as_bytes();
            let mut depth = 0i32;
            let mut i = bytes.len();
            while i > 1 {
                i -= 1;
                match bytes[i] {
                    b')' | b']' => depth += 1,
                    b'(' | b'[' => depth -= 1,
                    b'<' if depth == 0
                        && i > 0
                        && bytes[i - 1] == b'<'
                        && (i + 1 >= bytes.len() || bytes[i + 1] != b'=') =>
                    {
                        let left = self.eval_arith_expr_impl(&expr[..i - 1]);
                        let right = self.eval_arith_expr_impl(&expr[i + 1..]);
                        return left << right;
                    }
                    b'>' if depth == 0
                        && i > 0
                        && bytes[i - 1] == b'>'
                        && (i + 1 >= bytes.len() || bytes[i + 1] != b'=') =>
                    {
                        let left = self.eval_arith_expr_impl(&expr[..i - 1]);
                        let right = self.eval_arith_expr_impl(&expr[i + 1..]);
                        return left >> right;
                    }
                    _ => {}
                }
            }
        }

        // Handle addition/subtraction at top level
        {
            let bytes = expr.as_bytes();
            let mut depth = 0i32;
            let mut i = bytes.len();
            while i > 0 {
                i -= 1;
                match bytes[i] {
                    b')' | b']' => depth += 1,
                    b'(' | b'[' => depth -= 1,
                    b'+' | b'-' if depth == 0 && i > 0 => {
                        // Look past whitespace to find the real previous character
                        let effective_prev = {
                            let mut j = i - 1;
                            while j > 0 && bytes[j].is_ascii_whitespace() {
                                j -= 1;
                            }
                            bytes[j]
                        };
                        let next = if i + 1 < bytes.len() {
                            bytes[i + 1]
                        } else {
                            b' '
                        };
                        // Check if prev is ++ or -- after a variable (post-increment)
                        // e.g., in "a+++4" or "a ++ + 4"
                        let is_after_postop = matches!(effective_prev, b'+' | b'-') && {
                            // Find where the effective_prev is
                            let mut j = i - 1;
                            while j > 0 && bytes[j].is_ascii_whitespace() {
                                j -= 1;
                            }
                            // j points to the second +/- of ++/--
                            // Check if there's a matching +/- before it
                            if j > 0 && bytes[j - 1] == effective_prev && j >= 2 {
                                // Skip whitespace before the ++ or --
                                let mut k = j - 2;
                                while k > 0 && bytes[k].is_ascii_whitespace() {
                                    k -= 1;
                                }
                                // Check if there's a variable name before (not a digit)
                                bytes[k].is_ascii_alphabetic()
                                    || bytes[k] == b'_'
                                    || bytes[k] == b']'
                            } else {
                                false
                            }
                        };
                        // Skip ++ or -- or after an operator (but not if after post-increment)
                        if (!matches!(
                            effective_prev,
                            b'+' | b'-'
                                | b'*'
                                | b'/'
                                | b'%'
                                | b'('
                                | b'<'
                                | b'>'
                                | b'='
                                | b'!'
                                | b'~'
                                | b'&'
                                | b'|'
                        ) || is_after_postop)
                            && (next != bytes[i] || {
                                // Allow split when ++ or -- is followed by a variable
                                // e.g., "4+++a" splits as "4" + "++a"
                                // The right side starts at i+1, the ++ is at i+1..i+3,
                                // so the variable starts at i+3 (or after any whitespace)
                                let mut after_op = i + 3;
                                while after_op < bytes.len()
                                    && bytes[after_op].is_ascii_whitespace()
                                {
                                    after_op += 1;
                                }
                                after_op < bytes.len()
                                    && (bytes[after_op].is_ascii_alphabetic()
                                        || bytes[after_op] == b'_'
                                        || bytes[after_op] == b'$'
                                        || bytes[after_op] == b'(')
                            })
                        {
                            let left = self.eval_arith_expr_impl(&expr[..i]);
                            let right = self.eval_arith_expr_impl(&expr[i + 1..]);
                            return if bytes[i] == b'+' {
                                left.wrapping_add(right)
                            } else {
                                left.wrapping_sub(right)
                            };
                        }
                    }
                    _ => {}
                }
            }
        }

        // Handle multiplication/division/modulo at top level
        {
            let bytes = expr.as_bytes();
            let mut depth = 0i32;
            let mut i = bytes.len();
            while i > 0 {
                i -= 1;
                match bytes[i] {
                    b')' | b']' => depth += 1,
                    b'(' | b'[' => depth -= 1,
                    b'*' | b'/' | b'%' if depth == 0 && i > 0 => {
                        if bytes[i] == b'*' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
                            continue;
                        }
                        if bytes[i] == b'*' && i > 0 && bytes[i - 1] == b'*' {
                            continue;
                        }
                        // Don't split if left side is empty/whitespace-only
                        let left_str = expr[..i].trim();
                        if left_str.is_empty() {
                            continue;
                        }
                        let left = self.eval_arith_expr_impl(&expr[..i]);
                        let right = self.eval_arith_expr_impl(&expr[i + 1..]);
                        return match bytes[i] {
                            b'*' => left.wrapping_mul(right),
                            b'/' => {
                                if right == 0 {
                                    let top_expr = self.arith_top_expr.as_deref().unwrap_or(expr);
                                    let error_token = expr[i + 1..].trim_start();
                                    eprintln!(
                                        "{}: {}{}: division by 0 (error token is \"{}\")",
                                        self.arith_error_prefix(),
                                        self.arith_cmd_prefix(),
                                        top_expr,
                                        error_token
                                    );
                                    crate::expand::set_arith_error();
                                    0
                                } else {
                                    left.wrapping_div(right)
                                }
                            }
                            b'%' => {
                                if right == 0 || (left == i64::MIN && right == -1) {
                                    if right != 0 {
                                        // MIN % -1 = 0 in bash
                                        0
                                    } else {
                                        let top_expr =
                                            self.arith_top_expr.as_deref().unwrap_or(expr);
                                        let error_token = expr[i + 1..].trim_start();
                                        eprintln!(
                                            "{}: {}{}: division by 0 (error token is \"{}\")",
                                            self.arith_error_prefix(),
                                            self.arith_cmd_prefix(),
                                            top_expr,
                                            error_token
                                        );
                                        crate::expand::set_arith_error();
                                        0
                                    }
                                } else {
                                    left % right
                                }
                            }
                            _ => unreachable!(),
                        };
                    }
                    _ => {}
                }
            }
        }

        // Handle exponentiation
        if let Some(pos) = Self::find_top_level_arith_op(expr, "**") {
            let base = self.eval_arith_expr_impl(&expr[..pos]);
            let exp = self.eval_arith_expr_impl(&expr[pos + 2..]);
            if exp < 0 {
                eprintln!(
                    "{}: {}: exponent less than 0 (error token is \"{}\")",
                    self.arith_error_prefix(),
                    self.arith_top_expr.as_deref().unwrap_or(expr),
                    expr[pos + 2..].trim_start_matches('-')
                );
                crate::expand::set_arith_error();
                return 0;
            }
            return base.wrapping_pow(exp as u32);
        }

        // Unary operators
        if let Some(stripped) = expr.strip_prefix('-') {
            return self.eval_arith_expr_impl(stripped).wrapping_neg();
        }
        if let Some(stripped) = expr.strip_prefix('+') {
            return self.eval_arith_expr_impl(stripped);
        }
        if let Some(stripped) = expr.strip_prefix('!') {
            return if self.eval_arith_expr_impl(stripped) == 0 {
                1
            } else {
                0
            };
        }
        if let Some(stripped) = expr.strip_prefix('~') {
            return !self.eval_arith_expr_impl(stripped);
        }

        // Variable lookup or number literal
        let expr = expr.trim();
        if expr.is_empty() {
            return 0;
        }

        // $var and ${var} reference — strip $ and treat as variable name
        // But NOT when arith_is_let is true: literal $ from single-quoted
        // let args (e.g. let 'jv += $iv') should produce an error.
        // Also skip when preceded by backslash (\$var from $(( \$iv ))):
        // the backslash was preserved by expand_comsubs_in_arith to signal
        // that $ is literal.
        if !self.arith_is_let
            && !expr.trim().starts_with("\\$")
            && let Some(stripped) = expr.strip_prefix('$')
        {
            let name = stripped.trim();
            if name == "?" {
                return self.last_status as i64;
            }
            if name == "$" || name == "{$}" {
                return std::process::id() as i64;
            }
            // Handle ${var} syntax
            let name = if let Some(inner) = name.strip_prefix('{').and_then(|s| s.strip_suffix('}'))
            {
                inner
            } else {
                name
            };
            if !name.is_empty()
                && name
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                && name.chars().all(|c| c.is_alphanumeric() || c == '_')
            {
                let val = self.vars.get(name).cloned().unwrap_or_else(|| {
                    // For array variables, bare name resolves to element [0]
                    self.arrays
                        .get(name)
                        .and_then(|a| a.first())
                        .and_then(|v| v.clone())
                        .unwrap_or_default()
                });
                if val.is_empty() {
                    return 0;
                }
                if let Ok(n) = val.parse::<i64>() {
                    return n;
                }
                return self.eval_arith_expr_impl(&val);
            }
        }

        // Variable reference
        if expr
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && expr.chars().all(|c| c.is_alphanumeric() || c == '_')
        {
            // Special dynamic variables
            if expr == "RANDOM" {
                return crate::expand::next_random() as i64;
            }
            // Check for nounset (-u): unset variables in arithmetic are errors
            if self.opt_nounset && !self.vars.contains_key(expr) && std::env::var(expr).is_err() {
                let name = self
                    .vars
                    .get("_BASH_SOURCE_FILE")
                    .or_else(|| self.positional.first())
                    .map(|s| s.as_str())
                    .unwrap_or("bash");
                let lineno = self
                    .vars
                    .get("LINENO")
                    .and_then(|s| s.parse::<i64>().ok())
                    .unwrap_or(0);
                eprintln!("{}: line {}: {}: unbound variable", name, lineno, expr);
                // nounset errors cause the shell/subshell to exit
                std::process::exit(1);
            }
            let val = self.vars.get(expr).cloned().unwrap_or_else(|| {
                // For array variables, bare name resolves to element [0]
                self.arrays
                    .get(expr)
                    .and_then(|a| a.first())
                    .and_then(|v| v.clone())
                    .unwrap_or_default()
            });
            if val.is_empty() {
                return 0;
            }
            // If the variable's value is itself a valid expression, evaluate it
            if let Ok(n) = val.parse::<i64>() {
                return n;
            }
            return self.eval_arith_expr_impl(&val);
        }

        // Number literal
        if let Some(hex) = expr.strip_prefix("0x").or_else(|| expr.strip_prefix("0X")) {
            return i64::from_str_radix(hex.trim(), 16).unwrap_or(0);
        }
        // Base#value notation: e.g., 8#52, 16#2a, 2#1010
        if let Some(hash_pos) = expr.find('#') {
            let base_str = &expr[..hash_pos];
            let value_str = expr[hash_pos + 1..].trim();
            if let Ok(base) = base_str.parse::<u32>() {
                if !(2..=64).contains(&base) {
                    let msg = if base < 2 {
                        "invalid number"
                    } else {
                        "invalid arithmetic base"
                    };
                    eprintln!(
                        "{}: {}: {} (error token is \"{}\")",
                        self.arith_error_prefix(),
                        expr,
                        msg,
                        expr
                    );
                    crate::expand::set_arith_error();
                    return 0;
                }
                if value_str.is_empty() {
                    eprintln!(
                        "{}: {}: invalid integer constant (error token is \"{}\")",
                        self.arith_error_prefix(),
                        expr,
                        expr
                    );
                    crate::expand::set_arith_error();
                    return 0;
                }
                // Check for invalid chars like extra # in value
                if value_str.contains('#') {
                    eprintln!(
                        "{}: {}: invalid number (error token is \"{}\")",
                        self.arith_error_prefix(),
                        expr,
                        expr
                    );
                    crate::expand::set_arith_error();
                    return 0;
                }
                if base <= 36 {
                    return i64::from_str_radix(value_str, base).unwrap_or_else(|_| {
                        eprintln!(
                            "{}: {}: value too great for base (error token is \"{}\")",
                            self.arith_error_prefix(),
                            expr,
                            expr
                        );
                        crate::expand::set_arith_error();
                        0
                    });
                }
                // Bases 37-64: digits are 0-9, a-z, A-Z, @, _
                let mut result: i64 = 0;
                for ch in value_str.chars() {
                    let digit = match ch {
                        '0'..='9' => ch as u32 - '0' as u32,
                        'a'..='z' => ch as u32 - 'a' as u32 + 10,
                        'A'..='Z' => ch as u32 - 'A' as u32 + 36,
                        '@' => 62,
                        '_' => 63,
                        _ => {
                            eprintln!(
                                "{}: {}: value too great for base (error token is \"{}\")",
                                self.arith_error_prefix(),
                                expr,
                                expr
                            );
                            crate::expand::set_arith_error();
                            return 0;
                        }
                    };
                    if digit >= base {
                        eprintln!(
                            "{}: {}: value too great for base (error token is \"{}\")",
                            self.arith_error_prefix(),
                            expr,
                            expr
                        );
                        crate::expand::set_arith_error();
                        return 0;
                    }
                    result = result * base as i64 + digit as i64;
                }
                return result;
            }
        }
        if expr.starts_with('0')
            && expr.len() > 1
            && expr.chars().skip(1).all(|c| c.is_ascii_digit())
        {
            return i64::from_str_radix(&expr[1..], 8).unwrap_or(0);
        }
        if let Ok(n) = expr.parse::<i64>() {
            return n;
        }
        // Handle overflow: large decimal numbers wrap (like C unsigned → signed)
        if expr.chars().all(|c| c.is_ascii_digit())
            && !expr.is_empty()
            && let Ok(n) = expr.parse::<u64>()
        {
            return n as i64; // wrapping cast
        }

        // Array element: arr[idx]
        if let Some(bracket) = expr.find('[') {
            // Use depth-aware bracket matching to find the closing ']' that
            // matches the first '['.  This correctly handles expanded subscript
            // values that themselves contain ']' (e.g. a[$key] where $key
            // expanded to 'x],b[...]').
            let mut depth = 0i32;
            let mut close = None;
            for (idx, ch) in expr[bracket..].char_indices() {
                match ch {
                    '[' => depth += 1,
                    ']' => {
                        depth -= 1;
                        if depth == 0 {
                            close = Some(bracket + idx);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let close = match close {
                Some(c) => c,
                None => {
                    // No matching ']' found — this is a bad array subscript.
                    // Bash reports "bad array subscript" with the error token
                    // starting from the identifier before '['.
                    // e.g. `b[$(echo` → "bad array subscript (error token is "b[$(echo")"
                    let top_expr = self.arith_top_expr.as_deref().unwrap_or(expr);
                    let error_token = expr;
                    eprintln!(
                        "{}: {}: bad array subscript (error token is \"{}\")",
                        self.arith_error_prefix(),
                        top_expr,
                        error_token
                    );
                    crate::expand::set_arith_error();
                    return 0;
                }
            };
            if close <= bracket + 1 {
                return 0;
            }
            // Check for extra text after arr[idx] (e.g., b[c]d → "d" is extra)
            let after_bracket = &expr[close + 1..].trim();
            if !after_bracket.is_empty() {
                let top_expr = self.arith_top_expr.as_deref().unwrap_or(expr);
                eprintln!(
                    "{}: {}: arithmetic syntax error in expression (error token is \"{}\")",
                    self.arith_error_prefix(),
                    top_expr,
                    after_bracket
                );
                crate::expand::set_arith_error();
                return 0;
            }
            let name = &expr[..bracket];
            let idx_str = &expr[bracket + 1..close];
            // For indexed array subscript evaluation, temporarily set the
            // top expression to the subscript value so errors report the
            // subscript content (matching bash behavior where e.g.
            // `a[$key]` with key='x],b' shows 'x],b' as the expression).
            let saved_top = self.arith_top_expr.take();
            let sub = self.arith_subscript_key(name, idx_str);
            self.arith_top_expr = saved_top;
            if crate::expand::take_arith_error() {
                crate::expand::set_arith_error();
                return 0;
            }
            let val = self.arith_array_get(&sub);
            // Check nounset for unset array elements (value == 0 and no entry)
            if val == 0 && self.opt_nounset {
                let resolved = match &sub {
                    ArithSubscript::Assoc(r, _) | ArithSubscript::Indexed(r, _) => r.as_str(),
                };
                if !self.arrays.contains_key(resolved)
                    && !self.assoc_arrays.contains_key(resolved)
                    && !self.vars.contains_key(resolved)
                {
                    let src_name = self
                        .vars
                        .get("_BASH_SOURCE_FILE")
                        .or_else(|| self.positional.first())
                        .map(|s| s.as_str())
                        .unwrap_or("bash");
                    let lineno = self
                        .vars
                        .get("LINENO")
                        .and_then(|s| s.parse::<i64>().ok())
                        .unwrap_or(0);
                    eprintln!(
                        "{}: line {}: {}: unbound variable",
                        src_name, lineno, resolved
                    );
                    std::process::exit(1);
                }
            }
            return val;
        }

        // Fall back to reporting error
        // Check if this looks like "valid_expr extra_stuff" — syntax error in expression
        let trimmed = expr.trim();

        let first_word_end = trimmed
            .find(|c: char| c.is_whitespace() || c == '[')
            .unwrap_or(trimmed.len());
        let first_word = &trimmed[..first_word_end];
        let has_extra = first_word_end < trimmed.len()
            && (first_word.chars().all(|c| c.is_alphanumeric() || c == '_')
                || first_word.contains('['));
        if has_extra {
            // Find the extra text after the valid part
            let rest = trimmed[first_word_end..].trim_start();
            // Check for array subscript: skip past ]
            let rest = if first_word.contains('[') {
                if let Some(close) = trimmed[first_word_end..].find(']') {
                    trimmed[first_word_end + close + 1..].trim_start()
                } else {
                    rest
                }
            } else {
                rest
            };
            if !rest.is_empty() {
                let top_expr = self.arith_top_expr.as_deref().unwrap_or(expr);
                // Find the error token in the top expression (preserves trailing space)
                let error_token = if let Some(pos) = top_expr.find(rest) {
                    &top_expr[pos..]
                } else {
                    rest
                };
                eprintln!(
                    "{}: {}{}: arithmetic syntax error in expression (error token is \"{}\")",
                    self.arith_error_prefix(),
                    self.arith_cmd_prefix(),
                    top_expr,
                    error_token
                );
                crate::expand::set_arith_error();
                return 0;
            }
        }
        let top_expr = self.arith_top_expr.as_deref().unwrap_or(expr);
        // Use the current (inner) expression as error token when it differs
        // from the top-level expression — e.g. for `let 'jv += $iv'` the
        // error token should be "$iv", not the whole "jv += $iv".
        // When the inner expression is a suffix of the top expression, use
        // the suffix from the top expression to preserve trailing whitespace
        // (bash includes trailing space in error tokens like "$iv ").
        let trimmed_expr = expr.trim_start().replace("\\$", "$");
        let error_token_owned: String;
        let error_token = if top_expr != trimmed_expr && !trimmed_expr.is_empty() {
            // Check if the trimmed inner expr is a suffix of the top expr
            // (ignoring trailing whitespace on the inner expr). If so, use
            // the top-expr suffix which preserves trailing space.
            if let Some(pos) = top_expr.find(trimmed_expr.trim_end()) {
                &top_expr[pos..]
            } else {
                error_token_owned = trimmed_expr;
                error_token_owned.as_str()
            }
        } else {
            top_expr
        };
        eprintln!(
            "{}: {}{}: arithmetic syntax error: operand expected (error token is \"{}\")",
            self.arith_error_prefix(),
            self.arith_cmd_prefix(),
            top_expr,
            error_token
        );
        crate::expand::set_arith_error();
        0
    }

    /// Expand command substitutions $(...) and $var within an arithmetic expression string.
    pub(super) fn expand_comsubs_in_arith(&mut self, expr: &str) -> String {
        // When arith_is_let is true, the expression came from `let` with a
        // literal $ (e.g. let 'jv += $iv').  Don't expand $var references —
        // the $ should be passed through so the arithmetic evaluator produces
        // an "operand expected" error, matching bash behaviour.
        if self.arith_is_let {
            return expr.to_string();
        }
        let mut result = String::new();
        let chars: Vec<char> = expr.chars().collect();
        let mut i = 0;
        // Track whether we are inside an array subscript `[...]` so we can
        // skip $var expansion there.  For both associative AND indexed arrays,
        // the raw $var text must reach the arithmetic evaluator's
        // `arith_subscript_key` which does its own context-aware expansion.
        // Expanding $var here would embed the expanded value (which may
        // contain `]`, `[`, or `$(...)`) into the expression, breaking
        // bracket matching and causing spurious command execution.
        let mut array_bracket_depth: i32 = 0;
        // When true, we are inside `[...]` brackets preceded by a single-quoted
        // identifier in (( )) context.  $var expansion proceeds but the result
        // is backslash-escaped (`]`, `[`, `$`) to protect bracket matching and
        // match bash's error message format.
        let mut squote_bracket_escape = false;
        while i < chars.len() {
            // Detect `name[` where `name` is any array (or potential array variable).
            // Skip $var expansion inside [..] so that expanded values containing
            // `]` don't break bracket matching in the arithmetic evaluator.
            // The subscript content will be expanded later by `arith_subscript_key`.
            if chars[i] == '[' && (i == 0 || chars[i - 1] != '$') {
                if array_bracket_depth > 0 {
                    // Already inside a subscript — nested bracket
                    array_bracket_depth += 1;
                } else {
                    // Check if the identifier before `[` is a variable name
                    let name_end = i;
                    let mut name_start = name_end;
                    while name_start > 0
                        && (chars[name_start - 1].is_alphanumeric() || chars[name_start - 1] == '_')
                    {
                        name_start -= 1;
                    }
                    // Don't activate bracket protection if the identifier is
                    // preceded by `'` — single quotes are literal in (( ))
                    // arithmetic, so this is not a real array subscript access.
                    // $var should be expanded (and backslash-escaped) for the
                    // error message to match bash.
                    let preceded_by_squote = name_start > 0 && chars[name_start - 1] == '\'';
                    if name_start < name_end && !preceded_by_squote {
                        array_bracket_depth = 1;
                    } else if name_start < name_end && preceded_by_squote {
                        // Inside single-quoted (( )) expression — enter bracket
                        // tracking but with escape mode: $var expansion will
                        // proceed but results will be backslash-escaped.
                        array_bracket_depth = 1;
                        squote_bracket_escape = true;
                    }
                }
            } else if chars[i] == ']' && array_bracket_depth > 0 {
                array_bracket_depth -= 1;
                if array_bracket_depth == 0 {
                    squote_bracket_escape = false;
                }
                result.push(chars[i]);
                i += 1;
                continue;
            }
            // \$ → keep as literal \$ (skip expansion). The arithmetic
            // evaluator will strip the backslash later for display but
            // will NOT treat $ as a variable prefix.
            if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1] == '$' {
                result.push('\\');
                result.push('$');
                i += 2;
                continue;
            }
            // Handle ${ command; } funsub
            // Skip when inside array subscript brackets to prevent injection
            if i + 2 < chars.len()
                && chars[i] == '$'
                && chars[i + 1] == '{'
                && chars[i + 2] == ' '
                && array_bracket_depth == 0
            {
                // Find matching }
                let mut depth = 1i32;
                let mut j = i + 2;
                while j < chars.len() && depth > 0 {
                    match chars[j] {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        '\'' => {
                            j += 1;
                            while j < chars.len() && chars[j] != '\'' {
                                j += 1;
                            }
                        }
                        '"' => {
                            j += 1;
                            while j < chars.len() && chars[j] != '"' {
                                if chars[j] == '\\' && j + 1 < chars.len() {
                                    j += 1;
                                }
                                j += 1;
                            }
                        }
                        _ => {}
                    }
                    j += 1;
                }
                let cmd: String = chars[i + 2..j].iter().collect();
                let output = self.capture_output_nofork(&cmd);
                result.push_str(output.trim());
                i = j + 1;
                continue;
            }
            if i + 2 < chars.len() && chars[i] == '$' && chars[i + 1] == '(' && chars[i + 2] == '('
            {
                // $((arith)) — nested arithmetic expansion
                // NOTE: unlike $(...) command subs and ${ } funsubs, $((expr))
                // is pure arithmetic and safe to expand inside array subscript
                // brackets.  e.g. ${a[$(( 0 ))]} should evaluate the inner
                // arithmetic and use the result as the subscript index.
                // Recursively expand comsubs inside, then evaluate as arithmetic
                // Find the matching ))
                let mut depth = 1i32;
                let mut j = i + 3;
                while j < chars.len() {
                    if chars[j] == '(' && j > 0 && chars[j - 1] == '$' {
                        // Nested $( inside arithmetic
                        depth += 1;
                    } else if chars[j] == '(' {
                        depth += 1;
                    } else if chars[j] == ')'
                        && j + 1 < chars.len()
                        && chars[j + 1] == ')'
                        && depth == 1
                    {
                        // Found matching ))
                        let inner: String = chars[i + 3..j].iter().collect();
                        let expanded = self.expand_comsubs_in_arith(&inner);
                        let val = self.eval_arith_expr(&expanded);
                        result.push_str(&val.to_string());
                        i = j + 2;
                        break;
                    } else if chars[j] == ')' {
                        depth -= 1;
                    }
                    j += 1;
                }
                if i <= j {
                    // Didn't find matching )) — just pass through
                    continue;
                }
                continue;
            }
            if i + 1 < chars.len()
                && chars[i] == '$'
                && chars[i + 1] == '('
                && array_bracket_depth == 0
            {
                // Find matching closing paren with case/esac and quote awareness
                let mut depth = 0i32;
                let mut case_depth = 0i32;
                let mut j = i + 1;
                while j < chars.len() {
                    match chars[j] {
                        '\'' => {
                            j += 1;
                            while j < chars.len() && chars[j] != '\'' {
                                j += 1;
                            }
                            if j < chars.len() {
                                j += 1;
                            }
                            continue;
                        }
                        '"' => {
                            j += 1;
                            while j < chars.len() && chars[j] != '"' {
                                if chars[j] == '\\' && j + 1 < chars.len() {
                                    j += 1;
                                }
                                j += 1;
                            }
                            if j < chars.len() {
                                j += 1;
                            }
                            continue;
                        }
                        '(' => depth += 1,
                        ')' => {
                            if case_depth <= 0 {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                        }
                        _ => {}
                    }
                    // Track case/esac keywords
                    if chars[j].is_alphabetic() {
                        let mut word = String::new();
                        while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
                            word.push(chars[j]);
                            j += 1;
                        }
                        if word == "case" {
                            case_depth += 1;
                        } else if word == "esac" {
                            case_depth -= 1;
                        }
                        continue;
                    }
                    j += 1;
                }
                // Extract the command inside $(...)
                let cmd: String = chars[i + 2..j].iter().collect();
                let output = self.capture_output(&cmd);
                result.push_str(output.trim());
                i = j + 1;
            } else if chars[i] == '$'
                && i + 1 < chars.len()
                && chars[i + 1] == '{'
                && (i + 2 >= chars.len() || chars[i + 2] != ' ')
            {
                // ${...} parameter expansion — find matching } and expand
                // BUT: if we're inside array [...] brackets,
                // leave unexpanded for arith_subscript_key to handle —
                // UNLESS squote_bracket_escape is set (single-quoted (( ))
                // context where $var should be expanded with escaping).
                if array_bracket_depth > 0 && !squote_bracket_escape {
                    result.push(chars[i]);
                    i += 1;
                    continue;
                }
                let start = i;
                i += 2; // skip ${
                let mut depth = 1;
                while i < chars.len() && depth > 0 {
                    match chars[i] {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                if i < chars.len() {
                    i += 1; // skip closing }
                }
                let param_text: String = chars[start..i].iter().collect();
                let expanded =
                    self.expand_word_single(&crate::lexer::parse_word_string(&param_text));
                if squote_bracket_escape {
                    // Backslash-escape ], [, $ in the expanded value to protect
                    // bracket matching and match bash's error message format.
                    for ch in expanded.chars() {
                        if matches!(ch, ']' | '[' | '$') {
                            result.push('\\');
                        }
                        result.push(ch);
                    }
                } else {
                    result.push_str(&expanded);
                }
            } else if chars[i] == '$'
                && i + 1 < chars.len()
                && matches!(chars[i + 1], '#' | '?' | '$' | '!' | '-' | '@' | '*')
            {
                // Special parameter: $#, $?, $$, $!, $-, $@, $*
                let val = match chars[i + 1] {
                    '#' => (self.positional.len().saturating_sub(1)).to_string(),
                    '?' => self.last_status.to_string(),
                    '$' => std::process::id().to_string(),
                    '!' => self.last_bg_pid.to_string(),
                    '-' => self.get_opt_flags().to_string(),
                    '@' | '*' => self.positional[1..].join(" "),
                    _ => String::new(),
                };
                result.push_str(&val);
                i += 2;
            } else if chars[i] == '$' && i + 1 < chars.len() && chars[i + 1].is_ascii_digit() {
                // Positional parameter: $0, $1, etc.
                let idx = (chars[i + 1] as u8 - b'0') as usize;
                let val = self.positional.get(idx).cloned().unwrap_or_default();
                result.push_str(&val);
                i += 2;
            } else if chars[i] == '$'
                && i + 1 < chars.len()
                && (chars[i + 1].is_ascii_alphabetic() || chars[i + 1] == '_')
                && !(i > 0 && chars[i - 1] == '\\')
            {
                // Simple variable: $var — expand using word expansion (skip if preceded by \)
                // BUT: if we're inside array [...] brackets,
                // leave the $var unexpanded so arith_subscript_key can
                // handle it properly (the expanded value may contain ]
                // characters that would break bracket matching).
                // UNLESS squote_bracket_escape is set (single-quoted (( ))
                // context where $var should be expanded with escaping).
                if array_bracket_depth > 0 && !squote_bracket_escape {
                    result.push(chars[i]);
                    i += 1;
                    continue;
                }
                let start = i;
                i += 1;
                let mut name = String::new();
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    name.push(chars[i]);
                    i += 1;
                }
                // Use full word expansion for dynamic variables like $RANDOM
                let param_text: String = chars[start..i].iter().collect();
                let expanded =
                    self.expand_word_single(&crate::lexer::parse_word_string(&param_text));
                if squote_bracket_escape {
                    // Backslash-escape ], [, $ in the expanded value to protect
                    // bracket matching and match bash's error message format.
                    for ch in expanded.chars() {
                        if matches!(ch, ']' | '[' | '$') {
                            result.push('\\');
                        }
                        result.push(ch);
                    }
                } else {
                    result.push_str(&expanded);
                }
            } else if chars[i] == '`' && array_bracket_depth == 0 {
                // Backtick command substitution: `...`
                let start = i;
                i += 1; // skip opening backtick
                let mut cmd = String::new();
                while i < chars.len() && chars[i] != '`' {
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        // In backtick comsubs, \` is an escaped backtick
                        if chars[i + 1] == '`' || chars[i + 1] == '\\' || chars[i + 1] == '$' {
                            cmd.push(chars[i + 1]);
                            i += 2;
                            continue;
                        }
                    }
                    cmd.push(chars[i]);
                    i += 1;
                }
                if i < chars.len() {
                    i += 1; // skip closing backtick
                }
                let output = self.capture_output(&cmd);
                result.push_str(output.trim_end_matches('\n'));
                let _ = start; // suppress unused warning
            } else {
                result.push(chars[i]);
                i += 1;
            }
        }
        result
    }
}
