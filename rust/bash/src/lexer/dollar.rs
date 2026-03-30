use super::*;

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
                let mut expr = String::new();
                let mut has_semicolon_at_top = false;
                while *i < chars.len() && depth > 0 {
                    if *i + 1 < chars.len()
                        && chars[*i] == ')'
                        && chars[*i + 1] == ')'
                        && paren_depth <= 0
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
                    } else {
                        if chars[*i] == '(' {
                            paren_depth += 1;
                        } else if chars[*i] == ')' {
                            paren_depth -= 1;
                        } else if chars[*i] == ';' && paren_depth <= 0 {
                            has_semicolon_at_top = true;
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
            {
                // Command substitution: $( ... )
                // Must handle case...esac, quotes, nested $(...)
                let mut depth = 1;
                let mut brace_depth = 0i32; // track ${...} nesting
                let mut cmd = String::new();
                let mut case_depth = 0i32;
                let mut case_paren_depth = Vec::<i32>::new(); // paren depth at each case
                let mut case_action_stack = Vec::<bool>::new(); // saved in_case_action for outer cases
                let mut in_case_action = false; // true after case pattern ), false after ;;
                let mut compound_depth = 0i32; // tracks do/done, then/fi nesting
                while *i < chars.len() && depth > 0 {
                    match chars[*i] {
                        '\'' => {
                            // Single-quoted string — always active inside $() comsub
                            // (command substitution starts a new parsing context)
                            cmd.push(chars[*i]);
                            *i += 1;
                            while *i < chars.len() && chars[*i] != '\'' {
                                cmd.push(chars[*i]);
                                *i += 1;
                            }
                            if *i < chars.len() {
                                cmd.push(chars[*i]);
                                *i += 1;
                            }
                            continue;
                        }
                        '"' => {
                            // Double-quoted string — skip but handle $() and ${} inside
                            cmd.push(chars[*i]);
                            *i += 1;
                            while *i < chars.len() && chars[*i] != '"' {
                                if chars[*i] == '\\' && *i + 1 < chars.len() {
                                    cmd.push(chars[*i]);
                                    *i += 1;
                                    cmd.push(chars[*i]);
                                    *i += 1;
                                    continue;
                                }
                                if chars[*i] == '$' && *i + 1 < chars.len() {
                                    if chars[*i + 1] == '(' {
                                        // Nested $() inside dquotes — track paren depth
                                        cmd.push(chars[*i]);
                                        *i += 1;
                                        cmd.push(chars[*i]);
                                        *i += 1;
                                        let mut inner_depth = 1i32;
                                        while *i < chars.len() && inner_depth > 0 {
                                            match chars[*i] {
                                                '(' => inner_depth += 1,
                                                ')' => {
                                                    inner_depth -= 1;
                                                    if inner_depth == 0 {
                                                        cmd.push(chars[*i]);
                                                        *i += 1;
                                                        break;
                                                    }
                                                }
                                                '"' => {
                                                    // Nested dquotes inside inner $()
                                                    cmd.push(chars[*i]);
                                                    *i += 1;
                                                    while *i < chars.len() && chars[*i] != '"' {
                                                        if chars[*i] == '\\' && *i + 1 < chars.len()
                                                        {
                                                            cmd.push(chars[*i]);
                                                            *i += 1;
                                                        }
                                                        cmd.push(chars[*i]);
                                                        *i += 1;
                                                    }
                                                    if *i < chars.len() {
                                                        cmd.push(chars[*i]);
                                                        *i += 1;
                                                    }
                                                    continue;
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
                                                        *i += 1;
                                                    }
                                                    continue;
                                                }
                                                _ => {}
                                            }
                                            if *i < chars.len() && inner_depth > 0 {
                                                cmd.push(chars[*i]);
                                                *i += 1;
                                            }
                                        }
                                        continue;
                                    } else if chars[*i + 1] == '{' {
                                        // Nested ${} inside dquotes — track brace depth
                                        cmd.push(chars[*i]);
                                        *i += 1;
                                        cmd.push(chars[*i]);
                                        *i += 1;
                                        let mut inner_depth = 1i32;
                                        while *i < chars.len() && inner_depth > 0 {
                                            match chars[*i] {
                                                '{' => inner_depth += 1,
                                                '}' => {
                                                    inner_depth -= 1;
                                                    if inner_depth == 0 {
                                                        cmd.push(chars[*i]);
                                                        *i += 1;
                                                        break;
                                                    }
                                                }
                                                _ => {}
                                            }
                                            if *i < chars.len() && inner_depth > 0 {
                                                cmd.push(chars[*i]);
                                                *i += 1;
                                            }
                                        }
                                        continue;
                                    }
                                }
                                cmd.push(chars[*i]);
                                *i += 1;
                            }
                            if *i < chars.len() {
                                cmd.push(chars[*i]);
                                *i += 1;
                            }
                            continue;
                        }
                        '`' => {
                            // Backtick command sub — skip
                            cmd.push(chars[*i]);
                            *i += 1;
                            while *i < chars.len() && chars[*i] != '`' {
                                if chars[*i] == '\\' && *i + 1 < chars.len() {
                                    cmd.push(chars[*i]);
                                    *i += 1;
                                }
                                cmd.push(chars[*i]);
                                *i += 1;
                            }
                            if *i < chars.len() {
                                cmd.push(chars[*i]);
                                *i += 1;
                            }
                            continue;
                        }
                        '(' => {
                            // Check if preceded by $ (nested command sub or arith)
                            let preceded_by_dollar = cmd.ends_with('$') || cmd.ends_with("$(");
                            if case_depth > 0 && !in_case_action && !preceded_by_dollar {
                                // ( in case pattern context — pattern delimiter, skip
                            } else {
                                depth += 1;
                            }
                        }
                        ')' => {
                            if case_depth > 0 && !in_case_action {
                                // ) in case pattern context — ends pattern, enter action
                                in_case_action = true;
                            } else if compound_depth > 0 && depth <= 1 {
                                // Incomplete compound command (e.g. if/then without fi)
                                // at comsub depth — stop scanning and close the comsub
                                // with what we have, so the ) is left for the outer parser
                                // to report as a syntax error
                                depth = 0;
                                break;
                            } else {
                                depth -= 1;
                                if depth == 0 {
                                    *i += 1;
                                    break;
                                }
                            }
                        }
                        '}' if brace_depth > 0 => {
                            // Inside a ${...} block — this } closes that block
                            brace_depth -= 1;
                        }
                        '}' if in_dquote && depth == 1 => {
                            // In dquote context, } at comsub depth 1 means the
                            // closing } of the enclosing ${...}. Silent suppression.
                            return WordPart::CommandSub("\x00SILENT_COMSUB".to_string());
                        }
                        '$' if *i + 1 < chars.len() && chars[*i + 1] == '{' => {
                            // Track ${...} nesting
                            cmd.push(chars[*i]);
                            *i += 1;
                            cmd.push(chars[*i]);
                            *i += 1;
                            brace_depth += 1;
                            continue;
                        }
                        '#' if cmd.is_empty()
                            || cmd.ends_with('\n')
                            || (cmd.ends_with(";;")
                                || (cmd.ends_with(';') && !cmd.ends_with("\\;"))) =>
                        {
                            // Comment — skip to end of line (don't push to cmd)
                            while *i < chars.len() && chars[*i] != '\n' {
                                *i += 1;
                            }
                            continue;
                        }
                        // Backslash-newline: line continuation (remove both)
                        '\\' if *i + 1 < chars.len() && chars[*i + 1] == '\n' => {
                            *i += 2; // skip \ and newline
                            continue;
                        }
                        // Backslash: escape next char (prevents ) from closing comsub)
                        '\\' if *i + 1 < chars.len() => {
                            cmd.push(chars[*i]);
                            *i += 1;
                            cmd.push(chars[*i]);
                            *i += 1;
                            continue;
                        }
                        // Backslash-newline: line continuation
                        '\\' if *i + 1 < chars.len() && chars[*i + 1] == '\n' => {
                            // Skip both \ and newline
                            *i += 2;
                            continue;
                        }
                        // Heredoc: << or <<- inside comsub
                        '<' if *i + 1 < chars.len()
                            && chars[*i + 1] == '<'
                            && (*i + 2 >= chars.len() || chars[*i + 2] != '<') =>
                        {
                            let hd_start = *i;
                            cmd.push(chars[*i]); // first <
                            *i += 1;
                            cmd.push(chars[*i]); // second <
                            *i += 1;
                            // Optional - for <<-
                            let strip_tabs = *i < chars.len() && chars[*i] == '-';
                            if strip_tabs {
                                cmd.push(chars[*i]);
                                *i += 1;
                            }
                            // Skip whitespace
                            while *i < chars.len() && (chars[*i] == ' ' || chars[*i] == '\t') {
                                cmd.push(chars[*i]);
                                *i += 1;
                            }
                            // Read the delimiter (handle quoting)
                            let mut delim = String::new();
                            let mut heredoc_quoted = false;
                            while *i < chars.len()
                                && chars[*i] != '\n'
                                && chars[*i] != ' '
                                && chars[*i] != '\t'
                                && chars[*i] != ';'
                                && chars[*i] != '&'
                                && chars[*i] != ')'
                                && chars[*i] != '|'
                            {
                                let ch = chars[*i];
                                if ch == '\'' {
                                    heredoc_quoted = true;
                                    cmd.push(ch);
                                    *i += 1;
                                    while *i < chars.len() && chars[*i] != '\'' {
                                        delim.push(chars[*i]);
                                        cmd.push(chars[*i]);
                                        *i += 1;
                                    }
                                    if *i < chars.len() {
                                        cmd.push(chars[*i]);
                                        *i += 1;
                                    }
                                } else if ch == '"' {
                                    heredoc_quoted = true;
                                    cmd.push(ch);
                                    *i += 1;
                                    while *i < chars.len() && chars[*i] != '"' {
                                        delim.push(chars[*i]);
                                        cmd.push(chars[*i]);
                                        *i += 1;
                                    }
                                    if *i < chars.len() {
                                        cmd.push(chars[*i]);
                                        *i += 1;
                                    }
                                } else if ch == '\\' {
                                    heredoc_quoted = true;
                                    cmd.push(ch);
                                    *i += 1;
                                    if *i < chars.len() && chars[*i] == '\n' {
                                        // Line continuation: \<newline> joins next line
                                        cmd.push(chars[*i]);
                                        *i += 1;
                                    } else if *i < chars.len() {
                                        delim.push(chars[*i]);
                                        cmd.push(chars[*i]);
                                        *i += 1;
                                    }
                                } else {
                                    delim.push(ch);
                                    cmd.push(ch);
                                    *i += 1;
                                }
                            }
                            let _ = heredoc_quoted; // used later if needed
                            // Skip to end of line (rest of command after heredoc redirect)
                            while *i < chars.len() && chars[*i] != '\n' {
                                cmd.push(chars[*i]);
                                *i += 1;
                            }
                            if *i < chars.len() {
                                cmd.push(chars[*i]); // newline
                                *i += 1;
                            }
                            // Read heredoc body lines until delimiter
                            if !delim.is_empty() {
                                loop {
                                    if *i >= chars.len() {
                                        // EOF while reading heredoc in comsub — warn
                                        // Use newline count (not +1) since we're at EOF past
                                        // the last newline
                                        let current_line =
                                            chars.iter().filter(|&&c| c == '\n').count();
                                        let heredoc_start_line = chars[..hd_start]
                                            .iter()
                                            .filter(|&&c| c == '\n')
                                            .count()
                                            + 1;
                                        heredoc_eof_warnings.push((
                                            current_line,
                                            heredoc_start_line,
                                            delim.clone(),
                                        ));
                                        break;
                                    }
                                    // Read one line
                                    let line_start = *i;
                                    while *i < chars.len() && chars[*i] != '\n' {
                                        *i += 1;
                                    }
                                    let line: String = chars[line_start..*i].iter().collect();
                                    let check = if strip_tabs {
                                        line.trim_start_matches('\t').to_string()
                                    } else {
                                        line.clone()
                                    };
                                    if check == delim {
                                        cmd.push_str(&line);
                                        if *i < chars.len() {
                                            cmd.push('\n');
                                            *i += 1;
                                        }
                                        break;
                                    }
                                    // Check if line is delimiter followed by optional
                                    // whitespace then ) — heredoc ends here
                                    if check.starts_with(&delim)
                                        && check[delim.len()..].trim_start().starts_with(')')
                                    {
                                        // Warn: heredoc delimited by end-of-file (delimiter
                                        // on same line as comsub closing ))
                                        let current_line =
                                            chars[..*i].iter().filter(|&&c| c == '\n').count() + 1;
                                        let heredoc_start_line = chars[..hd_start]
                                            .iter()
                                            .filter(|&&c| c == '\n')
                                            .count()
                                            + 1;
                                        // Collect warning for the lexer to emit (not eprintln!
                                        // directly, to avoid duplication on parser backtrack)
                                        heredoc_eof_warnings.push((
                                            current_line,
                                            heredoc_start_line,
                                            delim.clone(),
                                        ));
                                        // Put delimiter on its own line in the content
                                        cmd.push_str(&delim);
                                        cmd.push('\n');
                                        // Position at the ) that closes the comsub
                                        // (after the delimiter match)
                                        *i = line_start + delim.len();
                                        break;
                                    }
                                    cmd.push_str(&line);
                                    if *i < chars.len() {
                                        cmd.push('\n');
                                        *i += 1;
                                    }
                                }
                            }
                            continue;
                        }
                        _ => {}
                    }
                    // Detect ;; to reset case action context
                    if chars[*i] == ';'
                        && *i + 1 < chars.len()
                        && chars[*i + 1] == ';'
                        && case_depth > 0
                    {
                        in_case_action = false;
                        cmd.push(';');
                        *i += 1;
                        cmd.push(';');
                        *i += 1;
                        continue;
                    }
                    // Handle # comments — # after newline, ;, &, | or at start
                    // of the comsub starts a comment that runs to end of line
                    // But NOT after escaped chars like \; (the ; is literal, not separator)
                    if chars[*i] == '#' {
                        let prev = if cmd.is_empty() {
                            '\n'
                        } else {
                            cmd.chars().last().unwrap_or('\n')
                        };
                        // Check if prev char was escaped (preceded by \)
                        let prev_escaped = cmd.len() >= 2 && cmd.as_bytes()[cmd.len() - 2] == b'\\';
                        if !prev_escaped
                            && matches!(prev, '\n' | ';' | '&' | '|' | '(' | ' ' | '\t')
                        {
                            // Skip to end of line — don't push comment content to cmd
                            // (comments may contain ) which would corrupt paren tracking)
                            while *i < chars.len() && chars[*i] != '\n' {
                                *i += 1;
                            }
                            continue;
                        }
                    }
                    // Track case/esac keywords
                    if chars[*i].is_alphabetic() {
                        let _start = *i;
                        let mut word = String::new();
                        while *i < chars.len() && (chars[*i].is_alphanumeric() || chars[*i] == '_')
                        {
                            word.push(chars[*i]);
                            *i += 1;
                        }
                        // Check aliases for case keyword expansion
                        let effective = COMSUB_ALIASES
                            .with(|a| a.borrow().get(word.as_str()).map(|v| v.trim().to_string()));
                        let kw = effective.as_deref().unwrap_or(word.as_str());
                        if kw == "case" {
                            case_depth += 1;
                            case_paren_depth.push(depth);
                            // Save current action state and reset for new case
                            case_action_stack.push(in_case_action);
                            in_case_action = false;
                        } else if (kw == "esac" || word == "esac") && case_depth > 0 {
                            // Only treat esac as case terminator when:
                            // - at the same paren depth as the case was opened
                            // - in action context or at pattern start (not after |)
                            let case_open_depth = case_paren_depth.last().copied().unwrap_or(depth);
                            if depth != case_open_depth {
                                // Inside a subshell — esac is just a command, not terminator
                            } else {
                                let trimmed = cmd.trim_end();
                                let prev_ch = trimmed.chars().last().unwrap_or('\n');
                                let after_in = trimmed.ends_with(" in")
                                    || trimmed.ends_with("\tin")
                                    || trimmed.ends_with("\nin");
                                if in_case_action || prev_ch == ';' || prev_ch == '\n' || after_in {
                                    case_depth -= 1;
                                    case_paren_depth.pop();
                                    // Restore outer case's action state
                                    in_case_action = case_action_stack.pop().unwrap_or(false);
                                }
                            }
                        }
                        // Track compound commands (do/done, then/fi) to prevent ) from
                        // closing the comsub when inside an incomplete compound
                        if matches!(kw, "do" | "then") {
                            compound_depth += 1;
                        } else if (matches!(kw, "done" | "fi")
                            || matches!(word.as_str(), "done" | "fi"))
                            && compound_depth > 0
                        {
                            compound_depth -= 1;
                        }
                        // Count ( and ) in alias expansion to adjust depth
                        if let Some(ref exp) = effective {
                            let mut close_idx = None;
                            for (ci, ch) in exp.chars().enumerate() {
                                match ch {
                                    '(' => depth += 1,
                                    ')' if case_depth <= 0 => {
                                        depth -= 1;
                                        if depth == 0 {
                                            close_idx = Some(ci);
                                            break;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            if depth == 0 {
                                // Alias closes the comsub — add expanded content up to )
                                if let Some(ci) = close_idx {
                                    let before_close: String = exp.chars().take(ci).collect();
                                    cmd.push_str(&before_close);
                                }
                                break;
                            }
                        }
                        cmd.push_str(&word);
                        continue;
                    }
                    cmd.push(chars[*i]);
                    *i += 1;
                }
                if depth > 0 {
                    // Incomplete comsub — signal error via special marker
                    // Embed the EOF line number for error reporting
                    let eof_line = chars.iter().filter(|&&c| c == '\n').count() + 1;
                    WordPart::CommandSub(format!("\x00INCOMPLETE_COMSUB:{}", eof_line))
                } else {
                    WordPart::CommandSub(cmd)
                }
            }
        }
        '{' => {
            *i += 1;
            // Check for funsub: ${ cmd; } — space/tab/newline/| after {
            if *i < chars.len() && matches!(chars[*i], ' ' | '\t' | '\n' | '|') {
                // Parse as command substitution delimited by }
                // Funsub requires that } is preceded by a command terminator (;/\n/&)
                // at the SAME depth level (not from nested blocks)
                let mut depth = 1;
                let mut cmd = String::new();
                let mut has_terminator_at_depth1 = false;
                let mut has_nonws_at_depth1 = false;
                while *i < chars.len() && depth > 0 {
                    match chars[*i] {
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
                    WordPart::CommandSub(format!("\x00INCOMPLETE_FUNSUB{}", cmd))
                } else {
                    WordPart::CommandSub(cmd)
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
                *i += 1;
                if *i < chars.len() && chars[*i] == '}' {
                    *i += 1;
                }
                return WordPart::Param(ParamExpr {
                    name,
                    op: ParamOp::NamePrefix(ch),
                });
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
        while *i < chars.len() && depth > 0 {
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
            let pattern = read_pattern_word_until(chars, i, '/');
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
