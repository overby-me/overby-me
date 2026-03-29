mod dollar;
mod heredoc;
mod word;

pub use dollar::{parse_dollar, parse_dollar_with_warnings, warn_incomplete_comsub_in_pattern};
pub use heredoc::parse_word_string;

use word::read_param_word_impl;

use crate::ast::*;
use std::cell::Cell;
use std::collections::{HashMap, HashSet};

thread_local! {
    /// Set when parsing heredoc body — suppresses $'...' processing in nested contexts
    static IN_HEREDOC: Cell<bool> = const { Cell::new(false) };
    /// Set when parsing pattern words (#, %, /) — enables single-quote quoting in dquote
    static PATTERN_WORD: Cell<bool> = const { Cell::new(false) };
    /// POSIX mode flag for dollar expansion (disables ${!name} indirect)
    pub(super) static POSIX_MODE_DOLLAR: Cell<bool> = const { Cell::new(false) };
    /// Aliases available for comsub keyword expansion
    static COMSUB_ALIASES: std::cell::RefCell<HashMap<String, String>> = std::cell::RefCell::new(HashMap::new());
    /// Heredoc EOF warnings from comsub scanner (line, start_line, delimiter)
    static COMSUB_HEREDOC_WARNINGS: std::cell::RefCell<Vec<(usize, usize, String)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Take and clear comsub heredoc warnings
pub fn take_comsub_heredoc_warnings() -> Vec<(usize, usize, String)> {
    COMSUB_HEREDOC_WARNINGS.with(|w| std::mem::take(&mut *w.borrow_mut()))
}

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Word(Word),
    Newline,
    Pipe,
    /// `|&` — pipe with stderr redirect
    PipeAmp,
    AndIf,
    OrIf,
    Semi,
    Amp,
    DSemi,
    /// `;&` — case fallthrough
    SemiAmp,
    /// `;;&` — case test-next
    DSemiAmp,
    LParen,
    RParen,
    Less,
    Great,
    DLess,
    DGreat,
    LessAnd,
    GreatAnd,
    LessGreat,
    DLessDash,
    Clobber,
    TripleLess,
    /// `&>` — redirect both stdout and stderr to file
    AmpGreat,
    /// `&>>` — append both stdout and stderr to file
    AmpDGreat,
    Eof,
}

/// Saved lexer state for backtracking in parser look-ahead.
pub struct LexerSaveState {
    pos: usize,
    pending_heredocs: Vec<HereDocPending>,
    heredoc_bodies_len: usize,
    heredoc_delimiters_len: usize,
    heredoc_eof_warnings_len: usize,
    line: usize,
    // Note: alias expansion state (expand_alias_next, expanding_aliases,
    // alias_end_markers) is NOT saved/restored because alias expansion
    // permanently modifies the input buffer. Restoring would undo marker
    // removals that correspond to already-consumed expansion text.
}

#[derive(Clone)]
struct HereDocPending {
    delimiter: String,
    strip_tabs: bool,
    quoted: bool,
    start_line: usize,
}

pub struct Lexer {
    input: Vec<char>,
    pos: usize,
    pub line: usize,
    pending_heredocs: Vec<HereDocPending>,
    heredoc_bodies: Vec<Word>,
    pub heredoc_delimiters: Vec<String>,
    heredoc_index: usize,
    pub heredoc_overflow_line: Option<usize>,
    pub heredoc_eof_warnings: Vec<(usize, usize, String)>, // (eof_line, start_line, delimiter)
    // Alias expansion
    pub aliases: HashMap<String, String>,
    pub shopt_expand_aliases: bool,
    pub posix_mode: bool,
    expanding_aliases: HashSet<String>, // aliases currently being expanded (prevent recursion)
    alias_end_markers: Vec<(String, usize, bool, usize)>, // (alias_name, end_pos, ends_with_space, newline_count) - when pos passes end_pos, remove from expanding and adjust line count
    expand_alias_next: bool, // next word in command position should be checked for alias expansion
    redirect_target_next: bool, // next word is a redirect target (not a command)
    pub in_case_pattern: bool, // suppress alias expansion in case patterns
    pub had_whitespace_before_token: bool, // tracks if whitespace preceded the last token
}

