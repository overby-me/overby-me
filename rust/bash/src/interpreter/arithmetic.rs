use super::*;

impl Shell {
    /// Evaluate an arithmetic expression and return the integer result.
    ///
    /// Find an operator in the expression at top-level (outside parentheses).
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
            for ch in expr.chars() {
                match ch {
                    '(' => paren_depth += 1,
                    ')' => paren_depth -= 1,
                    _ => {}
                }
            }
            if paren_depth > 0 {
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
        let expanded_cs: String;
        let expr = if expr.contains('$') || expr.contains('`') {
            expanded_cs = self.expand_comsubs_in_arith(expr);
            // Update top expression with expanded version for error messages
            if self.arith_depth == 1
                && let Some(ref mut top) = self.arith_top_expr
                && top.contains('$')
            {
                *top = expanded_cs.trim_start().replace("\\$", "$");
            }
            &expanded_cs
        } else {
            expr
        };

        // Strip double quotes from arith expressions (bash behavior)
        let unquoted: String;
        let expr = if expr.contains('"') {
            unquoted = expr.replace('"', "");
            // Update top expression to match stripped version for error messages
            if self.arith_depth == 1
                && let Some(ref mut top) = self.arith_top_expr
                && top.contains('"')
            {
                *top = top.replace('"', "");
            }
            &unquoted
        } else {
            expr
        };

        // Check for leading operators that can't start an expression (/, *, %)
        // These indicate "operand expected" — report once for the full expression
        if self.arith_depth == 1 {
            let trimmed = expr.trim();
            if !trimmed.is_empty() {
                let first = trimmed.as_bytes()[0];
                if matches!(first, b'/' | b'%') {
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
                        "{}: {}: arithmetic syntax error: operand expected (error token is \"{}\")",
                        self.arith_error_prefix(),
                        display_expr,
                        error_token
                    );
                    crate::expand::set_arith_error();
                    return 0;
                }
            }
        }

        // Handle comma operator (only at top level, not inside parens)
        {
            let mut depth = 0i32;
            let mut last_comma = None;
            for (i, ch) in expr.char_indices() {
                match ch {
                    '(' => depth += 1,
                    ')' => depth -= 1,
                    ',' if depth == 0 => last_comma = Some(i),
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
                        let resolved = self.resolve_nameref(base);
                        let idx = self.eval_arith_expr_impl(idx_str) as usize;
                        let arr = self.arrays.entry(resolved).or_default();
                        while arr.len() <= idx {
                            arr.push(None);
                        }
                        let lhs: i64 = arr[idx]
                            .as_deref()
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(0);
                        let result = func(lhs, rhs);
                        arr[idx] = Some(result.to_string());
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
                    let resolved = self.resolve_nameref(base);
                    let idx = self.eval_arith_expr_impl(idx_str) as usize;
                    let arr = self.arrays.entry(resolved).or_default();
                    while arr.len() <= idx {
                        arr.push(None);
                    }
                    arr[idx] = Some(val.to_string());
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
                    let resolved = self.resolve_nameref(base);
                    let idx = self.eval_arith_expr(idx_str) as usize;
                    let arr = self.arrays.entry(resolved).or_default();
                    while arr.len() <= idx {
                        arr.push(None);
                    }
                    let val: i64 = arr[idx]
                        .as_deref()
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0);
                    arr[idx] = Some((val + delta).to_string());
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
                    let idx = self.eval_arith_expr(idx_expr) as usize;
                    let val: i64 = self
                        .arrays
                        .get(base)
                        .and_then(|a| a.get(idx))
                        .and_then(|v| v.as_deref())
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0);
                    let new_val = val + 1;
                    let arr = self.arrays.entry(base.to_string()).or_default();
                    while arr.len() <= idx {
                        arr.push(None);
                    }
                    arr[idx] = Some(new_val.to_string());
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
                    let idx = self.eval_arith_expr(idx_expr) as usize;
                    let val: i64 = self
                        .arrays
                        .get(base)
                        .and_then(|a| a.get(idx))
                        .and_then(|v| v.as_deref())
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0);
                    let new_val = val - 1;
                    let arr = self.arrays.entry(base.to_string()).or_default();
                    while arr.len() <= idx {
                        arr.push(None);
                    }
                    arr[idx] = Some(new_val.to_string());
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
                    b')' => depth += 1,
                    b'(' => depth -= 1,
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
                    b')' => depth += 1,
                    b'(' => depth -= 1,
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
                    b')' => depth += 1,
                    b'(' => depth -= 1,
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
                    b')' => depth += 1,
                    b'(' => depth -= 1,
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
                    b')' => depth += 1,
                    b'(' => depth -= 1,
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
                    b')' => depth += 1,
                    b'(' => depth -= 1,
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
                    b')' => depth += 1,
                    b'(' => depth -= 1,
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
                    b')' => depth += 1,
                    b'(' => depth -= 1,
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
                    b')' => depth += 1,
                    b'(' => depth -= 1,
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
                let val = self.vars.get(name).cloned().unwrap_or_default();
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
            let val = self.vars.get(expr).cloned().unwrap_or_default();
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
            let close = expr.rfind(']').unwrap_or(expr.len());
            if close <= bracket + 1 {
                return 0;
            }
            // No closing ']' found — not a valid array reference
            if close >= expr.len() {
                let top_expr = self.arith_top_expr.as_deref().unwrap_or(expr);
                eprintln!(
                    "{}: {}: arithmetic syntax error: operand expected (error token is \"{}\")",
                    self.arith_error_prefix(),
                    top_expr,
                    expr
                );
                crate::expand::set_arith_error();
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
            let resolved = self.resolve_nameref(name);
            let idx = self.eval_arith_expr_impl(idx_str) as usize;
            if let Some(arr) = self.arrays.get(&resolved) {
                return arr
                    .get(idx)
                    .and_then(|v| v.as_deref())
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
            }
            // Check nounset for unset array
            if self.opt_nounset && !self.vars.contains_key(&resolved) {
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
                eprintln!("{}: line {}: {}: unbound variable", name, lineno, resolved);
                std::process::exit(1);
            }
            return 0;
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
        while i < chars.len() {
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
            if i + 2 < chars.len() && chars[i] == '$' && chars[i + 1] == '{' && chars[i + 2] == ' '
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
                let output = self.capture_output(&cmd);
                result.push_str(output.trim());
                i = j + 1;
                continue;
            }
            if i + 2 < chars.len() && chars[i] == '$' && chars[i + 1] == '(' && chars[i + 2] == '('
            {
                // $((arith)) — nested arithmetic expansion
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
            if i + 1 < chars.len() && chars[i] == '$' && chars[i + 1] == '(' {
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
                result.push_str(&expanded);
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
                result.push_str(&expanded);
            } else if chars[i] == '`' {
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
