// SPA-JSON tokenizer and pretty-printer.
//
// SPA-JSON is a relaxed superset of JSON used by every PipeWire config file
// and by the native protocol's metadata blobs. The parser here aims for
// byte-compatibility with the C `spa_json` tokenizer in
// `spa/include/spa/utils/json-core.h` — every input the C tool accepts must
// produce the same `dump()` output our pretty-printer does.
//
// Differences from strict JSON:
//   - Outer braces are optional (file is treated as a top-level object)
//   - Bare-word keys: `key = value` instead of `"key": value`
//   - `=` is accepted in place of `:`
//   - `,` between items is optional (whitespace separates)
//   - Bare-word values: `enabled` / `30` / `1.5` without quotes
//   - Comments: `# ...\n` and `// ...\n` and `/* ... */`
//
// Token spans returned by the tokenizer point into the original input; the
// pretty-printer copies them back verbatim, so primitives keep their
// original spelling (e.g. `1.5e10` doesn't get re-formatted as `15000000000`).

use std::fmt::Write;

/// A parsed SPA-JSON document.
#[derive(Debug, Clone)]
pub enum Value {
    /// A leaf token: bareword, number, bool, null, or quoted string.
    /// `src` is the **entire** source slice including any surrounding quotes.
    Leaf {
        src: String,
    },
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // fields are used by Display + diagnostics
pub struct ParseError {
    pub line: usize,
    pub column: usize,
    pub byte_offset: usize,
    pub message: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "syntax error at line {}, column {}: {}",
            self.line, self.column, self.message
        )
    }
}

impl std::error::Error for ParseError {}

/// Parse a SPA-JSON document. The input may or may not be wrapped in an
/// outer `{}` or `[]`; if it isn't, the input is treated as the contents
/// of an implicit top-level object (matching `spa_json_begin` semantics).
pub fn parse(input: &str) -> Result<Value, ParseError> {
    let mut p = Parser::new(input);
    p.skip_ws_and_comments();
    let v = if p.peek() == Some(b'[') || p.peek() == Some(b'{') {
        // Explicit container — parse it directly.
        let v = p.parse_value()?;
        p.skip_ws_and_comments();
        if p.pos < p.input.len() {
            return Err(p.error("trailing characters after value".into()));
        }
        v
    } else {
        // No outer braces — treat the whole file as object body.
        p.parse_object_body(/* sentinel */ None)?
    };
    Ok(v)
}

