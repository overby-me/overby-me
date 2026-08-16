//! The parser's cursor over the token stream.

use crate::lexer::{Position, Token};
use crate::{ParseError, Parser};

impl Parser {
    // ------------------------------------------------------------- utilities

    pub(crate) fn peek(&self) -> &Token {
        &self.tokens[self.index].token
    }

    pub(crate) fn peek_at(&self, ahead: usize) -> &Token {
        let index = (self.index + ahead).min(self.tokens.len() - 1);
        &self.tokens[index].token
    }

    pub(crate) fn position(&self) -> Position {
        self.tokens[self.index].position
    }

    pub(crate) fn advance(&mut self) -> Token {
        let token = self.tokens[self.index].token.clone();
        if self.index + 1 < self.tokens.len() {
            self.index += 1;
        }
        token
    }

    pub(crate) fn error<T>(&self, message: impl Into<String>) -> Result<T, ParseError> {
        Err(ParseError {
            position: self.position(),
            message: message.into(),
        })
    }

    pub(crate) fn require(&mut self, token: Token) -> Result<(), ParseError> {
        if self.peek() == &token {
            self.advance();
            Ok(())
        } else {
            self.error(format!(
                "expected {}, found {}",
                token.describe(),
                self.peek().describe()
            ))
        }
    }

    pub(crate) fn eat(&mut self, token: &Token) -> bool {
        if self.peek() == token {
            self.advance();
            true
        } else {
            false
        }
    }

    /// Consumes a bare word if it is the one asked for.
    pub(crate) fn eat_word(&mut self, word: &str) -> bool {
        if matches!(self.peek(), Token::Word(found) if found == word) {
            self.advance();
            true
        } else {
            false
        }
    }

    pub(crate) fn peek_word(&self) -> Option<&str> {
        match self.peek() {
            Token::Word(word) => Some(word),
            _ => None,
        }
    }

    pub(crate) fn require_word(&mut self) -> Result<String, ParseError> {
        match self.advance() {
            Token::Word(word) => Ok(word),
            other => {
                self.index -= 1;
                self.error(format!("expected a keyword, found {}", other.describe()))
            }
        }
    }

    pub(crate) fn require_unsigned(&mut self) -> Result<u64, ParseError> {
        match self.advance() {
            Token::Integer {
                negative: false,
                digits,
            } => digits.parse().map_err(|_| {
                self.error::<()>(format!("{digits} does not fit"))
                    .unwrap_err()
            }),
            // `[u0xedcba x i8]` writes a length the way a wide integer
            // literal is written, which upstream reads here too.
            Token::Word(word) if crate::attributes::wide_hex_u64(&word).is_some() => {
                Ok(crate::attributes::wide_hex_u64(&word).unwrap_or(0))
            }
            other => {
                self.index -= 1;
                self.error(format!("expected a number, found {}", other.describe()))
            }
        }
    }

    pub(crate) fn require_signed(&mut self) -> Result<i64, ParseError> {
        match self.advance() {
            Token::Integer { negative, digits } => {
                let magnitude: i64 = digits.parse().map_err(|_| {
                    self.error::<()>(format!("{digits} does not fit"))
                        .unwrap_err()
                })?;
                Ok(if negative { -magnitude } else { magnitude })
            }
            other => {
                self.index -= 1;
                self.error(format!("expected a number, found {}", other.describe()))
            }
        }
    }

    /// A quoted string that has to be text: a section name, a triple, an
    /// attribute key. Bytes outside UTF-8 are an error here rather than
    /// silently mangled, and metadata takes the bytes instead.
    pub(crate) fn require_quoted(&mut self) -> Result<String, ParseError> {
        let bytes = self.require_quoted_bytes()?;
        match String::from_utf8(bytes) {
            Ok(text) => Ok(text),
            Err(_) => self.error("this string has to be valid UTF-8"),
        }
    }

    /// A quoted string as written, for the places LLVM allows any byte.
    pub(crate) fn require_quoted_bytes(&mut self) -> Result<Vec<u8>, ParseError> {
        match self.advance() {
            Token::Quoted(text) => Ok(text),
            other => {
                self.index -= 1;
                self.error(format!("expected a string, found {}", other.describe()))
            }
        }
    }
}
