//! The lexer.
//!
//! Textual IR is easy to tokenise and full of small traps. `i32` is one token
//! and not an identifier followed by a number. `c"..."` is a byte string but
//! `"..."` is a name. A `!` can start a metadata reference, a metadata string
//! or a bare tuple. A label is an identifier that happens to be followed by a
//! colon, which is only knowable after reading it. Each of those is handled
//! here so the parser can stay a plain recursive descent.

use std::fmt;

/// Where a token starts, for error messages.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Position {
    pub line: u32,
    pub column: u32,
}

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.column)
    }
}

/// One token.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Token {
    /// `%name`, unquoted and unescaped.
    LocalName(String),
    /// `%3`.
    LocalNumber(u32),
    /// `@name`.
    GlobalName(String),
    /// `@3`.
    GlobalNumber(u32),
    /// `$name`.
    ComdatName(String),
    /// `!name`, including specialized node tags such as `DILocation`.
    MetadataName(Vec<u8>),
    /// `!3`.
    MetadataNumber(u32),
    /// `^3`, a reference into the ThinLTO summary index.
    SummaryNumber(u32),
    /// A colon standing on its own, which only the summary index writes.
    Colon,
    /// `!"text"`.
    MetadataString(Vec<u8>),
    /// A lone `!`, which starts an inline tuple.
    Exclaim,
    /// `#3`.
    AttributeGroup(u32),
    /// `#dbg_value` and its siblings.
    DebugRecord(String),
    /// `name:` at the start of a block.
    Label(String),
    /// `3:`.
    LabelNumber(u32),
    /// A bare word: a keyword, an opcode, or an unquoted identifier.
    Word(String),
    /// `i1`, `i32`, `i8388608`.
    IntType(u32),
    /// A decimal integer literal, with its sign, kept as text so an arbitrary
    /// width can be built once the type is known.
    Integer {
        negative: bool,
        digits: String,
    },
    /// A decimal or exponential floating-point literal.
    Float(String),
    /// `0x...`, with the form letter when there is one.
    HexFloat {
        form: Option<char>,
        digits: String,
    },
    /// `"text"`, unescaped.
    /// `"..."`, as bytes: LLVM's strings are byte strings and a debug-info
    /// path need not be UTF-8.
    Quoted(Vec<u8>),
    /// `c"bytes"`, unescaped.
    ByteString(Vec<u8>),
    Comma,
    Equals,
    Star,
    LeftParen,
    RightParen,
    LeftBracket,
    RightBracket,
    LeftBrace,
    RightBrace,
    Less,
    Greater,
    /// `|`, which joins debug-info flag names.
    Pipe,
    /// `...`, the variadic marker.
    Ellipsis,
    Eof,
}

impl Token {
    /// A short description for error messages.
    pub fn describe(&self) -> String {
        match self {
            Token::LocalName(name) => format!("%{name}"),
            Token::LocalNumber(n) => format!("%{n}"),
            Token::GlobalName(name) => format!("@{name}"),
            Token::GlobalNumber(n) => format!("@{n}"),
            Token::ComdatName(name) => format!("${name}"),
            Token::MetadataName(name) => format!("!{}", String::from_utf8_lossy(name)),
            Token::MetadataNumber(n) => format!("!{n}"),
            Token::SummaryNumber(n) => format!("^{n}"),
            Token::Colon => ":".to_string(),
            Token::MetadataString(_) => "a metadata string".to_string(),
            Token::Exclaim => "!".to_string(),
            Token::AttributeGroup(n) => format!("#{n}"),
            Token::DebugRecord(name) => format!("#{name}"),
            Token::Label(name) => format!("label {name}:"),
            Token::LabelNumber(n) => format!("label {n}:"),
            Token::Word(word) => format!("'{word}'"),
            Token::IntType(bits) => format!("i{bits}"),
            Token::Integer { negative, digits } => {
                format!("{}{digits}", if *negative { "-" } else { "" })
            }
            Token::Float(text) => text.clone(),
            Token::HexFloat { form, digits } => match form {
                Some(form) => format!("0x{form}{digits}"),
                None => format!("0x{digits}"),
            },
            Token::Quoted(_) => "a quoted string".to_string(),
            Token::ByteString(_) => "a byte string".to_string(),
            Token::Comma => "','".to_string(),
            Token::Equals => "'='".to_string(),
            Token::Star => "'*'".to_string(),
            Token::LeftParen => "'('".to_string(),
            Token::RightParen => "')'".to_string(),
            Token::LeftBracket => "'['".to_string(),
            Token::RightBracket => "']'".to_string(),
            Token::LeftBrace => "'{'".to_string(),
            Token::RightBrace => "'}'".to_string(),
            Token::Less => "'<'".to_string(),
            Token::Greater => "'>'".to_string(),
            Token::Pipe => "'|'".to_string(),
            Token::Ellipsis => "'...'".to_string(),
            Token::Eof => "end of file".to_string(),
        }
    }
}