struct Parser<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input: input.as_bytes(),
            pos: 0,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let c = self.peek()?;
        self.pos += 1;
        Some(c)
    }

    fn line_col(&self, pos: usize) -> (usize, usize) {
        let mut line = 1;
        let mut col = 1;
        for &b in &self.input[..pos.min(self.input.len())] {
            if b == b'\n' {
                line += 1;
                col = 1;
            } else {
                col += 1;
            }
        }
        (line, col)
    }

    fn error(&self, message: String) -> ParseError {
        let (line, column) = self.line_col(self.pos);
        ParseError {
            line,
            column,
            byte_offset: self.pos,
            message,
        }
    }

    fn skip_ws_and_comments(&mut self) {
        loop {
            match self.peek() {
                None => return,
                Some(b' ' | b'\t' | b'\r' | b'\n' | b',') => {
                    self.pos += 1;
                }
                Some(b'#') => {
                    while let Some(c) = self.bump() {
                        if c == b'\n' {
                            break;
                        }
                    }
                }
                Some(b'/') if self.input.get(self.pos + 1) == Some(&b'/') => {
                    self.pos += 2;
                    while let Some(c) = self.bump() {
                        if c == b'\n' {
                            break;
                        }
                    }
                }
                Some(b'/') if self.input.get(self.pos + 1) == Some(&b'*') => {
                    self.pos += 2;
                    while let Some(c) = self.bump() {
                        if c == b'*' && self.peek() == Some(b'/') {
                            self.pos += 1;
                            break;
                        }
                    }
                }
                _ => return,
            }
        }
    }

    fn parse_value(&mut self) -> Result<Value, ParseError> {
        self.skip_ws_and_comments();
        match self.peek() {
            Some(b'[') => self.parse_array(),
            Some(b'{') => self.parse_object(),
            Some(b'"') => Ok(Value::Leaf {
                src: self.parse_string_token()?,
            }),
            Some(_) => Ok(Value::Leaf {
                src: self.parse_bareword()?,
            }),
            None => Err(self.error("unexpected end of input".into())),
        }
    }

    fn parse_array(&mut self) -> Result<Value, ParseError> {
        debug_assert_eq!(self.peek(), Some(b'['));
        self.pos += 1;
        let mut items = Vec::new();
        loop {
            self.skip_ws_and_comments();
            match self.peek() {
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Value::Array(items));
                }
                Some(_) => items.push(self.parse_value()?),
                None => return Err(self.error("unterminated array".into())),
            }
        }
    }

    fn parse_object(&mut self) -> Result<Value, ParseError> {
        debug_assert_eq!(self.peek(), Some(b'{'));
        self.pos += 1;
        let body = self.parse_object_body(Some(b'}'))?;
        Ok(body)
    }

    /// Parse object key/value pairs until either `}` is hit (when `closer`
    /// is `Some('}')`) or end-of-input (when `closer` is `None`, for the
    /// implicit top-level object case).
    fn parse_object_body(&mut self, closer: Option<u8>) -> Result<Value, ParseError> {
        let mut entries: Vec<(String, Value)> = Vec::new();
        loop {
            self.skip_ws_and_comments();
            match (self.peek(), closer) {
                (Some(c), Some(close)) if c == close => {
                    self.pos += 1;
                    return Ok(Value::Object(entries));
                }
                (None, None) => return Ok(Value::Object(entries)),
                (None, Some(_)) => return Err(self.error("unterminated object".into())),
                (Some(_), _) => {
                    let key = self.parse_key()?;
                    self.skip_ws_and_comments();
                    // Optional `:` or `=`.
                    if matches!(self.peek(), Some(b':') | Some(b'=')) {
                        self.pos += 1;
                    }
                    self.skip_ws_and_comments();
                    let value = self.parse_value()?;
                    entries.push((key, value));
                }
            }
        }
    }

    fn parse_key(&mut self) -> Result<String, ParseError> {
        // Keys may be quoted strings or barewords. Either way the parsed
        // **value** is the unescaped key text (used for output formatting).
        if self.peek() == Some(b'"') {
            let raw = self.parse_string_token()?;
            // Strip the quotes and unescape.
            decode_string_literal(&raw, self.pos)
        } else {
            self.parse_bareword()
        }
    }

    fn parse_string_token(&mut self) -> Result<String, ParseError> {
        debug_assert_eq!(self.peek(), Some(b'"'));
        let start = self.pos;
        self.pos += 1;
        while let Some(c) = self.bump() {
            if c == b'\\' {
                // Skip the next byte (escape char). UTF-8 escapes (\uXXXX)
                // are validated separately in decode_string_literal; here we
                // just need to not stop at \"\\\".
                if self.bump().is_none() {
                    return Err(self.error("unterminated string escape".into()));
                }
                continue;
            }
            if c == b'"' {
                let end = self.pos;
                let bytes = &self.input[start..end];
                return std::str::from_utf8(bytes)
                    .map(|s| s.to_string())
                    .map_err(|_| self.error("invalid UTF-8 in string".into()));
            }
        }
        Err(self.error("unterminated string".into()))
    }

    fn parse_bareword(&mut self) -> Result<String, ParseError> {
        let start = self.pos;
        while let Some(c) = self.peek() {
            // Stop at any structural / whitespace / separator character.
            if matches!(
                c,
                b' ' | b'\t'
                    | b'\r'
                    | b'\n'
                    | b','
                    | b':'
                    | b'='
                    | b'['
                    | b']'
                    | b'{'
                    | b'}'
                    | b'#'
            ) {
                break;
            }
            // Stop before `//` and `/*` so comments terminate a bareword.
            if c == b'/' {
                let n = self.input.get(self.pos + 1).copied();
                if n == Some(b'/') || n == Some(b'*') {
                    break;
                }
            }
            self.pos += 1;
        }
        let bytes = &self.input[start..self.pos];
        if bytes.is_empty() {
            return Err(self.error("expected token".into()));
        }
        std::str::from_utf8(bytes)
            .map(|s| s.to_string())
            .map_err(|_| self.error("invalid UTF-8 in bareword".into()))
    }
}

