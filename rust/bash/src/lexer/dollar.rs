use super::*;

use crate::parser::parse_comsub;

pub fn parse_dollar(chars: &[char], i: &mut usize, in_dquote: bool) -> WordPart {
    parse_dollar_inner(chars, i, in_dquote, &mut Vec::new())
}

pub fn parse_dollar_with_warnings(
    chars: &[char],
    i: &mut usize,
    in_dquote: bool,
    warnings: &mut Vec<(usize, usize, String)>,
) -> WordPart {
    parse_dollar_inner(chars, i, in_dquote, warnings)
}

fn parse_dollar_inner(
    chars: &[char],
    i: &mut usize,
    in_dquote: bool,
    heredoc_eof_warnings: &mut Vec<(usize, usize, String)>,
) -> WordPart {
    if *i >= chars.len() {
        return WordPart::Literal("$".to_string());
    }

    match chars[*i] {
        '[' => {
            // Old-style arithmetic: $[expr]
            *i += 1;
            let mut expr = String::new();
            let mut depth = 1;
            while *i < chars.len() && depth > 0 {
                if chars[*i] == '[' {
                    depth += 1;
                } else if chars[*i] == ']' {
                    depth -= 1;
                    if depth == 0 {
                        *i += 1;
                        break;
                    }
                }
                expr.push(chars[*i]);
                *i += 1;
            }
            WordPart::ArithSub(expr)
        }
        '(' => {
            *i += 1;
            if *i < chars.len() && chars[*i] == '(' {
                // Try arithmetic: $(( ... )), fall back to command sub if content has ';'
                let saved_i = *i;
                *i += 1;
                let mut depth = 1; // nested $(( )) depth
                let mut paren_depth = 0i32; // inner () depth
                let mut brace_depth = 0i32; // track {} for funsubs
                let mut expr = String::new();
                let mut has_semicolon_at_top = false;
                while *i < chars.len() && depth > 0 {
                    if *i + 1 < chars.len()
                        && chars[*i] == ')'
                        && chars[*i + 1] == ')'
                        && paren_depth <= 0
                        && brace_depth <= 0
                    {
                        depth -= 1;
                        if depth == 0 {
                            *i += 2;
                            break;
                        }
                        expr.push(')');
                        expr.push(')');
                        *i += 2;
                    } else if *i + 1 < chars.len() && chars[*i] == '$' && chars[*i + 1] == '(' {
                        if *i + 2 < chars.len() && chars[*i + 2] == '(' {
                            depth += 1;
                        }
                        expr.push(chars[*i]);
                        *i += 1;
                    } else if chars[*i] == '$'
                        && *i + 1 < chars.len()
                        && chars[*i + 1] == '{'
                        && *i + 2 < chars.len()
                        && matches!(chars[*i + 2], ' ' | '\t' | '\n' | '|')
                    {
                        // Start of funsub ${ cmd; } or valuesub ${| cmd; }
                        // Push '$' and advance; '{' will be handled by the
                        // brace_depth tracking on the next iteration.
                        expr.push(chars[*i]);
                        *i += 1;
                    } else {
                        if chars[*i] == '{' {
                            brace_depth += 1;
                        } else if chars[*i] == '}' && brace_depth > 0 {
                            brace_depth -= 1;
                        } else if chars[*i] == '(' {
                            paren_depth += 1;
                        } else if chars[*i] == ')' {
                            paren_depth -= 1;
                        } else if chars[*i] == ';' && paren_depth <= 0 && brace_depth <= 0 {
                            has_semicolon_at_top = true;
                        } else if chars[*i] == '\'' && brace_depth > 0 {
                            // Skip single-quoted string inside funsub
                            expr.push(chars[*i]);
                            *i += 1;
                            while *i < chars.len() && chars[*i] != '\'' {
                                expr.push(chars[*i]);
                                *i += 1;
                            }
                            if *i < chars.len() {
                                expr.push(chars[*i]);
                                *i += 1;
                            }
                            continue;
                        } else if chars[*i] == '"' && brace_depth > 0 {
                            // Skip double-quoted string inside funsub
                            expr.push(chars[*i]);
                            *i += 1;
                            while *i < chars.len() && chars[*i] != '"' {
                                if chars[*i] == '\\' && *i + 1 < chars.len() {
                                    expr.push(chars[*i]);
                                    *i += 1;
                                }
                                expr.push(chars[*i]);
                                *i += 1;
                            }
                            if *i < chars.len() {
                                expr.push(chars[*i]);
                                *i += 1;
                            }
                            continue;
                        }
                        expr.push(chars[*i]);
                        *i += 1;
                    }
                }
                if has_semicolon_at_top || depth > 0 {
                    // Content has ';' at top paren level, or no matching ))
                    // found — reparse as $( (cmd) ) command substitution
                    *i = saved_i; // Back to the second '('
                // Fall through to command substitution below
                } else {
                    return WordPart::ArithSub(expr);
                }
            }
            // Command substitution: $( ... )
            // Use full recursive parse like C bash's parse_comsub/xparse_dolparen.
            // This runs the real parser on the comsub contents so that case/esac,
            // done/while, reserved words, heredocs etc. are handled correctly.
            let remaining: String = chars[*i..].iter().collect();
            let aliases = COMSUB_ALIASES.with(|a| a.borrow().clone());
            // Compute line offset: count newlines before the current position
            // so heredoc warnings report correct line numbers relative to the script.
            let line_offset = chars[..*i].iter().filter(|&&c| c == '\n').count();
            let result = parse_comsub(&remaining, aliases, false, false, in_dquote, line_offset);

            // Advance past the consumed characters
            *i += result.chars_consumed;

            // Forward heredoc EOF warnings
            for w in &result.heredoc_eof_warnings {
                heredoc_eof_warnings.push(w.clone());
            }

            // Forward unterminated-heredoc-in-comsub warnings to the
            // heredoc_eof_warnings vec so the interpreter can emit the
            // "command substitution: N unterminated here-document" warning.
            if result.unterminated_heredoc_count > 0 {
                // Use the line where `$(` appears, not where the heredoc body ends.
                // `line_offset` counts newlines before `*i` (before the comsub content),
                // so `line_offset + 1` is the 1-based line of the `$(`.
                let warn_line = line_offset + 1;
                heredoc_eof_warnings.push((
                    warn_line,
                    0, // sentinel: start_line=0 means comsub-unterminated warning
                    format!(
                        "\x00COMSUB_UNTERMINATED:{}",
                        result.unterminated_heredoc_count
                    ),
                ));
            }

            if result.text == "\x00SILENT_COMSUB" {
                // `}` at comsub depth 1 in dquote — the enclosing ${...} closes.
                // Don't advance *i — the `}` hasn't been consumed.
                return WordPart::CommandSub("\x00SILENT_COMSUB".to_string());
            }

            if let Some(ref err) = result.syntax_error {
                // Syntax error inside comsub — propagate as SyntaxError so the
                // parser can detect it and abort parsing, like C bash's
                // jump_to_top_level(FORCE_EOF) in parse_comsub.
                // Prefix with "COMSUB:" so the error handler adds
                // "while looking for matching ')'" suffix.
                WordPart::SyntaxError(format!("COMSUB:{}", err))
            } else if result.incomplete {
                // No closing ) found — incomplete comsub
                let eof_line = chars.iter().filter(|&&c| c == '\n').count() + 1;
                WordPart::CommandSub(format!("\x00INCOMPLETE_COMSUB:{}", eof_line))
            } else {
                WordPart::CommandSub(result.text)
            }
        }
        '{' => {
            *i += 1;
            // Check for funsub: ${ cmd; } — space/tab/newline after {
            // or valuesub: ${| cmd; } — pipe after {
            if *i < chars.len() && matches!(chars[*i], ' ' | '\t' | '\n' | '|') {
                // Detect value substitution: ${| ... }
                let is_valuesub = chars[*i] == '|';
                if is_valuesub {
                    // Skip the '|' — it's not part of the command
                    *i += 1;
                }
                // Parse as command substitution delimited by }
                // Funsub requires that } is preceded by a command terminator (;/\n/&)
                // or a closing compound command delimiter ()/`done`/`fi`/etc.)
                // at the SAME depth level (not from nested blocks)
                let mut depth = 1;
                let mut paren_depth = 0i32; // track () for subshells
                let mut cmd = String::new();
                let mut has_terminator_at_depth1 = false;
                let mut has_nonws_at_depth1 = false;
                while *i < chars.len() && depth > 0 {
                    match chars[*i] {
                        '(' => {
                            cmd.push(chars[*i]);
                            if depth == 1 {
                                paren_depth += 1;
                                has_nonws_at_depth1 = true;
                                has_terminator_at_depth1 = false;
                            }
                        }
                        ')' => {
                            cmd.push(chars[*i]);
                            if depth == 1 && paren_depth > 0 {
                                paren_depth -= 1;
                                // Closing a subshell/group at brace depth 1
                                // constitutes a complete command, so } can
                                // follow without a ; terminator.
                                if paren_depth == 0 {
                                    has_terminator_at_depth1 = true;
                                }
                                has_nonws_at_depth1 = true;
                            } else if depth == 1 {
                                has_nonws_at_depth1 = true;
                                has_terminator_at_depth1 = false;
                            }
                        }
                        '{' => {
                            depth += 1;
                            cmd.push(chars[*i]);
                        }
                        '}' => {
                            if depth == 1 && (has_terminator_at_depth1 || !has_nonws_at_depth1) {
                                // Valid funsub close: either has terminator or empty content
                                depth = 0;
                            } else if depth > 1 {
                                depth -= 1;
                                cmd.push(chars[*i]);
                            } else {
                                // depth == 1 but no terminator at this level
                                cmd.push(chars[*i]);
                                has_nonws_at_depth1 = true;
                            }
                        }
                        ';' | '&' | '\n' => {
                            cmd.push(chars[*i]);
                            if depth == 1 {
                                has_terminator_at_depth1 = true;
                                has_nonws_at_depth1 = true;
                            }
                        }
                        '\'' => {
                            cmd.push(chars[*i]);
                            *i += 1;
                            while *i < chars.len() && chars[*i] != '\'' {
                                cmd.push(chars[*i]);
                                *i += 1;
                            }
                            if *i < chars.len() {
                                cmd.push(chars[*i]);
                            }
                            if depth == 1 {
                                has_terminator_at_depth1 = false;
                                has_nonws_at_depth1 = true;
                            }
                        }
                        '"' => {
                            cmd.push(chars[*i]);
                            *i += 1;
                            while *i < chars.len() && chars[*i] != '"' {
                                if chars[*i] == '\\' && *i + 1 < chars.len() {
                                    cmd.push(chars[*i]);
                                    *i += 1;
                                }
                                cmd.push(chars[*i]);
                                *i += 1;
                            }
                            if *i < chars.len() {
                                cmd.push(chars[*i]);
                            }
                            if depth == 1 {
                                has_terminator_at_depth1 = false;
                                has_nonws_at_depth1 = true;
                            }
                        }
                        ' ' | '\t' => {
                            cmd.push(chars[*i]);
                            // Whitespace doesn't affect terminator or nonws state
                        }
                        '$' => {
                            cmd.push(chars[*i]);
                            // Check for $(...), $((...)), ${ ... }, $'...', $"..."
                            // These nested constructs should be skipped without
                            // affecting paren_depth or has_terminator_at_depth1.
                            // In particular, $() closing paren must NOT set the
                            // terminator flag (bash: `${ $() }` is invalid without `;`).
                            if *i + 1 < chars.len() && chars[*i + 1] == '(' {
                                *i += 1;
                                cmd.push(chars[*i]); // '('
                                // Check for $(( — arithmetic
                                if *i + 1 < chars.len() && chars[*i + 1] == '(' {
                                    *i += 1;
                                    cmd.push(chars[*i]); // second '('
                                    let mut arith_depth = 1i32;
                                    while *i + 1 < chars.len() && arith_depth > 0 {
                                        *i += 1;
                                        cmd.push(chars[*i]);
                                        if chars[*i] == '('
                                            && *i + 1 < chars.len()
                                            && chars[*i + 1] == '('
                                        {
                                            // nested $((
                                        } else if chars[*i] == ')'
                                            && *i + 1 < chars.len()
                                            && chars[*i + 1] == ')'
                                        {
                                            arith_depth -= 1;
                                            if arith_depth == 0 {
                                                *i += 1;
                                                cmd.push(chars[*i]); // second ')'
                                                break;
                                            }
                                        }
                                    }
                                } else {
                                    // $(...) — command substitution
                                    let mut comsub_depth = 1i32;
                                    while *i + 1 < chars.len() && comsub_depth > 0 {
                                        *i += 1;
                                        if chars[*i] == '(' {
                                            comsub_depth += 1;
                                        } else if chars[*i] == ')' {
                                            comsub_depth -= 1;
                                            if comsub_depth == 0 {
                                                cmd.push(chars[*i]);
                                                break;
                                            }
                                        } else if chars[*i] == '\'' {
                                            cmd.push(chars[*i]);
                                            *i += 1;
                                            while *i < chars.len() && chars[*i] != '\'' {
                                                cmd.push(chars[*i]);
                                                *i += 1;
                                            }
                                            if *i < chars.len() {
                                                cmd.push(chars[*i]);
                                            }
                                            continue;
                                        } else if chars[*i] == '"' {
                                            cmd.push(chars[*i]);
                                            *i += 1;
                                            while *i < chars.len() && chars[*i] != '"' {
                                                if chars[*i] == '\\' && *i + 1 < chars.len() {
                                                    cmd.push(chars[*i]);
                                                    *i += 1;
                                                }
                                                cmd.push(chars[*i]);
                                                *i += 1;
                                            }
                                            if *i < chars.len() {
                                                cmd.push(chars[*i]);
                                            }
                                            continue;
                                        }
                                        cmd.push(chars[*i]);
                                    }
                                }
                                if depth == 1 {
                                    has_terminator_at_depth1 = false;
                                    has_nonws_at_depth1 = true;
                                }
                            } else if *i + 1 < chars.len() && chars[*i + 1] == '{' {
                                *i += 1;
                                cmd.push(chars[*i]); // '{'
                                // Check if this is a nested funsub ${ ... } or ${param}
                                // Either way, skip matching braces
                                let mut nested_depth = 1i32;
                                while *i + 1 < chars.len() && nested_depth > 0 {
                                    *i += 1;
                                    cmd.push(chars[*i]);
                                    if chars[*i] == '{' {
                                        nested_depth += 1;
                                    } else if chars[*i] == '}' {
                                        nested_depth -= 1;
                                    } else if chars[*i] == '\'' {
                                        *i += 1;
                                        while *i < chars.len() && chars[*i] != '\'' {
                                            cmd.push(chars[*i]);
                                            *i += 1;
                                        }
                                        if *i < chars.len() {
                                            cmd.push(chars[*i]);
                                        }
                                    } else if chars[*i] == '"' {
                                        *i += 1;
                                        while *i < chars.len() && chars[*i] != '"' {
                                            if chars[*i] == '\\' && *i + 1 < chars.len() {
                                                cmd.push(chars[*i]);
                                                *i += 1;
                                            }
                                            cmd.push(chars[*i]);
                                            *i += 1;
                                        }
                                        if *i < chars.len() {
                                            cmd.push(chars[*i]);
                                        }
                                    }
                                }
                                if depth == 1 {
                                    has_terminator_at_depth1 = false;
                                    has_nonws_at_depth1 = true;
                                }
                            } else {
                                // $var, $$, $!, etc — just a regular char
                                if depth == 1 {
                                    has_terminator_at_depth1 = false;
                                    has_nonws_at_depth1 = true;
                                }
                            }
                        }
                        _ => {
                            cmd.push(chars[*i]);
                            if depth == 1 {
                                has_terminator_at_depth1 = false;
                                has_nonws_at_depth1 = true;
                            }
                        }
                    }
                    *i += 1;
                }
                if depth > 0 {
                    // Unclosed funsub — return as incomplete
                    if is_valuesub {
                        WordPart::ValueSub(format!("\x00INCOMPLETE_FUNSUB{}", cmd))
                    } else {
                        WordPart::FunSub(format!("\x00INCOMPLETE_FUNSUB{}", cmd))
                    }
                } else if is_valuesub {
                    WordPart::ValueSub(cmd)
                } else {
                    WordPart::FunSub(cmd)
                }
            } else {
                parse_brace_param(chars, i, in_dquote)
            }
        }
        ch if ch == '_' || ch.is_alphabetic() => {
            let mut name = String::new();
            while *i < chars.len() && (chars[*i] == '_' || chars[*i].is_alphanumeric()) {
                name.push(chars[*i]);
                *i += 1;
            }
            WordPart::Variable(name)
        }
        ch if ch.is_ascii_digit() => {
            let mut name = String::new();
            name.push(chars[*i]);
            *i += 1;
            WordPart::Variable(name)
        }
        '@' | '*' | '#' | '?' | '-' | '$' | '!' | '0' => {
            let name = chars[*i].to_string();
            *i += 1;
            WordPart::Variable(name)
        }
        '"' => {
            // $"..." locale-specific quoting — treat as regular double quoting
            *i += 1; // skip "
            let mut dq_parts = Vec::new();
            let mut dq_lit = String::new();
            while *i < chars.len() && chars[*i] != '"' {
                match chars[*i] {
                    '\\' if *i + 1 < chars.len() => {
                        let next = chars[*i + 1];
                        if matches!(next, '$' | '`' | '"' | '\\') {
                            dq_lit.push(next);
                        } else {
                            dq_lit.push('\\');
                            dq_lit.push(next);
                        }
                        *i += 2;
                    }
                    '$' => {
                        // Inside double quotes, $' and $" are literal
                        if *i + 1 < chars.len() && matches!(chars[*i + 1], '\'' | '"') {
                            dq_lit.push('$');
                            *i += 1;
                        } else {
                            if !dq_lit.is_empty() {
                                dq_parts.push(WordPart::Literal(std::mem::take(&mut dq_lit)));
                            }
                            *i += 1;
                            dq_parts.push(parse_dollar_inner(chars, i, true, heredoc_eof_warnings));
                        }
                    }
                    ch => {
                        dq_lit.push(ch);
                        *i += 1;
                    }
                }
            }
            if *i < chars.len() {
                *i += 1; // skip closing "
            }
            if !dq_lit.is_empty() {
                dq_parts.push(WordPart::Literal(dq_lit));
            }
            WordPart::DoubleQuoted(dq_parts)
        }
        '\'' if !IN_HEREDOC.with(|f| f.get()) => {
            // $'...' ANSI-C quoting (not in heredoc context where $' is literal)
            *i += 1; // skip '
            let mut s = String::new();
            while *i < chars.len() && chars[*i] != '\'' {
                if chars[*i] == '\\' && *i + 1 < chars.len() {
                    *i += 1;
                    match chars[*i] {
                        'n' => s.push('\n'),
                        't' => s.push('\t'),
                        'r' => s.push('\r'),
                        '\\' => s.push('\\'),
                        '\'' => s.push('\''),
                        '"' => s.push('"'),
                        'a' => s.push('\x07'),
                        'b' => s.push('\x08'),
                        'c' => {
                            // \cX — control character (X ^ 0x40), like bash
                            // If next char is \, process the escape first
                            if *i + 1 < chars.len() {
                                *i += 1;
                                let target_char = if chars[*i] == '\\' && *i + 1 < chars.len() {
                                    *i += 1;
                                    chars[*i]
                                } else {
                                    chars[*i]
                                };
                                let ctrl = (target_char as u8) ^ 0x40;
                                if ctrl == 0 {
                                    break; // \c@ terminates
                                }
                                s.push(ctrl as char);
                            }
                        }
                        'e' | 'E' => s.push('\x1b'),
                        'f' => s.push('\x0c'),
                        'v' => s.push('\x0b'),
                        c @ '0'..='7' => {
                            let mut val = c as u8 - b'0';
                            for _ in 0..2 {
                                if *i + 1 < chars.len() && matches!(chars[*i + 1], '0'..='7') {
                                    *i += 1;
                                    val = val * 8 + (chars[*i] as u8 - b'0');
                                } else {
                                    break;
                                }
                            }
                            if val == 0 {
                                break; // NUL terminates
                            }
                            s.push(val as char);
                        }
                        'x' => {
                            let mut val = 0u32;
                            let mut count = 0;
                            if *i + 1 < chars.len() && chars[*i + 1] == '{' {
                                *i += 1; // skip {
                                while *i + 1 < chars.len() {
                                    *i += 1;
                                    if chars[*i] == '}' {
                                        break;
                                    }
                                    if chars[*i].is_ascii_hexdigit() {
                                        val = val * 16 + chars[*i].to_digit(16).unwrap();
                                        count += 1;
                                    } else {
                                        break;
                                    }
                                }
                            } else {
                                for _ in 0..2 {
                                    if *i + 1 < chars.len() && chars[*i + 1].is_ascii_hexdigit() {
                                        *i += 1;
                                        val = val * 16 + chars[*i].to_digit(16).unwrap();
                                        count += 1;
                                    } else {
                                        break;
                                    }
                                }
                            }
                            if count > 0 {
                                // \x produces single bytes (truncate to 0xFF)
                                let byte_val = (val & 0xFF) as u8;
                                if byte_val == 0 {
                                    break; // NUL terminates
                                }
                                s.push(byte_val as char);
                            } else {
                                s.push('\\');
                                s.push('x');
                            }
                        }
                        'u' => {
                            let mut val = 0u32;
                            let mut count = 0;
                            if *i + 1 < chars.len() && chars[*i + 1] == '{' {
                                *i += 1;
                                while *i + 1 < chars.len() {
                                    *i += 1;
                                    if chars[*i] == '}' {
                                        break;
                                    }
                                    if chars[*i].is_ascii_hexdigit() {
                                        val = val * 16 + chars[*i].to_digit(16).unwrap();
                                        count += 1;
                                    } else {
                                        break;
                                    }
                                }
                            } else {
                                for _ in 0..4 {
                                    if *i + 1 < chars.len() && chars[*i + 1].is_ascii_hexdigit() {
                                        *i += 1;
                                        val = val * 16 + chars[*i].to_digit(16).unwrap();
                                        count += 1;
                                    } else {
                                        break;
                                    }
                                }
                            }
                            if count > 0 {
                                if let Some(c) = char::from_u32(val) {
                                    s.push(c);
                                }
                            } else {
                                s.push('\\');
                                s.push('u');
                            }
                        }
                        'U' => {
                            let mut val = 0u32;
                            let mut count = 0;
                            if *i + 1 < chars.len() && chars[*i + 1] == '{' {
                                *i += 1;
                                while *i + 1 < chars.len() {
                                    *i += 1;
                                    if chars[*i] == '}' {
                                        break;
                                    }
                                    if chars[*i].is_ascii_hexdigit() {
                                        val = val * 16 + chars[*i].to_digit(16).unwrap();
                                        count += 1;
                                    } else {
                                        break;
                                    }
                                }
                            } else {
                                for _ in 0..8 {
                                    if *i + 1 < chars.len() && chars[*i + 1].is_ascii_hexdigit() {
                                        *i += 1;
                                        val = val * 16 + chars[*i].to_digit(16).unwrap();
                                        count += 1;
                                    } else {
                                        break;
                                    }
                                }
                            }
                            if count > 0 {
                                if let Some(c) = char::from_u32(val) {
                                    s.push(c);
                                }
                            } else {
                                s.push('\\');
                                s.push('U');
                            }
                        }
                        c => {
                            s.push('\\');
                            s.push(c);
                        }
                    }
                } else {
                    s.push(chars[*i]);
                }
                *i += 1;
            }
            if *i < chars.len() {
                *i += 1; // skip closing '
            }
            WordPart::SingleQuoted(s)
        }
        _ => WordPart::Literal("$".to_string()),
    }
}

