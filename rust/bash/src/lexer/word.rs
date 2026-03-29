use super::*;

pub(super) fn read_param_word_impl(
    chars: &[char],
    i: &mut usize,
    delim: char,
    in_dquote: bool,
) -> Word {
    let mut parts = Vec::new();
    let mut literal = String::new();
    let mut depth = 0;

    while *i < chars.len() && (chars[*i] != delim || depth > 0) && chars[*i] != '}' {
        match chars[*i] {
            '\\' if *i + 1 < chars.len() => {
                let next = chars[*i + 1];
                if in_dquote
                    && !matches!(next, '$' | '`' | '"' | '\\' | '\n' | '}' | '/')
                    && !(next == '\'' && PATTERN_WORD.with(|f| f.get()))
                {
                    // At top level of param word in dquote, preserve backslash
                    literal.push('\\');
                    literal.push(next);
                } else if !in_dquote {
                    if next == '\n' {
                        // \<newline> is line continuation — discard both
                    } else {
                        // Mark escaped char as SingleQuoted for field splitting
                        if !literal.is_empty() {
                            parts.push(WordPart::Literal(std::mem::take(&mut literal)));
                        }
                        parts.push(WordPart::SingleQuoted(next.to_string()));
                    }
                } else if next == '\\' {
                    // \\ in dquote: produces a quoted literal backslash
                    // Mark as SingleQuoted so pattern matching doesn't treat
                    // it as an escape character (e.g., \\* = literal \ + wildcard *)
                    if !literal.is_empty() {
                        parts.push(WordPart::Literal(std::mem::take(&mut literal)));
                    }
                    parts.push(WordPart::SingleQuoted("\\".to_string()));
                } else if next == '\n' {
                    // \<newline> is line continuation — discard both chars
                } else {
                    literal.push(next);
                }
                *i += 2;
            }
            '$' => {
                if !literal.is_empty() {
                    parts.push(WordPart::Literal(std::mem::take(&mut literal)));
                }
                *i += 1;
                parts.push(parse_dollar(chars, i, in_dquote));
            }
            '\'' if !in_dquote || PATTERN_WORD.with(|f| f.get()) => {
                // Single quotes have quoting effect in unquoted context
                // AND in pattern words (#, %, /) even inside double quotes
                if !literal.is_empty() {
                    parts.push(WordPart::Literal(std::mem::take(&mut literal)));
                }
                *i += 1;
                let mut s = String::new();
                while *i < chars.len() && chars[*i] != '\'' {
                    s.push(chars[*i]);
                    *i += 1;
                }
                if *i < chars.len() {
                    *i += 1;
                }
                parts.push(WordPart::SingleQuoted(s));
            }
            '"' => {
                if !literal.is_empty() {
                    parts.push(WordPart::Literal(std::mem::take(&mut literal)));
                }
                *i += 1;
                let mut dq_parts = Vec::new();
                let mut dq_lit = String::new();
                while *i < chars.len() && chars[*i] != '"' {
                    match chars[*i] {
                        '\\' if *i + 1 < chars.len() => {
                            let next = chars[*i + 1];
                            if matches!(next, '$' | '`' | '"' | '\\') {
                                dq_lit.push(next);
                            } else if next == '\n' {
                                // \<newline> is line continuation — discard both
                            } else if in_dquote
                                && !PATTERN_WORD.with(|f| f.get())
                                && !IN_HEREDOC.with(|f| f.get())
                            {
                                // In nested dquote inside outer-dquoted Default/Alt words,
                                // strip backslash for non-special chars (\' → ')
                                dq_lit.push(next);
                            } else {
                                dq_lit.push('\\');
                                dq_lit.push(next);
                            }
                            *i += 2;
                        }
                        '$' => {
                            if !dq_lit.is_empty() {
                                dq_parts.push(WordPart::Literal(std::mem::take(&mut dq_lit)));
                            }
                            *i += 1;
                            dq_parts.push(parse_dollar(chars, i, true));
                        }
                        '`' => {
                            if !dq_lit.is_empty() {
                                dq_parts.push(WordPart::Literal(std::mem::take(&mut dq_lit)));
                            }
                            *i += 1;
                            let mut cmd = String::new();
                            while *i < chars.len() && chars[*i] != '`' {
                                if chars[*i] == '\\' && *i + 1 < chars.len() {
                                    let next = chars[*i + 1];
                                    if matches!(next, '$' | '`' | '\\' | '"') {
                                        cmd.push(next);
                                        *i += 2;
                                    } else if next == '\n' {
                                        *i += 2; // line continuation
                                    } else {
                                        cmd.push(chars[*i]);
                                        *i += 1;
                                    }
                                } else {
                                    cmd.push(chars[*i]);
                                    *i += 1;
                                }
                            }
                            if *i < chars.len() {
                                *i += 1;
                            }
                            dq_parts.push(WordPart::BacktickSub(cmd));
                        }
                        ch => {
                            dq_lit.push(ch);
                            *i += 1;
                        }
                    }
                }
                if *i < chars.len() {
                    *i += 1;
                }
                if !dq_lit.is_empty() {
                    dq_parts.push(WordPart::Literal(dq_lit));
                }
                parts.push(WordPart::DoubleQuoted(dq_parts));
            }
            '`' => {
                if !literal.is_empty() {
                    parts.push(WordPart::Literal(std::mem::take(&mut literal)));
                }
                *i += 1;
                let mut cmd = String::new();
                while *i < chars.len() && chars[*i] != '`' {
                    if chars[*i] == '\\' && *i + 1 < chars.len() {
                        let next = chars[*i + 1];
                        if matches!(next, '$' | '`' | '\\') {
                            // \\→\, \`→`, \$→$ (processed first)
                            cmd.push(next);
                            *i += 2;
                        } else if next == '\n' {
                            // \<newline> is line continuation — remove both
                            *i += 2;
                        } else {
                            cmd.push(chars[*i]);
                            *i += 1;
                        }
                    } else {
                        cmd.push(chars[*i]);
                        *i += 1;
                    }
                }
                if *i < chars.len() {
                    *i += 1; // skip closing `
                }
                parts.push(WordPart::BacktickSub(cmd));
            }
            '{' => {
                depth += 1;
                literal.push(chars[*i]);
                *i += 1;
            }
            '}' if depth > 0 => {
                depth -= 1;
                literal.push(chars[*i]);
                *i += 1;
            }
            ch => {
                literal.push(ch);
                *i += 1;
            }
        }
    }
    if !literal.is_empty() {
        parts.push(WordPart::Literal(literal));
    }
    parts
}