/// A token with the place it came from.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Spanned {
    pub token: Token,
    pub position: Position,
}

/// Why the input could not be turned into tokens.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LexError {
    pub position: Position,
    pub message: String,
}

impl fmt::Display for LexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.position, self.message)
    }
}

pub struct Lexer<'a> {
    /// The input, kept as text so that a token's bytes can be sliced straight
    /// out of it. Every token boundary is at an ASCII character, so the slice
    /// is always on a character boundary.
    text: &'a str,
    bytes: &'a [u8],
    offset: usize,
    line: u32,
    line_start: usize,
    /// Set once the `; ModuleID = '...'` comment has been consumed, so the
    /// same comment appearing later is treated as an ordinary comment.
    pub module_id: Option<String>,
}

impl<'a> Lexer<'a> {
    pub fn new(text: &'a str) -> Lexer<'a> {
        Lexer {
            text,
            bytes: text.as_bytes(),
            offset: 0,
            line: 1,
            line_start: 0,
            module_id: None,
        }
    }

    fn position(&self) -> Position {
        Position {
            line: self.line,
            column: (self.offset - self.line_start + 1) as u32,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.offset).copied()
    }

    fn peek_at(&self, ahead: usize) -> Option<u8> {
        self.bytes.get(self.offset + ahead).copied()
    }

    /// Moves back to an earlier offset. Only ever used inside a run of name
    /// bytes, which hold no newline, so the line count needs no undoing.
    fn rewind_to(&mut self, offset: usize) {
        debug_assert!(offset <= self.offset);
        self.offset = offset;
    }

    fn bump(&mut self) -> Option<u8> {
        let byte = self.peek()?;
        self.offset += 1;
        if byte == b'\n' {
            self.line += 1;
            self.line_start = self.offset;
        }
        Some(byte)
    }

    fn error(&self, message: impl Into<String>) -> LexError {
        LexError {
            position: self.position(),
            message: message.into(),
        }
    }

    /// Reads every token in the input.
    pub fn tokenize(mut self) -> Result<(Vec<Spanned>, Option<String>), LexError> {
        let mut tokens = Vec::new();
        loop {
            let spanned = self.next_token()?;
            let is_eof = spanned.token == Token::Eof;
            tokens.push(spanned);
            if is_eof {
                break;
            }
        }
        Ok((tokens, self.module_id))
    }

    fn skip_trivia(&mut self) -> Result<(), LexError> {
        loop {
            match self.peek() {
                Some(b' ' | b'\t' | b'\r' | b'\n') => {
                    self.bump();
                }
                // `/* ... */`, which upstream reads and prints nothing back
                // for. It nests no further than one level, and an unclosed
                // one runs to the end of the file, as upstream's does.
                Some(b'/') if self.peek_at(1) == Some(b'*') => {
                    self.bump();
                    self.bump();
                    loop {
                        let Some(byte) = self.bump() else {
                            return Err(self.error("unterminated comment"));
                        };
                        if byte == b'*' && self.peek() == Some(b'/') {
                            self.bump();
                            break;
                        }
                    }
                }
                Some(b';') => {
                    let comment_start = self.offset;
                    let at_line_start = self.offset == self.line_start;
                    while let Some(byte) = self.peek() {
                        if byte == b'\n' {
                            break;
                        }
                        self.bump();
                    }
                    if at_line_start && self.module_id.is_none() {
                        let text = &self.text[comment_start..self.offset];
                        if let Some(rest) = text.strip_prefix("; ModuleID = '")
                            && let Some(id) = rest.strip_suffix('\'')
                        {
                            self.module_id = Some(id.to_string());
                        }
                    }
                }
                _ => return Ok(()),
            }
        }
    }

    fn next_token(&mut self) -> Result<Spanned, LexError> {
        self.skip_trivia()?;
        let position = self.position();
        let Some(byte) = self.peek() else {
            return Ok(Spanned {
                token: Token::Eof,
                position,
            });
        };
        let token = match byte {
            b'%' => {
                self.bump();
                self.prefixed_name(position, Token::LocalName, Token::LocalNumber)?
            }
            b'@' => {
                self.bump();
                self.prefixed_name(position, Token::GlobalName, Token::GlobalNumber)?
            }
            b'$' => {
                self.bump();
                self.prefixed_name(position, Token::ComdatName, |_| {
                    Token::ComdatName(String::new())
                })?
            }
            b'!' => {
                self.bump();
                match self.peek() {
                    Some(b'"') => Token::MetadataString(self.quoted_bytes()?),
                    Some(b) if b.is_ascii_digit() => Token::MetadataNumber(self.number()?),
                    Some(b) if is_name_byte(b) || b == b'\\' => {
                        Token::MetadataName(self.escaped_name()?)
                    }
                    _ => Token::Exclaim,
                }
            }
            // A colon is part of a label or a keyed field almost everywhere,
            // and the summary index is the one place it stands alone.
            b':' => {
                self.bump();
                Token::Colon
            }
            b'^' => {
                self.bump();
                match self.peek() {
                    Some(b) if b.is_ascii_digit() => Token::SummaryNumber(self.number()?),
                    _ => return Err(self.error("expected a summary number after '^'")),
                }
            }
            b'#' => {
                self.bump();
                match self.peek() {
                    Some(b) if b.is_ascii_digit() => Token::AttributeGroup(self.number()?),
                    Some(b) if is_name_byte(b) => Token::DebugRecord(self.bare_name()),
                    _ => return Err(self.error("expected an attribute group number after '#'")),
                }
            }
            b'"' => {
                let bytes = self.quoted_bytes()?;
                if self.peek() == Some(b':') {
                    self.bump();
                    let text = String::from_utf8(bytes)
                        .map_err(|_| self.error("a label has to be valid UTF-8"))?;
                    if text.contains('\0') {
                        return Err(self.error("NUL character is not allowed in names"));
                    }
                    Token::Label(text)
                } else {
                    Token::Quoted(bytes)
                }
            }
            b'c' if self.peek_at(1) == Some(b'"') => {
                self.bump();
                Token::ByteString(self.quoted_bytes()?)
            }
            b',' => {
                self.bump();
                Token::Comma
            }
            b'=' => {
                self.bump();
                Token::Equals
            }
            b'*' => {
                self.bump();
                Token::Star
            }
            b'(' => {
                self.bump();
                Token::LeftParen
            }
            b')' => {
                self.bump();
                Token::RightParen
            }
            b'[' => {
                self.bump();
                Token::LeftBracket
            }
            b']' => {
                self.bump();
                Token::RightBracket
            }
            b'{' => {
                self.bump();
                Token::LeftBrace
            }
            b'}' => {
                self.bump();
                Token::RightBrace
            }
            b'<' => {
                self.bump();
                Token::Less
            }
            b'>' => {
                self.bump();
                Token::Greater
            }
            b'|' => {
                self.bump();
                Token::Pipe
            }
            b'.' if self.peek_at(1) == Some(b'.') && self.peek_at(2) == Some(b'.') => {
                self.bump();
                self.bump();
                self.bump();
                Token::Ellipsis
            }
            b'-' | b'0'..=b'9' | b'+' => self.numeric()?,
            b if is_name_start(b) => self.word_or_label()?,
            other => {
                return Err(self.error(format!("unexpected character '{}'", other as char)));
            }
        };
        Ok(Spanned { token, position })
    }

    /// The body of a `%`, `@` or `$` token: a number, a bare name, or a
    /// quoted name.
    fn prefixed_name(
        &mut self,
        position: Position,
        named: impl Fn(String) -> Token,
        numbered: impl Fn(u32) -> Token,
    ) -> Result<Token, LexError> {
        match self.peek() {
            Some(b'"') => {
                let name = self.quoted_string()?;
                // A symbol's name reaches the object file, where it ends at
                // the first NUL, so a name holding one names something else.
                if name.contains('\0') {
                    return Err(self.error("NUL character is not allowed in names"));
                }
                Ok(named(name))
            }
            Some(b) if b.is_ascii_digit() => {
                let value = self.number()?;
                if self.peek() == Some(b':') {
                    self.bump();
                    return Ok(Token::LabelNumber(value));
                }
                Ok(numbered(value))
            }
            Some(b) if is_name_byte(b) || b == b'-' => Ok(named(self.bare_name())),
            _ => Err(LexError {
                position,
                message: "expected a name after the sigil".to_string(),
            }),
        }
    }

    /// A name after a sigil, which may hold a hyphen where a bare word may
    /// not: `%-3` is a block upstream reads and `i16-1` is a type and a
    /// negative number rather than one word.
    fn bare_name(&mut self) -> String {
        let start = self.offset;
        while let Some(byte) = self.peek() {
            if is_name_byte(byte) || byte == b'-' {
                self.bump();
            } else {
                break;
            }
        }
        self.text[start..self.offset].to_string()
    }

    /// A metadata name, which may spell a byte the bare grammar has no room
    /// for as `\\23`. Upstream escapes rather than quotes here, so
    /// `!\\23pragma` is the named metadata `#pragma`.
    fn escaped_name(&mut self) -> Result<Vec<u8>, LexError> {
        let mut bytes = Vec::new();
        while let Some(byte) = self.peek() {
            if is_name_byte(byte) || byte == b'-' {
                bytes.push(byte);
                self.bump();
            } else if byte == b'\\' {
                // `\x` is not an escape, so upstream keeps the backslash and
                // prints it back as `\5C`.
                match (self.hex_at(1), self.hex_at(2)) {
                    (Some(high), Some(low)) => {
                        self.bump();
                        self.bump();
                        self.bump();
                        bytes.push(high * 16 + low);
                    }
                    _ => {
                        self.bump();
                        bytes.push(b'\\');
                    }
                }
            } else {
                break;
            }
        }
        Ok(bytes)
    }

    fn hex_at(&self, ahead: usize) -> Option<u8> {
        match self.text.as_bytes().get(self.offset + ahead)? {
            byte @ b'0'..=b'9' => Some(byte - b'0'),
            byte @ b'a'..=b'f' => Some(byte - b'a' + 10),
            byte @ b'A'..=b'F' => Some(byte - b'A' + 10),
            _ => None,
        }
    }

    fn number(&mut self) -> Result<u32, LexError> {
        let start = self.offset;
        while let Some(byte) = self.peek() {
            if byte.is_ascii_digit() {
                self.bump();
            } else {
                break;
            }
        }
        let text = &self.text[start..self.offset];
        text.parse()
            .map_err(|_| self.error(format!("number '{text}' does not fit")))
    }

    fn quoted_string(&mut self) -> Result<String, LexError> {
        let bytes = self.quoted_bytes()?;
        String::from_utf8(bytes).map_err(|_| self.error("quoted string is not valid UTF-8"))
    }

    /// A `"..."` body, with `\XX` hex escapes and the `\\` shorthand.
    fn quoted_bytes(&mut self) -> Result<Vec<u8>, LexError> {
        debug_assert_eq!(self.peek(), Some(b'"'));
        self.bump();
        let mut out = Vec::new();
        loop {
            let Some(byte) = self.bump() else {
                return Err(self.error("unterminated string"));
            };
            match byte {
                b'"' => return Ok(out),
                b'\\' => {
                    let high = self
                        .peek()
                        .ok_or_else(|| self.error("unterminated escape"))?;
                    if high == b'\\' {
                        self.bump();
                        out.push(b'\\');
                        continue;
                    }
                    let low = self
                        .peek_at(1)
                        .ok_or_else(|| self.error("unterminated escape"))?;
                    let (Some(high), Some(low)) =
                        ((high as char).to_digit(16), (low as char).to_digit(16))
                    else {
                        // `"c:\temp"` holds a backslash and a `t`, not an
                        // escape: upstream keeps a backslash that begins no
                        // escape and prints it back as `\5C`.
                        out.push(b'\\');
                        continue;
                    };
                    self.bump();
                    self.bump();
                    out.push((high * 16 + low) as u8);
                }
                other => out.push(other),
            }
        }
    }

    /// An integer, a float, or one of the `0x` float forms. The sign belongs
    /// to the token: `-1` is one literal.
    fn numeric(&mut self) -> Result<Token, LexError> {
        let negative = match self.peek() {
            Some(b'-') => {
                self.bump();
                true
            }
            Some(b'+') => {
                self.bump();
                false
            }
            _ => false,
        };

        if self.peek() == Some(b'0') && matches!(self.peek_at(1), Some(b'x' | b'X')) {
            self.bump();
            self.bump();
            let form = match self.peek() {
                Some(b @ (b'H' | b'K' | b'L' | b'M' | b'R')) => {
                    self.bump();
                    Some(b as char)
                }
                _ => None,
            };
            let start = self.offset;
            while let Some(byte) = self.peek() {
                if byte.is_ascii_hexdigit() {
                    self.bump();
                } else {
                    break;
                }
            }
            if start == self.offset {
                return Err(self.error("hexadecimal literal has no digits"));
            }
            let digits = self.text[start..self.offset].to_string();
            return Ok(Token::HexFloat { form, digits });
        }

        let start = self.offset;
        while let Some(byte) = self.peek() {
            if byte.is_ascii_digit() {
                self.bump();
            } else {
                break;
            }
        }
        let mut is_float = false;
        if self.peek() == Some(b'.') {
            is_float = true;
            self.bump();
            while let Some(byte) = self.peek() {
                if byte.is_ascii_digit() {
                    self.bump();
                } else {
                    break;
                }
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E'))
            && (matches!(self.peek_at(1), Some(b'0'..=b'9'))
                || (matches!(self.peek_at(1), Some(b'+' | b'-'))
                    && matches!(self.peek_at(2), Some(b'0'..=b'9'))))
        {
            is_float = true;
            self.bump();
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.bump();
            }
            while let Some(byte) = self.peek() {
                if byte.is_ascii_digit() {
                    self.bump();
                } else {
                    break;
                }
            }
        }

        let digits = self.text[start..self.offset].to_string();
        if is_float {
            let sign = if negative { "-" } else { "" };
            return Ok(Token::Float(format!("{sign}{digits}")));
        }
        if !negative
            && self.peek() == Some(b':')
            && digits.chars().all(|c| c.is_ascii_digit())
            && !digits.is_empty()
        {
            self.bump();
            let value = digits
                .parse()
                .map_err(|_| self.error("block label number does not fit"))?;
            return Ok(Token::LabelNumber(value));
        }
        Ok(Token::Integer { negative, digits })
    }

    /// A bare word, which is a type keyword, an opcode, an attribute, or a
    /// block label when a colon follows.
    fn word_or_label(&mut self) -> Result<Token, LexError> {
        let start = self.offset;
        // A label may hold a hyphen and a keyword may not: upstream writes
        // `for.cond2thread-pre-split:` as one label and `i16-1` as a type and
        // a negative number. Which one this is only becomes clear at the
        // `:`, so the run is scanned with hyphens and given back to the
        // first one when no colon follows.
        let mut first_hyphen = None;
        while let Some(byte) = self.peek() {
            if is_name_byte(byte) {
                self.bump();
            } else if byte == b'-' {
                first_hyphen.get_or_insert(self.offset);
                self.bump();
            } else {
                break;
            }
        }
        if self.peek() == Some(b':') {
            let word = self.text[start..self.offset].to_string();
            self.bump();
            return Ok(Token::Label(word));
        }
        if let Some(hyphen) = first_hyphen {
            self.rewind_to(hyphen);
        }
        let word = self.text[start..self.offset].to_string();
        if let Some(bits) = word.strip_prefix('i')
            && !bits.is_empty()
            && bits.chars().all(|c| c.is_ascii_digit())
        {
            let bits: u32 = bits
                .parse()
                .map_err(|_| self.error(format!("integer type '{word}' is too wide")))?;
            // An integer is at most 2^23 bits wide, which is what an APInt's
            // bit count fits and what upstream refuses past.
            if bits > 1 << 23 {
                return Err(self.error(format!("integer type '{word}' is too wide")));
            }
            return Ok(Token::IntType(bits));
        }
        Ok(Token::Word(word))
    }
}

fn is_name_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || matches!(byte, b'_' | b'.' | b'$')
}

fn is_name_byte(byte: u8) -> bool {
    // No hyphen: upstream's identifiers are `[A-Za-z$._][A-Za-z$._0-9]*`, and
    // taking one would make `i16-1` a single word rather than a type and a
    // negative number, which is how five CodeGen tests write it.
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'$')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(text: &str) -> Vec<Token> {
        let (spanned, _) = Lexer::new(text).tokenize().unwrap();
        spanned.into_iter().map(|s| s.token).collect()
    }

