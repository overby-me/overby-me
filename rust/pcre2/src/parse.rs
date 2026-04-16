/// PCRE2 pattern parser — converts pattern strings to AST nodes.
use crate::ast::*;
use crate::Error;

pub struct Parser<'a> {
    pattern: &'a [u8],
    pos: usize,
    group_count: u32,
    options: Options,
}

impl<'a> Parser<'a> {
    pub fn new(pattern: &'a str, options: Options) -> Self {
        Self {
            pattern: pattern.as_bytes(),
            pos: 0,
            group_count: 0,
            options,
        }
    }

    pub fn parse(&mut self) -> Result<Node, Error> {
        let node = self.parse_alternation()?;
        if self.pos < self.pattern.len() {
            return Err(self.error("unexpected character"));
        }
        Ok(node)
    }

    pub fn group_count(&self) -> u32 {
        self.group_count
    }

    fn error(&self, msg: &str) -> Error {
        Error::Compile {
            offset: self.pos,
            message: msg.to_string(),
        }
    }

    fn peek(&self) -> Option<u8> {
        self.pattern.get(self.pos).copied()
    }

    fn advance(&mut self) -> Option<u8> {
        let b = self.pattern.get(self.pos).copied()?;
        self.pos += 1;
        Some(b)
    }

    fn expect(&mut self, expected: u8) -> Result<(), Error> {
        match self.advance() {
            Some(b) if b == expected => Ok(()),
            _ => Err(self.error(&format!("expected '{}'", expected as char))),
        }
    }

    // ── Alternation: expr ('|' expr)* ──

    fn parse_alternation(&mut self) -> Result<Node, Error> {
        let mut branches = vec![self.parse_concat()?];
        while self.peek() == Some(b'|') {
            self.advance();
            branches.push(self.parse_concat()?);
        }
        if branches.len() == 1 {
            Ok(branches.pop().unwrap())
        } else {
            Ok(Node::Alternation(branches))
        }
    }

    // ── Concatenation: atom* ──

    fn parse_concat(&mut self) -> Result<Node, Error> {
        let mut nodes = Vec::new();
        while let Some(b) = self.peek() {
            if b == b')' || b == b'|' {
                break;
            }
            nodes.push(self.parse_quantified()?);
        }
        match nodes.len() {
            0 => Ok(Node::Empty),
            1 => Ok(nodes.pop().unwrap()),
            _ => Ok(Node::Concat(nodes)),
        }
    }

    // ── Quantified: atom quantifier? ──

    fn parse_quantified(&mut self) -> Result<Node, Error> {
        let node = self.parse_atom()?;
        if let Some(b) = self.peek() {
            let kind = match b {
                b'*' => {
                    self.advance();
                    Some(QuantKind::ZeroOrMore)
                }
                b'+' => {
                    self.advance();
                    Some(QuantKind::OneOrMore)
                }
                b'?' => {
                    self.advance();
                    Some(QuantKind::ZeroOrOne)
                }
                b'{' => self.parse_interval()?,
                _ => None,
            };
            if let Some(kind) = kind {
                let default_greedy = !self.options.ungreedy;
                let (greedy, possessive) = match self.peek() {
                    Some(b'?') => {
                        self.advance();
                        (!default_greedy, false)
                    }
                    Some(b'+') => {
                        self.advance();
                        (true, true)
                    }
                    _ => (default_greedy, false),
                };
                return Ok(Node::Quantifier {
                    node: Box::new(node),
                    kind,
                    greedy,
                    possessive,
                });
            }
        }
        Ok(node)
    }

    fn parse_interval(&mut self) -> Result<Option<QuantKind>, Error> {
        let save = self.pos;
        self.advance(); // skip {
        let min = self.parse_decimal();
        match self.peek() {
            Some(b'}') => {
                self.advance();
                if let Some(n) = min {
                    Ok(Some(QuantKind::Exactly(n)))
                } else {
                    // {}, not a valid quantifier — restore
                    self.pos = save;
                    Ok(None)
                }
            }
            Some(b',') => {
                self.advance();
                let max = self.parse_decimal();
                if self.peek() == Some(b'}') {
                    self.advance();
                    match (min, max) {
                        (Some(n), Some(m)) => Ok(Some(QuantKind::Range(n, m))),
                        (Some(n), None) => Ok(Some(QuantKind::AtLeast(n))),
                        _ => {
                            self.pos = save;
                            Ok(None)
                        }
                    }
                } else {
                    self.pos = save;
                    Ok(None)
                }
            }
            _ => {
                self.pos = save;
                Ok(None)
            }
        }
    }

