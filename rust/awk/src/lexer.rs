use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // Literals
    Number(f64),
    StringLit(String),
    Regex(String),

    // Identifiers and keywords
    Ident(String),
    Begin,
    End,
    If,
    Else,
    While,
    For,
    Do,
    In,
    Delete,
    Print,
    Printf,
    Getline,
    Next,
    Exit,
    Function,
    Return,
    Break,
    Continue,

    // Operators
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Caret,
    Assign,
    PlusAssign,
    MinusAssign,
    StarAssign,
    SlashAssign,
    PercentAssign,
    CaretAssign,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    And,
    Or,
    Not,
    Match,
    NotMatch,
    Increment,
    Decrement,
    Append,

    // Delimiters
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Semicolon,
    Comma,
    Dollar,
    Pipe,
    PipeBoth,
    Question,
    Colon,

    // Special
    Newline,
    Eof,
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Token::Number(n) => write!(f, "{n}"),
            Token::StringLit(s) => write!(f, "\"{s}\""),
            Token::Ident(s) => write!(f, "{s}"),
            _ => write!(f, "{self:?}"),
        }
    }
}

pub struct Lexer {
    input: Vec<char>,
    pos: usize,
    tokens: Vec<Token>,
    warned_escapes: std::collections::HashSet<char>,
}

impl Lexer {
    pub fn new(input: &str) -> Self {
        Lexer {
            input: input.chars().collect(),
            pos: 0,
            tokens: Vec::new(),
            warned_escapes: std::collections::HashSet::new(),
        }
    }

    fn peek(&self) -> Option<char> {
        self.input.get(self.pos).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let ch = self.input.get(self.pos).copied();
        if ch.is_some() {
            self.pos += 1;
        }
        ch
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        self.input.get(self.pos + offset).copied()
    }

    fn skip_whitespace(&mut self) {
        while let Some(ch) = self.peek() {
            if ch == ' ' || ch == '\t' || ch == '\r' {
                self.advance();
            } else if ch == '\\' && self.peek_at(1) == Some('\n') {
                self.advance();
                self.advance();
            } else if ch == '#' {
                while let Some(c) = self.peek() {
                    if c == '\n' {
                        break;
                    }
                    self.advance();
                }
            } else {
                break;
            }
        }
    }

    fn read_string(&mut self) -> String {
        let mut s = String::new();
        // skip opening quote
        self.advance();
        while let Some(ch) = self.advance() {
            if ch == '\\' {
                if let Some(esc) = self.advance() {
                    match esc {
                        'n' => s.push('\n'),
                        't' => s.push('\t'),
                        'r' => s.push('\r'),
                        '\\' => s.push('\\'),
                        '"' => s.push('"'),
                        'a' => s.push('\x07'),
                        'b' => s.push('\x08'),
                        'f' => s.push('\x0C'),
                        'v' => s.push('\x0B'),
                        '/' => s.push('/'),
                        _ => {
                            // Only warn for escapes that are truly unknown
                            // Skip digits, regex metachar escapes, and common chars
                            if !"0123456789[](){}|.^$*+?&-<>=#;:!~%"
                                .contains(esc)
                                && self.warned_escapes.insert(esc)
                            {
                                if esc == 'u' || esc == 'U' {
                                    eprintln!("awk: warning: no hex digits in `\\{esc}' escape sequence");
                                } else {
                                    eprintln!(
                                        "awk: warning: regexp escape sequence `\\{esc}' is not a known regexp operator"
                                    );
                                }
                            }
                            s.push('\\');
                            s.push(esc);
                        }
                    }
                }
            } else if ch == '"' {
                break;
            } else {
                s.push(ch);
            }
        }
        s
    }