impl Lexer {
    pub(super) fn read_word(&mut self) -> Token {
        let mut parts = Vec::new();
        let mut literal = String::new();

        loop {
            let ch = match self.peek() {
                None => break,
                Some(c) => c,
            };

            match ch {
                // Extglob patterns: @(...), ?(...), *(...), +(...), !(...)
                '@' | '?' | '+' | '!' if self.peek_at(1) == Some('(') => {
                    literal.push(ch);
                    self.advance(); // consume @/+/?/!
                    literal.push('(');
                    self.advance(); // consume (
                    let mut depth = 1;
                    while let Some(c) = self.peek() {
                        if c == '(' {
                            depth += 1;
                        } else if c == ')' {
                            depth -= 1;
                            if depth == 0 {
                                literal.push(')');
                                self.advance();
                                break;
                            }
                        }
                        literal.push(c);
                        self.advance();
                    }
                    continue;
                }
                // *(pattern) — extglob (distinct from bare *)
                '*' if self.peek_at(1) == Some('(') => {
                    literal.push('*');
                    self.advance(); // consume *
                    literal.push('(');
                    self.advance(); // consume (
                    let mut depth = 1;
                    while let Some(c) = self.peek() {
                        if c == '(' {
                            depth += 1;
                        } else if c == ')' {
                            depth -= 1;
                            if depth == 0 {
                                literal.push(')');
                                self.advance();
                                break;
                            }
                        }
                        literal.push(c);
                        self.advance();
                    }
                    continue;
                }
                // Word terminators
                ' ' | '\t' | '\n' | ';' | '&' | '|' | '(' | ')' => break,
                '<' | '>' => {
                    // Check for process substitution: <(cmd) or >(cmd)
                    if self.peek_at(1) == Some('(') {
                        if !literal.is_empty() {
                            parts.push(WordPart::Literal(std::mem::take(&mut literal)));
                        }
                        let kind = if ch == '<' {
                            ProcessSubKind::Input
                        } else {
                            ProcessSubKind::Output
                        };
                        self.advance(); // consume < or >
                        self.advance(); // consume (
                        let mut depth = 1;
                        let mut cmd = String::new();
                        while let Some(c) = self.peek() {
                            if c == '(' {
                                depth += 1;
                            } else if c == ')' {
                                depth -= 1;
                                if depth == 0 {
                                    self.advance();
                                    break;
                                }
                            }
                            cmd.push(c);
                            self.advance();
                        }
                        parts.push(WordPart::ProcessSub(kind, cmd));
                        continue;
                    }
                    // Check if this is an IO number
                    if !literal.is_empty()
                        && literal.chars().all(|c| c.is_ascii_digit())
                        && parts.is_empty()
                    {
                        break;
                    }
                    break;
                }
                '#' if parts.is_empty() && literal.is_empty() => break,
                '\\' => {
                    self.advance();
                    if let Some(next) = self.advance() {
                        if next == '\n' {
                            // Line continuation - skip
                        } else {
                            // Push escaped char as SingleQuoted so it's treated as
                            // literal in pattern matching (gets \x00 quoting)
                            if !literal.is_empty() {
                                parts.push(WordPart::Literal(std::mem::take(&mut literal)));
                            }
                            parts.push(WordPart::SingleQuoted(next.to_string()));
                        }
                    } else {
                        // \ at EOF — treat as literal backslash
                        literal.push('\\');
                    }
                }
                '\'' => {
                    if !literal.is_empty() {
                        parts.push(WordPart::Literal(std::mem::take(&mut literal)));
                    }
                    self.advance();
                    let mut s = String::new();
                    loop {
                        match self.advance() {
                            None | Some('\'') => break,
                            Some(c) => s.push(c),
                        }
                    }
                    parts.push(WordPart::SingleQuoted(s));
                }
                '"' => {
                    if !literal.is_empty() {
                        parts.push(WordPart::Literal(std::mem::take(&mut literal)));
                    }
                    self.advance();
                    let mut dq_parts = Vec::new();
                    let mut dq_lit = String::new();
                    loop {
                        match self.peek() {
                            None | Some('"') => {
                                self.advance();
                                break;
                            }
                            Some('\\') => {
                                self.advance();
                                match self.peek() {
                                    Some(c @ ('$' | '`' | '"' | '\\' | '\n')) => {
                                        self.advance();
                                        if c != '\n' {
                                            dq_lit.push(c);
                                        }
                                    }
                                    Some(c) => {
                                        dq_lit.push('\\');
                                        dq_lit.push(c);
                                        self.advance();
                                    }
                                    None => dq_lit.push('\\'),
                                }
                            }
                            Some('$') => {
                                // Inside double quotes, $' and $" are literal
                                if matches!(self.peek_at(1), Some('\'' | '"')) {
                                    dq_lit.push('$');
                                    self.advance();
                                } else {
                                    if !dq_lit.is_empty() {
                                        dq_parts
                                            .push(WordPart::Literal(std::mem::take(&mut dq_lit)));
                                    }
                                    self.advance();
                                    let input_clone = self.input.clone();
                                    let old_pos = self.pos;
                                    let part = parse_dollar_with_warnings(
                                        &input_clone,
                                        &mut self.pos,
                                        true,
                                        &mut self.heredoc_eof_warnings,
                                    );
                                    for &ch in &input_clone[old_pos..self.pos] {
                                        if ch == '\n' {
                                            self.line += 1;
                                        }
                                    }
                                    dq_parts.push(part);
                                }
                            }
                            Some('`') => {
                                if !dq_lit.is_empty() {
                                    dq_parts.push(WordPart::Literal(std::mem::take(&mut dq_lit)));
                                }
                                self.advance();
                                let mut cmd = String::new();
                                loop {
                                    match self.peek() {
                                        None | Some('`') => {
                                            self.advance();
                                            break;
                                        }
                                        Some('\\') => {
                                            self.advance();
                                            match self.peek() {
                                                // Only these chars are special after \ in double-quoted backtick
                                                Some(c @ ('$' | '\\' | '`' | '"')) => {
                                                    cmd.push(c);
                                                    self.advance();
                                                }
                                                Some('\n') => {
                                                    // \<newline> line continuation
                                                    self.advance();
                                                }
                                                Some(c) => {
                                                    cmd.push('\\');
                                                    cmd.push(c);
                                                    self.advance();
                                                }
                                                None => cmd.push('\\'),
                                            }
                                        }
                                        Some(c) => {
                                            cmd.push(c);
                                            self.advance();
                                        }
                                    }
                                }
                                dq_parts.push(WordPart::BacktickSub(cmd));
                            }
                            Some(c) => {
                                dq_lit.push(c);
                                self.advance();
                            }
                        }
                    }
                    if !dq_lit.is_empty() {
                        dq_parts.push(WordPart::Literal(dq_lit));
                    }
                    parts.push(WordPart::DoubleQuoted(dq_parts));
                }
                '$' => {
                    if !literal.is_empty() {
                        parts.push(WordPart::Literal(std::mem::take(&mut literal)));
                    }
                    self.advance();
                    if self.peek() == Some('"') {
                        // $"..." locale-specific quoting (treated as regular double quoting)
                        self.advance(); // consume "
                        let mut dq_parts = Vec::new();
                        let mut dq_literal = String::new();
                        loop {
                            match self.peek() {
                                None | Some('"') => {
                                    self.advance();
                                    break;
                                }
                                Some('\\') => {
                                    self.advance();
                                    match self.advance() {
                                        Some(c @ ('$' | '`' | '"' | '\\')) => {
                                            dq_literal.push(c);
                                        }
                                        Some(c) => {
                                            dq_literal.push('\\');
                                            dq_literal.push(c);
                                        }
                                        None => dq_literal.push('\\'),
                                    }
                                }
                                Some('$') => {
                                    if !dq_literal.is_empty() {
                                        dq_parts.push(WordPart::Literal(std::mem::take(
                                            &mut dq_literal,
                                        )));
                                    }
                                    self.advance();
                                    let input_clone = self.input.clone();
                                    let old_pos = self.pos;
                                    let part = parse_dollar_with_warnings(
                                        &input_clone,
                                        &mut self.pos,
                                        true,
                                        &mut self.heredoc_eof_warnings,
                                    );
                                    for &ch in &input_clone[old_pos..self.pos] {
                                        if ch == '\n' {
                                            self.line += 1;
                                        }
                                    }
                                    dq_parts.push(part);
                                }
                                Some('`') => {
                                    if !dq_literal.is_empty() {
                                        dq_parts.push(WordPart::Literal(std::mem::take(
                                            &mut dq_literal,
                                        )));
                                    }
                                    self.advance();
                                    let mut cmd = String::new();
                                    loop {
                                        match self.peek() {
                                            None | Some('`') => {
                                                self.advance();
                                                break;
                                            }
                                            Some(c) => {
                                                cmd.push(c);
                                                self.advance();
                                            }
                                        }
                                    }
                                    dq_parts.push(WordPart::BacktickSub(cmd));
                                }
                                Some(c) => {
                                    dq_literal.push(c);
                                    self.advance();
                                }
                            }
                        }
                        if !dq_literal.is_empty() {
                            dq_parts.push(WordPart::Literal(dq_literal));
                        }
                        parts.push(WordPart::DoubleQuoted(dq_parts));
                    } else if self.peek() == Some('\'') {
                        // $'...' ANSI-C quoting
                        self.advance();
                        let mut s = String::new();
                        let mut nul_terminated = false;
                        loop {
                            match self.advance() {
                                None | Some('\'') => break,
                                Some('\\') => match self.advance() {
                                    Some('n') => s.push('\n'),
                                    Some('t') => s.push('\t'),
                                    Some('r') => s.push('\r'),
                                    Some('\\') => s.push('\\'),
                                    Some('\'') => s.push('\''),
                                    Some('"') => s.push('"'),
                                    Some('a') => s.push('\x07'),
                                    Some('b') => s.push('\x08'),
                                    Some('c') => {
                                        // \cX — control character (X ^ 0x40), like bash
                                        // If next char is \, process the escape first
                                        if let Some(ch) = self.advance() {
                                            let target_char = if ch == '\\' {
                                                self.advance().unwrap_or('\\')
                                            } else {
                                                ch
                                            };
                                            let ctrl = (target_char as u8) ^ 0x40;
                                            if ctrl == 0 {
                                                nul_terminated = true;
                                                break;
                                            }
                                            s.push(ctrl as char);
                                        }
                                    }
                                    Some('e') | Some('E') => s.push('\x1b'),
                                    Some('f') => s.push('\x0c'),
                                    Some('v') => s.push('\x0b'),
                                    Some(oc @ '0'..='7') => {
                                        let mut val = oc as u8 - b'0';
                                        for _ in 0..2 {
                                            match self.peek() {
                                                Some(c @ '0'..='7') => {
                                                    val = val * 8 + (c as u8 - b'0');
                                                    self.advance();
                                                }
                                                _ => break,
                                            }
                                        }
                                        if val == 0 {
                                            nul_terminated = true;
                                            break; // NUL terminates string
                                        }
                                        s.push(val as char);
                                    }
                                    Some('x') => {
                                        let mut val = 0u32;
                                        let mut count = 0;
                                        let mut braced = false;
                                        // \x{NN} or \xNN (up to 2 hex digits without braces)
                                        if self.peek() == Some('{') {
                                            braced = true;
                                            self.advance(); // consume {
                                            while let Some(c) = self.peek() {
                                                if c == '}' {
                                                    self.advance();
                                                    break;
                                                }
                                                if c.is_ascii_hexdigit() {
                                                    val = val * 16 + c.to_digit(16).unwrap();
                                                    self.advance();
                                                    count += 1;
                                                } else {
                                                    break;
                                                }
                                            }
                                        } else {
                                            for _ in 0..2 {
                                                match self.peek() {
                                                    Some(c) if c.is_ascii_hexdigit() => {
                                                        val = val * 16 + c.to_digit(16).unwrap();
                                                        self.advance();
                                                        count += 1;
                                                    }
                                                    _ => break,
                                                }
                                            }
                                        }
                                        if count > 0 || braced {
                                            // \x produces single bytes (truncate to 0xFF)
                                            let byte_val = (val & 0xFF) as u8;
                                            if byte_val == 0 {
                                                nul_terminated = true;
                                                break;
                                            }
                                            s.push(byte_val as char);
                                        } else {
                                            s.push('\\');
                                            s.push('x');
                                        }
                                    }
                                    Some('u') => {
                                        let mut val = 0u32;
                                        let mut count = 0;
                                        if self.peek() == Some('{') {
                                            self.advance();
                                            while let Some(c) = self.peek() {
                                                if c == '}' {
                                                    self.advance();
                                                    break;
                                                }
                                                if c.is_ascii_hexdigit() {
                                                    val = val * 16 + c.to_digit(16).unwrap();
                                                    self.advance();
                                                    count += 1;
                                                } else {
                                                    break;
                                                }
                                            }
                                        } else {
                                            for _ in 0..4 {
                                                match self.peek() {
                                                    Some(c) if c.is_ascii_hexdigit() => {
                                                        val = val * 16 + c.to_digit(16).unwrap();
                                                        self.advance();
                                                        count += 1;
                                                    }
                                                    _ => break,
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
                                    Some('U') => {
                                        let mut val = 0u32;
                                        let mut count = 0;
                                        if self.peek() == Some('{') {
                                            self.advance();
                                            while let Some(c) = self.peek() {
                                                if c == '}' {
                                                    self.advance();
                                                    break;
                                                }
                                                if c.is_ascii_hexdigit() {
                                                    val = val * 16 + c.to_digit(16).unwrap();
                                                    self.advance();
                                                    count += 1;
                                                } else {
                                                    break;
                                                }
                                            }
                                        } else {
                                            for _ in 0..8 {
                                                match self.peek() {
                                                    Some(c) if c.is_ascii_hexdigit() => {
                                                        val = val * 16 + c.to_digit(16).unwrap();
                                                        self.advance();
                                                        count += 1;
                                                    }
                                                    _ => break,
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
                                    Some(c) => {
                                        s.push('\\');
                                        s.push(c);
                                    }
                                    None => s.push('\\'),
                                },
                                Some(c) => s.push(c),
                            }
                        }
                        // If NUL-terminated, skip to closing quote
                        if nul_terminated {
                            while let Some(c) = self.peek() {
                                self.advance();
                                if c == '\'' {
                                    break;
                                }
                            }
                        }
                        parts.push(WordPart::SingleQuoted(s));
                    } else {
                        let input_clone = self.input.clone();
                        let old_pos = self.pos;
                        let part = parse_dollar_with_warnings(
                            &input_clone,
                            &mut self.pos,
                            false,
                            &mut self.heredoc_eof_warnings,
                        );
                        // Update line counter for newlines consumed by parse_dollar
                        for &ch in &input_clone[old_pos..self.pos] {
                            if ch == '\n' {
                                self.line += 1;
                            }
                        }
                        parts.push(part);
                    }
                }
                '`' => {
                    if !literal.is_empty() {
                        parts.push(WordPart::Literal(std::mem::take(&mut literal)));
                    }
                    self.advance();
                    let mut cmd = String::new();
                    loop {
                        match self.peek() {
                            None | Some('`') => {
                                self.advance();
                                break;
                            }
                            Some('\\') => {
                                self.advance();
                                if let Some(c) = self.advance() {
                                    if matches!(c, '$' | '`' | '\\') {
                                        cmd.push(c);
                                    } else if c == '\n' {
                                        // \<newline> is line continuation — remove both
                                    } else {
                                        cmd.push('\\');
                                        cmd.push(c);
                                    }
                                }
                            }
                            Some(c) => {
                                cmd.push(c);
                                self.advance();
                            }
                        }
                    }
                    parts.push(WordPart::BacktickSub(cmd));
                }
                '~' if parts.is_empty() && literal.is_empty() => {
                    let _tilde_pos = self.pos;
                    self.advance();
                    let mut user = String::new();
                    let mut valid_tilde = true;
                    // Check for ~+ and ~- first
                    if let Some(c) = self.peek() {
                        if (c == '+' || c == '-')
                            && !self
                                .input
                                .get(self.pos + 1)
                                .is_some_and(|&nc| nc.is_alphanumeric() || nc == '_')
                        {
                            user.push(c);
                            self.advance();
                        } else {
                            while let Some(c) = self.peek() {
                                if c.is_alphanumeric() || c == '_' || c == '-' || c == '.' {
                                    user.push(c);
                                    self.advance();
                                } else {
                                    break;
                                }
                            }
                        }
                    }
                    // Tilde prefix is only valid if followed by /, :, or end of word
                    if let Some(next) = self.peek()
                        && next != '/'
                        && next != ':'
                        && !next.is_whitespace()
                        && next != ';'
                        && next != '|'
                        && next != '&'
                        && next != ')'
                        && next != '}'
                        && next != '\n'
                    {
                        valid_tilde = false;
                    }
                    if valid_tilde {
                        parts.push(WordPart::Tilde(user));
                    } else {
                        // Revert: treat ~ and consumed chars as literal
                        literal.push('~');
                        literal.push_str(&user);
                    }
                }
                c => {
                    literal.push(c);
                    self.advance();
                }
            }
        }

        if !literal.is_empty() {
            parts.push(WordPart::Literal(literal));
        }

        if parts.is_empty() {
            Token::Eof
        } else {
            Token::Word(parts)
        }
    }

    /// Read raw text until `))` is found (for arithmetic commands).
    /// Skip a `$(...)` command substitution starting at the `(` after `$`.
    /// Handles case/esac, quotes, nested $(), and backticks.
    /// Returns the consumed text including the outer `(...)`.
    fn skip_comsub(&mut self) -> String {
        let mut s = String::new();
        // self.pos is at the '(' of '$('
        s.push(self.input[self.pos]); // '('
        self.pos += 1;
        let mut depth = 1i32;
        let mut case_depth = 0i32;
        while self.pos < self.input.len() && depth > 0 {
            let ch = self.input[self.pos];
            match ch {
                '\'' => {
                    // Single-quoted string — skip entirely
                    s.push(ch);
                    self.pos += 1;
                    while self.pos < self.input.len() && self.input[self.pos] != '\'' {
                        s.push(self.input[self.pos]);
                        self.pos += 1;
                    }
                    if self.pos < self.input.len() {
                        s.push(self.input[self.pos]);
                        self.pos += 1;
                    }
                    continue;
                }
                '"' => {
                    // Double-quoted string — skip but handle escapes
                    s.push(ch);
                    self.pos += 1;
                    while self.pos < self.input.len() && self.input[self.pos] != '"' {
                        if self.input[self.pos] == '\\' && self.pos + 1 < self.input.len() {
                            s.push(self.input[self.pos]);
                            self.pos += 1;
                        }
                        s.push(self.input[self.pos]);
                        self.pos += 1;
                    }
                    if self.pos < self.input.len() {
                        s.push(self.input[self.pos]);
                        self.pos += 1;
                    }
                    continue;
                }
                '`' => {
                    // Backtick command sub — skip
                    s.push(ch);
                    self.pos += 1;
                    while self.pos < self.input.len() && self.input[self.pos] != '`' {
                        if self.input[self.pos] == '\\' && self.pos + 1 < self.input.len() {
                            s.push(self.input[self.pos]);
                            self.pos += 1;
                        }
                        s.push(self.input[self.pos]);
                        self.pos += 1;
                    }
                    if self.pos < self.input.len() {
                        s.push(self.input[self.pos]);
                        self.pos += 1;
                    }
                    continue;
                }
                '(' => depth += 1,
                ')' => {
                    if case_depth <= 0 {
                        depth -= 1;
                        if depth == 0 {
                            s.push(ch);
                            self.pos += 1;
                            return s;
                        }
                    }
                    // Inside a case block, ) is a pattern delimiter — skip
                }
                _ => {}
            }
            // Track case/esac keywords
            if ch.is_alphabetic() {
                let mut word = String::new();
                while self.pos < self.input.len()
                    && (self.input[self.pos].is_alphanumeric() || self.input[self.pos] == '_')
                {
                    word.push(self.input[self.pos]);
                    self.pos += 1;
                }
                // Check for case/esac keywords, also through aliases
                let effective_word = if self.shopt_expand_aliases {
                    self.aliases
                        .get(word.as_str())
                        .map(|v| v.trim().to_string())
                        .unwrap_or_else(|| word.clone())
                } else {
                    word.clone()
                };
                if effective_word == "case" {
                    case_depth += 1;
                } else if effective_word == "esac" || word == "esac" {
                    case_depth -= 1;
                } else if effective_word == "(" {
                    depth += 1;
                }
                s.push_str(&word);
                continue;
            }
            s.push(ch);
            self.pos += 1;
        }
        s
    }

    /// Skip a `${ ... }` funsub starting at the `{` after `$`.
    /// Returns the consumed text including the outer `{...}`.
    fn skip_funsub(&mut self) -> String {
        let mut s = String::new();
        // self.pos is at the '{' of '${'
        s.push(self.input[self.pos]); // '{'
        self.pos += 1;
        // Skip whitespace after '{' to confirm it's a funsub (has space)
        let mut depth = 1i32;
        while self.pos < self.input.len() && depth > 0 {
            let ch = self.input[self.pos];
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        s.push(ch);
                        self.pos += 1;
                        return s;
                    }
                }
                '\'' => {
                    s.push(ch);
                    self.pos += 1;
                    while self.pos < self.input.len() && self.input[self.pos] != '\'' {
                        s.push(self.input[self.pos]);
                        self.pos += 1;
                    }
                    if self.pos < self.input.len() {
                        s.push(self.input[self.pos]);
                        self.pos += 1;
                    }
                    continue;
                }
                '"' => {
                    s.push(ch);
                    self.pos += 1;
                    while self.pos < self.input.len() && self.input[self.pos] != '"' {
                        if self.input[self.pos] == '\\' && self.pos + 1 < self.input.len() {
                            s.push(self.input[self.pos]);
                            self.pos += 1;
                        }
                        s.push(self.input[self.pos]);
                        self.pos += 1;
                    }
                    if self.pos < self.input.len() {
                        s.push(self.input[self.pos]);
                        self.pos += 1;
                    }
                    continue;
                }
                _ => {}
            }
            s.push(ch);
            self.pos += 1;
        }
        s
    }

    /// The `((` has already been consumed by the parser.
    pub fn read_until_double_paren(&mut self) -> Result<String, String> {
        let mut expr = String::new();
        let mut paren_depth = 0; // Track nested ( ) inside the expression
        while self.pos < self.input.len() {
            let ch = self.input[self.pos];
            // Handle $(...) and ${ ... } — use proper parsers
            if ch == '$' && self.pos + 1 < self.input.len() {
                if self.input[self.pos + 1] == '(' {
                    expr.push(ch);
                    self.pos += 1;
                    expr.push_str(&self.skip_comsub());
                    continue;
                }
                if self.input[self.pos + 1] == '{'
                    && self.pos + 2 < self.input.len()
                    && self.input[self.pos + 2] == ' '
                {
                    expr.push(ch);
                    self.pos += 1;
                    expr.push_str(&self.skip_funsub());
                    continue;
                }
            }
            if ch == '(' {
                paren_depth += 1;
                expr.push(ch);
                self.pos += 1;
            } else if ch == ')' {
                if paren_depth > 0 {
                    // Close an inner paren
                    paren_depth -= 1;
                    expr.push(ch);
                    self.pos += 1;
                } else {
                    // At top level — check if this starts the closing ))
                    // Skip whitespace after first ) to find second )
                    self.pos += 1;
                    let saved = self.pos;
                    while self.pos < self.input.len() && matches!(self.input[self.pos], ' ' | '\t')
                    {
                        self.pos += 1;
                    }
                    if self.pos < self.input.len() && self.input[self.pos] == ')' {
                        // Found )) (possibly with whitespace between)
                        self.pos += 1;
                        return Ok(expr.trim_start().to_string());
                    }
                    // Not )), restore position and treat as expression
                    self.pos = saved;
                    expr.push(')');
                }
            } else if ch == ';' && paren_depth == 0 {
                return Err("syntax error: `;' unexpected".to_string());
            } else {
                if ch == '\n' {
                    self.line += 1;
                }
                expr.push(ch);
                self.pos += 1;
            }
        }
        Err("unexpected EOF while looking for matching `)'".to_string())
    }

    /// Read raw text until the given character is found (for C-style for loops).
    pub fn read_until_char(&mut self, target: char) -> Result<String, String> {
        let mut s = String::new();
        let mut depth = 0i32;
        while self.pos < self.input.len() {
            let ch = self.input[self.pos];
            // Handle $((...)) and $(...) — use proper parsers
            if ch == '$' && self.pos + 1 < self.input.len() {
                if self.input[self.pos + 1] == '('
                    && self.pos + 2 < self.input.len()
                    && self.input[self.pos + 2] == '('
                {
                    // $((arith)) — skip arithmetic expansion
                    s.push_str("$((");
                    self.pos += 3;
                    let mut arith_depth = 1i32;
                    while self.pos < self.input.len() && arith_depth > 0 {
                        let c = self.input[self.pos];
                        if c == '$'
                            && self.pos + 1 < self.input.len()
                            && self.input[self.pos + 1] == '('
                        {
                            // Nested $( — skip comsub inside arithmetic
                            s.push('$');
                            self.pos += 1;
                            s.push_str(&self.skip_comsub());
                            continue;
                        } else if c == ')'
                            && self.pos + 1 < self.input.len()
                            && self.input[self.pos + 1] == ')'
                        {
                            arith_depth -= 1;
                            if arith_depth == 0 {
                                s.push_str("))");
                                self.pos += 2;
                                break;
                            }
                        }
                        s.push(c);
                        self.pos += 1;
                    }
                    continue;
                }
                if self.input[self.pos + 1] == '(' {
                    s.push(ch);
                    self.pos += 1;
                    s.push_str(&self.skip_comsub());
                    continue;
                }
                if self.input[self.pos + 1] == '{'
                    && self.pos + 2 < self.input.len()
                    && self.input[self.pos + 2] == ' '
                {
                    s.push(ch);
                    self.pos += 1;
                    s.push_str(&self.skip_funsub());
                    continue;
                }
            }
            if ch == '(' {
                depth += 1;
            } else if ch == ')' {
                depth -= 1;
            }
            if ch == target && depth == 0 {
                self.pos += 1; // consume the delimiter
                return Ok(s.trim_start().to_string());
            }
            s.push(ch);
            self.pos += 1;
        }
        Err(format!("unexpected EOF looking for '{}'", target))
    }

    /// Get the next heredoc body (called by the parser when processing heredoc redirections).
    pub fn take_heredoc_body(&mut self) -> Option<Word> {
        if self.heredoc_index < self.heredoc_bodies.len() {
            let body = self.heredoc_bodies[self.heredoc_index].clone();
            self.heredoc_index += 1;
            Some(body)
        } else {
            None
        }
    }

    pub fn take_heredoc_delimiter(&mut self) -> Option<String> {
        if !self.heredoc_delimiters.is_empty() {
            Some(self.heredoc_delimiters.remove(0))
        } else {
            None
        }
    }
}
