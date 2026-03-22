/// Lexer for the Meson build DSL.
/// Tokenizes meson.build files into a stream of tokens.

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // Literals
    StringLiteral(String),
    MultilineStringLiteral(String),
    FStringLiteral(String),
    IntLiteral(i64),
    Identifier(String),

    // Keywords
    True,
    False,
    If,
    Elif,
    Else,
    Endif,
    And,
    Or,
    Not,
    Foreach,
    Endforeach,
    In,
    Continue,
    Break,

    // Operators
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Assign,
    PlusAssign,
    Eq,
    Neq,
    Lt,
    Gt,
    Le,
    Ge,
    Dot,
    Comma,
    Colon,
    Question,

    // Delimiters
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,

    // Special
    Newline,
    Eof,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub line: usize,
    pub col: usize,
}

pub struct Lexer {
    source: Vec<char>,
    pos: usize,
    line: usize,
    col: usize,
}

impl Lexer {
    pub fn new(source: &str) -> Self {
        Self {
            source: source.chars().collect(),
            pos: 0,
            line: 1,
            col: 1,
        }
    }

    pub fn tokenize(&mut self) -> Result<Vec<Token>, String> {
        let mut tokens = Vec::new();
        loop {
            let tok = self.next_token()?;
            let is_eof = tok.kind == TokenKind::Eof;
            tokens.push(tok);
            if is_eof {
                break;
            }
        }
        Ok(tokens)
    }

    fn peek(&self) -> Option<char> {
        self.source.get(self.pos).copied()
    }

    fn peek_ahead(&self, n: usize) -> Option<char> {
        self.source.get(self.pos + n).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let c = self.source.get(self.pos).copied()?;
        self.pos += 1;
        if c == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(c)
    }

    fn skip_whitespace_and_comments(&mut self) {
        loop {
            match self.peek() {
                Some(' ') | Some('\t') | Some('\r') => {
                    self.advance();
                }
                Some('#') => {
                    while let Some(c) = self.peek() {
                        if c == '\n' {
                            break;
                        }
                        self.advance();
                    }
                }
                _ => break,
            }
        }
    }

    fn next_token(&mut self) -> Result<Token, String> {
        self.skip_whitespace_and_comments();
        let line = self.line;
        let col = self.col;

        let c = match self.peek() {
            None => {
                return Ok(Token {
                    kind: TokenKind::Eof,
                    line,
                    col,
                });
            }
            Some(c) => c,
        };

        // Line continuation
        if c == '\\' && self.peek_ahead(1) == Some('\n') {
            self.advance(); // backslash
            self.advance(); // newline
            return self.next_token();
        }

        match c {
            '\n' => {
                self.advance();
                Ok(Token {
                    kind: TokenKind::Newline,
                    line,
                    col,
                })
            }
            '+' => {
                self.advance();
                if self.peek() == Some('=') {
                    self.advance();
                    Ok(Token {
                        kind: TokenKind::PlusAssign,
                        line,
                        col,
                    })
                } else {
                    Ok(Token {
                        kind: TokenKind::Plus,
                        line,
                        col,
                    })
                }
            }
            '-' => {
                self.advance();
                Ok(Token {
                    kind: TokenKind::Minus,
                    line,
                    col,
                })
            }
            '*' => {
                self.advance();
                Ok(Token {
                    kind: TokenKind::Star,
                    line,
                    col,
                })
            }
            '/' => {
                self.advance();
                Ok(Token {
                    kind: TokenKind::Slash,
                    line,
                    col,
                })
            }
            '%' => {
                self.advance();
                Ok(Token {
                    kind: TokenKind::Percent,
                    line,
                    col,
                })
            }
            '=' => {
                self.advance();
                if self.peek() == Some('=') {
                    self.advance();
                    Ok(Token {
                        kind: TokenKind::Eq,
                        line,
                        col,
                    })
                } else {
                    Ok(Token {
                        kind: TokenKind::Assign,
                        line,
                        col,
                    })
                }
            }
            '!' => {
                self.advance();
                if self.peek() == Some('=') {
                    self.advance();
                    Ok(Token {
                        kind: TokenKind::Neq,
                        line,
                        col,
                    })
                } else {
                    Err(format!("{}:{}: Unexpected character '!'", line, col))
                }
            }
            '<' => {
                self.advance();
                if self.peek() == Some('=') {
                    self.advance();
                    Ok(Token {
                        kind: TokenKind::Le,
                        line,
                        col,
                    })
                } else {
                    Ok(Token {
                        kind: TokenKind::Lt,
                        line,
                        col,
                    })
                }
            }
            '>' => {
                self.advance();
                if self.peek() == Some('=') {
                    self.advance();
                    Ok(Token {
                        kind: TokenKind::Ge,
                        line,
                        col,
                    })
                } else {
                    Ok(Token {
                        kind: TokenKind::Gt,
                        line,
                        col,
                    })
                }
            }
            '.' => {
                self.advance();
                Ok(Token {
                    kind: TokenKind::Dot,
                    line,
                    col,
                })
            }
            ',' => {
                self.advance();
                Ok(Token {
                    kind: TokenKind::Comma,
                    line,
                    col,
                })
            }
            ':' => {
                self.advance();
                Ok(Token {
                    kind: TokenKind::Colon,
                    line,
                    col,
                })
            }
            '?' => {
                self.advance();
                Ok(Token {
                    kind: TokenKind::Question,
                    line,
                    col,
                })
            }
            '(' => {
                self.advance();
                Ok(Token {
                    kind: TokenKind::LParen,
                    line,
                    col,
                })
            }
            ')' => {
                self.advance();
                Ok(Token {
                    kind: TokenKind::RParen,
                    line,
                    col,
                })
            }
            '[' => {
                self.advance();
                Ok(Token {
                    kind: TokenKind::LBracket,
                    line,
                    col,
                })
            }
            ']' => {
                self.advance();
                Ok(Token {
                    kind: TokenKind::RBracket,
                    line,
                    col,
                })
            }
            '{' => {
                self.advance();
                Ok(Token {
                    kind: TokenKind::LBrace,
                    line,
                    col,
                })
            }
            '}' => {
                self.advance();
                Ok(Token {
                    kind: TokenKind::RBrace,
                    line,
                    col,
                })
            }
            '\'' => self.lex_string(line, col),
            'f' if self.peek_ahead(1) == Some('\'') => self.lex_fstring(line, col),
            '0'..='9' => self.lex_number(line, col),
            'a'..='z' | 'A'..='Z' | '_' => self.lex_identifier(line, col),
            _ => {
                self.advance();
                Err(format!("{}:{}: Unexpected character '{}'", line, col, c))
            }
        }
    }

