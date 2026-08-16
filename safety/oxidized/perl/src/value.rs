/// Perl scalar value type.
///
/// Perl scalars have "dual-var" nature: they can be viewed as both strings
/// and numbers, with conversion on demand. We keep it simple: store one
/// representation and convert lazily.
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Clone, Debug)]
pub enum Value {
    Undef,
    /// String scalar with a per-string UTF-8 flag (a.k.a. "SvUTF8" in
    /// reference perl). When the flag is true the string was upgraded
    /// to UTF-8 — e.g. via `pack("U", N)`, `Encode::decode`, or any
    /// concat with another flagged scalar. When false the string is a
    /// byte sequence (latin1/ASCII), and `length` under `use bytes`
    /// returns its char count (each char ≤ 255 is one latin1 byte)
    /// rather than its UTF-8 byte length. Most call sites construct
    /// unflagged strings; use `Value::utf8_str(s)` when explicitly
    /// upgrading. opbasic/concat #26905 and op/length `use bytes`
    /// tests depend on this flag to distinguish `"\xFF"` (1 latin1
    /// byte) from `pack("U", 0xFF)` (the same codepoint but 2 UTF-8
    /// bytes when viewed as bytes).
    Str(String, bool),
    Num(f64),
    /// Reference to an array — stringifies as `ARRAY(0x...)`.
    ArrayRef(Rc<RefCell<Vec<Value>>>),
    /// Reference to a hash — stringifies as `HASH(0x...)`.
    HashRef(Rc<RefCell<HashMap<String, Value>>>),
    /// Reference to a scalar — stringifies as `SCALAR(0x...)`.
    ScalarRef(Rc<RefCell<Value>>),
    /// Reference to a subroutine (by name for now) — `CODE(0x...)`.
    CodeRef(String),
    /// Compiled regex from `qr//` (pattern, flags, id). `ref()` returns
    /// "Regexp"; stringifies as `(?^flags:pattern)` — the same form real
    /// perl uses so it can be matched against with `=~` transparently.
    /// `id` is a unique per-object counter used as a stand-in for the
    /// object's address when numerified (`$qr + 0` in op/qr).
    Regex(String, String, usize),
    /// Typeglob — a symbol-table entry identified by its fully-qualified
    /// name. Stringifies as `*main::NAME`. We model only what the Perl
    /// test suite exercises: passing a glob as a sub argument, then
    /// `local(*NAME) = @_` re-aliasing the current package's `NAME`
    /// filehandle to the glob's source name (and reverting on scope exit).
    Glob(String),
    /// Transparent alias to a scalar slot, backed by `Rc<RefCell<Value>>`.
    /// Used for Perl's `@_` argument aliasing: a sub sees shared storage
    /// for each slot, so `$_[0] = X` writes through to the caller's lvalue
    /// and `\$_[0] == \$_[1]` iff both slots point to the same Rc. Reads
    /// transparently follow via `resolve()`; writes through `assign_to` a
    /// slot that is itself an Alias update the RefCell contents. Distinct
    /// from `ScalarRef` which is a user-visible Perl ref (`\$x`).
    Alias(Rc<RefCell<Value>>),
}