impl Lexer {
    pub fn current_pos(&self) -> usize {
        self.pos
    }

    pub fn new(input: &str) -> Self {
        Self {
            input: input.chars().collect(),
            pos: 0,
            line: 1,
            pending_heredocs: Vec::new(),
            heredoc_bodies: Vec::new(),
            heredoc_delimiters: Vec::new(),
            heredoc_index: 0,
            heredoc_overflow_line: None,
            heredoc_eof_warnings: Vec::new(),
            aliases: HashMap::new(),
            shopt_expand_aliases: false,
            posix_mode: false,
            expanding_aliases: HashSet::new(),
            alias_end_markers: Vec::new(),
            expand_alias_next: true, // first word is command position
            redirect_target_next: false,
            in_case_pattern: false,
            had_whitespace_before_token: false,
        }
    }

    fn peek(&self) -> Option<char> {
        self.input.get(self.pos).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let ch = self.input.get(self.pos).copied();
        if let Some(c) = ch {
            if c == '\n' {
                // Don't count newlines that are inside alias expansions
                let in_alias = self
                    .alias_end_markers
                    .iter()
                    .any(|(_, end_pos, _, nl)| *nl > 0 && self.pos < *end_pos);
                if !in_alias {
                    self.line += 1;
                }
            }
            self.pos += 1;
        }
        ch
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        self.input.get(self.pos + offset).copied()
    }

    pub fn save_position(&self) -> LexerSaveState {
        LexerSaveState {
            pos: self.pos,
            pending_heredocs: self.pending_heredocs.clone(),
            heredoc_bodies_len: self.heredoc_bodies.len(),
            heredoc_delimiters_len: self.heredoc_delimiters.len(),
            heredoc_eof_warnings_len: self.heredoc_eof_warnings.len(),
            line: self.line,
        }
    }

    pub fn restore_position(&mut self, saved: LexerSaveState) {
        self.pos = saved.pos;
        self.pending_heredocs = saved.pending_heredocs;
        self.heredoc_bodies.truncate(saved.heredoc_bodies_len);
        self.heredoc_delimiters
            .truncate(saved.heredoc_delimiters_len);
        self.heredoc_eof_warnings
            .truncate(saved.heredoc_eof_warnings_len);
        self.line = saved.line;
    }

    fn skip_whitespace(&mut self) {
        while let Some(ch) = self.peek() {
            if ch == ' ' || ch == '\t' {
                self.advance();
            } else if ch == '\\' && self.peek_at(1) == Some('\n') {
                // Line continuation
                self.advance();
                self.advance();
            } else {
                break;
            }
        }
    }

    fn skip_comment(&mut self) {
        if self.peek() == Some('#') {
            while let Some(ch) = self.peek() {
                if ch == '\n' {
                    break;
                }
                self.advance();
            }
        }
    }

    /// Check alias end markers and remove aliases that have been fully consumed
    fn check_alias_end_markers(&mut self) {
        let mut i = 0;
        while i < self.alias_end_markers.len() {
            if self.pos >= self.alias_end_markers[i].1 {
                let (name, _end_pos, ends_with_space, newline_count) =
                    self.alias_end_markers.remove(i);
                self.expanding_aliases.remove(&name);
                if ends_with_space {
                    self.expand_alias_next = true;
                }
                // Newlines in alias expansions are no longer counted during advance(),
                // so no post-adjustment needed
                let _ = newline_count;
            } else {
                i += 1;
            }
        }
    }

    /// Extract the plain text of a word token (for alias lookup).
    /// Returns None if the word contains any expansions (variables, command subs, etc.)
    fn word_to_plain_text(word: &Word) -> Option<String> {
        let mut text = String::new();
        for part in word {
            match part {
                WordPart::Literal(s) => text.push_str(s),
                _ => return None,
            }
        }
        Some(text)
    }