    fn lex_string(&mut self, line: usize, col: usize) -> Result<Token, String> {
        self.advance(); // opening quote
        // Check for multiline string '''
        if self.peek() == Some('\'') && self.peek_ahead(1) == Some('\'') {
            self.advance();
            self.advance();
            return self.lex_multiline_string(line, col);
        }
        let mut s = String::new();
        loop {
            match self.peek() {
                None | Some('\n') => {
                    return Err(format!("{}:{}: Unterminated string literal", line, col));
                }
                Some('\'') => {
                    self.advance();
                    break;
                }
                Some('\\') => {
                    self.advance();
                    match self.advance() {
                        Some('\\') => s.push('\\'),
                        Some('\'') => s.push('\''),
                        Some('n') => s.push('\n'),
                        Some('t') => s.push('\t'),
                        Some('r') => s.push('\r'),
                        Some('a') => s.push('\x07'),
                        Some('b') => s.push('\x08'),
                        Some('f') => s.push('\x0C'),
                        Some('0') => s.push('\0'),
                        Some('x') => {
                            let hex = self.read_hex(2)?;
                            s.push(char::from_u32(hex).unwrap_or('\u{FFFD}'));
                        }
                        Some('u') => {
                            let hex = self.read_hex(4)?;
                            s.push(char::from_u32(hex).unwrap_or('\u{FFFD}'));
                        }
                        Some('U') => {
                            let hex = self.read_hex(8)?;
                            s.push(char::from_u32(hex).unwrap_or('\u{FFFD}'));
                        }
                        Some(c) => {
                            s.push('\\');
                            s.push(c);
                        }
                        None => {
                            return Err(format!("{}:{}: Unterminated escape in string", line, col));
                        }
                    }
                }
                Some(c) => {
                    self.advance();
                    s.push(c);
                }
            }
        }
        Ok(Token {
            kind: TokenKind::StringLiteral(s),
            line,
            col,
        })
    }

    fn lex_multiline_string(&mut self, line: usize, col: usize) -> Result<Token, String> {
        let mut s = String::new();
        loop {
            match self.peek() {
                None => {
                    return Err(format!("{}:{}: Unterminated multiline string", line, col));
                }
                Some('\'')
                    if self.peek_ahead(1) == Some('\'') && self.peek_ahead(2) == Some('\'') =>
                {
                    self.advance();
                    self.advance();
                    self.advance();
                    break;
                }
                Some(c) => {
                    self.advance();
                    s.push(c);
                }
            }
        }
        Ok(Token {
            kind: TokenKind::MultilineStringLiteral(s),
            line,
            col,
        })
    }

    fn lex_fstring(&mut self, line: usize, col: usize) -> Result<Token, String> {
        self.advance(); // 'f'
        self.advance(); // opening quote
        let mut s = String::new();
        loop {
            match self.peek() {
                None | Some('\n') => {
                    return Err(format!("{}:{}: Unterminated f-string", line, col));
                }
                Some('\'') => {
                    self.advance();
                    break;
                }
                Some('\\') => {
                    self.advance();
                    match self.advance() {
                        Some(c) => {
                            s.push('\\');
                            s.push(c);
                        }
                        None => {
                            return Err(format!(
                                "{}:{}: Unterminated escape in f-string",
                                line, col
                            ));
                        }
                    }
                }
                Some(c) => {
                    self.advance();
                    s.push(c);
                }
            }
        }
        Ok(Token {
            kind: TokenKind::FStringLiteral(s),
            line,
            col,
        })
    }

