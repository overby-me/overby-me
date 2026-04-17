/// Perl scalar value type.
///
/// Perl scalars have "dual-var" nature: they can be viewed as both strings
/// and numbers, with conversion on demand. We keep it simple: store one
/// representation and convert lazily.
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Clone, Debug)]
pub enum Value {
    Undef,
    Str(String),
    Num(f64),
    /// Reference to an array — stringifies as `ARRAY(0x...)`.
    ArrayRef(Rc<RefCell<Vec<Value>>>),
    /// Reference to a hash — stringifies as `HASH(0x...)`.
    HashRef(Rc<RefCell<HashMap<String, Value>>>),
    /// Reference to a scalar — stringifies as `SCALAR(0x...)`.
    ScalarRef(Rc<RefCell<Value>>),
    /// Reference to a subroutine (by name for now) — `CODE(0x...)`.
    CodeRef(String),
    /// Compiled regex from `qr//` (pattern, flags). `ref()` returns
    /// "Regexp"; stringifies as `(?^flags:pattern)` — the same form real
    /// perl uses so it can be matched against with `=~` transparently.
    Regex(String, String),
}

impl Value {
    pub fn to_str(&self) -> String {
        match self {
            Value::Undef => String::new(),
            Value::Str(s) => s.clone(),
            Value::Num(n) => format_number(*n),
            Value::ArrayRef(r) => format!("ARRAY(0x{:x})", Rc::as_ptr(r) as usize),
            Value::HashRef(r) => format!("HASH(0x{:x})", Rc::as_ptr(r) as usize),
            Value::ScalarRef(r) => format!("SCALAR(0x{:x})", Rc::as_ptr(r) as usize),
            Value::CodeRef(name) => format!("CODE({name})"),
            Value::Regex(pat, flags) => format!("(?^{flags}:{pat})"),
        }
    }

    pub fn to_num(&self) -> f64 {
        match self {
            Value::Undef => 0.0,
            Value::Num(n) => *n,
            Value::Str(s) => parse_number(s),
            // References stringify then parse as "ARRAY(0x..)" etc. — the
            // numeric coercion returns 0 since there are no leading digits.
            Value::ArrayRef(_)
            | Value::HashRef(_)
            | Value::ScalarRef(_)
            | Value::CodeRef(_)
            | Value::Regex(_, _) => 0.0,
        }
    }

    pub fn to_bool(&self) -> bool {
        match self {
            Value::Undef => false,
            Value::Num(n) => *n != 0.0 && !n.is_nan(),
            Value::Str(s) => !s.is_empty() && s != "0",
            // References are always true (they stringify to non-"" non-"0").
            Value::ArrayRef(_)
            | Value::HashRef(_)
            | Value::ScalarRef(_)
            | Value::CodeRef(_)
            | Value::Regex(_, _) => true,
        }
    }

    pub fn is_undef(&self) -> bool {
        matches!(self, Value::Undef)
    }

    /// Returns the reference type name for `ref()` — `""` for non-refs.
    pub fn ref_type(&self) -> &'static str {
        match self {
            Value::ArrayRef(_) => "ARRAY",
            Value::HashRef(_) => "HASH",
            Value::ScalarRef(_) => "SCALAR",
            Value::CodeRef(_) => "CODE",
            Value::Regex(_, _) => "Regexp",
            _ => "",
        }
    }
}

/// Format a number the way Perl does: equivalent to C's sprintf("%.15g", n).
pub fn format_number(n: f64) -> String {
    if n.is_nan() {
        return "NaN".to_string();
    }
    if n.is_infinite() {
        return if n > 0.0 {
            "Inf".to_string()
        } else {
            "-Inf".to_string()
        };
    }

    // Integer fast path: if it's an exact integer in safe range, format as int
    if n.fract() == 0.0 && n.abs() < 1e16 {
        return format!("{}", n as i64);
    }

    // Use %.15g behavior
    let abs_n = n.abs();
    if abs_n == 0.0 {
        return "0".to_string();
    }

    let exp = abs_n.log10().floor() as i32;
    let precision: i32 = 15;

    if exp >= -4 && exp < precision {
        // Fixed notation
        let decimal_digits = (precision - 1 - exp).max(0) as usize;
        let s = format!("{:.prec$}", n, prec = decimal_digits);
        trim_trailing_zeros(&s)
    } else {
        // Scientific notation
        let p = 10.0_f64.powi(exp);
        let mantissa = n / p;
        let s = format!("{:.prec$}", mantissa, prec = (precision - 1) as usize);
        let s = trim_trailing_zeros(&s);
        if exp >= 0 {
            format!("{s}e+{exp:02}")
        } else {
            format!("{}e-{:02}", s, -exp)
        }
    }
}

fn trim_trailing_zeros(s: &str) -> String {
    if s.contains('.') {
        let s = s.trim_end_matches('0');
        s.trim_end_matches('.').to_string()
    } else {
        s.to_string()
    }
}

/// Parse a string to a number the way Perl does.
/// Leading whitespace is skipped; parsing stops at the first non-numeric char.
pub fn parse_number(s: &str) -> f64 {
    let s = s.trim_start();
    if s.is_empty() {
        return 0.0;
    }

    // Try to parse as much of the string as possible
    let mut end = 0;
    let bytes = s.as_bytes();

    // Optional sign
    if end < bytes.len() && (bytes[end] == b'+' || bytes[end] == b'-') {
        end += 1;
    }

    // Check for hex/octal/binary prefixes
    if end < bytes.len() && bytes[end] == b'0' && end + 1 < bytes.len() {
        match bytes[end + 1] {
            b'x' | b'X' => {
                end += 2;
                let start = end;
                while end < bytes.len() && bytes[end].is_ascii_hexdigit() {
                    end += 1;
                }
                if end > start {
                    let hex_str = &s[..end];
                    if let Ok(v) = i64::from_str_radix(&hex_str[start..end], 16) {
                        let sign = if s.starts_with('-') { -1.0 } else { 1.0 };
                        return sign * v as f64;
                    }
                }
                return 0.0;
            }
            b'b' | b'B' => {
                end += 2;
                let start = end;
                while end < bytes.len() && (bytes[end] == b'0' || bytes[end] == b'1') {
                    end += 1;
                }
                if end > start {
                    if let Ok(v) = i64::from_str_radix(&s[start..end], 2) {
                        let sign = if s.starts_with('-') { -1.0 } else { 1.0 };
                        return sign * v as f64;
                    }
                }
                return 0.0;
            }
            _ => {}
        }
    }

    // Digits before decimal
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
    }

    // Decimal point and digits after
    if end < bytes.len() && bytes[end] == b'.' {
        end += 1;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }
    }

    // Exponent
    if end < bytes.len() && (bytes[end] == b'e' || bytes[end] == b'E') {
        end += 1;
        if end < bytes.len() && (bytes[end] == b'+' || bytes[end] == b'-') {
            end += 1;
        }
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }
    }

    if end == 0 || (end == 1 && (bytes[0] == b'+' || bytes[0] == b'-')) {
        return 0.0;
    }

    s[..end].parse::<f64>().unwrap_or(0.0)
}