    fn parse_decimal(&mut self) -> Option<u32> {
        let start = self.pos;
        while self.peek().is_some_and(|b| b.is_ascii_digit()) {
            self.advance();
        }
        if self.pos > start {
            let s = std::str::from_utf8(&self.pattern[start..self.pos]).ok()?;
            s.parse().ok()
        } else {
            None
        }
    }

    // ── Atoms ──

    fn parse_atom(&mut self) -> Result<Node, Error> {
        match self.peek() {
            None => Err(self.error("unexpected end of pattern")),
            Some(b'\\') => self.parse_escape(),
            Some(b'[') => self.parse_class(),
            Some(b'(') => self.parse_group(),
            Some(b'^') => {
                self.advance();
                if self.options.multiline {
                    Ok(Node::Anchor(AnchorKind::Start))
                } else {
                    Ok(Node::Anchor(AnchorKind::StartOfString))
                }
            }
            Some(b'$') => {
                self.advance();
                if self.options.multiline {
                    Ok(Node::Anchor(AnchorKind::End))
                } else if self.options.dollar_endonly {
                    Ok(Node::Anchor(AnchorKind::EndOfString))
                } else {
                    Ok(Node::Anchor(AnchorKind::EndOfStringBeforeNewline))
                }
            }
            Some(b'.') => {
                self.advance();
                if self.options.dotall {
                    Ok(Node::AnyByte)
                } else {
                    // . matches anything except \n
                    Ok(Node::Class(CharClass {
                        ranges: vec![ClassRange::Single(b'\n')],
                        negated: true,
                    }))
                }
            }
            Some(b) => {
                self.advance();
                Ok(Node::Literal(b))
            }
        }
    }

    // ── Escapes ──

    fn parse_escape(&mut self) -> Result<Node, Error> {
        self.advance(); // skip backslash
        match self.advance() {
            None => Err(self.error("trailing backslash")),
            Some(b'd') => Ok(Node::Class(CharClass {
                ranges: vec![ClassRange::Named(NamedClass::Digit)],
                negated: false,
            })),
            Some(b'D') => Ok(Node::Class(CharClass {
                ranges: vec![ClassRange::Named(NamedClass::NotDigit)],
                negated: false,
            })),
            Some(b'w') => Ok(Node::Class(CharClass {
                ranges: vec![ClassRange::Named(NamedClass::Word)],
                negated: false,
            })),
            Some(b'W') => Ok(Node::Class(CharClass {
                ranges: vec![ClassRange::Named(NamedClass::NotWord)],
                negated: false,
            })),
            Some(b's') => Ok(Node::Class(CharClass {
                ranges: vec![ClassRange::Named(NamedClass::Space)],
                negated: false,
            })),
            Some(b'S') => Ok(Node::Class(CharClass {
                ranges: vec![ClassRange::Named(NamedClass::NotSpace)],
                negated: false,
            })),
            Some(b'h') => Ok(Node::Class(CharClass {
                ranges: vec![ClassRange::Named(NamedClass::HSpace)],
                negated: false,
            })),
            Some(b'H') => Ok(Node::Class(CharClass {
                ranges: vec![ClassRange::Named(NamedClass::NotHSpace)],
                negated: false,
            })),
            Some(b'v') => Ok(Node::Class(CharClass {
                ranges: vec![ClassRange::Named(NamedClass::VSpace)],
                negated: false,
            })),
            Some(b'V') => Ok(Node::Class(CharClass {
                ranges: vec![ClassRange::Named(NamedClass::NotVSpace)],
                negated: false,
            })),
            Some(b'b') => Ok(Node::WordBoundary(true)),
            Some(b'B') => Ok(Node::WordBoundary(false)),
            Some(b'A') => Ok(Node::Anchor(AnchorKind::StartOfString)),
            Some(b'Z') => Ok(Node::Anchor(AnchorKind::EndOfStringBeforeNewline)),
            Some(b'z') => Ok(Node::Anchor(AnchorKind::EndOfString)),
            Some(b'n') => Ok(Node::Literal(b'\n')),
            Some(b'r') => Ok(Node::Literal(b'\r')),
            Some(b't') => Ok(Node::Literal(b'\t')),
            Some(b'f') => Ok(Node::Literal(0x0C)),
            Some(b'a') => Ok(Node::Literal(0x07)),
            Some(b'e') => Ok(Node::Literal(0x1B)),
            Some(b'0') => Ok(Node::Literal(0)),
            Some(b'x') => self.parse_hex_escape(),
            Some(b @ b'1'..=b'9') => {
                let n = (b - b'0') as u32;
                Ok(Node::Backref(n))
            }
            Some(b'p') | Some(b'P') => {
                let negated = self.pattern[self.pos - 1] == b'P';
                self.parse_unicode_property(negated)
            }
            Some(b'k') => self.parse_named_backref(),
            Some(b'Q') => self.parse_quoted_literal(),
            // Literal escape — any non-alphanumeric char is itself
            Some(b) if !b.is_ascii_alphanumeric() => Ok(Node::Literal(b)),
            Some(b) => Err(self.error(&format!(
                "unrecognized escape sequence \\{}",
                b as char
            ))),
        }
    }

