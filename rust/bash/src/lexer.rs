use crate::ast::*;

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Word(Word),
    Newline,
    Pipe,
    AndIf,
    OrIf,
    Semi,
    Amp,
    DSemi,
    /// `;&` — case fallthrough
    SemiAmp,
    /// `;;&` — case test-next
    DSemiAmp,
    LParen,
    RParen,
    Less,
    Great,
    DLess,
    DGreat,
    LessAnd,
    GreatAnd,
    LessGreat,
    DLessDash,
    Clobber,
    TripleLess,
    Eof,
}

struct HereDocPending {
    delimiter: String,
    strip_tabs: bool,
    quoted: bool,
}

pub struct Lexer {
    input: Vec<char>,
    pos: usize,
    pub line: usize,
    pending_heredocs: Vec<HereDocPending>,
    heredoc_bodies: Vec<Word>,
    heredoc_index: usize,
}

impl Lexer {
    pub fn new(input: &str) -> Self {
        Self {
            input: input.chars().collect(),
            pos: 0,
            line: 1,
            pending_heredocs: Vec::new(),
            heredoc_bodies: Vec::new(),
            heredoc_index: 0,
        }
    }

    fn peek(&self) -> Option<char> {
        self.input.get(self.pos).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let ch = self.input.get(self.pos).copied();
        if let Some(c) = ch {
            if c == '\n' {
                self.line += 1;
            }
            self.pos += 1;
        }
        ch
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        self.input.get(self.pos + offset).copied()
    }

    pub fn save_position(&self) -> (usize, usize, usize) {
        (self.pos, self.pending_heredocs.len(), self.line)
    }

    pub fn restore_position(&mut self, saved: (usize, usize, usize)) {
        self.pos = saved.0;
        self.pending_heredocs.truncate(saved.1);
        self.line = saved.2;
    }

    fn skip_whitespace(&mut self) {
        while let Some(ch) = self.peek() {
            if ch == ' ' || ch == '\t' {
                self.advance();
            } else if ch == '\\' && self.peek_at(1) == Some('\n') {
                // Line continuation
                self.advance();
                self.advance();
            } else {
                break;
            }
        }
    }

    fn skip_comment(&mut self) {
        if self.peek() == Some('#') {
            while let Some(ch) = self.peek() {
                if ch == '\n' {
                    break;
                }
                self.advance();
            }
        }
    }

    pub fn next_token(&mut self) -> Token {
        self.skip_whitespace();
        self.skip_comment();

        let ch = match self.peek() {
            None => return Token::Eof,
            Some(c) => c,
        };

        match ch {
            '\n' => {
                self.advance();
                self.read_heredoc_bodies();
                Token::Newline
            }
            '|' => {
                self.advance();
                if self.peek() == Some('|') {
                    self.advance();
                    Token::OrIf
                } else {
                    Token::Pipe
                }
            }
            '&' => {
                self.advance();
                if self.peek() == Some('&') {
                    self.advance();
                    Token::AndIf
                } else {
                    Token::Amp
                }
            }
            ';' => {
                self.advance();
                if self.peek() == Some(';') {
                    self.advance();
                    if self.peek() == Some('&') {
                        self.advance();
                        Token::DSemiAmp
                    } else {
                        Token::DSemi
                    }
                } else if self.peek() == Some('&') {
                    self.advance();
                    Token::SemiAmp
                } else {
                    Token::Semi
                }
            }
            '(' => {
                self.advance();
                Token::LParen
            }
            ')' => {
                self.advance();
                Token::RParen
            }
            '<' => {
                // Check for process substitution <(cmd) — must come before consuming <
                if self.peek_at(1) == Some('(') {
                    return self.read_word();
                }
                self.advance();
                match self.peek() {
                    Some('<') => {
                        self.advance();
                        if self.peek() == Some('<') {
                            self.advance();
                            Token::TripleLess
                        } else if self.peek() == Some('-') {
                            self.advance();
                            self.register_heredoc(true);
                            Token::DLessDash
                        } else {
                            self.register_heredoc(false);
                            Token::DLess
                        }
                    }
                    Some('&') => {
                        self.advance();
                        Token::LessAnd
                    }
                    Some('>') => {
                        self.advance();
                        Token::LessGreat
                    }
                    _ => Token::Less,
                }
            }
            '>' => {
                // Check for process substitution >(cmd)
                if self.peek_at(1) == Some('(') {
                    return self.read_word();
                }
                self.advance();
                match self.peek() {
                    Some('>') => {
                        self.advance();
                        Token::DGreat
                    }
                    Some('&') => {
                        self.advance();
                        Token::GreatAnd
                    }
                    Some('|') => {
                        self.advance();
                        Token::Clobber
                    }
                    _ => Token::Great,
                }
            }
            _ => self.read_word(),
        }
    }