/// Per-process monotonic counter for assigning unique IDs to `qr//`
/// objects. Each numerification (`$qr + 0`) yields this id so two
/// distinct qr// objects compare numerically unequal. op/qr 3.
pub fn next_regex_id() -> usize {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static COUNTER: AtomicUsize = AtomicUsize::new(1);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

impl Value {
    /// Follow `Value::Alias` transparently: return the concrete value
    /// the alias points at. Non-Alias values are returned by clone.
    pub fn resolve(&self) -> Value {
        match self {
            Value::Alias(rc) => rc.borrow().clone(),
            _ => self.clone(),
        }
    }

    /// Construct an unflagged byte/latin1 string scalar — the default
    /// shape produced by literals, numeric coercion, format!, etc.
    pub fn str(s: impl Into<String>) -> Value {
        Value::Str(s.into(), false)
    }

    /// Construct a UTF-8-flagged string scalar. Used by `pack("U", …)`,
    /// `Encode::decode`, and other paths that produce explicitly-
    /// upgraded strings. Tracked separately so `length` under
    /// `use bytes` matches reference perl's char-vs-byte distinction.
    pub fn utf8_str(s: impl Into<String>) -> Value {
        Value::Str(s.into(), true)
    }

    /// True if the value is a UTF-8-flagged string. False for
    /// unflagged strings, numbers, refs, etc.
    pub fn is_utf8_flagged(&self) -> bool {
        matches!(self, Value::Str(_, true))
            || matches!(self, Value::Alias(rc) if rc.borrow().is_utf8_flagged())
    }

    /// Byte-level view of the scalar — the same byte sequence reference
    /// perl exposes under `use bytes`. UTF-8-flagged strings yield their
    /// extended UTF-8 encoding (so `chr(0xD800)`, stored internally as
    /// the surrogate-marker `"\x00\x{D800}"`, returns the 3 bytes
    /// `ed a0 80` rather than the marker's literal bytes). Unflagged
    /// strings map each Unicode scalar to its latin1 byte. Non-string
    /// values stringify first.
    pub fn to_bytes(&self) -> Vec<u8> {
        match self {
            Value::Str(s, true) => {
                // Flagged: decode our extended-codepoint markers (used
                // for surrogates and codepoints > U+10FFFF) then
                // re-encode as extended UTF-8.
                let cps = decode_codepoints(s);
                let mut out = Vec::new();
                for cp in cps {
                    encode_extended_utf8(cp, &mut out);
                }
                out
            }
            Value::Str(s, false) => s.chars().map(|c| c as u32 as u8).collect(),
            Value::Alias(rc) => rc.borrow().to_bytes(),
            other => other.to_str().chars().map(|c| c as u32 as u8).collect(),
        }
    }

    pub fn to_str(&self) -> String {
        match self {
            Value::Undef => String::new(),
            Value::Str(s, _) => s.clone(),
            Value::Num(n) => format_number(*n),
            Value::ArrayRef(r) => format!("ARRAY(0x{:x})", Rc::as_ptr(r) as usize),
            Value::HashRef(r) => format!("HASH(0x{:x})", Rc::as_ptr(r) as usize),
            // A scalar ref whose inner value is a Glob stringifies as
            // `GLOB(0x…)` — `\*test` is a GLOB ref in reference perl.
            Value::ScalarRef(r) => {
                let is_glob = matches!(&*r.borrow(), Value::Glob(_));
                let kind = if is_glob { "GLOB" } else { "SCALAR" };
                format!("{kind}(0x{:x})", Rc::as_ptr(r) as usize)
            }
            // CodeRefs stringify as `CODE(0xADDR)` in reference perl —
            // produce a stable pseudo-address from the sub-name so
            // `\&foo == \&foo` (op/bless 18-20 expect the (0x...) shape).
            Value::CodeRef(name) => {
                use std::hash::{Hash, Hasher};
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                name.hash(&mut hasher);
                let addr = hasher.finish() as usize;
                format!("CODE(0x{addr:x})")
            }
            Value::Regex(pat, flags, _) => format!("(?^{flags}:{pat})"),
            Value::Glob(name) => {
                if name.contains("::") {
                    format!("*{name}")
                } else {
                    format!("*main::{name}")
                }
            }
            Value::Alias(rc) => rc.borrow().to_str(),
        }
    }

    pub fn to_num(&self) -> f64 {
        match self {
            Value::Undef => 0.0,
            Value::Num(n) => *n,
            Value::Str(s, _) => parse_number(s),
            // References stringify then parse as "ARRAY(0x..)" etc. — the
            // numeric coercion returns 0 since there are no leading digits.
            // References numerify to their pointer address (the same
            // value that appears in `ARRAY(0x…)` stringification). This
            // matches reference perl and lets `0+$ref == $ref` hold,
            // which several tests rely on (op/bless).
            Value::ArrayRef(r) => Rc::as_ptr(r) as usize as f64,
            Value::HashRef(r) => Rc::as_ptr(r) as usize as f64,
            Value::ScalarRef(r) => Rc::as_ptr(r) as usize as f64,
            // CodeRef numerifies to the same pseudo-address used in
            // its stringification (`CODE(0xADDR)`), so `0 + $coderef`
            // matches `hex` of the address (op/bless 20).
            Value::CodeRef(name) => {
                use std::hash::{Hash, Hasher};
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                name.hash(&mut hasher);
                hasher.finish() as usize as f64
            }
            Value::Glob(_) => 0.0,
            // qr// objects yield a unique id when numerified — Perl uses
            // the object's address, but a monotonic id is enough for the
            // `$qr_a + 0 != $qr_b + 0` identity test (op/qr 3).
            Value::Regex(_, _, id) => *id as f64,
            Value::Alias(rc) => rc.borrow().to_num(),
        }
    }

    pub fn to_bool(&self) -> bool {
        match self {
            Value::Undef => false,
            Value::Num(n) => *n != 0.0 && !n.is_nan(),
            Value::Str(s, _) => !s.is_empty() && s != "0",
            // References are always true (they stringify to non-"" non-"0").
            Value::ArrayRef(_)
            | Value::HashRef(_)
            | Value::ScalarRef(_)
            | Value::CodeRef(_)
            | Value::Regex(_, _, _)
            | Value::Glob(_) => true,
            Value::Alias(rc) => rc.borrow().to_bool(),
        }
    }

    pub fn is_undef(&self) -> bool {
        match self {
            Value::Undef => true,
            Value::Alias(rc) => rc.borrow().is_undef(),
            _ => false,
        }
    }

    /// Returns the reference type name for `ref()` — `""` for non-refs.
    pub fn ref_type(&self) -> &'static str {
        match self {
            Value::ArrayRef(_) => "ARRAY",
            Value::HashRef(_) => "HASH",
            // `\*name` returns a SCALAR ref whose inner Value is
            // `Value::Glob`. Look inside so `ref(\*test) == "GLOB"`
            // matches reference perl (op/bless 15).
            Value::ScalarRef(rc) => match &*rc.borrow() {
                Value::Glob(_) => "GLOB",
                _ => "SCALAR",
            },
            Value::CodeRef(_) => "CODE",
            Value::Regex(_, _, _) => "Regexp",
            Value::Glob(_) => "GLOB",
            // Resolve through Alias so `ref` on an aliased value
            // (e.g. from list-slice `(LIST)[N]` which emits
            // `Value::Alias(rc)`) sees the underlying value's type.
            Value::Alias(rc) => match &*rc.borrow() {
                Value::ArrayRef(_) => "ARRAY",
                Value::HashRef(_) => "HASH",
                Value::ScalarRef(_) => "SCALAR",
                Value::CodeRef(_) => "CODE",
                Value::Regex(_, _, _) => "Regexp",
                Value::Glob(_) => "GLOB",
                _ => "",
            },
            _ => "",
        }
    }
}