    #[test]
    fn sigils_and_names() {
        assert_eq!(
            tokens("%a @b $c !d #0"),
            vec![
                Token::LocalName("a".to_string()),
                Token::GlobalName("b".to_string()),
                Token::ComdatName("c".to_string()),
                Token::MetadataName("d".as_bytes().to_vec()),
                Token::AttributeGroup(0),
                Token::Eof,
            ]
        );
        assert_eq!(
            tokens("%3 @4 !5"),
            vec![
                Token::LocalNumber(3),
                Token::GlobalNumber(4),
                Token::MetadataNumber(5),
                Token::Eof,
            ]
        );
        assert_eq!(
            tokens("%\"quoted name\""),
            vec![Token::LocalName("quoted name".to_string()), Token::Eof]
        );
    }

    #[test]
    fn integer_types_are_one_token() {
        assert_eq!(
            tokens("i1 i32 i8388608 index"),
            vec![
                Token::IntType(1),
                Token::IntType(32),
                Token::IntType(8388608),
                Token::Word("index".to_string()),
                Token::Eof,
            ]
        );
    }

    #[test]
    fn labels_need_their_colon() {
        assert_eq!(
            tokens("entry:\n  ret"),
            vec![
                Token::Label("entry".to_string()),
                Token::Word("ret".to_string()),
                Token::Eof,
            ]
        );
        assert_eq!(tokens("3:"), vec![Token::LabelNumber(3), Token::Eof]);
        assert_eq!(tokens("%3:"), vec![Token::LabelNumber(3), Token::Eof]);
    }