    fn parse_hex_escape(&mut self) -> Result<Node, Error> {
        if self.peek() == Some(b'{') {
            self.advance();
            let start = self.pos;
            while self.peek().is_some_and(|b| b.is_ascii_hexdigit()) {
                self.advance();
            }
            self.expect(b'}')?;
            let hex = std::str::from_utf8(&self.pattern[start..self.pos - 1])
                .map_err(|_| self.error("invalid hex escape"))?;
            let val =
                u32::from_str_radix(hex, 16).map_err(|_| self.error("invalid hex value"))?;
            // For now, only handle single-byte values
            if val <= 0xFF {
                Ok(Node::Literal(val as u8))
            } else {
                // UTF-8 encode
                let ch = char::from_u32(val).ok_or_else(|| self.error("invalid Unicode codepoint"))?;
                let mut buf = [0u8; 4];
                let s = ch.encode_utf8(&mut buf);
                let bytes: Vec<Node> = s.bytes().map(Node::Literal).collect();
                Ok(Node::Concat(bytes))
            }
        } else {
            // \xHH — two hex digits
            let mut val = 0u8;
            for _ in 0..2 {
                if let Some(b) = self.peek() {
                    if b.is_ascii_hexdigit() {
                        self.advance();
                        val = val * 16
                            + match b {
                                b'0'..=b'9' => b - b'0',
                                b'a'..=b'f' => b - b'a' + 10,
                                b'A'..=b'F' => b - b'A' + 10,
                                _ => unreachable!(),
                            };
                    } else {
                        break;
                    }
                }
            }
            Ok(Node::Literal(val))
        }
    }

    fn parse_unicode_property(&mut self, negated: bool) -> Result<Node, Error> {
        if self.peek() == Some(b'{') {
            self.advance();
            let start = self.pos;
            while self.peek().is_some_and(|b| b != b'}') {
                self.advance();
            }
            self.expect(b'}')?;
            let name = std::str::from_utf8(&self.pattern[start..self.pos - 1])
                .map_err(|_| self.error("invalid property name"))?
                .to_string();
            Ok(Node::Class(CharClass {
                ranges: vec![ClassRange::UnicodeProperty(name, negated)],
                negated: false,
            }))
        } else if let Some(b) = self.advance() {
            // Single-letter property like \pL
            let name = (b as char).to_string();
            Ok(Node::Class(CharClass {
                ranges: vec![ClassRange::UnicodeProperty(name, negated)],
                negated: false,
            }))
        } else {
            Err(self.error("expected property name"))
        }
    }

    fn parse_named_backref(&mut self) -> Result<Node, Error> {
        match self.peek() {
            Some(b'<') => {
                self.advance();
                let start = self.pos;
                while self.peek().is_some_and(|b| b != b'>') {
                    self.advance();
                }
                self.expect(b'>')?;
                let _name = std::str::from_utf8(&self.pattern[start..self.pos - 1])
                    .map_err(|_| self.error("invalid backref name"))?;
                // TODO: resolve named backreference to group index
                Err(self.error("named backreferences not yet implemented"))
            }
            Some(b'\'') => {
                self.advance();
                let start = self.pos;
                while self.peek().is_some_and(|b| b != b'\'') {
                    self.advance();
                }
                self.expect(b'\'')?;
                let _name = std::str::from_utf8(&self.pattern[start..self.pos - 1])
                    .map_err(|_| self.error("invalid backref name"))?;
                Err(self.error("named backreferences not yet implemented"))
            }
            _ => Err(self.error("expected '<' or '\\'' after \\k")),
        }
    }

