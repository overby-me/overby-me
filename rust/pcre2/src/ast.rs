/// Abstract syntax tree for PCRE2 regular expressions.

/// A regex node in the AST.
#[derive(Debug, Clone)]
pub enum Node {
    /// Match a literal byte.
    Literal(u8),
    /// Match any byte (`.` — respects dotall mode).
    AnyByte,
    /// Match a character class (positive or negative).
    Class(CharClass),
    /// Anchors.
    Anchor(AnchorKind),
    /// Quantifier wrapping a sub-expression.
    Quantifier {
        node: Box<Node>,
        kind: QuantKind,
        greedy: bool,
        possessive: bool,
    },
    /// Concatenation of nodes.
    Concat(Vec<Node>),
    /// Alternation (`|`).
    Alternation(Vec<Node>),
    /// Capturing group.
    Group {
        index: u32,
        name: Option<String>,
        node: Box<Node>,
    },
    /// Non-capturing group `(?:...)`.
    NonCapGroup(Box<Node>),
    /// Atomic group `(?>...)` — no backtracking once matched.
    AtomicGroup(Box<Node>),
    /// Lookahead `(?=...)` or `(?!...)`.
    Lookahead {
        node: Box<Node>,
        positive: bool,
    },
    /// Lookbehind `(?<=...)` or `(?<!...)`.
    Lookbehind {
        node: Box<Node>,
        positive: bool,
    },
    /// Backreference `\1`, `\k<name>`.
    Backref(u32),
    /// Word boundary `\b` or `\B`.
    WordBoundary(bool),
    /// Inline options like `(?i)`, `(?m-s)`.
    SetOptions {
        set: Options,
        clear: Options,
        node: Option<Box<Node>>,
    },
    /// Empty match.
    Empty,
}

/// Quantifier kinds.
#[derive(Debug, Clone, Copy)]
pub enum QuantKind {
    /// `*` — zero or more.
    ZeroOrMore,
    /// `+` — one or more.
    OneOrMore,
    /// `?` — zero or one.
    ZeroOrOne,
    /// `{n}` — exactly n.
    Exactly(u32),
    /// `{n,}` — at least n.
    AtLeast(u32),
    /// `{n,m}` — between n and m.
    Range(u32, u32),
}

/// Anchor kinds.
#[derive(Debug, Clone, Copy)]
pub enum AnchorKind {
    /// `^` or `\A` — start of string/line.
    Start,
    /// `$` or `\Z` — end of string/line.
    End,
    /// `\A` — absolute start of string.
    StartOfString,
    /// `\z` — absolute end of string.
    EndOfString,
    /// `\Z` — end of string or before final newline.
    EndOfStringBeforeNewline,
}

/// A character class like `[a-z]` or `\d`.
#[derive(Debug, Clone)]
pub struct CharClass {
    pub ranges: Vec<ClassRange>,
    pub negated: bool,
}

/// A range within a character class.
#[derive(Debug, Clone)]
pub enum ClassRange {
    /// Single byte.
    Single(u8),
    /// Byte range (inclusive).
    Range(u8, u8),
    /// Named class like `\d`, `\w`, `\s`.
    Named(NamedClass),
    /// Unicode property `\p{...}`.
    UnicodeProperty(String, bool),
}

/// Named character classes.
#[derive(Debug, Clone, Copy)]
pub enum NamedClass {
    Digit,    // \d
    NotDigit, // \D
    Word,     // \w
    NotWord,  // \W
    Space,    // \s
    NotSpace, // \S
    HSpace,   // \h
    NotHSpace, // \H
    VSpace,   // \v
    NotVSpace, // \V
}

/// Regex options/flags.
#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    pub caseless: bool,    // (?i)
    pub multiline: bool,   // (?m)
    pub dotall: bool,      // (?s)
    pub extended: bool,    // (?x)
    pub ungreedy: bool,    // (?U)
}