    #[test]
    fn numbers_split_into_integers_and_floats() {
        assert_eq!(
            tokens("1 -2 +3"),
            vec![
                Token::Integer {
                    negative: false,
                    digits: "1".to_string()
                },
                Token::Integer {
                    negative: true,
                    digits: "2".to_string()
                },
                Token::Integer {
                    negative: false,
                    digits: "3".to_string()
                },
                Token::Eof,
            ]
        );
        assert_eq!(
            tokens("1.0 -2.5e+10 3.000000e+00"),
            vec![
                Token::Float("1.0".to_string()),
                Token::Float("-2.5e+10".to_string()),
                Token::Float("3.000000e+00".to_string()),
                Token::Eof,
            ]
        );
    }

    #[test]
    fn hexadecimal_float_forms() {
        assert_eq!(
            tokens("0x3FF0000000000000 0xH3C00 0xL0001 0xK4001"),
            vec![
                Token::HexFloat {
                    form: None,
                    digits: "3FF0000000000000".to_string()
                },
                Token::HexFloat {
                    form: Some('H'),
                    digits: "3C00".to_string()
                },
                Token::HexFloat {
                    form: Some('L'),
                    digits: "0001".to_string()
                },
                Token::HexFloat {
                    form: Some('K'),
                    digits: "4001".to_string()
                },
                Token::Eof,
            ]
        );
    }

