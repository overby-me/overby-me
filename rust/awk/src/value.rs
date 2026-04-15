#[derive(Debug)]
pub enum ControlFlow {
    None,
    Next,
    Exit(i32),
    Break,
    Continue,
    Return(Value),
}

#[derive(Debug, Clone)]
pub enum Value {
    Num(f64),
    Str(String),
    /// String from input (fields, getline, FILENAME) that may be numeric.
    /// In boolean/comparison context, uses numeric value if parseable.
    StrNum(String),
    Uninitialized,
}

impl Value {
    pub fn to_num(&self) -> f64 {
        match self {
            Value::Num(n) => *n,
            Value::Str(s) | Value::StrNum(s) => parse_num(s),
            Value::Uninitialized => 0.0,
        }
    }

    pub fn to_string_val(&self) -> String {
        match self {
            Value::Num(n) => format_number(*n),
            Value::Str(s) | Value::StrNum(s) => s.clone(),
            Value::Uninitialized => String::new(),
        }
    }

    /// Format a number using OFMT/CONVFMT format string
    pub fn to_string_with_fmt(&self, fmt: &str) -> String {
        match self {
            Value::Num(n) => {
                let n = *n;
                if n.is_nan() {
                    return "nan".to_string();
                }
                if n.is_infinite() {
                    return if n > 0.0 {
                        "+inf".to_string()
                    } else {
                        "-inf".to_string()
                    };
                }
                // If it's an integer, print as integer (no OFMT needed)
                if n.fract() == 0.0 && n.abs() < 1e16 {
                    format!("{}", n as i64)
                } else {
                    use crate::format::sprintf_impl;
                    sprintf_impl(&[Value::Str(fmt.to_string()), Value::Num(n)])
                }
            }
            Value::Str(s) | Value::StrNum(s) => s.clone(),
            Value::Uninitialized => String::new(),
        }
    }

    pub fn to_bool(&self) -> bool {
        match self {
            Value::Num(n) => *n != 0.0,
            Value::Str(s) => !s.is_empty(),
            Value::StrNum(s) => {
                // Input strings: if the string looks numeric, use numeric truth
                if s.is_empty() {
                    return false;
                }
                let trimmed = s.trim();
                // Only use numeric comparison if the string actually starts with
                // a digit, sign, or decimal point (i.e., parse_num would find digits)
                if trimmed.starts_with(|c: char| c.is_ascii_digit() || c == '+' || c == '-' || c == '.') {
                    parse_num(s) != 0.0
                } else {
                    // Non-numeric string: non-empty is true
                    true
                }
            }
            Value::Uninitialized => false,
        }
    }

    pub fn is_numeric_string(&self) -> bool {
        match self {
            Value::Num(_) => true,
            Value::StrNum(s) => {
                // Empty StrNum is not numeric (for comparison purposes)
                let s = s.trim();
                !s.is_empty()
            }
            Value::Str(s) => {
                let s = s.trim();
                if s.is_empty() {
                    return false;
                }
                s.parse::<f64>().is_ok()
            }
            Value::Uninitialized => false,
        }
    }
}

pub fn parse_num(s: &str) -> f64 {
    let s = s.trim();
    if s.is_empty() {
        return 0.0;
    }
    // Note: hex (0x...) is NOT parsed here — only in lexer for numeric literals.
    // POSIX awk does not convert hex strings to numbers (gawk requires --non-decimal-data).
    let chars: Vec<char> = s.chars().collect();

    // Parse as much of the leading part as possible
    let mut end = 0;
    if end < chars.len() && (chars[end] == '+' || chars[end] == '-') {
        end += 1;
    }
    let mut has_digits = false;
    while end < chars.len() && chars[end].is_ascii_digit() {
        end += 1;
        has_digits = true;
    }
    if end < chars.len() && chars[end] == '.' {
        end += 1;
        while end < chars.len() && chars[end].is_ascii_digit() {
            end += 1;
            has_digits = true;
        }
    }
    if has_digits && end < chars.len() && (chars[end] == 'e' || chars[end] == 'E') {
        end += 1;
        if end < chars.len() && (chars[end] == '+' || chars[end] == '-') {
            end += 1;
        }
        while end < chars.len() && chars[end].is_ascii_digit() {
            end += 1;
        }
    }
    if !has_digits {
        return 0.0;
    }
    let num_str: String = chars[..end].iter().collect();
    num_str.parse().unwrap_or(0.0)
}

pub fn format_number(n: f64) -> String {
    if n.is_nan() {
        return "nan".to_string();
    }
    if n.is_infinite() {
        return if n > 0.0 {
            "+inf".to_string()
        } else {
            "-inf".to_string()
        };
    }
    if n.fract() == 0.0 && n.abs() < 1e16 {
        format!("{}", n as i64)
    } else {
        // Use %.6g style formatting like awk
        let s = format!("{n:.6}");
        // Trim trailing zeros after decimal point
        if s.contains('.') {
            let s = s.trim_end_matches('0');
            let s = s.trim_end_matches('.');
            s.to_string()
        } else {
            s
        }
    }
}

pub fn compare_values(a: &Value, b: &Value) -> std::cmp::Ordering {
    // If both are numeric or numeric strings, compare as numbers
    if a.is_numeric_string() && b.is_numeric_string() {
        let na = a.to_num();
        let nb = b.to_num();
        na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal)
    } else {
        let sa = a.to_string_val();
        let sb = b.to_string_val();
        sa.cmp(&sb)
    }
}
