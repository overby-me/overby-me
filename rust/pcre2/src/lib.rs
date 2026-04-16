//! rust-pcre2: A pure Rust implementation of PCRE2
//!
//! This crate provides Perl Compatible Regular Expressions (PCRE2) without
//! requiring the C PCRE2 library.

pub mod ast;
mod matcher;
pub mod parse;

use ast::Options;
use matcher::MatchState;
use parse::Parser;

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

/// Compile-time options for regex construction.
#[derive(Debug, Clone, Copy, Default)]
pub struct CompileOptions {
    /// Case-insensitive matching.
    pub caseless: bool,
    /// `^` and `$` match line boundaries (not just string boundaries).
    pub multiline: bool,
    /// `.` matches newline.
    pub dotall: bool,
    /// Extended mode (whitespace ignored, `#` comments).
    pub extended: bool,
    /// Swap greedy/lazy quantifier default.
    pub ungreedy: bool,
    /// Enable UTF-8 mode.
    pub utf: bool,
}

/// Runtime match context with configurable limits.
#[derive(Debug, Clone)]
pub struct MatchContext {
    /// Maximum backtracking steps before returning an error.
    pub match_limit: u32,
    /// Maximum recursion depth.
    pub depth_limit: u32,
}

impl Default for MatchContext {
    fn default() -> Self {
        Self {
            match_limit: 1_000_000,
            depth_limit: 1_000,
        }
    }
}

/// Validate that all backreferences refer to existing capture groups.
fn validate_backrefs(node: &ast::Node, num_groups: u32) -> Result<(), Error> {
    match node {
        ast::Node::Backref(n) => {
            if *n > num_groups {
                return Err(Error::Compile {
                    offset: 0,
                    message: format!("reference to non-existent subpattern"),
                });
            }
            Ok(())
        }
        ast::Node::Concat(nodes) | ast::Node::Alternation(nodes) => {
            for n in nodes {
                validate_backrefs(n, num_groups)?;
            }
            Ok(())
        }
        ast::Node::Group { node, .. }
        | ast::Node::NonCapGroup(node)
        | ast::Node::AtomicGroup(node)
        | ast::Node::Lookahead { node, .. }
        | ast::Node::Lookbehind { node, .. } => validate_backrefs(node, num_groups),
        ast::Node::Quantifier { node, .. } => validate_backrefs(node, num_groups),
        ast::Node::SetOptions { node: Some(n), .. } => validate_backrefs(n, num_groups),
        _ => Ok(()),
    }
}

/// A compiled PCRE2 regular expression.
pub struct Regex {
    ast: ast::Node,
    num_captures: u32,
    options: Options,
    context: MatchContext,
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
        Self::with_options(pattern, CompileOptions::default())
    }

    /// Compile a pattern with the given options.
    pub fn with_options(pattern: &str, opts: CompileOptions) -> Result<Regex, Error> {
        let options = Options {
            caseless: opts.caseless,
            multiline: opts.multiline,
            dotall: opts.dotall,
            extended: opts.extended,
            ungreedy: opts.ungreedy,
        };
        let mut parser = Parser::new(pattern, options);
        let ast = parser.parse()?;
        let num_captures = parser.group_count();
        // Validate backreferences
        validate_backrefs(&ast, num_captures)?;
        Ok(Regex {
            ast,
            num_captures,
            options,
            context: MatchContext::default(),
        })
    }

    /// Set the match context (limits).
    pub fn set_match_context(&mut self, ctx: MatchContext) {
        self.context = ctx;
    }

    /// Set the match limit.
    pub fn set_match_limit(&mut self, limit: u32) {
        self.context.match_limit = limit;
    }

    /// Set the depth limit.
    pub fn set_depth_limit(&mut self, limit: u32) {
        self.context.depth_limit = limit;
    }

    /// Test whether the pattern matches anywhere in the subject.
    pub fn is_match(&self, subject: &[u8]) -> Result<bool, Error> {
        self.find(subject).map(|m| m.is_some())
    }

    /// Find the first match in the subject.
    pub fn find<'s>(&self, subject: &'s [u8]) -> Result<Option<Match<'s>>, Error> {
        self.find_at(subject, 0)
    }

    /// Find a match starting the search at the given offset.
    pub fn find_at<'s>(
        &self,
        subject: &'s [u8],
        offset: usize,
    ) -> Result<Option<Match<'s>>, Error> {
        for start in offset..=subject.len() {
            let mut state = MatchState::new(
                subject,
                self.num_captures,
                self.context.match_limit,
                self.context.depth_limit,
                self.options,
            );
            match state.try_match(&self.ast, start) {
                Ok(Some(end)) => {
                    return Ok(Some(Match {
                        subject,
                        start,
                        end,
                    }));
                }
                Ok(None) => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(None)
    }

    /// Find all non-overlapping matches.
    pub fn find_iter<'r, 's>(&'r self, subject: &'s [u8]) -> FindIter<'r, 's> {
        FindIter {
            regex: self,
            subject,
            offset: 0,
        }
    }
}