    fn lex_number(&mut self, line: usize, col: usize) -> Result<Token, String> {
        let mut s = String::new();
        // Handle 0x, 0o, 0b prefixes
        if self.peek() == Some('0') {
            s.push(self.advance().unwrap());
            match self.peek() {
                Some('x') | Some('X') => {
                    s.push(self.advance().unwrap());
                    while let Some(c) = self.peek() {
                        if c.is_ascii_hexdigit() || c == '_' {
                            s.push(self.advance().unwrap());
                        } else {
                            break;
                        }
                    }
                    let clean: String = s[2..].chars().filter(|c| *c != '_').collect();
                    let val = i64::from_str_radix(&clean, 16)
                        .map_err(|e| format!("{}:{}: Invalid hex literal: {}", line, col, e))?;
                    return Ok(Token {
                        kind: TokenKind::IntLiteral(val),
                        line,
                        col,
                    });
                }
                Some('o') | Some('O') => {
                    s.push(self.advance().unwrap());
                    while let Some(c) = self.peek() {
                        if ('0'..='7').contains(&c) || c == '_' {
                            s.push(self.advance().unwrap());
                        } else {
                            break;
                        }
                    }
                    let clean: String = s[2..].chars().filter(|c| *c != '_').collect();
                    let val = i64::from_str_radix(&clean, 8)
                        .map_err(|e| format!("{}:{}: Invalid octal literal: {}", line, col, e))?;
                    return Ok(Token {
                        kind: TokenKind::IntLiteral(val),
                        line,
                        col,
                    });
                }
                Some('b') | Some('B') => {
                    s.push(self.advance().unwrap());
                    while let Some(c) = self.peek() {
                        if c == '0' || c == '1' || c == '_' {
                            s.push(self.advance().unwrap());
                        } else {
                            break;
                        }
                    }
                    let clean: String = s[2..].chars().filter(|c| *c != '_').collect();
                    let val = i64::from_str_radix(&clean, 2)
                        .map_err(|e| format!("{}:{}: Invalid binary literal: {}", line, col, e))?;
                    return Ok(Token {
                        kind: TokenKind::IntLiteral(val),
                        line,
                        col,
                    });
                }
                _ => {}
            }
        }
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() || c == '_' {
                s.push(self.advance().unwrap());
            } else {
                break;
            }
        }
        let clean: String = s.chars().filter(|c| *c != '_').collect();
        let val: i64 = clean
            .parse()
            .map_err(|e| format!("{}:{}: Invalid integer literal: {}", line, col, e))?;
        Ok(Token {
            kind: TokenKind::IntLiteral(val),
            line,
            col,
        })
    }

    fn lex_identifier(&mut self, line: usize, col: usize) -> Result<Token, String> {
        let mut s = String::new();
        while let Some(c) = self.peek() {
            if c.is_ascii_alphanumeric() || c == '_' {
                s.push(self.advance().unwrap());
            } else {
                break;
            }
        }
        let kind = match s.as_str() {
            "true" => TokenKind::True,
            "false" => TokenKind::False,
            "if" => TokenKind::If,
            "elif" => TokenKind::Elif,
            "else" => TokenKind::Else,
            "endif" => TokenKind::Endif,
            "and" => TokenKind::And,
            "or" => TokenKind::Or,
            "not" => TokenKind::Not,
            "foreach" => TokenKind::Foreach,
            "endforeach" => TokenKind::Endforeach,
            "in" => TokenKind::In,
            "continue" => TokenKind::Continue,
            "break" => TokenKind::Break,
            _ => TokenKind::Identifier(s),
        };
        Ok(Token { kind, line, col })
    }

    fn read_hex(&mut self, count: usize) -> Result<u32, String> {
        let mut s = String::new();
        for _ in 0..count {
            match self.advance() {
                Some(c) if c.is_ascii_hexdigit() => s.push(c),
                Some(c) => return Err(format!("Invalid hex digit: '{}'", c)),
                None => return Err("Unexpected end of hex escape".to_string()),
            }
        }
        u32::from_str_radix(&s, 16).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_tokens() {
        let mut lexer = Lexer::new("project('hello', 'c')");
        let tokens = lexer.tokenize().unwrap();
        assert!(matches!(tokens[0].kind, TokenKind::Identifier(ref s) if s == "project"));
        assert!(matches!(tokens[1].kind, TokenKind::LParen));
        assert!(matches!(tokens[2].kind, TokenKind::StringLiteral(ref s) if s == "hello"));
    }

    #[test]
    fn test_numbers() {
        let mut lexer = Lexer::new("42 0xff 0o77 0b1010");
        let tokens = lexer.tokenize().unwrap();
        assert!(matches!(tokens[0].kind, TokenKind::IntLiteral(42)));
        assert!(matches!(tokens[1].kind, TokenKind::IntLiteral(255)));
        assert!(matches!(tokens[2].kind, TokenKind::IntLiteral(63)));
        assert!(matches!(tokens[3].kind, TokenKind::IntLiteral(10)));
    }
}