    fn parse_quoted_literal(&mut self) -> Result<Node, Error> {
        // \Q...\E — literal text
        let mut nodes = Vec::new();
        loop {
            match self.peek() {
                None => break,
                Some(b'\\') => {
                    if self.pattern.get(self.pos + 1) == Some(&b'E') {
                        self.pos += 2;
                        break;
                    }
                    self.advance();
                    nodes.push(Node::Literal(b'\\'));
                }
                Some(b) => {
                    self.advance();
                    nodes.push(Node::Literal(b));
                }
            }
        }
        match nodes.len() {
            0 => Ok(Node::Empty),
            1 => Ok(nodes.pop().unwrap()),
            _ => Ok(Node::Concat(nodes)),
        }
    }

    // ── Character classes ──

    fn parse_class(&mut self) -> Result<Node, Error> {
        self.advance(); // skip [
        let negated = if self.peek() == Some(b'^') {
            self.advance();
            true
        } else {
            false
        };
        let mut ranges = Vec::new();

        // ] as first char is literal
        if self.peek() == Some(b']') {
            self.advance();
            ranges.push(ClassRange::Single(b']'));
        }

        while self.peek().is_some_and(|b| b != b']') {
            let item = self.parse_class_item()?;
            // Check for range a-b
            if self.peek() == Some(b'-') && self.pattern.get(self.pos + 1) != Some(&b']') {
                if let ClassRange::Single(start) = item {
                    self.advance(); // skip -
                    let end_item = self.parse_class_item()?;
                    if let ClassRange::Single(end) = end_item {
                        ranges.push(ClassRange::Range(start, end));
                        continue;
                    }
                    // Not a simple range — add parts separately
                    ranges.push(ClassRange::Single(start));
                    ranges.push(ClassRange::Single(b'-'));
                    ranges.push(end_item);
                    continue;
                }
            }
            ranges.push(item);
        }
        self.expect(b']')?;
        Ok(Node::Class(CharClass { ranges, negated }))
    }

    fn parse_class_item(&mut self) -> Result<ClassRange, Error> {
        match self.peek() {
            Some(b'\\') => {
                self.advance();
                match self.advance() {
                    None => Err(self.error("trailing backslash in class")),
                    Some(b'd') => Ok(ClassRange::Named(NamedClass::Digit)),
                    Some(b'D') => Ok(ClassRange::Named(NamedClass::NotDigit)),
                    Some(b'w') => Ok(ClassRange::Named(NamedClass::Word)),
                    Some(b'W') => Ok(ClassRange::Named(NamedClass::NotWord)),
                    Some(b's') => Ok(ClassRange::Named(NamedClass::Space)),
                    Some(b'S') => Ok(ClassRange::Named(NamedClass::NotSpace)),
                    Some(b'h') => Ok(ClassRange::Named(NamedClass::HSpace)),
                    Some(b'H') => Ok(ClassRange::Named(NamedClass::NotHSpace)),
                    Some(b'v') => Ok(ClassRange::Named(NamedClass::VSpace)),
                    Some(b'V') => Ok(ClassRange::Named(NamedClass::NotVSpace)),
                    Some(b'n') => Ok(ClassRange::Single(b'\n')),
                    Some(b'r') => Ok(ClassRange::Single(b'\r')),
                    Some(b't') => Ok(ClassRange::Single(b'\t')),
                    Some(b'x') => {
                        // hex escape
                        if self.peek() == Some(b'{') {
                            self.advance();
                            let start = self.pos;
                            while self.peek().is_some_and(|b| b.is_ascii_hexdigit()) {
                                self.advance();
                            }
                            self.expect(b'}')?;
                            let hex =
                                std::str::from_utf8(&self.pattern[start..self.pos - 1])
                                    .map_err(|_| self.error("invalid hex"))?;
                            let val = u32::from_str_radix(hex, 16)
                                .map_err(|_| self.error("invalid hex value"))?;
                            Ok(ClassRange::Single(val as u8))
                        } else {
                            let mut val = 0u8;
                            for _ in 0..2 {
                                if let Some(b) = self.peek() {
                                    if b.is_ascii_hexdigit() {
                                        self.advance();
                                        val = val * 16
                                            + match b {
                                                b'0'..=b'9' => b - b'0',
                                                b'a'..=b'f' => b - b'a' + 10,
                                                b'A'..=b'F' => b - b'A' + 10,
                                                _ => unreachable!(),
                                            };
                                    }
                                }
                            }
                            Ok(ClassRange::Single(val))
                        }
                    }
                    Some(b) if !b.is_ascii_alphanumeric() => Ok(ClassRange::Single(b)),
                    Some(b) => Err(self.error(&format!(
                        "unrecognized escape in class: \\{}",
                        b as char
                    ))),
                }
            }
            Some(b'[') if self.pattern.get(self.pos + 1) == Some(&b':') => {
                // POSIX class [:alpha:]
                self.advance(); // [
                self.advance(); // :
                let start = self.pos;
                while self.peek().is_some_and(|b| b != b':') {
                    self.advance();
                }
                let name_end = self.pos;
                self.expect(b':')?;
                self.expect(b']')?;
                let name = std::str::from_utf8(&self.pattern[start..name_end])
                    .map_err(|_| self.error("invalid POSIX class name"))?;
                let class = match name {
                    "alpha" => ClassRange::Range(b'A', b'z'), // simplified
                    "digit" => ClassRange::Named(NamedClass::Digit),
                    "alnum" => ClassRange::Named(NamedClass::Word),
                    "space" | "blank" => ClassRange::Named(NamedClass::Space),
                    "upper" => ClassRange::Range(b'A', b'Z'),
                    "lower" => ClassRange::Range(b'a', b'z'),
                    "print" | "graph" => ClassRange::Range(0x20, 0x7E),
                    "punct" => ClassRange::Range(b'!', b'/'),
                    "cntrl" => ClassRange::Range(0, 0x1F),
                    "xdigit" => ClassRange::Named(NamedClass::Digit), // simplified
                    _ => return Err(self.error(&format!("unknown POSIX class: {name}"))),
                };
                Ok(class)
            }
            Some(b) => {
                self.advance();
                Ok(ClassRange::Single(b))
            }
            None => Err(self.error("unterminated character class")),
        }
    }

