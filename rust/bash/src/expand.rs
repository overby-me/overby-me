use crate::ast::*;
use std::collections::HashMap;

/// Function type for evaluating command substitutions.
pub type CmdSubFn<'a> = &'a mut dyn FnMut(&str) -> String;

/// Represents expanded text with quoting information preserved.
#[derive(Debug, Clone)]
pub enum Segment {
    Quoted(String),
    Unquoted(String),
}

/// Expand a word into a list of strings (after word splitting and globbing).
pub fn expand_word(
    word: &Word,
    vars: &HashMap<String, String>,
    positional: &[String],
    last_status: i32,
    ifs: &str,
    cmd_sub: CmdSubFn,
) -> Vec<String> {
    let segments = expand_word_to_segments(word, vars, positional, last_status, cmd_sub);
    let fields = word_split(&segments, ifs);
    let mut result = Vec::new();
    for field in fields {
        let globbed = glob_expand(&field);
        result.extend(globbed);
    }
    if result.is_empty() && !word.is_empty() {
        let all_quoted = word
            .iter()
            .all(|p| matches!(p, WordPart::SingleQuoted(_) | WordPart::DoubleQuoted(_)));
        if all_quoted {
            result.push(String::new());
        }
    }
    result
}

/// Expand a word to a single string (no word splitting or globbing).
pub fn expand_word_nosplit(
    word: &Word,
    vars: &HashMap<String, String>,
    positional: &[String],
    last_status: i32,
    cmd_sub: CmdSubFn,
) -> String {
    let segments = expand_word_to_segments(word, vars, positional, last_status, cmd_sub);
    segments
        .iter()
        .map(|s| match s {
            Segment::Quoted(t) | Segment::Unquoted(t) => t.as_str(),
        })
        .collect()
}

fn expand_word_to_segments(
    word: &Word,
    vars: &HashMap<String, String>,
    positional: &[String],
    last_status: i32,
    cmd_sub: CmdSubFn,
) -> Vec<Segment> {
    let mut segments = Vec::new();
    for part in word {
        expand_part(part, vars, positional, last_status, &mut segments, cmd_sub);
    }
    segments
}

fn expand_part(
    part: &WordPart,
    vars: &HashMap<String, String>,
    positional: &[String],
    last_status: i32,
    out: &mut Vec<Segment>,
    cmd_sub: CmdSubFn,
) {
    match part {
        WordPart::Literal(s) => {
            out.push(Segment::Unquoted(s.clone()));
        }
        WordPart::SingleQuoted(s) => {
            out.push(Segment::Quoted(s.clone()));
        }
        WordPart::DoubleQuoted(parts) => {
            let mut s = String::new();
            for p in parts {
                match p {
                    WordPart::Literal(t) => s.push_str(t),
                    WordPart::Variable(name) => {
                        s.push_str(&lookup_var(name, vars, positional, last_status));
                    }
                    WordPart::Param(expr) => {
                        s.push_str(&expand_param(expr, vars, positional, last_status, cmd_sub));
                    }
                    WordPart::CommandSub(cmd) => {
                        s.push_str(&cmd_sub(cmd));
                    }
                    WordPart::BacktickSub(cmd) => {
                        s.push_str(&cmd_sub(cmd));
                    }
                    WordPart::ArithSub(expr) => {
                        s.push_str(&expand_arith(expr, vars, positional, last_status));
                    }
                    _ => {
                        let mut inner = Vec::new();
                        expand_part(p, vars, positional, last_status, &mut inner, cmd_sub);
                        for seg in inner {
                            match seg {
                                Segment::Quoted(t) | Segment::Unquoted(t) => s.push_str(&t),
                            }
                        }
                    }
                }
            }
            out.push(Segment::Quoted(s));
        }
        WordPart::Tilde(user) => {
            let expanded = if user.is_empty() {
                vars.get("HOME").cloned().unwrap_or_else(|| "~".to_string())
            } else {
                // Look up user's home directory
                #[cfg(unix)]
                {
                    use std::ffi::CString;
                    if let Ok(cname) = CString::new(user.as_str()) {
                        let pw = unsafe { libc::getpwnam(cname.as_ptr()) };
                        if !pw.is_null() {
                            let dir = unsafe { std::ffi::CStr::from_ptr((*pw).pw_dir) };
                            dir.to_string_lossy().to_string()
                        } else {
                            format!("~{}", user)
                        }
                    } else {
                        format!("~{}", user)
                    }
                }
                #[cfg(not(unix))]
                {
                    format!("~{}", user)
                }
            };
            out.push(Segment::Unquoted(expanded));
        }
        WordPart::Variable(name) => {
            let val = lookup_var(name, vars, positional, last_status);
            out.push(Segment::Unquoted(val));
        }
        WordPart::Param(expr) => {
            let val = expand_param(expr, vars, positional, last_status, cmd_sub);
            out.push(Segment::Unquoted(val));
        }
        WordPart::CommandSub(cmd) => {
            let val = cmd_sub(cmd);
            out.push(Segment::Unquoted(val));
        }
        WordPart::BacktickSub(cmd) => {
            let val = cmd_sub(cmd);
            out.push(Segment::Unquoted(val));
        }
        WordPart::ArithSub(expr) => {
            let val = expand_arith(expr, vars, positional, last_status);
            out.push(Segment::Unquoted(val));
        }
    }
}