fn parse_brace_param(chars: &[char], i: &mut usize, in_dquote: bool) -> WordPart {
    // ${!name} — indirect expansion / name prefix / array indices
    // In POSIX mode, ${!...} is always $! with operator (no indirect expansion)
    let posix = POSIX_MODE_DOLLAR.with(|p| p.get());
    if *i < chars.len() && chars[*i] == '!' && !posix {
        // Check if '!' should be the variable name (not indirect prefix)
        // ${!} = $!, ${!-word} = $! with default, ${!:-word} = $! with colon-default
        // vs ${!name} = indirect, ${!name-word} = indirect with op
        let next_after_bang = if *i + 1 < chars.len() {
            chars[*i + 1]
        } else {
            '}'
        };
        if next_after_bang == '}'
            || next_after_bang == ':'
            || (matches!(next_after_bang, '-' | '+' | '=' | '?')
                && (*i + 2 >= chars.len() || chars[*i + 2] != '}'))
        {
            // Treat '!' as the variable name, not indirect prefix
            // Falls through to the normal param name reading below
        } else {
            *i += 1;
            let name = read_param_name_with_subscript(chars, i);

            // Check if name ends with [@] or [*] — this is ${!arr[@]} for array indices
            if name.ends_with("[@]") || name.ends_with("[*]") {
                let ch = if name.ends_with("[@]") { '@' } else { '*' };
                let arr_name = name[..name.len() - 3].to_string();
                if *i < chars.len() && chars[*i] == '}' {
                    *i += 1;
                }
                return WordPart::Param(ParamExpr {
                    name: arr_name,
                    op: ParamOp::ArrayIndices(ch),
                });
            }

            // ${!prefix*} or ${!prefix@} — names matching prefix
            if *i < chars.len() && (chars[*i] == '*' || chars[*i] == '@') {
                let ch = chars[*i];
                // Validate that the prefix is a valid variable name prefix
                // (starts with letter or underscore). ${!1*}, ${!@*} etc. are
                // bad substitution in bash 5.3+.
                let valid_prefix = !name.is_empty()
                    && name
                        .chars()
                        .next()
                        .map(|c| c == '_' || c.is_ascii_alphabetic())
                        .unwrap_or(false);
                if !valid_prefix {
                    // Skip to closing }
                    *i += 1;
                    while *i < chars.len() && chars[*i] != '}' {
                        *i += 1;
                    }
                    if *i < chars.len() {
                        *i += 1;
                    }
                    return WordPart::SyntaxError(format!(
                        "${{!{}{}}}: bad substitution",
                        name, ch
                    ));
                }
                *i += 1;
                if *i < chars.len() && chars[*i] == '}' {
                    *i += 1;
                    return WordPart::Param(ParamExpr {
                        name,
                        op: ParamOp::NamePrefix(ch),
                    });
                }
                // Extra content after * or @ before } → bad substitution
                // e.g. ${!_Q* } or ${!prefix*xyz}
                let mut trailing = String::new();
                while *i < chars.len() && chars[*i] != '}' {
                    trailing.push(chars[*i]);
                    *i += 1;
                }
                if *i < chars.len() {
                    *i += 1; // skip }
                }
                return WordPart::SyntaxError(format!(
                    "${{!{}{}{}}}: bad substitution",
                    name, ch, trailing
                ));
            }
            // Check for operator after indirect name: ${!name+word}, ${!name-word}, etc.
            if *i < chars.len() && chars[*i] != '}' {
                // There's an operator — parse it as indirect + operator
                let op = read_param_op(chars, i, &name, in_dquote);
                if *i < chars.len() && chars[*i] == '}' {
                    *i += 1;
                }
                // Wrap the result: we need indirect resolution first, then apply the op
                // For now, represent as Indirect with the name containing the op info
                // Actually, we need a proper representation. Let's use a special name prefix.
                return WordPart::Param(ParamExpr {
                    name: format!("!{}", name),
                    op,
                });
            }
            if *i < chars.len() && chars[*i] == '}' {
                *i += 1;
            }
            return WordPart::Param(ParamExpr {
                name,
                op: ParamOp::Indirect,
            });
        } // end of else (indirect expansion path)
    }

    // ${#name} - length, but ${#} ${#:-...} ${#-...} etc. are $# with operations
    if *i < chars.len() && chars[*i] == '#' {
        let next = if *i + 1 < chars.len() {
            chars[*i + 1]
        } else {
            '}'
        };
        // Check if this is $# with an operation vs ${#name} (length)
        // ${#:-word}, ${#-word}, ${#+word} are $# with operations
        // ${#-} alone = length of $-, ${#?} alone = length of $?
        // ${#:} alone = bad substitution
        let is_hash_param_op = match next {
            '}' => false,
            ':' => {
                // ${#:} = bad substitution, ${#:X} = $# with operation
                *i + 2 < chars.len() && chars[*i + 2] != '}'
            }
            '-' | '+' => {
                // ${#-} = length of $-, ${#-word} = $# default op
                // ${#+} = bad substitution, ${#+word} = $# alt op
                *i + 2 < chars.len() && chars[*i + 2] != '}'
            }
            '?' => {
                // ${#?} = length of $?, ${#?word} = $# error op
                *i + 2 < chars.len() && chars[*i + 2] != '}'
            }
            '!' => false, // ${#!} handled by indirect expansion
            _ => false,   // ${#name} is length
        };
        if !is_hash_param_op && next != '}' {
            *i += 1;
            let name = read_param_name_with_subscript(chars, i);
            // If name is empty and next char is not }, it's an invalid ${#X} form
            if name.is_empty() && *i < chars.len() && chars[*i] != '}' {
                // Skip to closing } and return bad substitution error
                let start = *i;
                while *i < chars.len() && chars[*i] != '}' {
                    *i += 1;
                }
                let rest: String = chars[start..*i].iter().collect();
                if *i < chars.len() {
                    *i += 1;
                }
                return WordPart::BadSubstitution(format!("${{#{}}}", rest));
            }
            // Check for trailing invalid chars after name (e.g., ${#1xyz})
            if *i < chars.len() && chars[*i] != '}' {
                let start_pos = *i;
                while *i < chars.len() && chars[*i] != '}' {
                    *i += 1;
                }
                let rest: String = std::iter::once('#')
                    .chain(name.chars())
                    .chain(chars[start_pos..*i].iter().copied())
                    .collect();
                if *i < chars.len() {
                    *i += 1;
                }
                return WordPart::BadSubstitution(format!("${{{}}}", rest));
            }
            if *i < chars.len() && chars[*i] == '}' {
                *i += 1;
            }
            return WordPart::Param(ParamExpr {
                name,
                op: ParamOp::Length,
            });
        }
    }

    let name = read_param_name_with_subscript(chars, i);

    // ${$(...)} or ${$((...))}: special param $ followed by ( is bad substitution
    // Also covers empty name followed by $ (shouldn't happen but defensive)
    if *i < chars.len() && chars[*i] == '(' && (name == "$" || name.is_empty()) {
        // Reconstruct the full content inside ${...} by scanning to closing }
        let start_content = *i;
        let mut depth = 1;
        while *i < chars.len() && depth > 0 {
            if chars[*i] == '{' {
                depth += 1;
            } else if chars[*i] == '}' {
                depth -= 1;
            }
            if depth > 0 {
                *i += 1;
            }
        }
        let rest: String = chars[start_content..*i].iter().collect();
        if *i < chars.len() {
            *i += 1;
        }
        return WordPart::BadSubstitution(format!("${{{}{}}}", name, rest));
    }

    // ${' is bad substitution (quoted variable names not allowed)
    if name.is_empty() && *i < chars.len() && chars[*i] == '\'' {
        // Scan to closing } and return as bad substitution
        let start = *i;
        let mut depth = 1;
        while *i < chars.len() && depth > 0 {
            if chars[*i] == '{' {
                depth += 1;
            } else if chars[*i] == '}' {
                depth -= 1;
            }
            if depth > 0 {
                *i += 1;
            }
        }
        let content: String = chars[start..*i].iter().collect();
        if *i < chars.len() {
            *i += 1;
        }
        // Strip $' → ' in the error message (bash resolves ANSI-C quoting in display)
        let display_content = content.replace("$'", "'");
        return WordPart::BadSubstitution(format!("${{{}}}", display_content));
    }

    // Check for @X transform operator before }
    if *i + 1 < chars.len() && chars[*i] == '@' && chars[*i + 1] != '}' {
        let transform_char = chars[*i + 1];
        if matches!(
            transform_char,
            'E' | 'Q' | 'P' | 'A' | 'a' | 'K' | 'k' | 'L' | 'U' | 'u'
        ) {
            *i += 2;
            if *i < chars.len() && chars[*i] == '}' {
                *i += 1;
            }
            return WordPart::Param(ParamExpr {
                name,
                op: ParamOp::Transform(transform_char),
            });
        }
    }

    if *i >= chars.len() || chars[*i] == '}' {
        if *i < chars.len() {
            *i += 1;
        }
        return WordPart::Param(ParamExpr {
            name,
            op: ParamOp::None,
        });
    }

    let op = read_param_op(chars, i, &name, in_dquote);

    // Check for @X transform after operator
    if *i + 1 < chars.len() && chars[*i] == '@' && chars[*i + 1] != '}' {
        let transform_char = chars[*i + 1];
        if matches!(
            transform_char,
            'E' | 'Q' | 'P' | 'A' | 'a' | 'K' | 'k' | 'L' | 'U' | 'u'
        ) {
            *i += 2;
        }
    }

    // The closing } should be right here after read_param_op consumed the word.
    // If it's not, a nested "..." inside the word consumed the } (e.g. "${foo:-"a}").
    if *i < chars.len() && chars[*i] == '}' {
        *i += 1; // consume the closing }
    } else {
        // Skip to closing }, handling nested braces — try to recover
        let mut depth = 1i32;
        let start_i = *i;
        while *i < chars.len() && depth > 0 {
            match chars[*i] {
                '{' => depth += 1,
                '}' => depth -= 1,
                _ => {}
            }
            *i += 1;
        }
        if depth > 0 || *i == start_i {
            // No closing } found — a quoted string inside the word consumed it.
            // Bash reports: "unexpected EOF while looking for matching `}'"
            return WordPart::SyntaxError(
                "unexpected EOF while looking for matching `}'".to_string(),
            );
        }
    }
    WordPart::Param(ParamExpr { name, op })
}

