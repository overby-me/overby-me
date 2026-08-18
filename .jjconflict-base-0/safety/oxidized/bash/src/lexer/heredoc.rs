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
        // Capture the current line at the start of body reading.  Bash uses
        // `line_number` at `gather_here_documents` time — which is the line
        // the parser is on when it starts reading bodies (i.e. the line the
        // command ended on), NOT the line where `<<` appeared.  Since
        // `read_heredoc_bodies` is called right after the newline token is
        // consumed, `self.line` has already been incremented, so subtract 1.
        let body_start_line = if self.line > 1 {
            self.line - 1
        } else {
            self.line
        };
        for hd in heredocs {
            let mut body = String::new();
            // Track whether we read at least one content line (even if the
            // line itself was empty).  This lets us distinguish "no body at
            // all" (delimiter on first line, or immediate EOF) from "body
            // with one empty line" — both produce body=="" but only the
            // latter should get a trailing newline when written to the
            // heredoc fd.
            let mut had_content_line = false;
            loop {
                // If input is already exhausted before reading any line,
                // emit the EOF warning immediately without adding an empty
                // body line.  This prevents an empty heredoc from gaining a
                // spurious newline (bash produces 0-byte content here).
                if self.pos >= self.input.len() {
                    let eof_line = body_start_line;
                    self.heredoc_eof_warnings.push((
                        eof_line,
                        body_start_line,
                        hd.delimiter.clone(),
                    ));
                    break;
                }
                let mut line = String::new();
                loop {
                    match self.advance() {
                        None => break,
                        Some('\n') => break,
                        Some(ch) => line.push(ch),
                    }
                }
                // Backslash-newline line continuation for unquoted heredocs:
                // if the line ends with `\`, join with the next line before
                // checking for the delimiter (bash processes \<newline> first).
                if !hd.quoted && line.ends_with('\\') {
                    line.pop(); // remove trailing backslash
                    // Read the next line and append
                    loop {
                        match self.advance() {
                            None => break,
                            Some('\n') => break,
                            Some(ch) => line.push(ch),
                        }
                    }
                }
                let check_line = if hd.strip_tabs {
                    line.trim_start_matches('\t').to_string()
                } else {
                    line.clone()
                };
                // When strip_tabs is set (<<-), also strip leading tabs
                // from the delimiter for matching.  Bash allows the
                // delimiter word itself to contain leading tabs (e.g.
                // `<<-'\tEND'`) and still matches after stripping.
                let check_delim = if hd.strip_tabs {
                    hd.delimiter.trim_start_matches('\t')
                } else {
                    &hd.delimiter
                };
                if check_line == check_delim {
                    break;
                }
                if had_content_line {
                    body.push('\n');
                }
                had_content_line = true;
                if hd.strip_tabs {
                    body.push_str(line.trim_start_matches('\t'));
                } else {
                    body.push_str(&line);
                }
                if self.pos >= self.input.len() {
                    // EOF terminated here-document — emit warning
                    // Use line - 1 since the newline after the last content
                    // incremented the line counter past the actual content
                    let eof_line = if self.line > body_start_line {
                        self.line - 1
                    } else {
                        self.line
                    };
                    self.heredoc_eof_warnings.push((
                        eof_line,
                        body_start_line,
                        hd.delimiter.clone(),
                    ));
                    break;
                }
            }

            // When no content lines were read (delimiter matched on the very
            // first line, or immediate EOF), produce an empty word `[]`.
            // This lets the redirect code distinguish "no body at all"
            // (should produce 0-byte content) from "body with content lines"
            // (needs a trailing newline).
            let mut word = if !had_content_line {
                vec![]
            } else if hd.quoted {
                vec![WordPart::SingleQuoted(body)]
            } else {
                let mut w = parse_double_quoted_content(&body);
                // parse_double_quoted_content("") returns [] for an empty
                // string.  Push an empty Literal so the word is non-empty,
                // signalling to the redirect code that there WAS content.
                if w.is_empty() {
                    w.push(WordPart::Literal(String::new()));
                }
                w
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
                if matches!(next, '$' | '`' | '\\' | '\n') {
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