    fn register_heredoc(&mut self, strip_tabs: bool) {
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
                    if ch.is_alphanumeric() || ch == '_' {
                        delimiter.push(ch);
                        self.advance();
                    } else {
                        break;
                    }
                }
            }
        }

        self.pending_heredocs.push(HereDocPending {
            delimiter,
            strip_tabs,
            quoted,
        });
    }

    fn read_heredoc_bodies(&mut self) {
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
                    break;
                }
            }

            let word = if hd.quoted {
                vec![WordPart::SingleQuoted(body)]
            } else {
                parse_double_quoted_content(&body)
            };
            self.heredoc_bodies.push(word);
        }
    }
}

fn parse_double_quoted_content(s: &str) -> Word {
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
                if !literal.is_empty() {
                    parts.push(WordPart::Literal(std::mem::take(&mut literal)));
                }
                i += 1;
                let part = parse_dollar(&chars, &mut i);
                parts.push(part);
            }
            '`' => {
                if !literal.is_empty() {
                    parts.push(WordPart::Literal(std::mem::take(&mut literal)));
                }
                i += 1;
                let mut cmd = String::new();
                while i < chars.len() && chars[i] != '`' {
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        cmd.push(chars[i + 1]);
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
            ch => {
                literal.push(ch);
                i += 1;
            }
        }
    }
    if !literal.is_empty() {
        parts.push(WordPart::Literal(literal));
    }
    parts
}