/// Read a parameter name, including array subscript like `arr[0]` or `arr[@]`.
fn read_param_name_with_subscript(chars: &[char], i: &mut usize) -> String {
    let mut name = read_param_name(chars, i);
    // Check for array subscript [...]
    if *i < chars.len() && chars[*i] == '[' {
        name.push('[');
        *i += 1;
        let mut depth = 1;
        // Inside [...], double-quote characters toggle quoting but are
        // stripped from the subscript text.  This matches bash behaviour
        // where `"${arr["{2..6}"]}"` produces subscript `{2..6}` (the
        // quotes are consumed as delimiters, not literal content).
        let mut in_subscript_dquote = false;
        while *i < chars.len() && depth > 0 {
            if chars[*i] == '"' {
                // Toggle subscript-level quoting; skip the quote char
                in_subscript_dquote = !in_subscript_dquote;
                *i += 1;
                continue;
            }
            if !in_subscript_dquote {
                if chars[*i] == '[' {
                    depth += 1;
                } else if chars[*i] == ']' {
                    depth -= 1;
                    if depth == 0 {
                        name.push(']');
                        *i += 1;
                        break;
                    }
                }
            }
            // Backslash escapes inside subscript (e.g. a[\ ])
            if chars[*i] == '\\' && *i + 1 < chars.len() && depth > 0 {
                name.push(chars[*i]);
                *i += 1;
                name.push(chars[*i]);
                *i += 1;
                continue;
            }
            name.push(chars[*i]);
            *i += 1;
        }
    }
    name
}