    fn read_regex(&mut self) -> String {
        let mut s = String::new();
        // skip opening /
        self.advance();
        while let Some(ch) = self.advance() {
            if ch == '\\' {
                if let Some(next) = self.advance() {
                    if next == '/' {
                        s.push('/');
                    } else {
                        // Warn about unknown regex escapes at parse time (like gawk)
                        if !"dDwWsSbBtbnrfax0123456789.^$*+?()[]{}|\\/&-"
                            .contains(next)
                            && self.warned_escapes.insert(next)
                        {
                            if next == 'u' {
                                eprintln!("awk: warning: no hex digits in `\\u' escape sequence");
                            } else {
                                eprintln!(
                                    "awk: warning: regexp escape sequence `\\{next}' is not a known regexp operator"
                                );
                            }
                        }
                        s.push('\\');
                        s.push(next);
                    }
                }
            } else if ch == '/' {
                break;
            } else {
                s.push(ch);
            }
        }
        s
    }

    fn read_number(&mut self) -> f64 {
        let mut s = String::new();
        if self.peek() == Some('0')
            && (self.peek_at(1) == Some('x') || self.peek_at(1) == Some('X'))
            && self.peek_at(2).is_some_and(|c| c.is_ascii_hexdigit())
        {
            s.push(self.advance().unwrap()); // 0
            s.push(self.advance().unwrap()); // x
            while let Some(ch) = self.peek() {
                if ch.is_ascii_hexdigit() {
                    s.push(self.advance().unwrap());
                } else {
                    break;
                }
            }
            return i64::from_str_radix(&s[2..], 16).unwrap_or(0) as f64;
        }
        let mut has_dot = false;
        let mut has_exp = false;
        while let Some(ch) = self.peek() {
            if ch.is_ascii_digit() {
                s.push(self.advance().unwrap());
            } else if ch == '.' && !has_dot && !has_exp {
                has_dot = true;
                s.push(self.advance().unwrap());
            } else if (ch == 'e' || ch == 'E') && !has_exp
                && !s.is_empty()
                && s.chars().last().is_some_and(|c| c.is_ascii_digit())
            {
                // Only consume 'e'/'E' if followed by digits or sign+digits
                let next = self.peek_at(1);
                let has_exp_digits = if next == Some('+') || next == Some('-') {
                    self.peek_at(2).is_some_and(|c| c.is_ascii_digit())
                } else {
                    next.is_some_and(|c| c.is_ascii_digit())
                };
                if has_exp_digits {
                    has_exp = true;
                    s.push(self.advance().unwrap()); // e/E
                    if self.peek() == Some('+') || self.peek() == Some('-') {
                        s.push(self.advance().unwrap());
                    }
                } else {
                    break;
                }
            } else {
                break;
            }
        }
        s.parse().unwrap_or(0.0)
    }

    fn read_ident(&mut self) -> String {
        let mut s = String::new();
        while let Some(ch) = self.peek() {
            if ch.is_alphanumeric() || ch == '_' {
                s.push(self.advance().unwrap());
            } else {
                break;
            }
        }
        s
    }

    fn can_be_regex(&self) -> bool {
        // A '/' is a regex if the previous token is one that can precede a regex
        if let Some(last) = self.tokens.last() {
            matches!(
                last,
                Token::Newline
                    | Token::Semicolon
                    | Token::LBrace
                    | Token::RBrace
                    | Token::LParen
                    | Token::Comma
                    | Token::Not
                    | Token::And
                    | Token::Or
                    | Token::Match
                    | Token::NotMatch
                    | Token::Print
                    | Token::Printf
                    | Token::Return
                    | Token::Pipe
                    | Token::Assign
                    | Token::PlusAssign
                    | Token::MinusAssign
                    | Token::StarAssign
                    | Token::SlashAssign
                    | Token::PercentAssign
                    | Token::CaretAssign
                    | Token::Colon
                    | Token::Question
            )
        } else {
            true // beginning of input
        }
    }

