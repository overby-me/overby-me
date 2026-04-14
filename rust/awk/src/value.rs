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
    Uninitialized,
}

impl Value {
    pub fn to_num(&self) -> f64 {
        match self {
            Value::Num(n) => *n,
            Value::Str(s) => parse_num(s),
            Value::Uninitialized => 0.0,
        }
    }

    pub fn to_string_val(&self) -> String {
        match self {
            Value::Num(n) => format_number(*n),
            Value::Str(s) => s.clone(),
            Value::Uninitialized => String::new(),
        }
    }

    /// Format a number using OFMT/CONVFMT format string
    pub fn to_string_with_fmt(&self, fmt: &str) -> String {
        match self {
            Value::Num(n) => {
                let n = *n;
                // If it's an integer, print as integer (no OFMT needed)
                if n.fract() == 0.0 && n.abs() < 1e16 && !n.is_nan() && !n.is_infinite() {
                    format!("{}", n as i64)
                } else {
                    use crate::format::sprintf_impl;
                    sprintf_impl(&[Value::Str(fmt.to_string()), Value::Num(n)])
                }
            }
            Value::Str(s) => s.clone(),
            Value::Uninitialized => String::new(),
        }
    }

    pub fn to_bool(&self) -> bool {
        match self {
            Value::Num(n) => *n != 0.0,
            Value::Str(s) => !s.is_empty(),
            Value::Uninitialized => false,
        }
    }

    pub fn is_numeric_string(&self) -> bool {
        match self {
            Value::Num(_) => true,
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
    let chars: Vec<char> = s.chars().collect();

    // Check for hex: 0x or 0X
    let mut start = 0;
    let neg = if !chars.is_empty() && chars[0] == '-' {
        start = 1;
        true
    } else if !chars.is_empty() && chars[0] == '+' {
        start = 1;
        false
    } else {
        false
    };

    if start + 1 < chars.len()
        && chars[start] == '0'
        && (chars[start + 1] == 'x' || chars[start + 1] == 'X')
    {
        let mut end = start + 2;
        while end < chars.len() && chars[end].is_ascii_hexdigit() {
            end += 1;
        }
        if end > start + 2 {
            let hex_str: String = chars[start + 2..end].iter().collect();
            let val = i64::from_str_radix(&hex_str, 16).unwrap_or(0) as f64;
            return if neg { -val } else { val };
        }
    }

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
    if n.fract() == 0.0 && n.abs() < 1e16 && !n.is_nan() && !n.is_infinite() {
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