fn read_param_name(chars: &[char], i: &mut usize) -> String {
    let mut name = String::new();
    // $'...' ANSI-C quoting as variable name: ${$'name'...}
    if *i + 1 < chars.len() && chars[*i] == '$' && chars[*i + 1] == '\'' {
        *i += 2; // skip $'
        while *i < chars.len() && chars[*i] != '\'' {
            if chars[*i] == '\\' && *i + 1 < chars.len() {
                *i += 1;
                match chars[*i] {
                    'n' => name.push('\n'),
                    't' => name.push('\t'),
                    '\\' => name.push('\\'),
                    '\'' => name.push('\''),
                    c => {
                        name.push('\\');
                        name.push(c);
                    }
                }
            } else {
                name.push(chars[*i]);
            }
            *i += 1;
        }
        if *i < chars.len() {
            *i += 1; // skip closing '
        }
    } else if *i < chars.len()
        && (chars[*i] == '@'
            || chars[*i] == '*'
            || chars[*i] == '#'
            || chars[*i] == '?'
            || chars[*i] == '-'
            || chars[*i] == '$'
            || chars[*i] == '!')
    {
        name.push(chars[*i]);
        *i += 1;
    } else if *i < chars.len() && chars[*i].is_ascii_digit() {
        // Read all consecutive digits for positional parameters like ${10}
        while *i < chars.len() && chars[*i].is_ascii_digit() {
            name.push(chars[*i]);
            *i += 1;
        }
    } else {
        while *i < chars.len() && (chars[*i] == '_' || chars[*i].is_alphanumeric()) {
            name.push(chars[*i]);
            *i += 1;
        }
    }
    name
}

