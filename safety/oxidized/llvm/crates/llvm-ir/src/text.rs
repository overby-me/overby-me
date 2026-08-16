//! Text that is bytes rather than characters.
//!
//! LLVM's strings are byte strings. `!DIFile(filename: "\FF")` is a module
//! `llvm-as` reads and writes back unchanged, and a Rust `String` cannot hold
//! it, so metadata text is a [`ByteString`] here instead. That matters beyond
//! the two files in upstream's suites that exercise it: a debug-info path on
//! a system whose filenames are not UTF-8 is the ordinary case, not a corner.
//!
//! Symbol names, section names and attribute text are still `String`. The
//! line is drawn where the bytes actually come from outside the compiler:
//! debug info carries paths, and everything else carries identifiers the
//! compiler chose. `docs/dialect-notes.md` records what that leaves.

use std::borrow::Cow;

/// A string of bytes, which may or may not be UTF-8.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct ByteString(Vec<u8>);

impl ByteString {
    pub fn new(bytes: Vec<u8>) -> ByteString {
        ByteString(bytes)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// The text, when it happens to be UTF-8. Callers that need to compare
    /// against a literal should use `==` instead, which works either way.
    pub fn as_str(&self) -> Option<&str> {
        std::str::from_utf8(&self.0).ok()
    }

    /// The text with anything that is not UTF-8 replaced, for a message a
    /// person reads rather than a module we print.
    pub fn to_lossy(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.0)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn starts_with(&self, prefix: &str) -> bool {
        self.0.starts_with(prefix.as_bytes())
    }
}

impl From<&str> for ByteString {
    fn from(text: &str) -> ByteString {
        ByteString(text.as_bytes().to_vec())
    }
}

impl From<String> for ByteString {
    fn from(text: String) -> ByteString {
        ByteString(text.into_bytes())
    }
}

impl From<Vec<u8>> for ByteString {
    fn from(bytes: Vec<u8>) -> ByteString {
        ByteString(bytes)
    }
}

impl PartialEq<str> for ByteString {
    fn eq(&self, other: &str) -> bool {
        self.0 == other.as_bytes()
    }
}

impl PartialEq<&str> for ByteString {
    fn eq(&self, other: &&str) -> bool {
        self.0 == other.as_bytes()
    }
}

impl std::fmt::Debug for ByteString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.to_lossy())
    }
}

impl std::fmt::Display for ByteString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_lossy())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_outside_utf8_survive() {
        let text = ByteString::new(vec![0x00, 0x80, 0xFF]);
        assert_eq!(text.as_bytes(), &[0x00, 0x80, 0xFF]);
        assert!(text.as_str().is_none());
        assert!(!text.is_empty());
    }

    /// The point of the `PartialEq<str>` impls: a caller holding a
    /// `ByteString` compares it to a literal without converting either side,
    /// so `attachment.kind == "prof"` reads the way it did before.
    #[test]
    fn comparing_against_a_literal_works_either_way() {
        let text = ByteString::from("prof");
        let bytes = ByteString::new(vec![0xFF]);
        assert_eq!(text, *"prof");
        assert_ne!(bytes, *"prof");
    }
}