fn decode_string_literal(raw: &str, err_pos: usize) -> Result<String, ParseError> {
    // raw includes leading and trailing quotes.
    let inner = raw
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .ok_or(ParseError {
            line: 0,
            column: 0,
            byte_offset: err_pos,
            message: "string literal missing quotes".into(),
        })?;

    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            None => {
                return Err(ParseError {
                    line: 0,
                    column: 0,
                    byte_offset: err_pos,
                    message: "trailing backslash in string".into(),
                });
            }
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('b') => out.push('\u{0008}'),
            Some('f') => out.push('\u{000c}'),
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some('/') => out.push('/'),
            Some('u') => {
                let mut hex = String::new();
                for _ in 0..4 {
                    match chars.next() {
                        Some(h) if h.is_ascii_hexdigit() => hex.push(h),
                        _ => {
                            return Err(ParseError {
                                line: 0,
                                column: 0,
                                byte_offset: err_pos,
                                message: "invalid \\u escape".into(),
                            });
                        }
                    }
                }
                let cp = u32::from_str_radix(&hex, 16).unwrap();
                if let Some(ch) = char::from_u32(cp) {
                    out.push(ch);
                }
            }
            Some(other) => out.push(other),
        }
    }
    Ok(out)
}

// ── Pretty-printing ──────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct DumpOptions {
    pub indent: usize,
    pub simple: bool,
}

impl Default for DumpOptions {
    fn default() -> Self {
        Self {
            indent: 2,
            simple: false,
        }
    }
}

const REJECT: &[u8] = b"\"\\'=:,{}[]()#";

fn is_simple_string(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    s.bytes().all(|b| b >= 0x20 && !REJECT.contains(&b))
}

fn encode_string(out: &mut String, val: &str, simple: bool) {
    if simple && is_simple_string(val) {
        out.push_str(val);
        return;
    }
    out.push('"');
    for b in val.bytes() {
        match b {
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            b'\t' => out.push_str("\\t"),
            0x08 => out.push_str("\\b"),
            0x0c => out.push_str("\\f"),
            b'\\' => out.push_str("\\\\"),
            b'"' => out.push_str("\\\""),
            v if v < 0x20 => {
                let _ = write!(out, "\\u{:04x}", v);
            }
            v => out.push(v as char),
        }
    }
    out.push('"');
}

/// Pretty-print a parsed SPA-JSON value matching the C
/// `spa-json-dump` output byte-for-byte (after the trailing newline that
/// the tool emits separately).
pub fn dump(value: &Value, opts: &DumpOptions) -> String {
    let mut out = String::new();
    let comma = if opts.simple { "" } else { "," };
    let key_sep = if opts.simple { " =" } else { ":" };
    dump_inner(&mut out, value, 0, opts, comma, key_sep);
    out
}

