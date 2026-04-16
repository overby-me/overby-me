//! rust-pcre2: A pure Rust implementation of PCRE2
//!
//! This crate provides Perl Compatible Regular Expressions (PCRE2) without
//! requiring the C PCRE2 library.

/// Compilation and matching errors.
#[derive(Debug)]
pub enum Error {
    /// Pattern compilation error with offset and message.
    Compile { offset: usize, message: String },
    /// Match limit exceeded during matching.
    MatchLimit,
    /// Recursion depth limit exceeded.
    DepthLimit,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Compile { offset, message } => {
                write!(f, "compilation error at offset {offset}: {message}")
            }
            Error::MatchLimit => write!(f, "match limit exceeded"),
            Error::DepthLimit => write!(f, "depth limit exceeded"),
        }
    }
}

impl std::error::Error for Error {}

/// A compiled PCRE2 regular expression.
pub struct Regex {
    _pattern: String,
}

/// A match found in a subject string.
pub struct Match<'a> {
    subject: &'a [u8],
    start: usize,
    end: usize,
}

impl<'a> Match<'a> {
    /// Start offset of the match.
    pub fn start(&self) -> usize {
        self.start
    }

    /// End offset of the match (exclusive).
    pub fn end(&self) -> usize {
        self.end
    }

    /// The matched bytes.
    pub fn as_bytes(&self) -> &'a [u8] {
        &self.subject[self.start..self.end]
    }
}

impl Regex {
    /// Compile a pattern with default options.
    pub fn new(pattern: &str) -> Result<Regex, Error> {
        // TODO: implement pattern compilation
        Ok(Regex {
            _pattern: pattern.to_string(),
        })
    }

    /// Test whether the pattern matches anywhere in the subject.
    pub fn is_match(&self, _subject: &[u8]) -> Result<bool, Error> {
        // TODO: implement matching
        Ok(false)
    }

    /// Find the first match in the subject.
    pub fn find<'s>(&self, _subject: &'s [u8]) -> Result<Option<Match<'s>>, Error> {
        // TODO: implement matching
        Ok(None)
    }

    /// Find a match starting at the given offset.
    pub fn find_at<'s>(
        &self,
        _subject: &'s [u8],
        _offset: usize,
    ) -> Result<Option<Match<'s>>, Error> {
        // TODO: implement matching
        Ok(None)
    }
}