/// Decode a Rust string (potentially containing our internal
/// `"\x00\x{HHHH}"` markers used for surrogates and codepoints above
/// U+10FFFF) into a sequence of codepoints.
pub(crate) fn decode_codepoints(s: &str) -> Vec<u32> {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0 && bytes.get(i + 1..i + 4) == Some(b"\\x{") {
            if let Some(end_off) = bytes[i + 4..].iter().position(|&b| b == b'}') {
                let hex_str = std::str::from_utf8(&bytes[i + 4..i + 4 + end_off]).unwrap_or("");
                if let Ok(cp) = u32::from_str_radix(hex_str, 16) {
                    out.push(cp);
                    i += 4 + end_off + 1;
                    continue;
                }
            }
        }
        let first = bytes[i];
        let len = if first < 0xC0 {
            1
        } else if first < 0xE0 {
            2
        } else if first < 0xF0 {
            3
        } else {
            4
        };
        let end = (i + len).min(bytes.len());
        if let Ok(piece) = std::str::from_utf8(&bytes[i..end]) {
            if let Some(c) = piece.chars().next() {
                out.push(c as u32);
            }
        }
        i = end;
    }
    out
}

/// Encode a codepoint as extended (loose) UTF-8 — the 1992 FSS/UTF-8
/// form that allows up to 6 bytes per codepoint and does not exclude
/// surrogates. Used by `to_bytes` on UTF-8-flagged strings so
/// `chr(0xD800)` round-trips as `ed a0 80` rather than the literal
/// marker bytes.
pub(crate) fn encode_extended_utf8(cp: u32, out: &mut Vec<u8>) {
    if cp < 0x80 {
        out.push(cp as u8);
    } else if cp < 0x800 {
        out.push(0xC0 | ((cp >> 6) as u8));
        out.push(0x80 | ((cp & 0x3F) as u8));
    } else if cp < 0x10000 {
        out.push(0xE0 | ((cp >> 12) as u8));
        out.push(0x80 | (((cp >> 6) & 0x3F) as u8));
        out.push(0x80 | ((cp & 0x3F) as u8));
    } else if cp < 0x20_0000 {
        out.push(0xF0 | ((cp >> 18) as u8));
        out.push(0x80 | (((cp >> 12) & 0x3F) as u8));
        out.push(0x80 | (((cp >> 6) & 0x3F) as u8));
        out.push(0x80 | ((cp & 0x3F) as u8));
    } else if cp < 0x400_0000 {
        out.push(0xF8 | ((cp >> 24) as u8));
        out.push(0x80 | (((cp >> 18) & 0x3F) as u8));
        out.push(0x80 | (((cp >> 12) & 0x3F) as u8));
        out.push(0x80 | (((cp >> 6) & 0x3F) as u8));
        out.push(0x80 | ((cp & 0x3F) as u8));
    } else {
        out.push(0xFC | ((cp >> 30) as u8));
        out.push(0x80 | (((cp >> 24) & 0x3F) as u8));
        out.push(0x80 | (((cp >> 18) & 0x3F) as u8));
        out.push(0x80 | (((cp >> 12) & 0x3F) as u8));
        out.push(0x80 | (((cp >> 6) & 0x3F) as u8));
        out.push(0x80 | ((cp & 0x3F) as u8));
    }
}

