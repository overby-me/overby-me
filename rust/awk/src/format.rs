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

            let width_num: usize = width.parse().unwrap_or(0);
            let prec_num: usize = precision.parse().unwrap_or(6);
            let left_align = flags.contains('-');
            let zero_pad = flags.contains('0') && !left_align;
            let plus_sign = flags.contains('+');
            let space_sign = flags.contains(' ');

            let formatted = match conv {
                'd' | 'i' => {
                    let n = val.to_num() as i64;

                    if plus_sign && n >= 0 {
                        format!("+{n}")
                    } else if space_sign && n >= 0 {
                        format!(" {n}")
                    } else {
                        format!("{n}")
                    }
                }
                'o' => format!("{:o}", val.to_num() as u64),
                'x' => format!("{:x}", val.to_num() as u64),
                'X' => format!("{:X}", val.to_num() as u64),
                'u' => format!("{}", val.to_num() as u64),
                'c' => {
                    let n = val.to_num() as u32;
                    if let Some(c) = char::from_u32(n) {
                        c.to_string()
                    } else {
                        let s = val.to_string_val();
                        if let Some(c) = s.chars().next() {
                            c.to_string()
                        } else {
                            "\0".to_string()
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
                    let p = if has_precision { prec_num } else { 6 };
                    let s = format!("{n:.prec$}", prec = p);
                    if plus_sign && n >= 0.0 {
                        format!("+{s}")
                    } else if space_sign && n >= 0.0 {
                        format!(" {s}")
                    } else {
                        s
                    }
                }
                'e' => {
                    let n = val.to_num();
                    let p = if has_precision { prec_num } else { 6 };
                    format_scientific(n, p, false)
                }
                'E' => {
                    let n = val.to_num();
                    let p = if has_precision { prec_num } else { 6 };
                    format_scientific(n, p, true)
                }
                'g' | 'G' => {
                    let n = val.to_num();
                    let p = if has_precision { prec_num.max(1) } else { 6 };
                    format_g(n, p, conv == 'G')
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
                    && matches!(conv, 'd' | 'i' | 'f' | 'e' | 'E' | 'g' | 'G')
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

pub fn awk_replace(replacement: &str, caps: &regex::Captures) -> String {
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
    if n == 0.0 {
        return "0".to_string();
    }
    let exp = if n == 0.0 {
        0
    } else {
        n.abs().log10().floor() as i32
    };
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