fn parse_dollar(chars: &[char], i: &mut usize) -> WordPart {
    if *i >= chars.len() {
        return WordPart::Literal("$".to_string());
    }

    match chars[*i] {
        '(' => {
            *i += 1;
            if *i < chars.len() && chars[*i] == '(' {
                // Arithmetic: $(( ... ))
                *i += 1;
                let mut depth = 1;
                let mut expr = String::new();
                while *i < chars.len() && depth > 0 {
                    if *i + 1 < chars.len() && chars[*i] == ')' && chars[*i + 1] == ')' {
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
                        expr.push(chars[*i]);
                        *i += 1;
                    }
                }
                WordPart::ArithSub(expr)
            } else {
                // Command substitution: $( ... )
                let mut depth = 1;
                let mut cmd = String::new();
                while *i < chars.len() && depth > 0 {
                    if chars[*i] == '(' {
                        depth += 1;
                    } else if chars[*i] == ')' {
                        depth -= 1;
                        if depth == 0 {
                            *i += 1;
                            break;
                        }
                    }
                    cmd.push(chars[*i]);
                    *i += 1;
                }
                WordPart::CommandSub(cmd)
            }
        }
        '{' => {
            *i += 1;
            parse_brace_param(chars, i)
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
                        if !dq_lit.is_empty() {
                            dq_parts.push(WordPart::Literal(std::mem::take(&mut dq_lit)));
                        }
                        *i += 1;
                        dq_parts.push(parse_dollar(chars, i));
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
        '\'' => {
            // $'...' ANSI-C quoting
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
                        'a' => s.push('\x07'),
                        'b' => s.push('\x08'),
                        'e' | 'E' => s.push('\x1b'),
                        'f' => s.push('\x0c'),
                        'v' => s.push('\x0b'),
                        '0' => {
                            let mut val = 0u8;
                            for _ in 0..3 {
                                if *i + 1 < chars.len() && matches!(chars[*i + 1], '0'..='7') {
                                    *i += 1;
                                    val = val * 8 + (chars[*i] as u8 - b'0');
                                } else {
                                    break;
                                }
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
                                if let Some(c) = char::from_u32(val) {
                                    s.push(c);
                                }
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

fn parse_brace_param(chars: &[char], i: &mut usize) -> WordPart {
    // ${!name} — indirect expansion / name prefix / array indices
    if *i < chars.len() && chars[*i] == '!' {
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
            let op = read_param_op(chars, i, &name);
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
    }

    // ${#name} - length
    if *i < chars.len() && chars[*i] == '#' {
        let next = if *i + 1 < chars.len() {
            chars[*i + 1]
        } else {
            '}'
        };
        if next != '}' {
            *i += 1;
            let name = read_param_name_with_subscript(chars, i);
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

    // Check for @X transform operator before }
    if *i + 1 < chars.len() && chars[*i] == '@' && chars[*i + 1] != '}' {
        let transform_char = chars[*i + 1];
        if matches!(
            transform_char,
            'E' | 'Q' | 'P' | 'A' | 'a' | 'K' | 'k' | 'L' | 'U'
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

    let op = read_param_op(chars, i, &name);

    // Check for @X transform after operator
    if *i + 1 < chars.len() && chars[*i] == '@' && chars[*i + 1] != '}' {
        let transform_char = chars[*i + 1];
        if matches!(
            transform_char,
            'E' | 'Q' | 'P' | 'A' | 'a' | 'K' | 'k' | 'L' | 'U'
        ) {
            *i += 2;
        }
    }

    // Skip to closing } — handles unrecognized syntax gracefully
    // Skip to closing }, handling nested braces
    let mut depth = 1i32;
    while *i < chars.len() && depth > 0 {
        match chars[*i] {
            '{' => depth += 1,
            '}' => depth -= 1,
            _ => {}
        }
        *i += 1;
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
    if *i < chars.len()
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

fn read_param_op(chars: &[char], i: &mut usize, _name: &str) -> ParamOp {
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
                    let word = read_param_word(chars, i);
                    ParamOp::Default(true, word)
                }
                '=' => {
                    *i += 1;
                    let word = read_param_word(chars, i);
                    ParamOp::Assign(true, word)
                }
                '?' => {
                    *i += 1;
                    let word = read_param_word(chars, i);
                    ParamOp::Error(true, word)
                }
                '+' => {
                    *i += 1;
                    let word = read_param_word(chars, i);
                    ParamOp::Alt(true, word)
                }
                _ => {
                    // ${var:offset} or ${var:offset:length}
                    let mut offset = String::new();
                    while *i < chars.len() && chars[*i] != ':' && chars[*i] != '}' {
                        offset.push(chars[*i]);
                        *i += 1;
                    }
                    let length = if *i < chars.len() && chars[*i] == ':' {
                        *i += 1;
                        let mut l = String::new();
                        while *i < chars.len() && chars[*i] != '}' {
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
            let word = read_param_word(chars, i);
            ParamOp::Default(false, word)
        }
        '=' => {
            *i += 1;
            let word = read_param_word(chars, i);
            ParamOp::Assign(false, word)
        }
        '?' => {
            *i += 1;
            let word = read_param_word(chars, i);
            ParamOp::Error(false, word)
        }
        '+' => {
            *i += 1;
            let word = read_param_word(chars, i);
            ParamOp::Alt(false, word)
        }
        '#' => {
            *i += 1;
            if *i < chars.len() && chars[*i] == '#' {
                *i += 1;
                let word = read_param_word(chars, i);
                ParamOp::TrimLargeLeft(word)
            } else {
                let word = read_param_word(chars, i);
                ParamOp::TrimSmallLeft(word)
            }
        }
        '%' => {
            *i += 1;
            if *i < chars.len() && chars[*i] == '%' {
                *i += 1;
                let word = read_param_word(chars, i);
                ParamOp::TrimLargeRight(word)
            } else {
                let word = read_param_word(chars, i);
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
            let pattern = read_param_word_until(chars, i, '/');
            let replacement = if *i < chars.len() && chars[*i] == '/' {
                *i += 1;
                read_param_word(chars, i)
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
                let pattern = read_param_word(chars, i);
                ParamOp::UpperAll(pattern)
            } else {
                let pattern = read_param_word(chars, i);
                ParamOp::UpperFirst(pattern)
            }
        }
        ',' => {
            *i += 1;
            if *i < chars.len() && chars[*i] == ',' {
                *i += 1;
                let pattern = read_param_word(chars, i);
                ParamOp::LowerAll(pattern)
            } else {
                let pattern = read_param_word(chars, i);
                ParamOp::LowerFirst(pattern)
            }
        }
        _ => ParamOp::None,
    }
}

fn read_param_word(chars: &[char], i: &mut usize) -> Word {
    read_param_word_until(chars, i, '}')
}

fn read_param_word_until(chars: &[char], i: &mut usize, delim: char) -> Word {
    let mut parts = Vec::new();
    let mut literal = String::new();
    let mut depth = 0;

    while *i < chars.len() && (chars[*i] != delim || depth > 0) && chars[*i] != '}' {
        match chars[*i] {
            '\\' if *i + 1 < chars.len() => {
                literal.push(chars[*i + 1]);
                *i += 2;
            }
            '$' => {
                if !literal.is_empty() {
                    parts.push(WordPart::Literal(std::mem::take(&mut literal)));
                }
                *i += 1;
                parts.push(parse_dollar(chars, i));
            }
            '\'' => {
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
                *i += 1;
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
                            if !dq_lit.is_empty() {
                                dq_parts.push(WordPart::Literal(std::mem::take(&mut dq_lit)));
                            }
                            *i += 1;
                            dq_parts.push(parse_dollar(chars, i));
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
    fn read_word(&mut self) -> Token {
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
                            literal.push(next);
                        }
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
                                if !dq_lit.is_empty() {
                                    dq_parts.push(WordPart::Literal(std::mem::take(&mut dq_lit)));
                                }
                                self.advance();
                                let input_clone = self.input.clone();
                                let part = parse_dollar(&input_clone, &mut self.pos);
                                dq_parts.push(part);
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
                                    let part = parse_dollar(&input_clone, &mut self.pos);
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
                                    Some('a') => s.push('\x07'),
                                    Some('b') => s.push('\x08'),
                                    Some('e') | Some('E') => s.push('\x1b'),
                                    Some('f') => s.push('\x0c'),
                                    Some('v') => s.push('\x0b'),
                                    Some('0') => {
                                        let mut val = 0u8;
                                        for _ in 0..3 {
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
                                            if val == 0 {
                                                nul_terminated = true;
                                                break;
                                            }
                                            if let Some(c) = char::from_u32(val) {
                                                s.push(c);
                                            }
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
                        let part = parse_dollar(&input_clone, &mut self.pos);
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
                    self.advance();
                    let mut user = String::new();
                    while let Some(c) = self.peek() {
                        if c.is_alphanumeric() || c == '_' || c == '-' || c == '.' {
                            user.push(c);
                            self.advance();
                        } else {
                            break;
                        }
                    }
                    parts.push(WordPart::Tilde(user));
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
    /// The `((` has already been consumed by the parser.
    pub fn read_until_double_paren(&mut self) -> Result<String, String> {
        let mut expr = String::new();
        let mut depth = 1; // We're inside one level of ((
        while self.pos < self.input.len() {
            let ch = self.input[self.pos];
            if ch == '(' && self.peek_at(1) == Some('(') {
                depth += 1;
                expr.push('(');
                expr.push('(');
                self.pos += 2;
            } else if ch == ')' && self.peek_at(1) == Some(')') {
                depth -= 1;
                if depth == 0 {
                    self.pos += 2;
                    return Ok(expr.trim().to_string());
                }
                expr.push(')');
                expr.push(')');
                self.pos += 2;
            } else {
                expr.push(ch);
                self.pos += 1;
            }
        }
        Err("unexpected EOF looking for ))".to_string())
    }

    /// Read raw text until the given character is found (for C-style for loops).
    pub fn read_until_char(&mut self, target: char) -> Result<String, String> {
        let mut s = String::new();
        let mut depth = 0i32;
        while self.pos < self.input.len() {
            let ch = self.input[self.pos];
            if ch == '(' {
                depth += 1;
            } else if ch == ')' {
                depth -= 1;
            }
            if ch == target && depth == 0 {
                self.pos += 1; // consume the delimiter
                return Ok(s.trim().to_string());
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
}