fn lookup_var(
    name: &str,
    vars: &HashMap<String, String>,
    positional: &[String],
    last_status: i32,
) -> String {
    match name {
        "?" => last_status.to_string(),
        "$" => std::process::id().to_string(),
        "#" => {
            let count = if positional.is_empty() {
                0
            } else {
                positional.len() - 1
            };
            count.to_string()
        }
        "0" => positional.first().cloned().unwrap_or_default(),
        "@" | "*" => {
            if positional.len() > 1 {
                positional[1..].join(" ")
            } else {
                String::new()
            }
        }
        "-" => String::new(), // TODO: current shell flags
        "!" => String::new(), // TODO: last background PID
        _ => {
            // Check positional parameters
            if let Ok(n) = name.parse::<usize>() {
                if n < positional.len() {
                    return positional[n].clone();
                }
                return String::new();
            }
            // Check variables, then environment
            vars.get(name)
                .cloned()
                .or_else(|| std::env::var(name).ok())
                .unwrap_or_default()
        }
    }
}

fn expand_param(
    expr: &ParamExpr,
    vars: &HashMap<String, String>,
    positional: &[String],
    last_status: i32,
    cmd_sub: CmdSubFn,
) -> String {
    let val = lookup_var(&expr.name, vars, positional, last_status);

    match &expr.op {
        ParamOp::None => val,
        ParamOp::Length => val.len().to_string(),
        ParamOp::Default(colon, word) => {
            let empty = if *colon { val.is_empty() } else { false };
            let unset = vars.get(&expr.name).is_none() && std::env::var(&expr.name).is_err();
            if unset || empty {
                expand_word_nosplit(word, vars, positional, last_status, cmd_sub)
            } else {
                val
            }
        }
        ParamOp::Assign(colon, word) => {
            let empty = if *colon { val.is_empty() } else { false };
            let unset = vars.get(&expr.name).is_none() && std::env::var(&expr.name).is_err();
            if unset || empty {
                expand_word_nosplit(word, vars, positional, last_status, cmd_sub)
            } else {
                val
            }
        }
        ParamOp::Error(colon, word) => {
            let empty = if *colon { val.is_empty() } else { false };
            let unset = vars.get(&expr.name).is_none() && std::env::var(&expr.name).is_err();
            if unset || empty {
                let msg = expand_word_nosplit(word, vars, positional, last_status, cmd_sub);
                eprintln!(
                    "bash: {}: {}",
                    expr.name,
                    if msg.is_empty() {
                        "parameter null or not set"
                    } else {
                        &msg
                    }
                );
                std::process::exit(1);
            }
            val
        }
        ParamOp::Alt(colon, word) => {
            let empty = if *colon { val.is_empty() } else { false };
            let unset = vars.get(&expr.name).is_none() && std::env::var(&expr.name).is_err();
            if unset || empty {
                String::new()
            } else {
                expand_word_nosplit(word, vars, positional, last_status, cmd_sub)
            }
        }
        ParamOp::TrimSmallLeft(pattern) => {
            let pat = expand_word_nosplit(pattern, vars, positional, last_status, cmd_sub);
            trim_pattern(&val, &pat, TrimMode::SmallLeft)
        }
        ParamOp::TrimLargeLeft(pattern) => {
            let pat = expand_word_nosplit(pattern, vars, positional, last_status, cmd_sub);
            trim_pattern(&val, &pat, TrimMode::LargeLeft)
        }
        ParamOp::TrimSmallRight(pattern) => {
            let pat = expand_word_nosplit(pattern, vars, positional, last_status, cmd_sub);
            trim_pattern(&val, &pat, TrimMode::SmallRight)
        }
        ParamOp::TrimLargeRight(pattern) => {
            let pat = expand_word_nosplit(pattern, vars, positional, last_status, cmd_sub);
            trim_pattern(&val, &pat, TrimMode::LargeRight)
        }
        ParamOp::Replace(pattern, replacement) => {
            let pat = expand_word_nosplit(pattern, vars, positional, last_status, cmd_sub);
            let rep = expand_word_nosplit(replacement, vars, positional, last_status, cmd_sub);
            pattern_replace(&val, &pat, &rep, false)
        }
        ParamOp::ReplaceAll(pattern, replacement) => {
            let pat = expand_word_nosplit(pattern, vars, positional, last_status, cmd_sub);
            let rep = expand_word_nosplit(replacement, vars, positional, last_status, cmd_sub);
            pattern_replace(&val, &pat, &rep, true)
        }
        ParamOp::Substring(offset_str, length_str) => {
            let offset: i64 = offset_str.trim().parse().unwrap_or(0);
            let start = if offset < 0 {
                (val.len() as i64 + offset).max(0) as usize
            } else {
                (offset as usize).min(val.len())
            };
            if let Some(len_str) = length_str {
                let len: i64 = len_str.trim().parse().unwrap_or(val.len() as i64);
                let end = if len < 0 {
                    (val.len() as i64 + len).max(start as i64) as usize
                } else {
                    (start + len as usize).min(val.len())
                };
                val[start..end].to_string()
            } else {
                val[start..].to_string()
            }
        }
    }
}