/// Format a number the way Perl does: equivalent to C's sprintf("%.15g", n).
pub fn format_number(n: f64) -> String {
    if n.is_nan() {
        return "NaN".to_string();
    }
    if n.is_infinite() {
        return if n > 0.0 {
            "Inf".to_string()
        } else {
            "-Inf".to_string()
        };
    }

    // Integer fast path: if it's an exact integer in safe range, format as int
    if n.fract() == 0.0 && n.abs() < 1e16 {
        return format!("{}", n as i64);
    }

    // Use %.15g behavior
    let abs_n = n.abs();
    if abs_n == 0.0 {
        return "0".to_string();
    }

    let exp = abs_n.log10().floor() as i32;
    let precision: i32 = 15;

    if exp >= -4 && exp < precision {
        // Fixed notation
        let decimal_digits = (precision - 1 - exp).max(0) as usize;
        let s = format!("{:.prec$}", n, prec = decimal_digits);
        trim_trailing_zeros(&s)
    } else {
        // Scientific notation
        let p = 10.0_f64.powi(exp);
        let mantissa = n / p;
        let s = format!("{:.prec$}", mantissa, prec = (precision - 1) as usize);
        let s = trim_trailing_zeros(&s);
        if exp >= 0 {
            format!("{s}e+{exp:02}")
        } else {
            format!("{}e-{:02}", s, -exp)
        }
    }
}

fn trim_trailing_zeros(s: &str) -> String {
    if s.contains('.') {
        let s = s.trim_end_matches('0');
        s.trim_end_matches('.').to_string()
    } else {
        s.to_string()
    }
}

/// Parse a string to a number the way Perl does.
/// Leading whitespace is skipped; parsing stops at the first non-numeric char.
pub fn parse_number(s: &str) -> f64 {
    let s = s.trim_start();
    if s.is_empty() {
        return 0.0;
    }

    // Try to parse as much of the string as possible
    let mut end = 0;
    let bytes = s.as_bytes();

    // Optional sign
    if end < bytes.len() && (bytes[end] == b'+' || bytes[end] == b'-') {
        end += 1;
    }

    // Check for hex/octal/binary prefixes
    if end < bytes.len() && bytes[end] == b'0' && end + 1 < bytes.len() {
        match bytes[end + 1] {
            b'x' | b'X' => {
                end += 2;
                let start = end;
                while end < bytes.len() && bytes[end].is_ascii_hexdigit() {
                    end += 1;
                }
                if end > start {
                    let hex_str = &s[..end];
                    if let Ok(v) = i64::from_str_radix(&hex_str[start..end], 16) {
                        let sign = if s.starts_with('-') { -1.0 } else { 1.0 };
                        return sign * v as f64;
                    }
                }
                return 0.0;
            }
            b'b' | b'B' => {
                end += 2;
                let start = end;
                while end < bytes.len() && (bytes[end] == b'0' || bytes[end] == b'1') {
                    end += 1;
                }
                if end > start {
                    if let Ok(v) = i64::from_str_radix(&s[start..end], 2) {
                        let sign = if s.starts_with('-') { -1.0 } else { 1.0 };
                        return sign * v as f64;
                    }
                }
                return 0.0;
            }
            _ => {}
        }
    }

    // Digits before decimal
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
    }

    // Decimal point and digits after
    if end < bytes.len() && bytes[end] == b'.' {
        end += 1;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }
    }

    // Exponent
    if end < bytes.len() && (bytes[end] == b'e' || bytes[end] == b'E') {
        end += 1;
        if end < bytes.len() && (bytes[end] == b'+' || bytes[end] == b'-') {
            end += 1;
        }
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }
    }

    if end == 0 || (end == 1 && (bytes[0] == b'+' || bytes[0] == b'-')) {
        return 0.0;
    }

    s[..end].parse::<f64>().unwrap_or(0.0)
}