    #[test]
    fn strings_and_escapes() {
        assert_eq!(
            tokens(r#"c"hi\0A" "plain" "quote\22inside""#),
            vec![
                Token::ByteString(vec![b'h', b'i', 0x0a]),
                Token::Quoted("plain".as_bytes().to_vec()),
                Token::Quoted("quote\"inside".as_bytes().to_vec()),
                Token::Eof,
            ]
        );
        // The `\\` shorthand is the one escape that is not two hex digits.
        assert_eq!(
            tokens(r#"c"back\\slash""#),
            vec![Token::ByteString(b"back\\slash".to_vec()), Token::Eof]
        );
    }

    #[test]
    fn comments_are_trivia_but_the_module_id_is_not() {
        let (spanned, module_id) = Lexer::new("; ModuleID = 'thing.ll'\n; other\nret")
            .tokenize()
            .unwrap();
        assert_eq!(module_id.as_deref(), Some("thing.ll"));
        assert_eq!(
            spanned.into_iter().map(|s| s.token).collect::<Vec<_>>(),
            vec![Token::Word("ret".to_string()), Token::Eof]
        );
    }

    #[test]
    fn positions_track_lines_and_columns() {
        let (spanned, _) = Lexer::new("ret\n  void").tokenize().unwrap();
        assert_eq!(spanned[0].position, Position { line: 1, column: 1 });
        assert_eq!(spanned[1].position, Position { line: 2, column: 3 });
    }

    #[test]
    fn metadata_forms() {
        assert_eq!(
            tokens("!{!0, !\"s\"} !DILocation !dbg"),
            vec![
                Token::Exclaim,
                Token::LeftBrace,
                Token::MetadataNumber(0),
                Token::Comma,
                Token::MetadataString("s".as_bytes().to_vec()),
                Token::RightBrace,
                Token::MetadataName("DILocation".as_bytes().to_vec()),
                Token::MetadataName("dbg".as_bytes().to_vec()),
                Token::Eof,
            ]
        );
    }

    #[test]
    fn unterminated_input_is_an_error_with_a_place() {
        let error = Lexer::new("c\"oops").tokenize().unwrap_err();
        assert!(error.message.contains("unterminated"));
        assert_eq!(error.position.line, 1);
        let error = Lexer::new("ret\n  \u{1}").tokenize().unwrap_err();
        assert_eq!(error.position.line, 2);
    }
}