fn expand_arith(
    expr: &str,
    vars: &HashMap<String, String>,
    positional: &[String],
    last_status: i32,
) -> String {
    // Simple arithmetic evaluator
    let resolved = resolve_arith_vars(expr, vars, positional, last_status);
    match eval_arith(&resolved) {
        Ok(n) => n.to_string(),
        Err(_) => "0".to_string(),
    }
}

fn resolve_arith_vars(
    expr: &str,
    vars: &HashMap<String, String>,
    positional: &[String],
    last_status: i32,
) -> String {
    let mut result = String::new();
    let chars: Vec<char> = expr.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '$' {
            i += 1;
            let mut name = String::new();
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                name.push(chars[i]);
                i += 1;
            }
            let val = lookup_var(&name, vars, positional, last_status);
            let val = if val.is_empty() { "0".to_string() } else { val };
            result.push_str(&val);
        } else if chars[i].is_alphabetic() || chars[i] == '_' {
            let mut name = String::new();
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                name.push(chars[i]);
                i += 1;
            }
            let val = vars
                .get(&name)
                .cloned()
                .or_else(|| std::env::var(&name).ok())
                .unwrap_or_else(|| "0".to_string());
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

    // Comparison operators
    for op in &["==", "!=", "<=", ">=", "<", ">"] {
        if let Some(pos) = rfind_op(expr, op) {
            let left = eval_arith(&expr[..pos])?;
            let right = eval_arith(&expr[pos + op.len()..])?;
            let result = match *op {
                "==" => left == right,
                "!=" => left != right,
                "<=" => left <= right,
                ">=" => left >= right,
                "<" => left < right,
                ">" => left > right,
                _ => false,
            };
            return Ok(if result { 1 } else { 0 });
        }
    }

    // Addition and subtraction (right-to-left scan for lowest precedence)
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
                    // Make sure this isn't part of **, ++ etc.
                    let prev = chars[i - 1];
                    if !matches!(
                        prev,
                        '+' | '-' | '*' | '/' | '%' | '(' | '<' | '>' | '=' | '!' | '&' | '|'
                    ) {
                        let left = eval_arith(&expr[..i])?;
                        let right = eval_arith(&expr[i + 1..])?;
                        return Ok(if chars[i] == '+' {
                            left + right
                        } else {
                            left - right
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
                    // Make sure * isn't part of **
                    if chars[i] == '*' && i + 1 < chars.len() && chars[i + 1] == '*' {
                        continue;
                    }
                    if chars[i] == '*' && i > 0 && chars[i - 1] == '*' {
                        continue;
                    }
                    let left = eval_arith(&expr[..i])?;
                    let right = eval_arith(&expr[i + 1..])?;
                    return match chars[i] {
                        '*' => Ok(left * right),
                        '/' => {
                            if right == 0 {
                                Err("division by zero".to_string())
                            } else {
                                Ok(left / right)
                            }
                        }
                        '%' => {
                            if right == 0 {
                                Err("division by zero".to_string())
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
        return Ok(base.pow(exp as u32));
    }

    // Unary operators
    if let Some(stripped) = expr.strip_prefix('-') {
        return eval_arith(stripped).map(|n| -n);
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
    if let Some(hex) = expr.strip_prefix("0x").or_else(|| expr.strip_prefix("0X")) {
        i64::from_str_radix(hex, 16).map_err(|e| e.to_string())
    } else if let Some(oct) = expr.strip_prefix('0') {
        if !oct.is_empty() && oct.chars().all(|c| c.is_ascii_digit()) {
            i64::from_str_radix(oct, 8).map_err(|e| e.to_string())
        } else {
            expr.parse::<i64>().map_err(|e| e.to_string())
        }
    } else {
        expr.parse::<i64>().map_err(|e| e.to_string())
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

fn word_split(segments: &[Segment], ifs: &str) -> Vec<String> {
    if segments.is_empty() {
        return vec![];
    }

    // If everything is quoted, no splitting
    let all_quoted = segments.iter().all(|s| matches!(s, Segment::Quoted(_)));
    if all_quoted {
        let s: String = segments
            .iter()
            .map(|seg| match seg {
                Segment::Quoted(t) | Segment::Unquoted(t) => t.as_str(),
            })
            .collect();
        return vec![s];
    }

    let mut fields: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut in_field = false;

    for segment in segments {
        match segment {
            Segment::Quoted(s) => {
                current.push_str(s);
                in_field = true;
            }
            Segment::Unquoted(s) => {
                for ch in s.chars() {
                    if ifs.contains(ch) {
                        if in_field {
                            fields.push(std::mem::take(&mut current));
                            in_field = false;
                        }
                    } else {
                        current.push(ch);
                        in_field = true;
                    }
                }
            }
        }
    }

    if in_field || !current.is_empty() {
        fields.push(current);
    }

    fields
}

fn glob_expand(field: &str) -> Vec<String> {
    // Check if field contains unquoted glob characters
    if field.contains('*') || field.contains('?') || field.contains('[') {
        match glob::glob(field) {
            Ok(paths) => {
                let mut results: Vec<String> = paths
                    .filter_map(|p| p.ok())
                    .map(|p| p.to_string_lossy().to_string())
                    .collect();
                if results.is_empty() {
                    vec![field.to_string()]
                } else {
                    results.sort();
                    results
                }
            }
            Err(_) => vec![field.to_string()],
        }
    } else {
        vec![field.to_string()]
    }
}

enum TrimMode {
    SmallLeft,
    LargeLeft,
    SmallRight,
    LargeRight,
}

fn trim_pattern(value: &str, pattern: &str, mode: TrimMode) -> String {
    // Convert shell glob pattern to a simple matcher
    match mode {
        TrimMode::SmallLeft => {
            for i in 0..=value.len() {
                if shell_pattern_match(&value[..i], pattern) {
                    return value[i..].to_string();
                }
            }
            value.to_string()
        }
        TrimMode::LargeLeft => {
            for i in (0..=value.len()).rev() {
                if shell_pattern_match(&value[..i], pattern) {
                    return value[i..].to_string();
                }
            }
            value.to_string()
        }
        TrimMode::SmallRight => {
            for i in (0..=value.len()).rev() {
                if shell_pattern_match(&value[i..], pattern) {
                    return value[..i].to_string();
                }
            }
            value.to_string()
        }
        TrimMode::LargeRight => {
            for i in 0..=value.len() {
                if shell_pattern_match(&value[i..], pattern) {
                    return value[..i].to_string();
                }
            }
            value.to_string()
        }
    }
}

fn pattern_replace(value: &str, pattern: &str, replacement: &str, all: bool) -> String {
    if pattern.is_empty() {
        return value.to_string();
    }

    let mut result = String::new();
    let mut i = 0;
    let chars: Vec<char> = value.chars().collect();

    while i < chars.len() {
        let mut found = false;
        // Try matching at position i with increasing lengths
        for j in (i + 1..=chars.len()).rev() {
            let substr: String = chars[i..j].iter().collect();
            if shell_pattern_match(&substr, pattern) {
                result.push_str(replacement);
                i = j;
                found = true;
                if !all {
                    // Append the rest and return
                    let rest: String = chars[i..].iter().collect();
                    result.push_str(&rest);
                    return result;
                }
                break;
            }
        }
        if !found {
            result.push(chars[i]);
            i += 1;
        }
    }
    result
}

fn shell_pattern_match(text: &str, pattern: &str) -> bool {
    let t: Vec<char> = text.chars().collect();
    let p: Vec<char> = pattern.chars().collect();
    pattern_match_impl(&t, 0, &p, 0)
}

fn pattern_match_impl(text: &[char], ti: usize, pattern: &[char], pi: usize) -> bool {
    let mut ti = ti;
    let mut pi = pi;

    while pi < pattern.len() {
        match pattern[pi] {
            '*' => {
                pi += 1;
                // Skip consecutive *
                while pi < pattern.len() && pattern[pi] == '*' {
                    pi += 1;
                }
                if pi == pattern.len() {
                    return true;
                }
                for i in ti..=text.len() {
                    if pattern_match_impl(text, i, pattern, pi) {
                        return true;
                    }
                }
                return false;
            }
            '?' => {
                if ti >= text.len() {
                    return false;
                }
                ti += 1;
                pi += 1;
            }
            '[' => {
                if ti >= text.len() {
                    return false;
                }
                pi += 1;
                let negate = pi < pattern.len() && (pattern[pi] == '!' || pattern[pi] == '^');
                if negate {
                    pi += 1;
                }
                let mut matched = false;
                let ch = text[ti];
                while pi < pattern.len() && pattern[pi] != ']' {
                    if pi + 2 < pattern.len() && pattern[pi + 1] == '-' {
                        if ch >= pattern[pi] && ch <= pattern[pi + 2] {
                            matched = true;
                        }
                        pi += 3;
                    } else {
                        if ch == pattern[pi] {
                            matched = true;
                        }
                        pi += 1;
                    }
                }
                if pi < pattern.len() {
                    pi += 1; // skip ]
                }
                if matched == negate {
                    return false;
                }
                ti += 1;
            }
            '\\' => {
                pi += 1;
                if pi >= pattern.len() || ti >= text.len() {
                    return false;
                }
                if text[ti] != pattern[pi] {
                    return false;
                }
                ti += 1;
                pi += 1;
            }
            ch => {
                if ti >= text.len() || text[ti] != ch {
                    return false;
                }
                ti += 1;
                pi += 1;
            }
        }
    }

    ti == text.len()
}
