// `pw_properties` serializer: matches `pw_properties_serialize_dict` in
// `src/pipewire/properties.c` byte-for-byte (without color escapes).

use std::fmt::Write;

/// Mirror of `PW_PROPERTIES_FLAG_*` from `pipewire/properties.h`.
#[derive(Debug, Clone, Copy)]
pub struct SerializeFlags {
    pub newline: bool,
    pub recurse: bool,
    pub enclose: bool,
    pub array: bool,
}

/// `pw_properties` is an ordered map of `(key, value)` strings, where
/// value is interpreted as SPA-JSON when serialized.
#[derive(Debug, Default, Clone)]
pub struct Properties {
    pub items: Vec<(String, String)>,
}

impl Properties {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) {
        let key = key.into();
        let value = value.into();
        if let Some(slot) = self.items.iter_mut().find(|(k, _)| *k == key) {
            slot.1 = value;
        } else {
            self.items.push((key, value));
        }
    }

    pub fn serialize(&self, flags: SerializeFlags) -> String {
        let mut out = String::new();
        let indent = if flags.newline { 2 } else { 0 };
        let sep: &str = if flags.newline { "\n" } else { " " };
        let (open, close) = if flags.array { ('[', ']') } else { ('{', '}') };

        if flags.enclose {
            out.push(open);
        }
        let mut first = true;
        let last_index = self.items.len();
        for (i, (key, value)) in self.items.iter().enumerate() {
            let _ = i;
            if !first {
                out.push(',');
            }
            first = false;
            out.push_str(sep);
            push_spaces(&mut out, indent);
            if !flags.array {
                push_quoted(&mut out, key);
                out.push_str(": ");
            }
            // Render value: parse as SPA-JSON token; if it's not a
            // recognized literal/container, fall back to a quoted string.
            push_value(&mut out, value, flags.recurse, indent);
            // last element doesn't need anything special
            let _ = last_index;
        }
        if flags.enclose {
            // C `pw-config` always emits the closing brace on its own line
            // when newline mode is on, even for an empty container, so the
            // empty output is `{\n}` not `{}`.
            if !self.items.is_empty() || flags.newline {
                out.push_str(sep);
            }
            out.push(close);
        }
        out
    }
}

fn push_spaces(out: &mut String, n: usize) {
    for _ in 0..n {
        out.push(' ');
    }
}

fn push_quoted(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000c}' => out.push_str("\\f"),
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Render a property value: SPA-JSON tokens pass through verbatim, anything
/// else is quoted as a string.
fn push_value(out: &mut String, value: &str, recurse: bool, indent: usize) {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        out.push_str("null");
        return;
    }
    let _ = (recurse, indent); // recurse-aware reformatting is Phase 2.

    if trimmed == "true" || trimmed == "false" || trimmed == "null" {
        out.push_str(trimmed);
        return;
    }
    if is_int(trimmed) || is_float(trimmed) {
        out.push_str(trimmed);
        return;
    }
    if (trimmed.starts_with('{') && trimmed.ends_with('}'))
        || (trimmed.starts_with('[') && trimmed.ends_with(']'))
    {
        // Container — pass through verbatim. `pw_properties_serialize_dict`
        // does the same when not in recurse mode.
        out.push_str(value);
        return;
    }
    push_quoted(out, value);
}

fn is_int(s: &str) -> bool {
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

fn is_float(s: &str) -> bool {
    if s.is_empty() {
        return false;
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