    /// Try to expand a word as an alias. Returns true if expansion happened
    /// (caller should re-lex from the expansion start).
    fn try_alias_expand(&mut self, word: &Word, word_start: usize) -> bool {
        if !self.shopt_expand_aliases
            || !self.expand_alias_next
            || self.redirect_target_next
            || self.in_case_pattern
        {
            return false;
        }
        let text = match Self::word_to_plain_text(word) {
            Some(t) => t,
            None => return false,
        };
        if text.is_empty() || self.expanding_aliases.contains(&text) {
            return false;
        }
        // In POSIX mode, don't alias-expand reserved words
        if self.posix_mode
            && matches!(
                text.as_str(),
                "if" | "then"
                    | "else"
                    | "elif"
                    | "fi"
                    | "case"
                    | "esac"
                    | "for"
                    | "while"
                    | "until"
                    | "do"
                    | "done"
                    | "in"
                    | "function"
                    | "select"
                    | "coproc"
                    | "{"
                    | "}"
                    | "!"
                    | "[["
                    | "]]"
            )
        {
            return false;
        }
        let expansion = match self.aliases.get(&text) {
            Some(v) => v.clone(),
            None => return false,
        };

        let word_end = self.pos;
        let old_len = word_end - word_start;
        // Add a space boundary after expansion text if needed to prevent the
        // expansion's last characters from merging with the original input.
        // Specifically: if the expansion ends with digits and the next char
        // is a redirect operator (< or >), the digit would be parsed as an
        // IO number redirect rather than a word.
        let next_char = self.input.get(word_end).copied();
        let needs_boundary = !expansion.is_empty()
            && !expansion.ends_with(|c: char| c.is_whitespace())
            && expansion
                .chars()
                .rev()
                .take_while(|c| c.is_ascii_digit())
                .count()
                > 0
            && matches!(next_char, Some('<') | Some('>'));
        let mut expansion_chars: Vec<char> = expansion.chars().collect();
        if needs_boundary {
            expansion_chars.push(' ');
        }
        let expansion_len = expansion_chars.len();
        let delta = expansion_len as isize - old_len as isize;

        // Replace the word in the input with the expansion text
        self.input.splice(word_start..word_end, expansion_chars);

        let ends_with_space = expansion.ends_with(' ') || expansion.ends_with('\t');

        // Adjust existing alias end markers that are past the splice point
        for (_, end_pos, _, _) in &mut self.alias_end_markers {
            if *end_pos > word_start {
                *end_pos = (*end_pos as isize + delta) as usize;
            }
        }

        // Set up end marker so we can un-mark this alias when we pass the end.
        // Use the original expansion length (without boundary space) for the marker.
        let orig_expansion_len = expansion.chars().count();
        let newline_count = expansion.chars().filter(|&c| c == '\n').count();
        self.expanding_aliases.insert(text.clone());
        let end_pos = word_start + orig_expansion_len;
        self.alias_end_markers
            .push((text, end_pos, ends_with_space, newline_count));

        // The first word of the expansion is in command position, so always
        // check it for alias expansion
        self.expand_alias_next = true;

        // If the expansion ends with space, the next word from the ORIGINAL
        // input should also be alias-expanded. We do this here (before re-lexing)
        // to handle cases like alias c='< ' where the expansion contains a
        // redirect operator that would prevent the next word from being
        // alias-expanded during normal tokenization.
        if ends_with_space {
            // Skip whitespace after the expansion end to find the next word
            let mut next_pos = word_start + expansion_len;
            while next_pos < self.input.len() && matches!(self.input[next_pos], ' ' | '\t') {
                next_pos += 1;
            }
            // Read the next word (simple: just alphanumeric/underscore chars)
            if next_pos < self.input.len()
                && !matches!(
                    self.input[next_pos],
                    '\n' | ';' | '&' | '|' | '(' | ')' | '<' | '>' | '#'
                )
            {
                let word_start2 = next_pos;
                while next_pos < self.input.len()
                    && !matches!(
                        self.input[next_pos],
                        ' ' | '\t' | '\n' | ';' | '&' | '|' | '(' | ')' | '<' | '>'
                    )
                {
                    next_pos += 1;
                }
                let next_word: String = self.input[word_start2..next_pos].iter().collect();
                let is_reserved_in_posix = self.posix_mode
                    && matches!(
                        next_word.as_str(),
                        "if" | "then"
                            | "else"
                            | "elif"
                            | "fi"
                            | "case"
                            | "esac"
                            | "for"
                            | "while"
                            | "until"
                            | "do"
                            | "done"
                            | "in"
                            | "function"
                            | "select"
                            | "{"
                            | "}"
                            | "!"
                            | "[["
                            | "]]"
                    );
                if !next_word.is_empty()
                    && !self.expanding_aliases.contains(&next_word)
                    && !is_reserved_in_posix
                    && let Some(next_expansion) = self.aliases.get(&next_word).cloned()
                {
                    // Expand the next word too
                    let old_len2 = next_pos - word_start2;
                    let next_ends_with_space =
                        next_expansion.ends_with(' ') || next_expansion.ends_with('\t');
                    let next_expansion_chars: Vec<char> = next_expansion.chars().collect();
                    let next_expansion_len = next_expansion_chars.len();
                    let delta2 = next_expansion_len as isize - old_len2 as isize;

                    self.input
                        .splice(word_start2..next_pos, next_expansion_chars);

                    // Adjust existing markers
                    for (_, ep, _, _) in &mut self.alias_end_markers {
                        if *ep > word_start2 {
                            *ep = (*ep as isize + delta2) as usize;
                        }
                    }

                    let next_orig_len = next_expansion.chars().count();
                    let next_newline_count = next_expansion.chars().filter(|&c| c == '\n').count();
                    self.expanding_aliases.insert(next_word.clone());
                    self.alias_end_markers.push((
                        next_word,
                        word_start2 + next_orig_len,
                        next_ends_with_space,
                        next_newline_count,
                    ));
                }
            }
        }

        // Rewind to start of expansion and re-lex
        self.pos = word_start;

        true
    }