    // ── Groups ──

    fn parse_group(&mut self) -> Result<Node, Error> {
        self.advance(); // skip (
        if self.peek() == Some(b'?') {
            self.advance();
            match self.peek() {
                Some(b':') => {
                    self.advance();
                    let node = self.parse_alternation()?;
                    self.expect(b')')?;
                    Ok(Node::NonCapGroup(Box::new(node)))
                }
                Some(b'>') => {
                    self.advance();
                    let node = self.parse_alternation()?;
                    self.expect(b')')?;
                    Ok(Node::AtomicGroup(Box::new(node)))
                }
                Some(b'=') => {
                    self.advance();
                    let node = self.parse_alternation()?;
                    self.expect(b')')?;
                    Ok(Node::Lookahead {
                        node: Box::new(node),
                        positive: true,
                    })
                }
                Some(b'!') => {
                    self.advance();
                    let node = self.parse_alternation()?;
                    self.expect(b')')?;
                    Ok(Node::Lookahead {
                        node: Box::new(node),
                        positive: false,
                    })
                }
                Some(b'<') => {
                    self.advance();
                    match self.peek() {
                        Some(b'=') => {
                            self.advance();
                            let node = self.parse_alternation()?;
                            self.expect(b')')?;
                            Ok(Node::Lookbehind {
                                node: Box::new(node),
                                positive: true,
                            })
                        }
                        Some(b'!') => {
                            self.advance();
                            let node = self.parse_alternation()?;
                            self.expect(b')')?;
                            Ok(Node::Lookbehind {
                                node: Box::new(node),
                                positive: false,
                            })
                        }
                        _ => {
                            // Named capture (?<name>...)
                            let start = self.pos;
                            while self.peek().is_some_and(|b| b != b'>') {
                                self.advance();
                            }
                            let name = std::str::from_utf8(&self.pattern[start..self.pos])
                                .map_err(|_| self.error("invalid group name"))?
                                .to_string();
                            self.expect(b'>')?;
                            self.group_count += 1;
                            let index = self.group_count;
                            let node = self.parse_alternation()?;
                            self.expect(b')')?;
                            Ok(Node::Group {
                                index,
                                name: Some(name),
                                node: Box::new(node),
                            })
                        }
                    }
                }
                Some(b'P') => {
                    self.advance();
                    match self.peek() {
                        Some(b'<') => {
                            // Python named group (?P<name>...)
                            self.advance();
                            let start = self.pos;
                            while self.peek().is_some_and(|b| b != b'>') {
                                self.advance();
                            }
                            let name = std::str::from_utf8(&self.pattern[start..self.pos])
                                .map_err(|_| self.error("invalid group name"))?
                                .to_string();
                            self.expect(b'>')?;
                            self.group_count += 1;
                            let index = self.group_count;
                            let node = self.parse_alternation()?;
                            self.expect(b')')?;
                            Ok(Node::Group {
                                index,
                                name: Some(name),
                                node: Box::new(node),
                            })
                        }
                        _ => Err(self.error("expected '<' after (?P")),
                    }
                }
                Some(b'#') => {
                    // Comment (?#...)
                    while self.peek().is_some_and(|b| b != b')') {
                        self.advance();
                    }
                    self.expect(b')')?;
                    Ok(Node::Empty)
                }
                Some(b) if b == b'i' || b == b'm' || b == b's' || b == b'x' || b == b'U' || b == b'-' => {
                    // Inline options (?imsx-imsx) or (?imsx:...)
                    let (set, clear) = self.parse_option_flags()?;
                    if self.peek() == Some(b':') {
                        self.advance();
                        let node = self.parse_alternation()?;
                        self.expect(b')')?;
                        Ok(Node::SetOptions {
                            set,
                            clear,
                            node: Some(Box::new(node)),
                        })
                    } else {
                        self.expect(b')')?;
                        Ok(Node::SetOptions {
                            set,
                            clear,
                            node: None,
                        })
                    }
                }
                _ => Err(self.error("unrecognized group type")),
            }
        } else {
            // Capturing group
            self.group_count += 1;
            let index = self.group_count;
            let node = self.parse_alternation()?;
            self.expect(b')')?;
            Ok(Node::Group {
                index,
                name: None,
                node: Box::new(node),
            })
        }
    }