/// Iterator over non-overlapping matches.
pub struct FindIter<'r, 's> {
    regex: &'r Regex,
    subject: &'s [u8],
    offset: usize,
}

impl<'r, 's> Iterator for FindIter<'r, 's> {
    type Item = Result<Match<'s>, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.offset > self.subject.len() {
            return None;
        }
        match self.regex.find_at(self.subject, self.offset) {
            Ok(Some(m)) => {
                if m.start == m.end {
                    self.offset = m.end + 1;
                } else {
                    self.offset = m.end;
                }
                Some(Ok(m))
            }
            Ok(None) => None,
            Err(e) => Some(Err(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_match() {
        let re = Regex::new("hello").unwrap();
        assert!(re.is_match(b"say hello world").unwrap());
        assert!(!re.is_match(b"goodbye").unwrap());
    }

    #[test]
    fn case_insensitive() {
        let re = Regex::with_options("hello", CompileOptions { caseless: true, ..Default::default() }).unwrap();
        assert!(re.is_match(b"HELLO").unwrap());
    }

    #[test]
    fn find_position() {
        let re = Regex::new("world").unwrap();
        let m = re.find(b"hello world").unwrap().unwrap();
        assert_eq!(m.start(), 6);
        assert_eq!(m.end(), 11);
        assert_eq!(m.as_bytes(), b"world");
    }

    #[test]
    fn find_iter_all() {
        let re = Regex::new("\\d+").unwrap();
        let matches: Vec<_> = re
            .find_iter(b"a1b23c456")
            .map(|m| m.unwrap())
            .map(|m| m.as_bytes().to_vec())
            .collect();
        assert_eq!(matches, vec![b"1".to_vec(), b"23".to_vec(), b"456".to_vec()]);
    }

    #[test]
    fn match_limit_exceeded() {
        let mut re = Regex::new("((a+)*)+$").unwrap();
        re.set_match_limit(10_000);
        let result = re.is_match(b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaab");
        assert!(result.is_err(), "Expected match limit error");
    }

    #[test]
    fn pcre2_lookahead() {
        let re = Regex::new("(?<=aaa).*").unwrap();
        let m = re.find(b"aaab3").unwrap().unwrap();
        assert_eq!(m.as_bytes(), b"b3");
    }

    #[test]
    fn pcre2_word_boundary() {
        let re = Regex::new("(?<!\\w)(?:test)(?!\\w)").unwrap();
        assert!(re.is_match(b"a test here").unwrap());
        assert!(!re.is_match(b"atesthere").unwrap());
    }

    #[test]
    fn backreference() {
        let re = Regex::new("(\\w+)\\s+\\1").unwrap();
        assert!(re.is_match(b"hello hello").unwrap());
        assert!(!re.is_match(b"hello world").unwrap());
    }

    #[test]
    fn multiline() {
        let re = Regex::with_options("^test$", CompileOptions { multiline: true, ..Default::default() }).unwrap();
        assert!(re.is_match(b"line1\ntest\nline3").unwrap());
    }

    #[test]
    fn dotall() {
        let re_no = Regex::new("a.b").unwrap();
        assert!(!re_no.is_match(b"a\nb").unwrap());

        let re_yes = Regex::with_options("a.b", CompileOptions { dotall: true, ..Default::default() }).unwrap();
        assert!(re_yes.is_match(b"a\nb").unwrap());
    }
}