    pub fn tokenize(&mut self) -> Vec<Token> {
        loop {
            self.skip_whitespace();
            let ch = match self.peek() {
                Some(c) => c,
                None => {
                    self.tokens.push(Token::Eof);
                    break;
                }
            };

            let tok = match ch {
                '\n' => {
                    self.advance();
                    Token::Newline
                }
                '"' => Token::StringLit(self.read_string()),
                '/' if self.can_be_regex() => Token::Regex(self.read_regex()),
                '0'..='9' | '.'
                    if ch.is_ascii_digit() || {
                        ch == '.' && self.peek_at(1).is_some_and(|c| c.is_ascii_digit())
                    } =>
                {
                    Token::Number(self.read_number())
                }
                'a'..='z' | 'A'..='Z' | '_' => {
                    let ident = self.read_ident();
                    match ident.as_str() {
                        "BEGIN" => Token::Begin,
                        "END" => Token::End,
                        "if" => Token::If,
                        "else" => Token::Else,
                        "while" => Token::While,
                        "for" => Token::For,
                        "do" => Token::Do,
                        "in" => Token::In,
                        "delete" => Token::Delete,
                        "print" => Token::Print,
                        "printf" => Token::Printf,
                        "getline" => Token::Getline,
                        "next" => Token::Next,
                        "exit" => Token::Exit,
                        "function" => Token::Function,
                        "return" => Token::Return,
                        "break" => Token::Break,
                        "continue" => Token::Continue,
                        _ => Token::Ident(ident),
                    }
                }
                '+' => {
                    self.advance();
                    if self.peek() == Some('+') {
                        self.advance();
                        Token::Increment
                    } else if self.peek() == Some('=') {
                        self.advance();
                        Token::PlusAssign
                    } else {
                        Token::Plus
                    }
                }
                '-' => {
                    self.advance();
                    if self.peek() == Some('-') {
                        self.advance();
                        Token::Decrement
                    } else if self.peek() == Some('=') {
                        self.advance();
                        Token::MinusAssign
                    } else {
                        Token::Minus
                    }
                }
                '*' => {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        Token::StarAssign
                    } else {
                        Token::Star
                    }
                }
                '/' => {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        Token::SlashAssign
                    } else {
                        Token::Slash
                    }
                }
                '%' => {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        Token::PercentAssign
                    } else {
                        Token::Percent
                    }
                }
                '^' => {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        Token::CaretAssign
                    } else {
                        Token::Caret
                    }
                }
                '=' => {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        Token::Eq
                    } else {
                        Token::Assign
                    }
                }
                '!' => {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        Token::Ne
                    } else if self.peek() == Some('~') {
                        self.advance();
                        Token::NotMatch
                    } else {
                        Token::Not
                    }
                }
                '<' => {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        Token::Le
                    } else {
                        Token::Lt
                    }
                }
                '>' => {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        Token::Ge
                    } else if self.peek() == Some('>') {
                        self.advance();
                        Token::Append
                    } else {
                        Token::Gt
                    }
                }
                '&' => {
                    self.advance();
                    if self.peek() == Some('&') {
                        self.advance();
                        Token::And
                    } else {
                        // lone & not typically used in awk but handle gracefully
                        Token::Ident("&".to_string())
                    }
                }
                '|' => {
                    self.advance();
                    if self.peek() == Some('|') {
                        self.advance();
                        Token::Or
                    } else if self.peek() == Some('&') {
                        self.advance();
                        Token::PipeBoth
                    } else {
                        Token::Pipe
                    }
                }
                '~' => {
                    self.advance();
                    Token::Match
                }
                '(' => {
                    self.advance();
                    Token::LParen
                }
                ')' => {
                    self.advance();
                    Token::RParen
                }
                '{' => {
                    self.advance();
                    Token::LBrace
                }
                '}' => {
                    self.advance();
                    Token::RBrace
                }
                '[' => {
                    self.advance();
                    Token::LBracket
                }
                ']' => {
                    self.advance();
                    Token::RBracket
                }
                ';' => {
                    self.advance();
                    Token::Semicolon
                }
                ',' => {
                    self.advance();
                    Token::Comma
                }
                '$' => {
                    self.advance();
                    Token::Dollar
                }
                '?' => {
                    self.advance();
                    Token::Question
                }
                ':' => {
                    self.advance();
                    Token::Colon
                }
                _ => {
                    self.advance();
                    continue;
                }
            };

            self.tokens.push(tok);
        }

        self.tokens.clone()
    }
}
