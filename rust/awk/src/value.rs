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
    // Parse as much of the leading part as possible
    let mut end = 0;
    let chars: Vec<char> = s.chars().collect();
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