/// Warn when a pattern word has $( inside single quotes (would be comsub without quoting)
pub fn warn_incomplete_comsub_in_pattern(word: &Word, lineno: &str) {
    crate::expand::warn_incomplete_comsub_in_pattern_impl(word, lineno);
}

fn read_param_op(chars: &[char], i: &mut usize, _name: &str, in_dquote: bool) -> ParamOp {
    // For pattern operations (#, %, /), $'...' should still be expanded even in heredoc
    let read_word =
        |chars: &[char], i: &mut usize| -> Word { read_param_word_impl(chars, i, '}', in_dquote) };
    let _read_word_until = |chars: &[char], i: &mut usize, delim: char| -> Word {
        read_param_word_impl(chars, i, delim, in_dquote)
    };
    let read_pattern_word = |chars: &[char], i: &mut usize| -> Word {
        // For pattern words: clear IN_HEREDOC and set PATTERN_WORD
        let was_heredoc = IN_HEREDOC.with(|f| f.replace(false));
        let was_pattern = PATTERN_WORD.with(|f| f.replace(true));
        let result = read_param_word_impl(chars, i, '}', in_dquote);
        IN_HEREDOC.with(|f| f.set(was_heredoc));
        PATTERN_WORD.with(|f| f.set(was_pattern));
        result
    };
    let read_pattern_word_until = |chars: &[char], i: &mut usize, delim: char| -> Word {
        let was_heredoc = IN_HEREDOC.with(|f| f.replace(false));
        let was_pattern = PATTERN_WORD.with(|f| f.replace(true));
        let result = read_param_word_impl(chars, i, delim, in_dquote);
        IN_HEREDOC.with(|f| f.set(was_heredoc));
        PATTERN_WORD.with(|f| f.set(was_pattern));
        result
    };

    if *i >= chars.len() {
        return ParamOp::None;
    }

    match chars[*i] {
        ':' => {
            *i += 1;
            if *i >= chars.len() {
                return ParamOp::None;
            }
            match chars[*i] {
                '-' => {
                    *i += 1;
                    let word = read_word(chars, i);
                    ParamOp::Default(true, word)
                }
                '=' => {
                    *i += 1;
                    let word = read_word(chars, i);
                    ParamOp::Assign(true, word)
                }
                '?' => {
                    *i += 1;
                    let word = read_word(chars, i);
                    ParamOp::Error(true, word)
                }
                '+' => {
                    *i += 1;
                    let word = read_word(chars, i);
                    ParamOp::Alt(true, word)
                }
                _ => {
                    // ${var:offset} or ${var:offset:length}
                    // Must handle nested ternary (?:) in arithmetic expressions
                    // e.g., ${var:1 ? 4 : 2} — the : after 4 is ternary, not length separator
                    let mut offset = String::new();
                    let mut ternary_depth = 0i32;
                    let mut paren_depth = 0i32;
                    let mut brace_depth = 0i32;
                    while *i < chars.len() && !(chars[*i] == '}' && brace_depth == 0) {
                        match chars[*i] {
                            '`' => {
                                // Backtick command substitution — consume until matching `
                                offset.push(chars[*i]);
                                *i += 1;
                                while *i < chars.len() && chars[*i] != '`' {
                                    if chars[*i] == '\\' && *i + 1 < chars.len() {
                                        offset.push(chars[*i]);
                                        *i += 1;
                                    }
                                    offset.push(chars[*i]);
                                    *i += 1;
                                }
                                if *i < chars.len() {
                                    offset.push(chars[*i]); // closing `
                                    *i += 1;
                                }
                                continue;
                            }
                            '"' if brace_depth == 0 && paren_depth == 0 => {
                                // Double-quoted string — consume until matching "
                                offset.push(chars[*i]);
                                *i += 1;
                                while *i < chars.len() && chars[*i] != '"' {
                                    if chars[*i] == '\\' && *i + 1 < chars.len() {
                                        offset.push(chars[*i]);
                                        *i += 1;
                                    }
                                    offset.push(chars[*i]);
                                    *i += 1;
                                }
                                if *i < chars.len() {
                                    offset.push(chars[*i]); // closing "
                                    *i += 1;
                                }
                                continue;
                            }
                            '$' if *i + 1 < chars.len() && chars[*i + 1] == '(' => {
                                // $(...) command substitution — track via paren_depth
                                offset.push(chars[*i]);
                                *i += 1;
                                offset.push(chars[*i]);
                                *i += 1;
                                paren_depth += 1;
                                continue;
                            }
                            '{' => brace_depth += 1,
                            '}' => brace_depth -= 1,
                            '(' => paren_depth += 1,
                            ')' if paren_depth > 0 => paren_depth -= 1,
                            '?' if paren_depth == 0 && brace_depth == 0 => ternary_depth += 1,
                            ':' if paren_depth == 0 && brace_depth == 0 => {
                                if ternary_depth > 0 {
                                    ternary_depth -= 1;
                                } else {
                                    break; // This is the length separator
                                }
                            }
                            _ => {}
                        }
                        offset.push(chars[*i]);
                        *i += 1;
                    }
                    let length = if *i < chars.len() && chars[*i] == ':' {
                        *i += 1;
                        let mut l = String::new();
                        let mut brace_depth2 = 0i32;
                        let mut paren_depth2 = 0i32;
                        while *i < chars.len() && !(chars[*i] == '}' && brace_depth2 == 0) {
                            match chars[*i] {
                                '`' => {
                                    l.push(chars[*i]);
                                    *i += 1;
                                    while *i < chars.len() && chars[*i] != '`' {
                                        if chars[*i] == '\\' && *i + 1 < chars.len() {
                                            l.push(chars[*i]);
                                            *i += 1;
                                        }
                                        l.push(chars[*i]);
                                        *i += 1;
                                    }
                                    if *i < chars.len() {
                                        l.push(chars[*i]);
                                        *i += 1;
                                    }
                                    continue;
                                }
                                '"' if brace_depth2 == 0 && paren_depth2 == 0 => {
                                    l.push(chars[*i]);
                                    *i += 1;
                                    while *i < chars.len() && chars[*i] != '"' {
                                        if chars[*i] == '\\' && *i + 1 < chars.len() {
                                            l.push(chars[*i]);
                                            *i += 1;
                                        }
                                        l.push(chars[*i]);
                                        *i += 1;
                                    }
                                    if *i < chars.len() {
                                        l.push(chars[*i]);
                                        *i += 1;
                                    }
                                    continue;
                                }
                                '$' if *i + 1 < chars.len() && chars[*i + 1] == '(' => {
                                    l.push(chars[*i]);
                                    *i += 1;
                                    l.push(chars[*i]);
                                    *i += 1;
                                    paren_depth2 += 1;
                                    continue;
                                }
                                '{' => brace_depth2 += 1,
                                '}' => brace_depth2 -= 1,
                                '(' => paren_depth2 += 1,
                                ')' if paren_depth2 > 0 => paren_depth2 -= 1,
                                _ => {}
                            }
                            l.push(chars[*i]);
                            *i += 1;
                        }
                        Some(l)
                    } else {
                        None
                    };
                    ParamOp::Substring(offset, length)
                }
            }
        }
        '-' => {
            *i += 1;
            let word = read_word(chars, i);
            ParamOp::Default(false, word)
        }
        '=' => {
            *i += 1;
            let word = read_word(chars, i);
            ParamOp::Assign(false, word)
        }
        '?' => {
            *i += 1;
            let word = read_word(chars, i);
            ParamOp::Error(false, word)
        }
        '+' => {
            *i += 1;
            let word = read_word(chars, i);
            ParamOp::Alt(false, word)
        }
        '#' => {
            *i += 1;
            if *i < chars.len() && chars[*i] == '#' {
                *i += 1;
                let word = read_pattern_word(chars, i);
                // Warn about $(  inside single-quoted pattern parts
                ParamOp::TrimLargeLeft(word)
            } else {
                let word = read_pattern_word(chars, i);
                ParamOp::TrimSmallLeft(word)
            }
        }
        '%' => {
            *i += 1;
            if *i < chars.len() && chars[*i] == '%' {
                *i += 1;
                let word = read_pattern_word(chars, i);
                ParamOp::TrimLargeRight(word)
            } else {
                let word = read_pattern_word(chars, i);
                ParamOp::TrimSmallRight(word)
            }
        }
        '/' => {
            *i += 1;
            let mode = if *i < chars.len() {
                match chars[*i] {
                    '/' => {
                        *i += 1;
                        'a'
                    } // replace all
                    '#' => {
                        *i += 1;
                        'p'
                    } // replace prefix
                    '%' => {
                        *i += 1;
                        's'
                    } // replace suffix
                    _ => 'f', // replace first
                }
            } else {
                'f'
            };
            // If the first char after // is '/', it's part of the pattern
            // (e.g. ${a///a/} means replace-all "/a" with empty).
            // Push it as a literal before reading the rest of the pattern.
            // If the first char after //  (or / for first-match) is '/',
            // it's part of the pattern — but only for ReplaceAll ('a') and
            // ReplaceFirst ('f') modes.  For prefix ('#') and suffix ('%')
            // modes the '/' is the pattern/replacement separator.
            // e.g. ${a///a/} means replace-all "/a" with empty,
            //      ${a/#/x} means replace empty prefix with "x".
            let mut pattern_prefix: Word = vec![];
            if matches!(mode, 'a' | 'f') && *i < chars.len() && chars[*i] == '/' {
                pattern_prefix.push(WordPart::Literal("/".to_string()));
                *i += 1;
            }
            let mut pattern = read_pattern_word_until(chars, i, '/');
            if !pattern_prefix.is_empty() {
                pattern_prefix.extend(pattern);
                pattern = pattern_prefix;
            }
            let replacement = if *i < chars.len() && chars[*i] == '/' {
                *i += 1;
                read_pattern_word(chars, i)
            } else {
                vec![]
            };
            match mode {
                'a' => ParamOp::ReplaceAll(pattern, replacement),
                'p' => ParamOp::ReplacePrefix(pattern, replacement),
                's' => ParamOp::ReplaceSuffix(pattern, replacement),
                _ => ParamOp::Replace(pattern, replacement),
            }
        }
        '^' => {
            *i += 1;
            if *i < chars.len() && chars[*i] == '^' {
                *i += 1;
                let pattern = read_word(chars, i);
                ParamOp::UpperAll(pattern)
            } else {
                let pattern = read_word(chars, i);
                ParamOp::UpperFirst(pattern)
            }
        }
        ',' => {
            *i += 1;
            if *i < chars.len() && chars[*i] == ',' {
                *i += 1;
                let pattern = read_word(chars, i);
                ParamOp::LowerAll(pattern)
            } else {
                let pattern = read_word(chars, i);
                ParamOp::LowerFirst(pattern)
            }
        }
        '~' => {
            *i += 1;
            if *i < chars.len() && chars[*i] == '~' {
                *i += 1;
                let pattern = read_word(chars, i);
                ParamOp::ToggleAll(pattern)
            } else {
                let pattern = read_word(chars, i);
                ParamOp::ToggleFirst(pattern)
            }
        }
        _ => ParamOp::None,
    }
}
