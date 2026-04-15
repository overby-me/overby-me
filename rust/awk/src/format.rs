use crate::value::Value;

pub fn sprintf_impl(vals: &[Value]) -> String {
    if vals.is_empty() {
        return String::new();
    }
    let fmt = vals[0].to_string_val();
    let mut result = String::new();
    let chars: Vec<char> = fmt.chars().collect();
    let mut i = 0;
    let mut arg_idx = 1;

    while i < chars.len() {
        if chars[i] == '%' {
            i += 1;
            if i >= chars.len() {
                result.push('%');
                break;
            }
            if chars[i] == '%' {
                result.push('%');
                i += 1;
                continue;
            }

            // Parse format spec
            let mut flags = String::new();
            while i < chars.len() && "-+ #0".contains(chars[i]) {
                flags.push(chars[i]);
                i += 1;
            }

            let mut width = String::new();
            if i < chars.len() && chars[i] == '*' {
                if arg_idx < vals.len() {
                    width = format!("{}", vals[arg_idx].to_num() as i64);
                    arg_idx += 1;
                }
                i += 1;
            } else {
                while i < chars.len() && chars[i].is_ascii_digit() {
                    width.push(chars[i]);
                    i += 1;
                }
            }

            let mut precision = String::new();
            let has_precision = if i < chars.len() && chars[i] == '.' {
                i += 1;
                if i < chars.len() && chars[i] == '*' {
                    if arg_idx < vals.len() {
                        precision = format!("{}", vals[arg_idx].to_num() as i64);
                        arg_idx += 1;
                    }
                    i += 1;
                } else {
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        precision.push(chars[i]);
                        i += 1;
                    }
                }
                true
            } else {
                false
            };

            if i >= chars.len() {
                break;
            }

            let conv = chars[i];
            i += 1;

            let val = if arg_idx < vals.len() {
                &vals[arg_idx]
            } else {
                &Value::Uninitialized
            };
            arg_idx += 1;

            let width_val: i64 = width.parse().unwrap_or(0);
            let width_num: usize = width_val.unsigned_abs() as usize;
            let prec_num: usize = precision.parse().unwrap_or(6);
            // Negative width from * means left-align
            let left_align = flags.contains('-') || width_val < 0;
            // Zero flag is ignored when precision is given for integer conversions
            let zero_pad = flags.contains('0')
                && !left_align
                && !(has_precision && matches!(conv, 'd' | 'i' | 'o' | 'x' | 'X' | 'u'));
            let plus_sign = flags.contains('+');
            let space_sign = flags.contains(' ');
            let alt_form = flags.contains('#');

            let formatted = match conv {
                'd' | 'i' => {
                    let n = val.to_num() as i64;
                    // Handle precision: %.0d with 0 produces empty string (or just sign)
                    if has_precision && prec_num == 0 && n == 0 {
                        if plus_sign {
                            "+".to_string()
                        } else if space_sign {
                            " ".to_string()
                        } else {
                            String::new()
                        }
                    } else {
                        let abs_str = if has_precision {
                            let s = format!("{}", n.unsigned_abs());
                            if s.len() < prec_num {
                                format!("{:0>width$}", s, width = prec_num)
                            } else {
                                s
                            }
                        } else {
                            format!("{}", n.unsigned_abs())
                        };
                        if n < 0 {
                            format!("-{abs_str}")
                        } else if plus_sign {
                            format!("+{abs_str}")
                        } else if space_sign {
                            format!(" {abs_str}")
                        } else {
                            abs_str
                        }
                    }
                }
                'o' => {
                    let n = val.to_num() as u64;
                    if has_precision && prec_num == 0 && n == 0 {
                        // %.0o with 0: empty, but # flag gives "0"
                        if alt_form { "0".to_string() } else { String::new() }
                    } else {
                        let s = format!("{n:o}");
                        let s = if has_precision && s.len() < prec_num {
                            format!("{s:0>width$}", width = prec_num)
                        } else {
                            s
                        };
                        if alt_form && !s.starts_with('0') {
                            format!("0{s}")
                        } else {
                            s
                        }
                    }
                }
                'x' => {
                    let n = val.to_num() as u64;
                    if has_precision && prec_num == 0 && n == 0 {
                        String::new()
                    } else {
                        let s = format!("{n:x}");
                        let s = if has_precision && s.len() < prec_num {
                            format!("{s:0>width$}", width = prec_num)
                        } else {
                            s
                        };
                        if alt_form && n != 0 {
                            format!("0x{s}")
                        } else {
                            s
                        }
                    }
                }
                'X' => {
                    let n = val.to_num() as u64;
                    if has_precision && prec_num == 0 && n == 0 {
                        String::new()
                    } else {
                        let s = format!("{n:X}");
                        let s = if has_precision && s.len() < prec_num {
                            format!("{s:0>width$}", width = prec_num)
                        } else {
                            s
                        };
                        if alt_form && n != 0 {
                            format!("0X{s}")
                        } else {
                            s
                        }
                    }
                }
                'u' => format!("{}", val.to_num() as u64),
                'c' => {
                    // If the value is a string, use first character
                    match val {
                        Value::Str(s) if !s.is_empty() => {
                            s.chars().next().unwrap().to_string()
                        }
                        _ => {
                            let n = val.to_num() as u32;
                            if n == 0 {
                                "\0".to_string()
                            } else if let Some(c) = char::from_u32(n) {
                                c.to_string()
                            } else {
                                "\0".to_string()
                            }
                        }
                    }
                }
                's' => {
                    let s = val.to_string_val();
                    if has_precision {
                        let chars: Vec<char> = s.chars().collect();
                        chars[..chars.len().min(prec_num)].iter().collect()
                    } else {
                        s
                    }
                }
                'f' => {
                    let n = val.to_num();
                    if n.is_nan() {
                        "nan".to_string()
                    } else if n.is_infinite() {
                        if n < 0.0 { "-inf".to_string() } else { "inf".to_string() }
                    } else {
                        let p = if has_precision { prec_num } else { 6 };
                        let mut s = format!("{n:.prec$}", prec = p);
                        // # flag: always show decimal point
                        if alt_form && p == 0 && !s.contains('.') {
                            s.push('.');
                        }
                        if plus_sign && n >= 0.0 {
                            format!("+{s}")
                        } else if space_sign && n >= 0.0 {
                            format!(" {s}")
                        } else {
                            s
                        }
                    }
                }
                'e' => {
                    let n = val.to_num();
                    let p = if has_precision { prec_num } else { 6 };
                    let s = format_scientific(n, p, false);
                    if plus_sign && n >= 0.0 {
                        format!("+{s}")
                    } else if space_sign && n >= 0.0 {
                        format!(" {s}")
                    } else {
                        s
                    }
                }
                'E' => {
                    let n = val.to_num();
                    let p = if has_precision { prec_num } else { 6 };
                    let s = format_scientific(n, p, true);
                    if plus_sign && n >= 0.0 {
                        format!("+{s}")
                    } else if space_sign && n >= 0.0 {
                        format!(" {s}")
                    } else {
                        s
                    }
                }
                'g' | 'G' => {
                    let n = val.to_num();
                    let p = if has_precision { prec_num.max(1) } else { 6 };
                    let s = if alt_form {
                        // # flag: don't strip trailing zeros
                        format_g_alt(n, p, conv == 'G')
                    } else {
                        format_g(n, p, conv == 'G')
                    };
                    if plus_sign && n >= 0.0 {
                        format!("+{s}")
                    } else if space_sign && n >= 0.0 {
                        format!(" {s}")
                    } else {
                        s
                    }
                }
                _ => format!("%{conv}"),
            };

            // Apply width padding
            if width_num > 0 && formatted.len() < width_num {
                let pad = width_num - formatted.len();
                if left_align {
                    result.push_str(&formatted);
                    for _ in 0..pad {
                        result.push(' ');
                    }
                } else if zero_pad
                    && matches!(conv, 'd' | 'i' | 'o' | 'x' | 'X' | 'u' | 'f' | 'e' | 'E' | 'g' | 'G')
                {
                    // Put sign before zeros
                    if formatted.starts_with('-') || formatted.starts_with('+') {
                        result.push(formatted.chars().next().unwrap());
                        for _ in 0..pad {
                            result.push('0');
                        }
                        result.push_str(&formatted[1..]);
                    } else {
                        for _ in 0..pad {
                            result.push('0');
                        }
                        result.push_str(&formatted);
                    }
                } else {
                    for _ in 0..pad {
                        result.push(' ');
                    }
                    result.push_str(&formatted);
                }
            } else {
                result.push_str(&formatted);
            }
        } else if chars[i] == '\\' {
            i += 1;
            if i < chars.len() {
                match chars[i] {
                    'n' => result.push('\n'),
                    't' => result.push('\t'),
                    'r' => result.push('\r'),
                    '\\' => result.push('\\'),
                    '"' => result.push('"'),
                    'a' => result.push('\x07'),
                    'b' => result.push('\x08'),
                    'f' => result.push('\x0C'),
                    '/' => result.push('/'),
                    _ => {
                        result.push('\\');
                        result.push(chars[i]);
                    }
                }
                i += 1;
            }
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    result
}

/// Replacement for sub/gsub: only & and \\ and \& are special
pub fn awk_replace(replacement: &str, matched: &str) -> String {
    let mut result = String::new();
    let chars: Vec<char> = replacement.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' {
            i += 1;
            if i < chars.len() {
                if chars[i] == '&' {
                    result.push('&');
                } else if chars[i] == '\\' {
                    result.push('\\');
                } else {
                    // In POSIX sub/gsub, \x where x is not & or \ produces \x
                    result.push('\\');
                    result.push(chars[i]);
                }
            } else {
                result.push('\\');
            }
            i += 1;
        } else if chars[i] == '&' {
            result.push_str(matched);
            i += 1;
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    result
}

/// Replacement for gensub: supports & \\ \& and \1..\9 backreferences
pub fn gensub_replace(replacement: &str, caps: &regex::Captures) -> String {
    let mut result = String::new();
    let chars: Vec<char> = replacement.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' {
            i += 1;
            if i < chars.len() {
                if chars[i] == '&' {
                    result.push('&');
                } else if chars[i] == '\\' {
                    result.push('\\');
                } else if chars[i].is_ascii_digit() {
                    let n = (chars[i] as u32 - '0' as u32) as usize;
                    if let Some(m) = caps.get(n) {
                        result.push_str(m.as_str());
                    }
                } else {
                    result.push('\\');
                    result.push(chars[i]);
                }
            }
            i += 1;
        } else if chars[i] == '&' {
            result.push_str(&caps[0]);
            i += 1;
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    result
}

pub fn format_scientific(n: f64, prec: usize, upper: bool) -> String {
    if n.is_nan() {
        return if upper { "NAN".to_string() } else { "nan".to_string() };
    }
    if n.is_infinite() {
        let s = if upper { "INF" } else { "inf" };
        return if n < 0.0 { format!("-{s}") } else { s.to_string() };
    }
    if n == 0.0 {
        let e_char = if upper { 'E' } else { 'e' };
        return format!("0.{:0>width$}{e_char}+00", "", width = prec);
    }
    let sign = if n < 0.0 { "-" } else { "" };
    let n = n.abs();
    let exp = n.log10().floor() as i32;
    let mantissa = n / 10f64.powi(exp);
    let e_char = if upper { 'E' } else { 'e' };
    let exp_sign = if exp >= 0 { '+' } else { '-' };
    let exp_abs = exp.unsigned_abs();
    format!(
        "{sign}{mantissa:.prec$}{e_char}{exp_sign}{exp_abs:02}",
        prec = prec
    )
}

pub fn format_g(n: f64, prec: usize, upper: bool) -> String {
    if n.is_nan() {
        return if upper { "NAN".to_string() } else { "nan".to_string() };
    }
    if n.is_infinite() {
        let s = if upper { "INF" } else { "inf" };
        return if n < 0.0 { format!("-{s}") } else { s.to_string() };
    }
    if n == 0.0 {
        return "0".to_string();
    }
    let exp = n.abs().log10().floor() as i32;
    if exp >= -(prec as i32) && exp < prec as i32 {
        // Use fixed notation
        let decimal_places = if prec as i32 - 1 - exp > 0 {
            (prec as i32 - 1 - exp) as usize
        } else {
            0
        };
        let s = format!("{n:.prec$}", prec = decimal_places);
        // Trim trailing zeros
        if s.contains('.') {
            let s = s.trim_end_matches('0');
            let s = s.trim_end_matches('.');
            s.to_string()
        } else {
            s
        }
    } else {
        // Use scientific notation
        let p = if prec > 0 { prec - 1 } else { 0 };
        format_scientific(n, p, upper)
    }
}

/// Like format_g but with # flag: don't strip trailing zeros, always show decimal point
pub fn format_g_alt(n: f64, prec: usize, upper: bool) -> String {
    if n.is_nan() {
        return if upper { "NAN".to_string() } else { "nan".to_string() };
    }
    if n.is_infinite() {
        let s = if upper { "INF" } else { "inf" };
        return if n < 0.0 { format!("-{s}") } else { s.to_string() };
    }
    if n == 0.0 {
        return format!("0.{:0>width$}", "", width = prec.saturating_sub(1).max(1));
    }
    let exp = n.abs().log10().floor() as i32;
    if exp >= -(prec as i32) && exp < prec as i32 {
        let decimal_places = if prec as i32 - 1 - exp > 0 {
            (prec as i32 - 1 - exp) as usize
        } else {
            0
        };
        let s = format!("{n:.prec$}", prec = decimal_places);
        // # flag: keep trailing zeros and decimal point
        if !s.contains('.') {
            format!("{s}.")
        } else {
            s
        }
    } else {
        let p = if prec > 0 { prec - 1 } else { 0 };
        format_scientific(n, p, upper)
    }
}