    fn parse_option_flags(&mut self) -> Result<(Options, Options), Error> {
        let mut set = Options::default();
        let mut clear = Options::default();
        let mut clearing = false;
        loop {
            match self.peek() {
                Some(b'i') => {
                    self.advance();
                    if clearing { clear.caseless = true } else { set.caseless = true }
                }
                Some(b'm') => {
                    self.advance();
                    if clearing { clear.multiline = true } else { set.multiline = true }
                }
                Some(b's') => {
                    self.advance();
                    if clearing { clear.dotall = true } else { set.dotall = true }
                }
                Some(b'x') => {
                    self.advance();
                    if clearing { clear.extended = true } else { set.extended = true }
                }
                Some(b'U') => {
                    self.advance();
                    if clearing { clear.ungreedy = true } else { set.ungreedy = true }
                }
                Some(b'-') => {
                    self.advance();
                    clearing = true;
                }
                _ => break,
            }
        }
        Ok((set, clear))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(pattern: &str) -> Node {
        Parser::new(pattern, Options::default()).parse().unwrap()
    }

    #[test]
    fn literal() {
        match parse("abc") {
            Node::Concat(nodes) => assert_eq!(nodes.len(), 3),
            _ => panic!("expected Concat"),
        }
    }

    #[test]
    fn alternation() {
        match parse("a|b") {
            Node::Alternation(branches) => assert_eq!(branches.len(), 2),
            _ => panic!("expected Alternation"),
        }
    }

    #[test]
    fn quantifiers() {
        match parse("a+") {
            Node::Quantifier { kind: QuantKind::OneOrMore, greedy: true, .. } => {}
            other => panic!("expected Quantifier, got {other:?}"),
        }
    }

    #[test]
    fn group() {
        match parse("(a)") {
            Node::Group { index: 1, .. } => {}
            other => panic!("expected Group, got {other:?}"),
        }
    }

    #[test]
    fn lookahead() {
        match parse("(?=a)") {
            Node::Lookahead { positive: true, .. } => {}
            other => panic!("expected Lookahead, got {other:?}"),
        }
    }

    #[test]
    fn backref() {
        match parse("(a)\\1") {
            Node::Concat(nodes) => {
                assert!(matches!(nodes[1], Node::Backref(1)));
            }
            other => panic!("expected Concat with Backref, got {other:?}"),
        }
    }

    #[test]
    fn char_class() {
        match parse("[a-z]") {
            Node::Class(cc) => {
                assert!(!cc.negated);
                assert_eq!(cc.ranges.len(), 1);
            }
            other => panic!("expected Class, got {other:?}"),
        }
    }

    #[test]
    fn negated_class() {
        match parse("[^a]") {
            Node::Class(cc) => assert!(cc.negated),
            other => panic!("expected negated Class, got {other:?}"),
        }
    }

    #[test]
    fn interval() {
        match parse("a{2,5}") {
            Node::Quantifier { kind: QuantKind::Range(2, 5), .. } => {}
            other => panic!("expected Range(2,5), got {other:?}"),
        }
    }
}
