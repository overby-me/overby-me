use super::*;

#[allow(dead_code)]
pub fn eval_arith_full(
    expr: &str,
    vars: &HashMap<String, String>,
    arrays: &HashMap<String, Vec<Option<String>>>,
    positional: &[String],
    last_status: i32,
    opt_flags: &str,
) -> i64 {
    eval_arith_full_with_assoc(
        expr,
        vars,
        arrays,
        &HashMap::new(),
        &HashMap::new(),
        positional,
        last_status,
        opt_flags,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn eval_arith_full_with_assoc(
    expr: &str,
    vars: &HashMap<String, String>,
    arrays: &HashMap<String, Vec<Option<String>>>,
    assoc_arrays: &HashMap<String, crate::interpreter::AssocArray>,
    namerefs: &HashMap<String, String>,
    positional: &[String],
    last_status: i32,
    opt_flags: &str,
) -> i64 {
    let resolved = resolve_arith_vars(
        expr,
        vars,
        arrays,
        assoc_arrays,
        namerefs,
        positional,
        last_status,
        opt_flags,
    );
    match eval_arith(&resolved) {
        Ok(val) => val,
        Err(e) => {
            let name = vars
                .get("_BASH_SOURCE_FILE")
                .or_else(|| positional.first())
                .map(|s| s.as_str())
                .unwrap_or("bash");
            let lineno = vars.get("LINENO").map(|s| s.as_str()).unwrap_or("0");
            // Error from eval_arith is already fully formatted with error token
            eprintln!("{}: line {}: {}: {}", name, lineno, expr.trim(), e);
            ARITH_ERROR.with(|f| *f.borrow_mut() = true);
            0
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn resolve_arith_vars(
    expr: &str,
    vars: &HashMap<String, String>,
    arrays: &HashMap<String, Vec<Option<String>>>,
    assoc_arrays: &HashMap<String, crate::interpreter::AssocArray>,
    namerefs: &HashMap<String, String>,
    positional: &[String],
    last_status: i32,
    opt_flags: &str,
) -> String {
    let mut result = String::new();
    let chars: Vec<char> = expr.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '$' {
            i += 1;
            // Handle special parameters: $#, $?, $$, $!, $-
            if i < chars.len() && matches!(chars[i], '#' | '?' | '-' | '!') {
                let val = match chars[i] {
                    '#' => (positional.len().saturating_sub(1)).to_string(),
                    '?' => last_status.to_string(),
                    '-' => opt_flags.to_string(),
                    '!' => "0".to_string(),
                    _ => "0".to_string(),
                };
                result.push_str(&val);
                i += 1;
            } else if i < chars.len() && chars[i] == '$' {
                result.push_str(&std::process::id().to_string());
                i += 1;
            } else if i < chars.len() && chars[i] == '{' {
                // ${var}, ${var:-default}, ${var:+alt}, ${#var}, ${-%%pat}, etc.
                i += 1; // skip '{'
                // Check for ${#var} (length) — but not ${#} (param count)
                let is_length = i < chars.len()
                    && chars[i] == '#'
                    && i + 1 < chars.len()
                    && chars[i + 1] != '}';
                if is_length {
                    i += 1;
                }
                let mut name = String::new();
                // Handle special parameter names: -, ?, #, !, $, @, *
                if i < chars.len() && matches!(chars[i], '-' | '?' | '#' | '!' | '$' | '@' | '*') {
                    name.push(chars[i]);
                    i += 1;
                } else {
                    while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                        name.push(chars[i]);
                        i += 1;
                    }
                }
                // Check for subscript [...]
                if i < chars.len() && chars[i] == '[' {
                    name.push('[');
                    i += 1;
                    let mut bracket_depth = 1;
                    while i < chars.len() && bracket_depth > 0 {
                        if chars[i] == '[' {
                            bracket_depth += 1;
                        } else if chars[i] == ']' {
                            bracket_depth -= 1;
                        }
                        name.push(chars[i]);
                        i += 1;
                    }
                }
                let ctx_real = ExpCtx {
                    vars,
                    arrays,
                    assoc_arrays,
                    namerefs,
                    positional,
                    last_status,
                    last_bg_pid: 0,
                    top_level_pid: std::process::id(),
                    opt_flags,
                };
                let raw_val = lookup_var(&name, &ctx_real);
                // Handle operator
                if i < chars.len() && chars[i] == '}' {
                    i += 1; // simple ${var}
                    if is_length {
                        result.push_str(&raw_val.len().to_string());
                    } else {
                        let val = if raw_val.is_empty() {
                            "0".to_string()
                        } else {
                            raw_val
                        };
                        result.push_str(&val);
                    }
                } else if i < chars.len() {
                    // Check for %%, %, ##, # trim operators first
                    if i < chars.len() && (chars[i] == '%' || chars[i] == '#') {
                        let trim_char = chars[i];
                        let is_greedy = i + 1 < chars.len() && chars[i + 1] == trim_char;
                        if is_greedy {
                            i += 2; // skip %% or ##
                        } else {
                            i += 1; // skip % or #
                        }
                        // Read the pattern until closing }
                        let mut pattern = String::new();
                        let mut brace_depth = 1i32;
                        while i < chars.len() && brace_depth > 0 {
                            if chars[i] == '{' {
                                brace_depth += 1;
                            } else if chars[i] == '}' {
                                brace_depth -= 1;
                                if brace_depth == 0 {
                                    i += 1;
                                    break;
                                }
                            }
                            pattern.push(chars[i]);
                            i += 1;
                        }
                        // Apply trim operation
                        let trimmed = if trim_char == '%' {
                            // Remove suffix
                            if is_greedy {
                                crate::expand::pattern::trim_pattern(
                                    &raw_val,
                                    &pattern,
                                    crate::expand::pattern::TrimMode::LargeRight,
                                )
                            } else {
                                crate::expand::pattern::trim_pattern(
                                    &raw_val,
                                    &pattern,
                                    crate::expand::pattern::TrimMode::SmallRight,
                                )
                            }
                        } else {
                            // Remove prefix
                            if is_greedy {
                                crate::expand::pattern::trim_pattern(
                                    &raw_val,
                                    &pattern,
                                    crate::expand::pattern::TrimMode::LargeLeft,
                                )
                            } else {
                                crate::expand::pattern::trim_pattern(
                                    &raw_val,
                                    &pattern,
                                    crate::expand::pattern::TrimMode::SmallLeft,
                                )
                            }
                        };
                        let val = if trimmed.is_empty() {
                            "0".to_string()
                        } else {
                            trimmed
                        };
                        result.push_str(&val);
                    } else {
                        // Parse operator: :-, :+, :=, :?, -, +, =, ?
                        let has_colon = chars[i] == ':';
                        if has_colon {
                            i += 1;
                        }
                        let op_char = if i < chars.len() { chars[i] } else { '}' };
                        if matches!(op_char, '-' | '+' | '=' | '?') {
                            i += 1; // skip operator char
                            // Read the word until closing }
                            let mut word = String::new();
                            let mut brace_depth = 1i32;
                            while i < chars.len() && brace_depth > 0 {
                                if chars[i] == '{' {
                                    brace_depth += 1;
                                } else if chars[i] == '}' {
                                    brace_depth -= 1;
                                    if brace_depth == 0 {
                                        i += 1;
                                        break;
                                    }
                                }
                                word.push(chars[i]);
                                i += 1;
                            }
                            let is_set =
                                !raw_val.is_empty() || (!has_colon && vars.contains_key(&name));
                            let val = match op_char {
                                '-' => {
                                    if is_set {
                                        raw_val.clone()
                                    } else {
                                        word.clone()
                                    }
                                }
                                '+' => {
                                    if is_set {
                                        word.clone()
                                    } else {
                                        String::new()
                                    }
                                }
                                '=' => {
                                    if is_set {
                                        raw_val.clone()
                                    } else {
                                        word.clone()
                                    }
                                }
                                '?' => raw_val.clone(),
                                _ => raw_val.clone(),
                            };
                            let val = if val.is_empty() { "0".to_string() } else { val };
                            result.push_str(&val);
                        } else {
                            // Unknown operator — skip to closing }
                            let mut brace_depth = 1i32;
                            while i < chars.len() && brace_depth > 0 {
                                if chars[i] == '{' {
                                    brace_depth += 1;
                                } else if chars[i] == '}' {
                                    brace_depth -= 1;
                                }
                                i += 1;
                            }
                            let val = if raw_val.is_empty() {
                                "0".to_string()
                            } else {
                                raw_val
                            };
                            result.push_str(&val);
                        }
                    }
                } else {
                    // Unterminated ${
                    let val = if raw_val.is_empty() {
                        "0".to_string()
                    } else {
                        raw_val
                    };
                    result.push_str(&val);
                }
            } else {
                let mut name = String::new();
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    name.push(chars[i]);
                    i += 1;
                }
                let ctx_real = ExpCtx {
                    vars,
                    arrays,
                    assoc_arrays,
                    namerefs,
                    positional,
                    last_status,
                    last_bg_pid: 0,
                    top_level_pid: std::process::id(),
                    opt_flags,
                };
                let val = lookup_var(&name, &ctx_real);
                let val = if val.is_empty() { "0".to_string() } else { val };
                result.push_str(&val);
            }
        } else if chars[i].is_alphabetic() || chars[i] == '_' {
            let mut name = String::new();
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                name.push(chars[i]);
                i += 1;
            }
            // If the name is immediately followed by '$', keep it literal so
            // that the upcoming '$' expansion concatenates with it to form
            // a new variable name.  E.g. in `(_$-=0)`, `_` should stay
            // literal and `$-` expands to `hB`, giving variable name `_hB`.
            if i < chars.len() && chars[i] == '$' {
                result.push_str(&name);
                continue;
            }
            // Check for assignment operators: =, +=, -=, *=, /=, %=, ++, --
            // When an assignment is detected, keep the name literal (it's a
            // target, not a value to resolve).  eval_arith handles the
            // assignment semantics.
            let rest: String = chars[i..].iter().collect();
            if rest.starts_with("++") || rest.starts_with("--") {
                // Post-increment/decrement: keep name literal, eval_arith
                // will handle the side-effect semantics.
                result.push_str(&name);
                result.push_str(&rest[..2]);
                i += 2;
                continue;
            }
            // Simple or compound assignment: var=, var+=, var-=, var*=, var/=, var%=
            if (rest.starts_with('=') && !rest.starts_with("=="))
                || (rest.len() >= 2
                    && matches!(rest.as_bytes()[0], b'+' | b'-' | b'*' | b'/' | b'%')
                    && rest.as_bytes()[1] == b'=')
            {
                result.push_str(&name);
                continue;
            }
            // Check if followed by [...] subscript (array element access)
            let val = if i < chars.len() && chars[i] == '[' {
                // Collect the subscript including brackets
                let mut subscript = String::new();
                subscript.push('[');
                i += 1;
                let mut bracket_depth = 1i32;
                while i < chars.len() && bracket_depth > 0 {
                    if chars[i] == '[' {
                        bracket_depth += 1;
                    } else if chars[i] == ']' {
                        bracket_depth -= 1;
                    }
                    subscript.push(chars[i]);
                    i += 1;
                }
                // subscript is now e.g. "[%]" or "[$k1]"
                let full_name = format!("{}{}", name, subscript);
                // Check for assignment operators after the subscript —
                // if so, keep the name literal for eval_arith to handle
                let rest: String = chars[i..].iter().collect();
                if rest.starts_with("++")
                    || rest.starts_with("--")
                    || (rest.starts_with('=') && !rest.starts_with("=="))
                    || (rest.len() >= 2
                        && matches!(rest.as_bytes()[0], b'+' | b'-' | b'*' | b'/' | b'%')
                        && rest.as_bytes()[1] == b'=')
                {
                    result.push_str(&full_name);
                    continue;
                }
                let ctx_real = ExpCtx {
                    vars,
                    arrays,
                    assoc_arrays,
                    namerefs,
                    positional,
                    last_status,
                    last_bg_pid: 0,
                    top_level_pid: std::process::id(),
                    opt_flags,
                };
                let looked_up = lookup_var(&full_name, &ctx_real);
                if looked_up.is_empty() {
                    "0".to_string()
                } else {
                    looked_up
                }
            } else {
                vars.get(&name)
                    .cloned()
                    .or_else(|| std::env::var(&name).ok())
                    .unwrap_or_else(|| "0".to_string())
            };
            // If val is not a number, try to resolve it again (for variable indirection in arith)
            let val = if val.parse::<i64>().is_err() && !val.is_empty() {
                val.parse::<i64>().map(|n| n.to_string()).unwrap_or(val)
            } else {
                val
            };
            result.push_str(&val);
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    result
}

fn eval_arith(expr: &str) -> Result<i64, String> {
    let expr = expr.trim();
    if expr.is_empty() {
        return Ok(0);
    }

    // Handle comma operator (evaluate both, return right)
    if let Some(pos) = rfind_op(expr, ",") {
        let _left = eval_arith(&expr[..pos])?;
        let right = eval_arith(&expr[pos + 1..])?;
        return Ok(right);
    }

    // Handle variable assignment: var=expr, var+=expr, var-=expr, etc.
    // Assignments have lower precedence than everything except comma.
    // We can't actually store the value (no &mut vars), but we evaluate
    // the RHS and return it so the expression produces the correct result.
    {
        let chars: Vec<char> = expr.chars().collect();
        // Check if expression starts with a valid identifier
        if !chars.is_empty() && (chars[0].is_alphabetic() || chars[0] == '_') {
            let name_end = chars
                .iter()
                .position(|c| !c.is_alphanumeric() && *c != '_')
                .unwrap_or(chars.len());
            if name_end < chars.len() {
                let after_name: String = chars[name_end..].iter().collect();
                // Simple assignment: var=expr
                if after_name.starts_with('=') && !after_name.starts_with("==") {
                    let rhs = &after_name[1..];
                    return eval_arith(rhs);
                }
                // Compound assignments: var+=, var-=, var*=, var/=, var%=
                if after_name.len() >= 2
                    && matches!(after_name.as_bytes()[0], b'+' | b'-' | b'*' | b'/' | b'%')
                    && after_name.as_bytes()[1] == b'='
                {
                    let rhs = &after_name[2..];
                    return eval_arith(rhs);
                }
            }
        }
    }

    // Handle ternary operator
    if let Some(q_pos) = find_balanced(expr, '?') {
        let cond = eval_arith(&expr[..q_pos])?;
        let rest = &expr[q_pos + 1..];
        if let Some(c_pos) = find_balanced(rest, ':') {
            let then_val = eval_arith(&rest[..c_pos])?;
            let else_val = eval_arith(&rest[c_pos + 1..])?;
            return Ok(if cond != 0 { then_val } else { else_val });
        }
    }

    // Handle || (logical OR)
    if let Some(pos) = rfind_op(expr, "||") {
        let left = eval_arith(&expr[..pos])?;
        let right = eval_arith(&expr[pos + 2..])?;
        return Ok(if left != 0 || right != 0 { 1 } else { 0 });
    }

    // Handle && (logical AND)
    if let Some(pos) = rfind_op(expr, "&&") {
        let left = eval_arith(&expr[..pos])?;
        let right = eval_arith(&expr[pos + 2..])?;
        return Ok(if left != 0 && right != 0 { 1 } else { 0 });
    }

    // Bitwise OR (not ||)
    if let Some(pos) = rfind_op(expr, "|")
        && pos > 0
        && expr.as_bytes()[pos - 1] != b'|'
        && (pos + 1 >= expr.len() || expr.as_bytes()[pos + 1] != b'|')
    {
        let left = eval_arith(&expr[..pos])?;
        let right = eval_arith(&expr[pos + 1..])?;
        return Ok(left | right);
    }

    // Bitwise XOR
    if let Some(pos) = rfind_op(expr, "^") {
        let left = eval_arith(&expr[..pos])?;
        let right = eval_arith(&expr[pos + 1..])?;
        return Ok(left ^ right);
    }

    // Bitwise AND (not &&)
    if let Some(pos) = rfind_op(expr, "&")
        && pos > 0
        && expr.as_bytes()[pos - 1] != b'&'
        && (pos + 1 >= expr.len() || expr.as_bytes()[pos + 1] != b'&')
    {
        let left = eval_arith(&expr[..pos])?;
        let right = eval_arith(&expr[pos + 1..])?;
        return Ok(left & right);
    }

    // Comparison operators (check multi-char ops first to avoid matching << >> as < >)
    for op in &["==", "!=", "<=", ">=", "<<", ">>", "<", ">"] {
        if let Some(pos) = rfind_op(expr, op) {
            let left = eval_arith(&expr[..pos])?;
            let right = eval_arith(&expr[pos + op.len()..])?;
            return match *op {
                "==" => Ok(if left == right { 1 } else { 0 }),
                "!=" => Ok(if left != right { 1 } else { 0 }),
                "<=" => Ok(if left <= right { 1 } else { 0 }),
                ">=" => Ok(if left >= right { 1 } else { 0 }),
                "<" => Ok(if left < right { 1 } else { 0 }),
                ">" => Ok(if left > right { 1 } else { 0 }),
                "<<" => Ok(left << right),
                ">>" => Ok(left >> right),
                _ => unreachable!(),
            };
        }
    }

    // Addition and subtraction
    {
        let mut depth = 0i32;
        let chars: Vec<char> = expr.chars().collect();
        let mut i = chars.len();
        while i > 0 {
            i -= 1;
            match chars[i] {
                ')' => depth += 1,
                '(' => depth -= 1,
                '+' | '-' if depth == 0 && i > 0 => {
                    // Skip if this is part of ++ or -- (check next char)
                    let next = if i + 1 < chars.len() {
                        chars[i + 1]
                    } else {
                        ' '
                    };
                    // Look past whitespace to find the real previous character
                    let effective_prev = {
                        let mut j = i - 1;
                        while j > 0 && chars[j].is_ascii_whitespace() {
                            j -= 1;
                        }
                        chars[j]
                    };
                    if !matches!(
                        effective_prev,
                        '+' | '-' | '*' | '/' | '%' | '(' | '<' | '>' | '=' | '!' | '&' | '|'
                    ) && (next != chars[i])
                    {
                        let left = eval_arith(&expr[..i])?;
                        let right = eval_arith(&expr[i + 1..])?;
                        return Ok(if chars[i] == '+' {
                            left.wrapping_add(right)
                        } else {
                            left.wrapping_sub(right)
                        });
                    }
                }
                _ => {}
            }
        }
    }

    // Multiplication, division, modulo
    {
        let mut depth = 0i32;
        let chars: Vec<char> = expr.chars().collect();
        let mut i = chars.len();
        while i > 0 {
            i -= 1;
            match chars[i] {
                ')' => depth += 1,
                '(' => depth -= 1,
                '*' | '/' | '%' if depth == 0 => {
                    if chars[i] == '*' && i + 1 < chars.len() && chars[i + 1] == '*' {
                        continue;
                    }
                    if chars[i] == '*' && i > 0 && chars[i - 1] == '*' {
                        continue;
                    }
                    let left = eval_arith(&expr[..i])?;
                    let right = eval_arith(&expr[i + 1..])?;
                    return match chars[i] {
                        '*' => Ok(left.wrapping_mul(right)),
                        '/' => {
                            if right == 0 {
                                Err("division by 0 (error token is \"0\")".to_string())
                            } else {
                                Ok(left.wrapping_div(right))
                            }
                        }
                        '%' => {
                            if right == 0 {
                                Err("division by 0 (error token is \"0\")".to_string())
                            } else if left == i64::MIN && right == -1 {
                                Ok(0)
                            } else {
                                Ok(left % right)
                            }
                        }
                        _ => unreachable!(),
                    };
                }
                _ => {}
            }
        }
    }

    // Exponentiation
    if let Some(pos) = find_op(expr, "**") {
        let base = eval_arith(&expr[..pos])?;
        let exp = eval_arith(&expr[pos + 2..])?;
        if exp < 0 {
            return Err(format!(
                "exponent less than 0 (error token is \"{}\")",
                &expr[pos + 2..].trim()
            ));
        }
        return Ok(base.wrapping_pow(exp as u32));
    }

    // Try parsing as a number first (handles negative literals like -9223372036854775808)
    if expr.starts_with('-')
        && let Ok(n) = expr.parse::<i64>()
    {
        return Ok(n);
    }

    // Unary operators
    if let Some(stripped) = expr.strip_prefix('-') {
        return eval_arith(stripped).map(|n| n.wrapping_neg());
    }
    if let Some(stripped) = expr.strip_prefix('+') {
        return eval_arith(stripped);
    }
    if let Some(stripped) = expr.strip_prefix('!') {
        return eval_arith(stripped).map(|n| if n == 0 { 1 } else { 0 });
    }
    if let Some(stripped) = expr.strip_prefix('~') {
        return eval_arith(stripped).map(|n| !n);
    }

    // Parentheses
    if expr.starts_with('(') && expr.ends_with(')') {
        return eval_arith(&expr[1..expr.len() - 1]);
    }

    // Number literal
    let expr = expr.trim();
    if expr.is_empty() {
        return Err("syntax error: operand expected".to_string());
    }
    if let Some(hex) = expr.strip_prefix("0x").or_else(|| expr.strip_prefix("0X")) {
        i64::from_str_radix(hex, 16)
            .map_err(|_| format!("value too great for base (error token is \"{}\")", expr))
    } else if let Some(oct) = expr.strip_prefix('0') {
        if !oct.is_empty() && oct.chars().all(|c| c.is_ascii_digit()) {
            i64::from_str_radix(oct, 8)
                .map_err(|_| format!("value too great for base (error token is \"{}\")", expr))
        } else {
            expr.parse::<i64>().map_err(|_| {
                format!(
                    "syntax error: operand expected (error token is \"{}\")",
                    expr
                )
            })
        }
    } else if expr.chars().all(|c| c.is_alphanumeric() || c == '_')
        && expr
            .chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_')
    {
        // Bare variable name — evaluates to 0 (unset) in arithmetic context.
        // resolve_arith_vars already expanded $-prefixed references; any
        // remaining identifier is an unset/unknown variable.
        Ok(0)
    } else {
        expr.parse::<i64>().map_err(|_| {
            format!(
                "syntax error: operand expected (error token is \"{}\")",
                expr
            )
        })
    }
}

fn find_balanced(expr: &str, target: char) -> Option<usize> {
    let mut depth = 0i32;
    for (i, ch) in expr.chars().enumerate() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            c if c == target && depth == 0 => return Some(i),
            _ => {}
        }
    }
    None
}

fn find_op(expr: &str, op: &str) -> Option<usize> {
    let mut depth = 0i32;
    let bytes = expr.as_bytes();
    let op_bytes = op.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'(' {
            depth += 1;
        } else if bytes[i] == b')' {
            depth -= 1;
        } else if depth == 0
            && i + op_bytes.len() <= bytes.len()
            && &bytes[i..i + op_bytes.len()] == op_bytes
        {
            return Some(i);
        }
    }
    None
}

fn rfind_op(expr: &str, op: &str) -> Option<usize> {
    let mut depth = 0i32;
    let bytes = expr.as_bytes();
    let op_bytes = op.as_bytes();
    let mut result = None;
    for i in 0..bytes.len() {
        if bytes[i] == b'(' {
            depth += 1;
        } else if bytes[i] == b')' {
            depth -= 1;
        } else if depth == 0
            && i + op_bytes.len() <= bytes.len()
            && &bytes[i..i + op_bytes.len()] == op_bytes
        {
            result = Some(i);
        }
    }
    result
}
