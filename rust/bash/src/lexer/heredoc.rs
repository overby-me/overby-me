use super::*;

impl Lexer {
    pub(super) fn register_heredoc(&mut self, strip_tabs: bool) {
        self.skip_whitespace();
        let mut delimiter = String::new();
        let mut quoted = false;

        match self.peek() {
            Some('\'') => {
                quoted = true;
                self.advance();
                while let Some(ch) = self.advance() {
                    if ch == '\'' {
                        break;
                    }
                    delimiter.push(ch);
                }
            }
            Some('"') => {
                quoted = true;
                self.advance();
                while let Some(ch) = self.advance() {
                    if ch == '"' {
                        break;
                    }
                    delimiter.push(ch);
                }
            }
            _ => {
                while let Some(ch) = self.peek() {
                    if ch == '\\' {
                        // Backslash quoting in heredoc delimiter
                        quoted = true;
                        self.advance();
                        if let Some(next) = self.peek() {
                            if next == '\n' {
                                // Line continuation: \<newline> is discarded, join next line
                                self.advance();
                                self.line += 1;
                            } else {
                                delimiter.push(next);
                                self.advance();
                            }
                        }
                    } else if ch == '\'' {
                        // Single-quoted portion of delimiter
                        quoted = true;
                        self.advance();
                        while let Some(c) = self.advance() {
                            if c == '\'' {
                                break;
                            }
                            delimiter.push(c);
                        }
                    } else if ch == '"' {
                        // Double-quoted portion of delimiter
                        quoted = true;
                        self.advance();
                        while let Some(c) = self.advance() {
                            if c == '"' {
                                break;
                            }
                            delimiter.push(c);
                        }
                    } else if !ch.is_whitespace()
                        && ch != '\n'
                        && ch != ';'
                        && ch != '&'
                        && ch != '|'
                        && ch != ')'
                        && ch != '>'
                        && ch != '<'
                    {
                        delimiter.push(ch);
                        self.advance();
                    } else {
                        break;
                    }
                }
            }
        }

        // Bash limits to 16 here-documents per command
        if self.pending_heredocs.len() >= 16 {
            self.heredoc_overflow_line = Some(self.line);
            return;
        }
        self.heredoc_delimiters.push(delimiter.clone());
        self.pending_heredocs.push(HereDocPending {
            delimiter,
            strip_tabs,
            quoted,
            start_line: self.line,
        });
    }

    pub(super) fn read_heredoc_bodies(&mut self) {
        let heredocs: Vec<HereDocPending> = self.pending_heredocs.drain(..).collect();
        for hd in heredocs {
            let mut body = String::new();
            loop {
                let mut line = String::new();
                loop {
                    match self.advance() {
                        None => break,
                        Some('\n') => break,
                        Some(ch) => line.push(ch),
                    }
                }
                let check_line = if hd.strip_tabs {
                    line.trim_start_matches('\t').to_string()
                } else {
                    line.clone()
                };
                if check_line == hd.delimiter {
                    break;
                }
                if !body.is_empty() {
                    body.push('\n');
                }
                if hd.strip_tabs {
                    body.push_str(line.trim_start_matches('\t'));
                } else {
                    body.push_str(&line);
                }
                if self.pos >= self.input.len() {
                    // EOF terminated here-document — emit warning
                    // Use line - 1 since the newline after the last content
                    // incremented the line counter past the actual content
                    let eof_line = if self.line > hd.start_line {
                        self.line - 1
                    } else {
                        self.line
                    };
                    self.heredoc_eof_warnings
                        .push((eof_line, hd.start_line, hd.delimiter.clone()));
                    break;
                }
            }

            let mut word = if hd.quoted {
                vec![WordPart::SingleQuoted(body)]
            } else {
                parse_double_quoted_content(&body)
            };
            // Fix up INCOMPLETE_COMSUB line numbers: offset by heredoc start line
            // (since parse_dollar only sees body-relative chars without file context)
            for part in &mut word {
                if let WordPart::CommandSub(s) = part
                    && let Some(rest) = s.strip_prefix("\x00INCOMPLETE_COMSUB:")
                    && let Ok(body_line) = rest.parse::<usize>()
                {
                    // File line = start_line + 1 (body starts after <<) + body_eof_line
                    let file_line = hd.start_line + 1 + body_line;
                    *s = format!("\x00INCOMPLETE_COMSUB:{}", file_line);
                }
            }
            self.heredoc_bodies.push(word);
        }
    }
}