    pub fn next_token(&mut self) -> Token {
        // Sync aliases for comsub keyword expansion
        if self.shopt_expand_aliases {
            COMSUB_ALIASES.with(|a| {
                *a.borrow_mut() = self.aliases.clone();
            });
        }
        let pos_before_ws = self.pos;
        self.skip_whitespace();
        self.had_whitespace_before_token = self.pos > pos_before_ws;

        // Check alias end markers AFTER skipping whitespace, so that trailing
        // spaces in alias expansions are consumed before we check positions
        self.check_alias_end_markers();

        self.skip_comment();

        let ch = match self.peek() {
            None => return Token::Eof,
            Some(c) => c,
        };

        let token = match ch {
            '\n' => {
                self.advance();
                if !self.pending_heredocs.is_empty() {
                    self.read_heredoc_bodies();
                }
                Token::Newline
            }
            '|' => {
                self.advance();
                if self.peek() == Some('|') {
                    self.advance();
                    Token::OrIf
                } else if self.peek() == Some('&') {
                    self.advance();
                    Token::PipeAmp
                } else {
                    Token::Pipe
                }
            }
            '&' => {
                self.advance();
                if self.peek() == Some('&') {
                    self.advance();
                    Token::AndIf
                } else if self.peek() == Some('>') {
                    // &> or &>> — redirect both stdout and stderr
                    self.advance();
                    if self.peek() == Some('>') {
                        self.advance();
                        Token::AmpDGreat // &>>
                    } else {
                        Token::AmpGreat // &>
                    }
                } else {
                    Token::Amp
                }
            }
            ';' => {
                self.advance();
                if self.peek() == Some(';') {
                    self.advance();
                    if self.peek() == Some('&') {
                        self.advance();
                        Token::DSemiAmp
                    } else {
                        Token::DSemi
                    }
                } else if self.peek() == Some('&') {
                    self.advance();
                    Token::SemiAmp
                } else {
                    Token::Semi
                }
            }
            '(' => {
                self.advance();
                Token::LParen
            }
            ')' => {
                self.advance();
                Token::RParen
            }
            '<' => {
                // Check for process substitution <(cmd) — must come before consuming <
                if self.peek_at(1) == Some('(') {
                    return self.read_word();
                }
                self.advance();
                match self.peek() {
                    Some('<') => {
                        self.advance();
                        if self.peek() == Some('<') {
                            self.advance();
                            Token::TripleLess
                        } else if self.peek() == Some('-') {
                            self.advance();
                            self.register_heredoc(true);
                            Token::DLessDash
                        } else {
                            self.register_heredoc(false);
                            Token::DLess
                        }
                    }
                    Some('&') => {
                        self.advance();
                        Token::LessAnd
                    }
                    Some('>') => {
                        self.advance();
                        Token::LessGreat
                    }
                    _ => Token::Less,
                }
            }
            '>' => {
                // Check for process substitution >(cmd)
                if self.peek_at(1) == Some('(') {
                    return self.read_word();
                }
                self.advance();
                match self.peek() {
                    Some('>') => {
                        self.advance();
                        Token::DGreat
                    }
                    Some('&') => {
                        self.advance();
                        Token::GreatAnd
                    }
                    Some('|') => {
                        self.advance();
                        Token::Clobber
                    }
                    _ => Token::Great,
                }
            }
            _ => {
                let word_start = self.pos;
                let word_token = self.read_word();
                // Try alias expansion on the word
                if let Token::Word(ref word) = word_token
                    && self.try_alias_expand(word, word_start)
                {
                    return self.next_token();
                }
                word_token
            }
        };

        // Update expand_alias_next based on the token type
        match &token {
            // These tokens indicate the next word is in command position
            Token::Newline
            | Token::Semi
            | Token::Amp
            | Token::Pipe
            | Token::PipeAmp
            | Token::AndIf
            | Token::OrIf
            | Token::LParen
            | Token::DSemi
            | Token::SemiAmp
            | Token::DSemiAmp => {
                self.expand_alias_next = true;
                self.redirect_target_next = false;
            }
            // Word tokens
            Token::Word(word) => {
                if self.redirect_target_next {
                    // This word is a redirect target — don't change expand_alias_next
                    self.redirect_target_next = false;
                } else if let Some(text) = Self::word_to_plain_text(word) {
                    match text.as_str() {
                        "!" | "time" | "{" | "do" | "then" | "else" | "elif" => {
                            self.expand_alias_next = true;
                        }
                        _ => {
                            // Check if this looks like an assignment (var=value or var+=value)
                            // Assignments before the command word don't consume command position
                            let is_assignment = self.expand_alias_next && text.contains('=') && {
                                let name_part = text.split('=').next().unwrap_or("");
                                let name = name_part.strip_suffix('+').unwrap_or(name_part);
                                !name.is_empty()
                                    && name
                                        .starts_with(|c: char| c == '_' || c.is_ascii_alphabetic())
                                    && name.chars().all(|c| c == '_' || c.is_ascii_alphanumeric())
                            };
                            if !is_assignment && self.expand_alias_next {
                                self.expand_alias_next = false;
                            }
                        }
                    }
                } else {
                    self.expand_alias_next = false;
                }
            }
            // Redirect tokens: next word is a redirect target, not a command
            Token::Less
            | Token::Great
            | Token::DLess
            | Token::DGreat
            | Token::LessAnd
            | Token::GreatAnd
            | Token::LessGreat
            | Token::DLessDash
            | Token::Clobber
            | Token::TripleLess
            | Token::AmpGreat
            | Token::AmpDGreat => {
                self.redirect_target_next = true;
                // Don't change expand_alias_next
            }
            Token::RParen | Token::Eof => {
                self.expand_alias_next = false;
                self.redirect_target_next = false;
            }
        }

        // Drain any heredoc warnings from comsub scanning
        // Drain comsub heredoc warnings (already printed directly)
        let _ = take_comsub_heredoc_warnings();

        token
    }
}