fn dump_inner(
    out: &mut String,
    value: &Value,
    indent: usize,
    opts: &DumpOptions,
    comma: &str,
    key_sep: &str,
) {
    match value {
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(comma);
                }
                out.push('\n');
                push_spaces(out, indent + opts.indent);
                dump_inner(out, item, indent + opts.indent, opts, comma, key_sep);
            }
            if !items.is_empty() {
                out.push('\n');
                push_spaces(out, indent);
            }
            out.push(']');
        }
        Value::Object(entries) => {
            out.push('{');
            for (i, (k, v)) in entries.iter().enumerate() {
                if i > 0 {
                    out.push_str(comma);
                }
                out.push('\n');
                push_spaces(out, indent + opts.indent);
                encode_string(out, k, opts.simple);
                out.push_str(key_sep);
                out.push(' ');
                dump_inner(out, v, indent + opts.indent, opts, comma, key_sep);
            }
            if !entries.is_empty() {
                out.push('\n');
                push_spaces(out, indent);
            }
            out.push('}');
        }
        Value::Leaf { src } => {
            // Quoted strings, numbers, true/false/null pass through verbatim.
            if is_passthrough_leaf(src) {
                out.push_str(src);
            } else {
                // Bareword that needs quoting in standard JSON output.
                encode_string(out, src, opts.simple);
            }
        }
    }
}

fn push_spaces(out: &mut String, n: usize) {
    for _ in 0..n {
        out.push(' ');
    }
}

fn is_passthrough_leaf(src: &str) -> bool {
    if src.len() >= 2 && src.starts_with('"') && src.ends_with('"') {
        return true;
    }
    if matches!(src, "true" | "false" | "null") {
        return true;
    }
    is_int_literal(src) || is_float_literal(src)
}

/// Matches `spa_json_is_int`: optional sign, then digits, end-of-input.
fn is_int_literal(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return false;
    }
    let mut i = 0;
    if bytes[0] == b'+' || bytes[0] == b'-' {
        i += 1;
    }
    if i == bytes.len() {
        return false;
    }
    bytes[i..].iter().all(|b| b.is_ascii_digit())
}

/// Matches `spa_json_is_float`: a single number that can be parsed by
/// `strtof` *and* consumes the entire input. We approximate with a
/// tokenizer that accepts the C-grammar float forms.
fn is_float_literal(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let lower = s.to_ascii_lowercase();
    if matches!(lower.as_str(), "inf" | "infinity" | "nan" | "+inf" | "-inf") {
        return true;
    }
    let bytes = s.as_bytes();
    let mut i = 0;
    if bytes[0] == b'+' || bytes[0] == b'-' {
        i += 1;
    }
    let mut saw_digit = false;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        saw_digit = true;
        i += 1;
    }
    if i < bytes.len() && bytes[i] == b'.' {
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            saw_digit = true;
            i += 1;
        }
    }
    if !saw_digit {
        return false;
    }
    if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
        i += 1;
        if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
            i += 1;
        }
        let exp_start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i == exp_start {
            return false;
        }
    }
    i == bytes.len()
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_object() {
        let v = parse("").unwrap();
        let s = dump(&v, &DumpOptions::default());
        assert_eq!(s, "{}");
    }

    #[test]
    fn simple_object() {
        let v = parse("{ a = 1 }").unwrap();
        let s = dump(&v, &DumpOptions::default());
        assert_eq!(s, "{\n  \"a\": 1\n}");
    }

    #[test]
    fn nested_object() {
        let v = parse("a = { b = 2 }").unwrap();
        let s = dump(&v, &DumpOptions::default());
        assert_eq!(s, "{\n  \"a\": {\n    \"b\": 2\n  }\n}");
    }

    #[test]
    fn array_with_three_items() {
        let v = parse("a = [1 2 3]").unwrap();
        let s = dump(&v, &DumpOptions::default());
        assert_eq!(s, "{\n  \"a\": [\n    1,\n    2,\n    3\n  ]\n}");
    }

    #[test]
    fn comments() {
        let v = parse("# hi\na = 1\n# bye").unwrap();
        let s = dump(&v, &DumpOptions::default());
        assert_eq!(s, "{\n  \"a\": 1\n}");
    }

    #[test]
    fn simple_mode_keeps_unquoted() {
        let v = parse("a = foo").unwrap();
        let s = dump(
            &v,
            &DumpOptions {
                indent: 2,
                simple: true,
            },
        );
        assert_eq!(s, "{\n  a = foo\n}");
    }
}