fn parse_double_quoted_content(s: &str) -> Word {
    IN_HEREDOC.with(|f| f.set(true));
    let chars: Vec<char> = s.chars().collect();
    let mut parts = Vec::new();
    let mut literal = String::new();
    let mut i = 0;

    while i < chars.len() {
        match chars[i] {
            '\\' if i + 1 < chars.len() => {
                let next = chars[i + 1];
                if matches!(next, '$' | '`' | '"' | '\\' | '\n') {
                    if next != '\n' {
                        literal.push(next);
                    }
                    i += 2;
                } else {
                    literal.push('\\');
                    i += 1;
                }
            }
            '$' => {
                // Inside double quotes, $' and $" are literal
                if i + 1 < chars.len() && matches!(chars[i + 1], '\'' | '"') {
                    literal.push('$');
                    i += 1;
                } else {
                    if !literal.is_empty() {
                        parts.push(WordPart::Literal(std::mem::take(&mut literal)));
                    }
                    i += 1;
                    let part = parse_dollar(&chars, &mut i, true);
                    parts.push(part);
                }
            }
            '`' => {
                if !literal.is_empty() {
                    parts.push(WordPart::Literal(std::mem::take(&mut literal)));
                }
                i += 1;
                let mut cmd = String::new();
                while i < chars.len() && chars[i] != '`' {
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        let next = chars[i + 1];
                        // Inside double-quoted context backticks, also unescape \"
                        if matches!(next, '$' | '`' | '\\' | '"') {
                            cmd.push(next);
                        } else if next == '\n' {
                            // \<newline> is line continuation — remove both
                        } else {
                            cmd.push('\\');
                            cmd.push(next);
                        }
                        i += 2;
                    } else {
                        cmd.push(chars[i]);
                        i += 1;
                    }
                }
                if i < chars.len() {
                    i += 1; // skip closing `
                }
                parts.push(WordPart::BacktickSub(cmd));
            }
            '<' | '>' if i + 1 < chars.len() && chars[i + 1] == '(' => {
                if !literal.is_empty() {
                    parts.push(WordPart::Literal(std::mem::take(&mut literal)));
                }
                let kind = if chars[i] == '<' {
                    crate::ast::ProcessSubKind::Input
                } else {
                    crate::ast::ProcessSubKind::Output
                };
                i += 2;
                let mut depth = 1i32;
                let mut cmd = String::new();
                while i < chars.len() && depth > 0 {
                    match chars[i] {
                        '(' => depth += 1,
                        ')' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        '\'' => {
                            cmd.push('\'');
                            i += 1;
                            while i < chars.len() && chars[i] != '\'' {
                                cmd.push(chars[i]);
                                i += 1;
                            }
                            if i < chars.len() {
                                cmd.push('\'');
                            }
                        }
                        _ => {}
                    }
                    if depth > 0 {
                        cmd.push(chars[i]);
                    }
                    i += 1;
                }
                if i < chars.len() {
                    i += 1;
                }
                parts.push(WordPart::ProcessSub(kind, cmd));
            }
            ch => {
                literal.push(ch);
                i += 1;
            }
        }
    }
    if !literal.is_empty() {
        parts.push(WordPart::Literal(literal));
    }
    IN_HEREDOC.with(|f| f.set(false));
    parts
}

/// Parse a string as a shell word (for expanding ${...} in arithmetic contexts)
pub fn parse_word_string(s: &str) -> Word {
    let chars: Vec<char> = s.chars().collect();
    let mut parts = Vec::new();
    let mut i = 0;
    let mut lit = String::new();
    while i < chars.len() {
        if chars[i] == '$' {
            if !lit.is_empty() {
                parts.push(WordPart::Literal(std::mem::take(&mut lit)));
            }
            i += 1;
            parts.push(parse_dollar(&chars, &mut i, false));
        } else {
            lit.push(chars[i]);
            i += 1;
        }
    }
    if !lit.is_empty() {
        parts.push(WordPart::Literal(lit));
    }
    parts
}
