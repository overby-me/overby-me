use crate::ast::*;
use std::cell::Cell;
use std::collections::{HashMap, HashSet};

thread_local! {
    /// Set when parsing heredoc body — suppresses $'...' processing in nested contexts
    static IN_HEREDOC: Cell<bool> = const { Cell::new(false) };
    /// Set when parsing pattern words (#, %, /) — enables single-quote quoting in dquote
    static PATTERN_WORD: Cell<bool> = const { Cell::new(false) };
    /// Aliases available for comsub keyword expansion
    static COMSUB_ALIASES: std::cell::RefCell<HashMap<String, String>> = std::cell::RefCell::new(HashMap::new());
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
            line: self.line,
        }
    }

    pub fn restore_position(&mut self, saved: LexerSaveState) {
        self.pos = saved.pos;
        self.pending_heredocs = saved.pending_heredocs;
        self.heredoc_bodies.truncate(saved.heredoc_bodies_len);
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

        token
    }

    fn register_heredoc(&mut self, strip_tabs: bool) {
        self.skip_whitespace();
        let mut delimiter = String::new();
        let mut quoted = false;

        match self.peek() {
            Some('\'') => {
                quoted = true;
                self.advance();
                while let Some(ch) = self.advance() {
                    if ch == '\'' {
                        break;
                    }
                    delimiter.push(ch);
                }
            }
            Some('"') => {
                quoted = true;
                self.advance();
                while let Some(ch) = self.advance() {
                    if ch == '"' {
                        break;
                    }
                    delimiter.push(ch);
                }
            }
            _ => {
                while let Some(ch) = self.peek() {
                    if ch == '\\' {
                        // Backslash quoting in heredoc delimiter
                        quoted = true;
                        self.advance();
                        if let Some(next) = self.peek() {
                            delimiter.push(next);
                            self.advance();
                        }
                    } else if ch == '\'' {
                        // Single-quoted portion of delimiter
                        quoted = true;
                        self.advance();
                        while let Some(c) = self.advance() {
                            if c == '\'' {
                                break;
                            }
                            delimiter.push(c);
                        }
                    } else if ch == '"' {
                        // Double-quoted portion of delimiter
                        quoted = true;
                        self.advance();
                        while let Some(c) = self.advance() {
                            if c == '"' {
                                break;
                            }
                            delimiter.push(c);
                        }
                    } else if !ch.is_whitespace()
                        && ch != '\n'
                        && ch != ';'
                        && ch != '&'
                        && ch != '|'
                        && ch != ')'
                        && ch != '>'
                        && ch != '<'
                    {
                        delimiter.push(ch);
                        self.advance();
                    } else {
                        break;
                    }
                }
            }
        }

        // Bash limits to 16 here-documents per command
        if self.pending_heredocs.len() >= 16 {
            self.heredoc_overflow_line = Some(self.line);
            return;
        }
        self.heredoc_delimiters.push(delimiter.clone());
        self.pending_heredocs.push(HereDocPending {
            delimiter,
            strip_tabs,
            quoted,
            start_line: self.line,
        });
    }

    fn read_heredoc_bodies(&mut self) {
        let heredocs: Vec<HereDocPending> = self.pending_heredocs.drain(..).collect();
        for hd in heredocs {
            let mut body = String::new();
            loop {
                let mut line = String::new();
                loop {
                    match self.advance() {
                        None => break,
                        Some('\n') => break,
                        Some(ch) => line.push(ch),
                    }
                }
                let check_line = if hd.strip_tabs {
                    line.trim_start_matches('\t').to_string()
                } else {
                    line.clone()
                };
                if check_line == hd.delimiter {
                    break;
                }
                if !body.is_empty() {
                    body.push('\n');
                }
                if hd.strip_tabs {
                    body.push_str(line.trim_start_matches('\t'));
                } else {
                    body.push_str(&line);
                }
                if self.pos >= self.input.len() {
                    // EOF terminated here-document — emit warning
                    self.heredoc_eof_warnings.push((
                        self.line,
                        hd.start_line,
                        hd.delimiter.clone(),
                    ));
                    break;
                }
            }

            let word = if hd.quoted {
                vec![WordPart::SingleQuoted(body)]
            } else {
                parse_double_quoted_content(&body)
            };
            self.heredoc_bodies.push(word);
        }
    }
}

fn parse_double_quoted_content(s: &str) -> Word {
    IN_HEREDOC.with(|f| f.set(true));
    let chars: Vec<char> = s.chars().collect();
    let mut parts = Vec::new();
    let mut literal = String::new();
    let mut i = 0;

    while i < chars.len() {
        match chars[i] {
            '\\' if i + 1 < chars.len() => {
                let next = chars[i + 1];
                if matches!(next, '$' | '`' | '"' | '\\' | '\n') {
                    if next != '\n' {
                        literal.push(next);
                    }
                    i += 2;
                } else {
                    literal.push('\\');
                    i += 1;
                }
            }
            '$' => {
                // Inside double quotes, $' and $" are literal
                if i + 1 < chars.len() && matches!(chars[i + 1], '\'' | '"') {
                    literal.push('$');
                    i += 1;
                } else {
                    if !literal.is_empty() {
                        parts.push(WordPart::Literal(std::mem::take(&mut literal)));
                    }
                    i += 1;
                    let part = parse_dollar(&chars, &mut i, true);
                    parts.push(part);
                }
            }
            '`' => {
                if !literal.is_empty() {
                    parts.push(WordPart::Literal(std::mem::take(&mut literal)));
                }
                i += 1;
                let mut cmd = String::new();
                while i < chars.len() && chars[i] != '`' {
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        let next = chars[i + 1];
                        // Inside double-quoted context backticks, also unescape \"
                        if matches!(next, '$' | '`' | '\\' | '"') {
                            cmd.push(next);
                        } else if next == '\n' {
                            // \<newline> is line continuation — remove both
                        } else {
                            cmd.push('\\');
                            cmd.push(next);
                        }
                        i += 2;
                    } else {
                        cmd.push(chars[i]);
                        i += 1;
                    }
                }
                if i < chars.len() {
                    i += 1; // skip closing `
                }
                parts.push(WordPart::BacktickSub(cmd));
            }
            ch => {
                literal.push(ch);
                i += 1;
            }
        }
    }
    if !literal.is_empty() {
        parts.push(WordPart::Literal(literal));
    }
    IN_HEREDOC.with(|f| f.set(false));
    parts
}

/// Parse a string as a shell word (for expanding ${...} in arithmetic contexts)
pub fn parse_word_string(s: &str) -> Word {
    let chars: Vec<char> = s.chars().collect();
    let mut parts = Vec::new();
    let mut i = 0;
    let mut lit = String::new();
    while i < chars.len() {
        if chars[i] == '$' {
            if !lit.is_empty() {
                parts.push(WordPart::Literal(std::mem::take(&mut lit)));
            }
            i += 1;
            parts.push(parse_dollar(&chars, &mut i, false));
        } else {
            lit.push(chars[i]);
            i += 1;
        }
    }
    if !lit.is_empty() {
        parts.push(WordPart::Literal(lit));
    }
    parts
}

pub fn parse_dollar(chars: &[char], i: &mut usize, in_dquote: bool) -> WordPart {
    if *i >= chars.len() {
        return WordPart::Literal("$".to_string());
    }

    match chars[*i] {
        '[' => {
            // Old-style arithmetic: $[expr]
            *i += 1;
            let mut expr = String::new();
            let mut depth = 1;
            while *i < chars.len() && depth > 0 {
                if chars[*i] == '[' {
                    depth += 1;
                } else if chars[*i] == ']' {
                    depth -= 1;
                    if depth == 0 {
                        *i += 1;
                        break;
                    }
                }
                expr.push(chars[*i]);
                *i += 1;
            }
            WordPart::ArithSub(expr)
        }
        '(' => {
            *i += 1;
            if *i < chars.len() && chars[*i] == '(' {
                // Try arithmetic: $(( ... )), fall back to command sub if content has ';'
                let saved_i = *i;
                *i += 1;
                let mut depth = 1; // nested $(( )) depth
                let mut paren_depth = 0i32; // inner () depth
                let mut expr = String::new();
                let mut has_semicolon_at_top = false;
                while *i < chars.len() && depth > 0 {
                    if *i + 1 < chars.len()
                        && chars[*i] == ')'
                        && chars[*i + 1] == ')'
                        && paren_depth <= 0
                    {
                        depth -= 1;
                        if depth == 0 {
                            *i += 2;
                            break;
                        }
                        expr.push(')');
                        expr.push(')');
                        *i += 2;
                    } else if *i + 1 < chars.len() && chars[*i] == '$' && chars[*i + 1] == '(' {
                        if *i + 2 < chars.len() && chars[*i + 2] == '(' {
                            depth += 1;
                        }
                        expr.push(chars[*i]);
                        *i += 1;
                    } else {
                        if chars[*i] == '(' {
                            paren_depth += 1;
                        } else if chars[*i] == ')' {
                            paren_depth -= 1;
                        } else if chars[*i] == ';' && paren_depth <= 0 {
                            has_semicolon_at_top = true;
                        }
                        expr.push(chars[*i]);
                        *i += 1;
                    }
                }
                if has_semicolon_at_top {
                    // Content has ';' at top paren level — reparse as $( (cmd); (cmd) )
                    *i = saved_i; // Back to the second '('
                // Fall through to command substitution below
                } else {
                    return WordPart::ArithSub(expr);
                }
            }
            {
                // Command substitution: $( ... )
                // Must handle case...esac, quotes, nested $(...)
                let mut depth = 1;
                let mut brace_depth = 0i32; // track ${...} nesting
                let mut cmd = String::new();
                let mut case_depth = 0i32;
                while *i < chars.len() && depth > 0 {
                    match chars[*i] {
                        '\'' if !in_dquote || brace_depth > 0 => {
                            // Single-quoted string — track in non-dquote comsub
                            // OR when inside ${...} (where single quotes are always active)
                            cmd.push(chars[*i]);
                            *i += 1;
                            while *i < chars.len() && chars[*i] != '\'' {
                                cmd.push(chars[*i]);
                                *i += 1;
                            }
                            if *i < chars.len() {
                                cmd.push(chars[*i]);
                                *i += 1;
                            }
                            continue;
                        }
                        '"' => {
                            // Double-quoted string — skip but handle $() and ${} inside
                            cmd.push(chars[*i]);
                            *i += 1;
                            while *i < chars.len() && chars[*i] != '"' {
                                if chars[*i] == '\\' && *i + 1 < chars.len() {
                                    cmd.push(chars[*i]);
                                    *i += 1;
                                    cmd.push(chars[*i]);
                                    *i += 1;
                                    continue;
                                }
                                if chars[*i] == '$' && *i + 1 < chars.len() {
                                    if chars[*i + 1] == '(' {
                                        // Nested $() inside dquotes — track paren depth
                                        cmd.push(chars[*i]);
                                        *i += 1;
                                        cmd.push(chars[*i]);
                                        *i += 1;
                                        let mut inner_depth = 1i32;
                                        while *i < chars.len() && inner_depth > 0 {
                                            match chars[*i] {
                                                '(' => inner_depth += 1,
                                                ')' => {
                                                    inner_depth -= 1;
                                                    if inner_depth == 0 {
                                                        cmd.push(chars[*i]);
                                                        *i += 1;
                                                        break;
                                                    }
                                                }
                                                '"' => {
                                                    // Nested dquotes inside inner $()
                                                    cmd.push(chars[*i]);
                                                    *i += 1;
                                                    while *i < chars.len() && chars[*i] != '"' {
                                                        if chars[*i] == '\\' && *i + 1 < chars.len()
                                                        {
                                                            cmd.push(chars[*i]);
                                                            *i += 1;
                                                        }
                                                        cmd.push(chars[*i]);
                                                        *i += 1;
                                                    }
                                                    if *i < chars.len() {
                                                        cmd.push(chars[*i]);
                                                        *i += 1;
                                                    }
                                                    continue;
                                                }
                                                '\'' => {
                                                    cmd.push(chars[*i]);
                                                    *i += 1;
                                                    while *i < chars.len() && chars[*i] != '\'' {
                                                        cmd.push(chars[*i]);
                                                        *i += 1;
                                                    }
                                                    if *i < chars.len() {
                                                        cmd.push(chars[*i]);
                                                        *i += 1;
                                                    }
                                                    continue;
                                                }
                                                _ => {}
                                            }
                                            if *i < chars.len() && inner_depth > 0 {
                                                cmd.push(chars[*i]);
                                                *i += 1;
                                            }
                                        }
                                        continue;
                                    } else if chars[*i + 1] == '{' {
                                        // Nested ${} inside dquotes — track brace depth
                                        cmd.push(chars[*i]);
                                        *i += 1;
                                        cmd.push(chars[*i]);
                                        *i += 1;
                                        let mut inner_depth = 1i32;
                                        while *i < chars.len() && inner_depth > 0 {
                                            match chars[*i] {
                                                '{' => inner_depth += 1,
                                                '}' => {
                                                    inner_depth -= 1;
                                                    if inner_depth == 0 {
                                                        cmd.push(chars[*i]);
                                                        *i += 1;
                                                        break;
                                                    }
                                                }
                                                _ => {}
                                            }
                                            if *i < chars.len() && inner_depth > 0 {
                                                cmd.push(chars[*i]);
                                                *i += 1;
                                            }
                                        }
                                        continue;
                                    }
                                }
                                cmd.push(chars[*i]);
                                *i += 1;
                            }
                            if *i < chars.len() {
                                cmd.push(chars[*i]);
                                *i += 1;
                            }
                            continue;
                        }
                        '`' => {
                            // Backtick command sub — skip
                            cmd.push(chars[*i]);
                            *i += 1;
                            while *i < chars.len() && chars[*i] != '`' {
                                if chars[*i] == '\\' && *i + 1 < chars.len() {
                                    cmd.push(chars[*i]);
                                    *i += 1;
                                }
                                cmd.push(chars[*i]);
                                *i += 1;
                            }
                            if *i < chars.len() {
                                cmd.push(chars[*i]);
                                *i += 1;
                            }
                            continue;
                        }
                        '(' => {
                            // Don't count ( inside case blocks as it's a pattern delimiter
                            if case_depth <= 0 {
                                depth += 1;
                            }
                        }
                        ')' => {
                            if case_depth <= 0 {
                                depth -= 1;
                                if depth == 0 {
                                    *i += 1;
                                    break;
                                }
                            }
                            // Inside a case block, ) is a pattern delimiter — skip
                        }
                        '}' if brace_depth > 0 => {
                            // Inside a ${...} block — this } closes that block
                            brace_depth -= 1;
                        }
                        '}' if in_dquote && depth == 1 => {
                            // In dquote context, } at comsub depth 1 means the
                            // closing } of the enclosing ${...}. Silent suppression.
                            return WordPart::CommandSub("\x00SILENT_COMSUB".to_string());
                        }
                        '$' if *i + 1 < chars.len() && chars[*i + 1] == '{' => {
                            // Track ${...} nesting
                            cmd.push(chars[*i]);
                            *i += 1;
                            cmd.push(chars[*i]);
                            *i += 1;
                            brace_depth += 1;
                            continue;
                        }
                        '#' if cmd.is_empty()
                            || cmd.ends_with('\n')
                            || (cmd.ends_with(";;")
                                || (cmd.ends_with(';') && !cmd.ends_with("\\;"))) =>
                        {
                            // Comment after newline, ;;, or unescaped ; (not after space — could be escaped word)
                            while *i < chars.len() && chars[*i] != '\n' {
                                cmd.push(chars[*i]);
                                *i += 1;
                            }
                            continue;
                        }
                        _ => {}
                    }
                    // Track case/esac keywords
                    if chars[*i].is_alphabetic() {
                        let _start = *i;
                        let mut word = String::new();
                        while *i < chars.len() && (chars[*i].is_alphanumeric() || chars[*i] == '_')
                        {
                            word.push(chars[*i]);
                            *i += 1;
                        }
                        // Check aliases for case keyword expansion
                        let effective = COMSUB_ALIASES
                            .with(|a| a.borrow().get(word.as_str()).map(|v| v.trim().to_string()));
                        let kw = effective.as_deref().unwrap_or(word.as_str());
                        if kw == "case" {
                            case_depth += 1;
                        } else if kw == "esac" || word == "esac" {
                            case_depth -= 1;
                        }
                        // Count ( and ) in alias expansion to adjust depth
                        if let Some(ref exp) = effective {
                            let mut close_idx = None;
                            for (ci, ch) in exp.chars().enumerate() {
                                match ch {
                                    '(' => depth += 1,
                                    ')' if case_depth <= 0 => {
                                        depth -= 1;
                                        if depth == 0 {
                                            close_idx = Some(ci);
                                            break;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            if depth == 0 {
                                // Alias closes the comsub — add expanded content up to )
                                if let Some(ci) = close_idx {
                                    let before_close: String = exp.chars().take(ci).collect();
                                    cmd.push_str(&before_close);
                                }
                                break;
                            }
                        }
                        cmd.push_str(&word);
                        continue;
                    }
                    cmd.push(chars[*i]);
                    *i += 1;
                }
                if depth > 0 {
                    // Incomplete comsub — signal error via special marker
                    WordPart::CommandSub("\x00INCOMPLETE_COMSUB".to_string())
                } else {
                    WordPart::CommandSub(cmd)
                }
            }
        }
        '{' => {
            *i += 1;
            // Check for funsub: ${ cmd; } — space/tab/newline/| after {
            if *i < chars.len() && matches!(chars[*i], ' ' | '\t' | '\n' | '|') {
                // Parse as command substitution delimited by }
                // Funsub requires that } is preceded by a command terminator (;/\n/&)
                // at the SAME depth level (not from nested blocks)
                let mut depth = 1;
                let mut cmd = String::new();
                let mut has_terminator_at_depth1 = false;
                let mut has_nonws_at_depth1 = false;
                while *i < chars.len() && depth > 0 {
                    match chars[*i] {
                        '{' => {
                            depth += 1;
                            cmd.push(chars[*i]);
                        }
                        '}' => {
                            if depth == 1 && (has_terminator_at_depth1 || !has_nonws_at_depth1) {
                                // Valid funsub close: either has terminator or empty content
                                depth = 0;
                            } else if depth > 1 {
                                depth -= 1;
                                cmd.push(chars[*i]);
                            } else {
                                // depth == 1 but no terminator at this level
                                cmd.push(chars[*i]);
                                has_nonws_at_depth1 = true;
                            }
                        }
                        ';' | '&' | '\n' => {
                            cmd.push(chars[*i]);
                            if depth == 1 {
                                has_terminator_at_depth1 = true;
                                has_nonws_at_depth1 = true;
                            }
                        }
                        '\'' => {
                            cmd.push(chars[*i]);
                            *i += 1;
                            while *i < chars.len() && chars[*i] != '\'' {
                                cmd.push(chars[*i]);
                                *i += 1;
                            }
                            if *i < chars.len() {
                                cmd.push(chars[*i]);
                            }
                            if depth == 1 {
                                has_terminator_at_depth1 = false;
                                has_nonws_at_depth1 = true;
                            }
                        }
                        '"' => {
                            cmd.push(chars[*i]);
                            *i += 1;
                            while *i < chars.len() && chars[*i] != '"' {
                                if chars[*i] == '\\' && *i + 1 < chars.len() {
                                    cmd.push(chars[*i]);
                                    *i += 1;
                                }
                                cmd.push(chars[*i]);
                                *i += 1;
                            }
                            if *i < chars.len() {
                                cmd.push(chars[*i]);
                            }
                            if depth == 1 {
                                has_terminator_at_depth1 = false;
                                has_nonws_at_depth1 = true;
                            }
                        }
                        ' ' | '\t' => {
                            cmd.push(chars[*i]);
                            // Whitespace doesn't affect terminator or nonws state
                        }
                        _ => {
                            cmd.push(chars[*i]);
                            if depth == 1 {
                                has_terminator_at_depth1 = false;
                                has_nonws_at_depth1 = true;
                            }
                        }
                    }
                    *i += 1;
                }
                if depth > 0 {
                    // Unclosed funsub — return as incomplete
                    WordPart::CommandSub(format!("\x00INCOMPLETE_FUNSUB{}", cmd))
                } else {
                    WordPart::CommandSub(cmd)
                }
            } else {
                parse_brace_param(chars, i, in_dquote)
            }
        }
        ch if ch == '_' || ch.is_alphabetic() => {
            let mut name = String::new();
            while *i < chars.len() && (chars[*i] == '_' || chars[*i].is_alphanumeric()) {
                name.push(chars[*i]);
                *i += 1;
            }
            WordPart::Variable(name)
        }
        ch if ch.is_ascii_digit() => {
            let mut name = String::new();
            name.push(chars[*i]);
            *i += 1;
            WordPart::Variable(name)
        }
        '@' | '*' | '#' | '?' | '-' | '$' | '!' | '0' => {
            let name = chars[*i].to_string();
            *i += 1;
            WordPart::Variable(name)
        }
        '"' => {
            // $"..." locale-specific quoting — treat as regular double quoting
            *i += 1; // skip "
            let mut dq_parts = Vec::new();
            let mut dq_lit = String::new();
            while *i < chars.len() && chars[*i] != '"' {
                match chars[*i] {
                    '\\' if *i + 1 < chars.len() => {
                        let next = chars[*i + 1];
                        if matches!(next, '$' | '`' | '"' | '\\') {
                            dq_lit.push(next);
                        } else {
                            dq_lit.push('\\');
                            dq_lit.push(next);
                        }
                        *i += 2;
                    }
                    '$' => {
                        // Inside double quotes, $' and $" are literal
                        if *i + 1 < chars.len() && matches!(chars[*i + 1], '\'' | '"') {
                            dq_lit.push('$');
                            *i += 1;
                        } else {
                            if !dq_lit.is_empty() {
                                dq_parts.push(WordPart::Literal(std::mem::take(&mut dq_lit)));
                            }
                            *i += 1;
                            dq_parts.push(parse_dollar(chars, i, true));
                        }
                    }
                    ch => {
                        dq_lit.push(ch);
                        *i += 1;
                    }
                }
            }
            if *i < chars.len() {
                *i += 1; // skip closing "
            }
            if !dq_lit.is_empty() {
                dq_parts.push(WordPart::Literal(dq_lit));
            }
            WordPart::DoubleQuoted(dq_parts)
        }
        '\'' if !IN_HEREDOC.with(|f| f.get()) => {
            // $'...' ANSI-C quoting (not in heredoc context)
            *i += 1; // skip '
            let mut s = String::new();
            while *i < chars.len() && chars[*i] != '\'' {
                if chars[*i] == '\\' && *i + 1 < chars.len() {
                    *i += 1;
                    match chars[*i] {
                        'n' => s.push('\n'),
                        't' => s.push('\t'),
                        'r' => s.push('\r'),
                        '\\' => s.push('\\'),
                        '\'' => s.push('\''),
                        '"' => s.push('"'),
                        'a' => s.push('\x07'),
                        'b' => s.push('\x08'),
                        'c' => {
                            // \cX — control character (X ^ 0x40), like bash
                            // If next char is \, process the escape first
                            if *i + 1 < chars.len() {
                                *i += 1;
                                let target_char = if chars[*i] == '\\' && *i + 1 < chars.len() {
                                    *i += 1;
                                    chars[*i]
                                } else {
                                    chars[*i]
                                };
                                let ctrl = (target_char as u8) ^ 0x40;
                                if ctrl == 0 {
                                    break; // \c@ terminates
                                }
                                s.push(ctrl as char);
                            }
                        }
                        'e' | 'E' => s.push('\x1b'),
                        'f' => s.push('\x0c'),
                        'v' => s.push('\x0b'),
                        c @ '0'..='7' => {
                            let mut val = c as u8 - b'0';
                            for _ in 0..2 {
                                if *i + 1 < chars.len() && matches!(chars[*i + 1], '0'..='7') {
                                    *i += 1;
                                    val = val * 8 + (chars[*i] as u8 - b'0');
                                } else {
                                    break;
                                }
                            }
                            if val == 0 {
                                break; // NUL terminates
                            }
                            s.push(val as char);
                        }
                        'x' => {
                            let mut val = 0u32;
                            let mut count = 0;
                            if *i + 1 < chars.len() && chars[*i + 1] == '{' {
                                *i += 1; // skip {
                                while *i + 1 < chars.len() {
                                    *i += 1;
                                    if chars[*i] == '}' {
                                        break;
                                    }
                                    if chars[*i].is_ascii_hexdigit() {
                                        val = val * 16 + chars[*i].to_digit(16).unwrap();
                                        count += 1;
                                    } else {
                                        break;
                                    }
                                }
                            } else {
                                for _ in 0..2 {
                                    if *i + 1 < chars.len() && chars[*i + 1].is_ascii_hexdigit() {
                                        *i += 1;
                                        val = val * 16 + chars[*i].to_digit(16).unwrap();
                                        count += 1;
                                    } else {
                                        break;
                                    }
                                }
                            }
                            if count > 0 {
                                // \x produces single bytes (truncate to 0xFF)
                                let byte_val = (val & 0xFF) as u8;
                                if byte_val == 0 {
                                    break; // NUL terminates
                                }
                                s.push(byte_val as char);
                            } else {
                                s.push('\\');
                                s.push('x');
                            }
                        }
                        'u' => {
                            let mut val = 0u32;
                            let mut count = 0;
                            if *i + 1 < chars.len() && chars[*i + 1] == '{' {
                                *i += 1;
                                while *i + 1 < chars.len() {
                                    *i += 1;
                                    if chars[*i] == '}' {
                                        break;
                                    }
                                    if chars[*i].is_ascii_hexdigit() {
                                        val = val * 16 + chars[*i].to_digit(16).unwrap();
                                        count += 1;
                                    } else {
                                        break;
                                    }
                                }
                            } else {
                                for _ in 0..4 {
                                    if *i + 1 < chars.len() && chars[*i + 1].is_ascii_hexdigit() {
                                        *i += 1;
                                        val = val * 16 + chars[*i].to_digit(16).unwrap();
                                        count += 1;
                                    } else {
                                        break;
                                    }
                                }
                            }
                            if count > 0 {
                                if let Some(c) = char::from_u32(val) {
                                    s.push(c);
                                }
                            } else {
                                s.push('\\');
                                s.push('u');
                            }
                        }
                        'U' => {
                            let mut val = 0u32;
                            let mut count = 0;
                            if *i + 1 < chars.len() && chars[*i + 1] == '{' {
                                *i += 1;
                                while *i + 1 < chars.len() {
                                    *i += 1;
                                    if chars[*i] == '}' {
                                        break;
                                    }
                                    if chars[*i].is_ascii_hexdigit() {
                                        val = val * 16 + chars[*i].to_digit(16).unwrap();
                                        count += 1;
                                    } else {
                                        break;
                                    }
                                }
                            } else {
                                for _ in 0..8 {
                                    if *i + 1 < chars.len() && chars[*i + 1].is_ascii_hexdigit() {
                                        *i += 1;
                                        val = val * 16 + chars[*i].to_digit(16).unwrap();
                                        count += 1;
                                    } else {
                                        break;
                                    }
                                }
                            }
                            if count > 0 {
                                if let Some(c) = char::from_u32(val) {
                                    s.push(c);
                                }
                            } else {
                                s.push('\\');
                                s.push('U');
                            }
                        }
                        c => {
                            s.push('\\');
                            s.push(c);
                        }
                    }
                } else {
                    s.push(chars[*i]);
                }
                *i += 1;
            }
            if *i < chars.len() {
                *i += 1; // skip closing '
            }
            WordPart::SingleQuoted(s)
        }
        _ => WordPart::Literal("$".to_string()),
    }
}

fn parse_brace_param(chars: &[char], i: &mut usize, in_dquote: bool) -> WordPart {
    // ${!name} — indirect expansion / name prefix / array indices
    if *i < chars.len() && chars[*i] == '!' {
        // Check if '!' should be the variable name (not indirect prefix)
        // ${!} = $!, ${!-word} = $! with default, ${!:-word} = $! with colon-default
        // vs ${!name} = indirect, ${!name-word} = indirect with op
        let next_after_bang = if *i + 1 < chars.len() {
            chars[*i + 1]
        } else {
            '}'
        };
        if next_after_bang == '}'
            || next_after_bang == ':'
            || (matches!(next_after_bang, '-' | '+' | '=' | '?')
                && (*i + 2 >= chars.len() || chars[*i + 2] != '}'))
        {
            // Treat '!' as the variable name, not indirect prefix
            // Falls through to the normal param name reading below
        } else {
            *i += 1;
            let name = read_param_name_with_subscript(chars, i);

            // Check if name ends with [@] or [*] — this is ${!arr[@]} for array indices
            if name.ends_with("[@]") || name.ends_with("[*]") {
                let ch = if name.ends_with("[@]") { '@' } else { '*' };
                let arr_name = name[..name.len() - 3].to_string();
                if *i < chars.len() && chars[*i] == '}' {
                    *i += 1;
                }
                return WordPart::Param(ParamExpr {
                    name: arr_name,
                    op: ParamOp::ArrayIndices(ch),
                });
            }

            // ${!prefix*} or ${!prefix@} — names matching prefix
            if *i < chars.len() && (chars[*i] == '*' || chars[*i] == '@') {
                let ch = chars[*i];
                *i += 1;
                if *i < chars.len() && chars[*i] == '}' {
                    *i += 1;
                }
                return WordPart::Param(ParamExpr {
                    name,
                    op: ParamOp::NamePrefix(ch),
                });
            }
            // Check for operator after indirect name: ${!name+word}, ${!name-word}, etc.
            if *i < chars.len() && chars[*i] != '}' {
                // There's an operator — parse it as indirect + operator
                let op = read_param_op(chars, i, &name, in_dquote);
                if *i < chars.len() && chars[*i] == '}' {
                    *i += 1;
                }
                // Wrap the result: we need indirect resolution first, then apply the op
                // For now, represent as Indirect with the name containing the op info
                // Actually, we need a proper representation. Let's use a special name prefix.
                return WordPart::Param(ParamExpr {
                    name: format!("!{}", name),
                    op,
                });
            }
            if *i < chars.len() && chars[*i] == '}' {
                *i += 1;
            }
            return WordPart::Param(ParamExpr {
                name,
                op: ParamOp::Indirect,
            });
        } // end of else (indirect expansion path)
    }

    // ${#name} - length, but ${#} ${#:-...} ${#-...} etc. are $# with operations
    if *i < chars.len() && chars[*i] == '#' {
        let next = if *i + 1 < chars.len() {
            chars[*i + 1]
        } else {
            '}'
        };
        // Check if this is $# with an operation vs ${#name} (length)
        // ${#:-word}, ${#-word}, ${#+word} are $# with operations
        // ${#-} alone = length of $-, ${#?} alone = length of $?
        // ${#:} alone = bad substitution
        let is_hash_param_op = match next {
            '}' => false,
            ':' => {
                // ${#:} = bad substitution, ${#:X} = $# with operation
                *i + 2 < chars.len() && chars[*i + 2] != '}'
            }
            '-' | '+' => {
                // ${#-} = length of $-, ${#-word} = $# default op
                // ${#+} = bad substitution, ${#+word} = $# alt op
                *i + 2 < chars.len() && chars[*i + 2] != '}'
            }
            '?' => {
                // ${#?} = length of $?, ${#?word} = $# error op
                *i + 2 < chars.len() && chars[*i + 2] != '}'
            }
            '!' => false, // ${#!} handled by indirect expansion
            _ => false,   // ${#name} is length
        };
        if !is_hash_param_op && next != '}' {
            *i += 1;
            let name = read_param_name_with_subscript(chars, i);
            // If name is empty and next char is not }, it's an invalid ${#X} form
            if name.is_empty() && *i < chars.len() && chars[*i] != '}' {
                // Skip to closing } and return bad substitution error
                let start = *i;
                while *i < chars.len() && chars[*i] != '}' {
                    *i += 1;
                }
                let rest: String = chars[start..*i].iter().collect();
                if *i < chars.len() {
                    *i += 1;
                }
                return WordPart::BadSubstitution(format!("${{#{}}}", rest));
            }
            // Check for trailing invalid chars after name (e.g., ${#1xyz})
            if *i < chars.len() && chars[*i] != '}' {
                let start_pos = *i;
                while *i < chars.len() && chars[*i] != '}' {
                    *i += 1;
                }
                let rest: String = std::iter::once('#')
                    .chain(name.chars())
                    .chain(chars[start_pos..*i].iter().copied())
                    .collect();
                if *i < chars.len() {
                    *i += 1;
                }
                return WordPart::BadSubstitution(format!("${{{}}}", rest));
            }
            if *i < chars.len() && chars[*i] == '}' {
                *i += 1;
            }
            return WordPart::Param(ParamExpr {
                name,
                op: ParamOp::Length,
            });
        }
    }

    let name = read_param_name_with_subscript(chars, i);

    // Check for @X transform operator before }
    if *i + 1 < chars.len() && chars[*i] == '@' && chars[*i + 1] != '}' {
        let transform_char = chars[*i + 1];
        if matches!(
            transform_char,
            'E' | 'Q' | 'P' | 'A' | 'a' | 'K' | 'k' | 'L' | 'U' | 'u'
        ) {
            *i += 2;
            if *i < chars.len() && chars[*i] == '}' {
                *i += 1;
            }
            return WordPart::Param(ParamExpr {
                name,
                op: ParamOp::Transform(transform_char),
            });
        }
    }

    if *i >= chars.len() || chars[*i] == '}' {
        if *i < chars.len() {
            *i += 1;
        }
        return WordPart::Param(ParamExpr {
            name,
            op: ParamOp::None,
        });
    }

    let op = read_param_op(chars, i, &name, in_dquote);

    // Check for @X transform after operator
    if *i + 1 < chars.len() && chars[*i] == '@' && chars[*i + 1] != '}' {
        let transform_char = chars[*i + 1];
        if matches!(
            transform_char,
            'E' | 'Q' | 'P' | 'A' | 'a' | 'K' | 'k' | 'L' | 'U' | 'u'
        ) {
            *i += 2;
        }
    }

    // Skip to closing } — handles unrecognized syntax gracefully
    // Skip to closing }, handling nested braces
    let mut depth = 1i32;
    while *i < chars.len() && depth > 0 {
        match chars[*i] {
            '{' => depth += 1,
            '}' => depth -= 1,
            _ => {}
        }
        *i += 1;
    }
    WordPart::Param(ParamExpr { name, op })
}

/// Read a parameter name, including array subscript like `arr[0]` or `arr[@]`.
fn read_param_name_with_subscript(chars: &[char], i: &mut usize) -> String {
    let mut name = read_param_name(chars, i);
    // Check for array subscript [...]
    if *i < chars.len() && chars[*i] == '[' {
        name.push('[');
        *i += 1;
        let mut depth = 1;
        while *i < chars.len() && depth > 0 {
            if chars[*i] == '[' {
                depth += 1;
            } else if chars[*i] == ']' {
                depth -= 1;
                if depth == 0 {
                    name.push(']');
                    *i += 1;
                    break;
                }
            }
            name.push(chars[*i]);
            *i += 1;
        }
    }
    name
}

fn read_param_name(chars: &[char], i: &mut usize) -> String {
    let mut name = String::new();
    if *i < chars.len()
        && (chars[*i] == '@'
            || chars[*i] == '*'
            || chars[*i] == '#'
            || chars[*i] == '?'
            || chars[*i] == '-'
            || chars[*i] == '$'
            || chars[*i] == '!')
    {
        name.push(chars[*i]);
        *i += 1;
    } else if *i < chars.len() && chars[*i].is_ascii_digit() {
        // Read all consecutive digits for positional parameters like ${10}
        while *i < chars.len() && chars[*i].is_ascii_digit() {
            name.push(chars[*i]);
            *i += 1;
        }
    } else {
        while *i < chars.len() && (chars[*i] == '_' || chars[*i].is_alphanumeric()) {
            name.push(chars[*i]);
            *i += 1;
        }
    }
    name
}

/// Warn when a pattern word has $( inside single quotes (would be comsub without quoting)
pub fn warn_incomplete_comsub_in_pattern(word: &Word, lineno: &str) {
    crate::expand::warn_incomplete_comsub_in_pattern_impl(word, lineno);
}

fn read_param_op(chars: &[char], i: &mut usize, _name: &str, in_dquote: bool) -> ParamOp {
    // For pattern operations (#, %, /), $'...' should still be expanded even in heredoc
    let read_word =
        |chars: &[char], i: &mut usize| -> Word { read_param_word_impl(chars, i, '}', in_dquote) };
    let _read_word_until = |chars: &[char], i: &mut usize, delim: char| -> Word {
        read_param_word_impl(chars, i, delim, in_dquote)
    };
    let read_pattern_word = |chars: &[char], i: &mut usize| -> Word {
        // For pattern words: clear IN_HEREDOC and set PATTERN_WORD
        let was_heredoc = IN_HEREDOC.with(|f| f.replace(false));
        let was_pattern = PATTERN_WORD.with(|f| f.replace(true));
        let result = read_param_word_impl(chars, i, '}', in_dquote);
        IN_HEREDOC.with(|f| f.set(was_heredoc));
        PATTERN_WORD.with(|f| f.set(was_pattern));
        result
    };
    let read_pattern_word_until = |chars: &[char], i: &mut usize, delim: char| -> Word {
        let was_heredoc = IN_HEREDOC.with(|f| f.replace(false));
        let was_pattern = PATTERN_WORD.with(|f| f.replace(true));
        let result = read_param_word_impl(chars, i, delim, in_dquote);
        IN_HEREDOC.with(|f| f.set(was_heredoc));
        PATTERN_WORD.with(|f| f.set(was_pattern));
        result
    };

    if *i >= chars.len() {
        return ParamOp::None;
    }

    match chars[*i] {
        ':' => {
            *i += 1;
            if *i >= chars.len() {
                return ParamOp::None;
            }
            match chars[*i] {
                '-' => {
                    *i += 1;
                    let word = read_word(chars, i);
                    ParamOp::Default(true, word)
                }
                '=' => {
                    *i += 1;
                    let word = read_word(chars, i);
                    ParamOp::Assign(true, word)
                }
                '?' => {
                    *i += 1;
                    let word = read_word(chars, i);
                    ParamOp::Error(true, word)
                }
                '+' => {
                    *i += 1;
                    let word = read_word(chars, i);
                    ParamOp::Alt(true, word)
                }
                _ => {
                    // ${var:offset} or ${var:offset:length}
                    let mut offset = String::new();
                    while *i < chars.len() && chars[*i] != ':' && chars[*i] != '}' {
                        offset.push(chars[*i]);
                        *i += 1;
                    }
                    let length = if *i < chars.len() && chars[*i] == ':' {
                        *i += 1;
                        let mut l = String::new();
                        while *i < chars.len() && chars[*i] != '}' {
                            l.push(chars[*i]);
                            *i += 1;
                        }
                        Some(l)
                    } else {
                        None
                    };
                    ParamOp::Substring(offset, length)
                }
            }
        }
        '-' => {
            *i += 1;
            let word = read_word(chars, i);
            ParamOp::Default(false, word)
        }
        '=' => {
            *i += 1;
            let word = read_word(chars, i);
            ParamOp::Assign(false, word)
        }
        '?' => {
            *i += 1;
            let word = read_word(chars, i);
            ParamOp::Error(false, word)
        }
        '+' => {
            *i += 1;
            let word = read_word(chars, i);
            ParamOp::Alt(false, word)
        }
        '#' => {
            *i += 1;
            if *i < chars.len() && chars[*i] == '#' {
                *i += 1;
                let word = read_pattern_word(chars, i);
                // Warn about $(  inside single-quoted pattern parts
                ParamOp::TrimLargeLeft(word)
            } else {
                let word = read_pattern_word(chars, i);
                ParamOp::TrimSmallLeft(word)
            }
        }
        '%' => {
            *i += 1;
            if *i < chars.len() && chars[*i] == '%' {
                *i += 1;
                let word = read_pattern_word(chars, i);
                ParamOp::TrimLargeRight(word)
            } else {
                let word = read_pattern_word(chars, i);
                ParamOp::TrimSmallRight(word)
            }
        }
        '/' => {
            *i += 1;
            let mode = if *i < chars.len() {
                match chars[*i] {
                    '/' => {
                        *i += 1;
                        'a'
                    } // replace all
                    '#' => {
                        *i += 1;
                        'p'
                    } // replace prefix
                    '%' => {
                        *i += 1;
                        's'
                    } // replace suffix
                    _ => 'f', // replace first
                }
            } else {
                'f'
            };
            let pattern = read_pattern_word_until(chars, i, '/');
            let replacement = if *i < chars.len() && chars[*i] == '/' {
                *i += 1;
                read_pattern_word(chars, i)
            } else {
                vec![]
            };
            match mode {
                'a' => ParamOp::ReplaceAll(pattern, replacement),
                'p' => ParamOp::ReplacePrefix(pattern, replacement),
                's' => ParamOp::ReplaceSuffix(pattern, replacement),
                _ => ParamOp::Replace(pattern, replacement),
            }
        }
        '^' => {
            *i += 1;
            if *i < chars.len() && chars[*i] == '^' {
                *i += 1;
                let pattern = read_word(chars, i);
                ParamOp::UpperAll(pattern)
            } else {
                let pattern = read_word(chars, i);
                ParamOp::UpperFirst(pattern)
            }
        }
        ',' => {
            *i += 1;
            if *i < chars.len() && chars[*i] == ',' {
                *i += 1;
                let pattern = read_word(chars, i);
                ParamOp::LowerAll(pattern)
            } else {
                let pattern = read_word(chars, i);
                ParamOp::LowerFirst(pattern)
            }
        }
        '~' => {
            *i += 1;
            if *i < chars.len() && chars[*i] == '~' {
                *i += 1;
                let pattern = read_word(chars, i);
                ParamOp::ToggleAll(pattern)
            } else {
                let pattern = read_word(chars, i);
                ParamOp::ToggleFirst(pattern)
            }
        }
        _ => ParamOp::None,
    }
}

fn read_param_word_impl(chars: &[char], i: &mut usize, delim: char, in_dquote: bool) -> Word {
    let mut parts = Vec::new();
    let mut literal = String::new();
    let mut depth = 0;

    while *i < chars.len() && (chars[*i] != delim || depth > 0) && chars[*i] != '}' {
        match chars[*i] {
            '\\' if *i + 1 < chars.len() => {
                let next = chars[*i + 1];
                if in_dquote
                    && !matches!(next, '$' | '`' | '"' | '\\' | '\n' | '}' | '/')
                    && !(next == '\'' && PATTERN_WORD.with(|f| f.get()))
                {
                    // At top level of param word in dquote, preserve backslash
                    literal.push('\\');
                    literal.push(next);
                } else if !in_dquote {
                    if next == '\n' {
                        // \<newline> is line continuation — discard both
                    } else {
                        // Mark escaped char as SingleQuoted for field splitting
                        if !literal.is_empty() {
                            parts.push(WordPart::Literal(std::mem::take(&mut literal)));
                        }
                        parts.push(WordPart::SingleQuoted(next.to_string()));
                    }
                } else if next == '\\' {
                    // \\ in dquote: produces a quoted literal backslash
                    // Mark as SingleQuoted so pattern matching doesn't treat
                    // it as an escape character (e.g., \\* = literal \ + wildcard *)
                    if !literal.is_empty() {
                        parts.push(WordPart::Literal(std::mem::take(&mut literal)));
                    }
                    parts.push(WordPart::SingleQuoted("\\".to_string()));
                } else if next == '\n' {
                    // \<newline> is line continuation — discard both chars
                } else {
                    literal.push(next);
                }
                *i += 2;
            }
            '$' => {
                if !literal.is_empty() {
                    parts.push(WordPart::Literal(std::mem::take(&mut literal)));
                }
                *i += 1;
                parts.push(parse_dollar(chars, i, in_dquote));
            }
            '\'' if !in_dquote || PATTERN_WORD.with(|f| f.get()) => {
                // Single quotes have quoting effect in unquoted context
                // AND in pattern words (#, %, /) even inside double quotes
                if !literal.is_empty() {
                    parts.push(WordPart::Literal(std::mem::take(&mut literal)));
                }
                *i += 1;
                let mut s = String::new();
                while *i < chars.len() && chars[*i] != '\'' {
                    s.push(chars[*i]);
                    *i += 1;
                }
                if *i < chars.len() {
                    *i += 1;
                }
                parts.push(WordPart::SingleQuoted(s));
            }
            '"' => {
                if !literal.is_empty() {
                    parts.push(WordPart::Literal(std::mem::take(&mut literal)));
                }
                *i += 1;
                let mut dq_parts = Vec::new();
                let mut dq_lit = String::new();
                while *i < chars.len() && chars[*i] != '"' {
                    match chars[*i] {
                        '\\' if *i + 1 < chars.len() => {
                            let next = chars[*i + 1];
                            if matches!(next, '$' | '`' | '"' | '\\') {
                                dq_lit.push(next);
                            } else if next == '\n' {
                                // \<newline> is line continuation — discard both
                            } else if in_dquote
                                && !PATTERN_WORD.with(|f| f.get())
                                && !IN_HEREDOC.with(|f| f.get())
                            {
                                // In nested dquote inside outer-dquoted Default/Alt words,
                                // strip backslash for non-special chars (\' → ')
                                dq_lit.push(next);
                            } else {
                                dq_lit.push('\\');
                                dq_lit.push(next);
                            }
                            *i += 2;
                        }
                        '$' => {
                            if !dq_lit.is_empty() {
                                dq_parts.push(WordPart::Literal(std::mem::take(&mut dq_lit)));
                            }
                            *i += 1;
                            dq_parts.push(parse_dollar(chars, i, true));
                        }
                        '`' => {
                            if !dq_lit.is_empty() {
                                dq_parts.push(WordPart::Literal(std::mem::take(&mut dq_lit)));
                            }
                            *i += 1;
                            let mut cmd = String::new();
                            while *i < chars.len() && chars[*i] != '`' {
                                if chars[*i] == '\\' && *i + 1 < chars.len() {
                                    let next = chars[*i + 1];
                                    if matches!(next, '$' | '`' | '\\' | '"') {
                                        cmd.push(next);
                                        *i += 2;
                                    } else if next == '\n' {
                                        *i += 2; // line continuation
                                    } else {
                                        cmd.push(chars[*i]);
                                        *i += 1;
                                    }
                                } else {
                                    cmd.push(chars[*i]);
                                    *i += 1;
                                }
                            }
                            if *i < chars.len() {
                                *i += 1;
                            }
                            dq_parts.push(WordPart::BacktickSub(cmd));
                        }
                        ch => {
                            dq_lit.push(ch);
                            *i += 1;
                        }
                    }
                }
                if *i < chars.len() {
                    *i += 1;
                }
                if !dq_lit.is_empty() {
                    dq_parts.push(WordPart::Literal(dq_lit));
                }
                parts.push(WordPart::DoubleQuoted(dq_parts));
            }
            '`' => {
                if !literal.is_empty() {
                    parts.push(WordPart::Literal(std::mem::take(&mut literal)));
                }
                *i += 1;
                let mut cmd = String::new();
                while *i < chars.len() && chars[*i] != '`' {
                    if chars[*i] == '\\' && *i + 1 < chars.len() {
                        let next = chars[*i + 1];
                        if matches!(next, '$' | '`' | '\\') {
                            // \\→\, \`→`, \$→$ (processed first)
                            cmd.push(next);
                            *i += 2;
                        } else if next == '\n' {
                            // \<newline> is line continuation — remove both
                            // This applies regardless of quoting context (lexical)
                            *i += 2;
                        } else {
                            cmd.push(chars[*i]);
                            *i += 1;
                        }
                    } else {
                        cmd.push(chars[*i]);
                        *i += 1;
                    }
                }
                if *i < chars.len() {
                    *i += 1; // skip closing `
                }
                parts.push(WordPart::BacktickSub(cmd));
            }
            '{' => {
                depth += 1;
                literal.push(chars[*i]);
                *i += 1;
            }
            '}' if depth > 0 => {
                depth -= 1;
                literal.push(chars[*i]);
                *i += 1;
            }
            ch => {
                literal.push(ch);
                *i += 1;
            }
        }
    }
    if !literal.is_empty() {
        parts.push(WordPart::Literal(literal));
    }
    parts
}

impl Lexer {
    fn read_word(&mut self) -> Token {
        let mut parts = Vec::new();
        let mut literal = String::new();

        loop {
            let ch = match self.peek() {
                None => break,
                Some(c) => c,
            };

            match ch {
                // Extglob patterns: @(...), ?(...), *(...), +(...), !(...)
                '@' | '?' | '+' | '!' if self.peek_at(1) == Some('(') => {
                    literal.push(ch);
                    self.advance(); // consume @/+/?/!
                    literal.push('(');
                    self.advance(); // consume (
                    let mut depth = 1;
                    while let Some(c) = self.peek() {
                        if c == '(' {
                            depth += 1;
                        } else if c == ')' {
                            depth -= 1;
                            if depth == 0 {
                                literal.push(')');
                                self.advance();
                                break;
                            }
                        }
                        literal.push(c);
                        self.advance();
                    }
                    continue;
                }
                // *(pattern) — extglob (distinct from bare *)
                '*' if self.peek_at(1) == Some('(') => {
                    literal.push('*');
                    self.advance(); // consume *
                    literal.push('(');
                    self.advance(); // consume (
                    let mut depth = 1;
                    while let Some(c) = self.peek() {
                        if c == '(' {
                            depth += 1;
                        } else if c == ')' {
                            depth -= 1;
                            if depth == 0 {
                                literal.push(')');
                                self.advance();
                                break;
                            }
                        }
                        literal.push(c);
                        self.advance();
                    }
                    continue;
                }
                // Word terminators
                ' ' | '\t' | '\n' | ';' | '&' | '|' | '(' | ')' => break,
                '<' | '>' => {
                    // Check for process substitution: <(cmd) or >(cmd)
                    if self.peek_at(1) == Some('(') {
                        if !literal.is_empty() {
                            parts.push(WordPart::Literal(std::mem::take(&mut literal)));
                        }
                        let kind = if ch == '<' {
                            ProcessSubKind::Input
                        } else {
                            ProcessSubKind::Output
                        };
                        self.advance(); // consume < or >
                        self.advance(); // consume (
                        let mut depth = 1;
                        let mut cmd = String::new();
                        while let Some(c) = self.peek() {
                            if c == '(' {
                                depth += 1;
                            } else if c == ')' {
                                depth -= 1;
                                if depth == 0 {
                                    self.advance();
                                    break;
                                }
                            }
                            cmd.push(c);
                            self.advance();
                        }
                        parts.push(WordPart::ProcessSub(kind, cmd));
                        continue;
                    }
                    // Check if this is an IO number
                    if !literal.is_empty()
                        && literal.chars().all(|c| c.is_ascii_digit())
                        && parts.is_empty()
                    {
                        break;
                    }
                    break;
                }
                '#' if parts.is_empty() && literal.is_empty() => break,
                '\\' => {
                    self.advance();
                    if let Some(next) = self.advance() {
                        if next == '\n' {
                            // Line continuation - skip
                        } else {
                            // Push escaped char as SingleQuoted so it's treated as
                            // literal in pattern matching (gets \x00 quoting)
                            if !literal.is_empty() {
                                parts.push(WordPart::Literal(std::mem::take(&mut literal)));
                            }
                            parts.push(WordPart::SingleQuoted(next.to_string()));
                        }
                    } else {
                        // \ at EOF — treat as literal backslash
                        literal.push('\\');
                    }
                }
                '\'' => {
                    if !literal.is_empty() {
                        parts.push(WordPart::Literal(std::mem::take(&mut literal)));
                    }
                    self.advance();
                    let mut s = String::new();
                    loop {
                        match self.advance() {
                            None | Some('\'') => break,
                            Some(c) => s.push(c),
                        }
                    }
                    parts.push(WordPart::SingleQuoted(s));
                }
                '"' => {
                    if !literal.is_empty() {
                        parts.push(WordPart::Literal(std::mem::take(&mut literal)));
                    }
                    self.advance();
                    let mut dq_parts = Vec::new();
                    let mut dq_lit = String::new();
                    loop {
                        match self.peek() {
                            None | Some('"') => {
                                self.advance();
                                break;
                            }
                            Some('\\') => {
                                self.advance();
                                match self.peek() {
                                    Some(c @ ('$' | '`' | '"' | '\\' | '\n')) => {
                                        self.advance();
                                        if c != '\n' {
                                            dq_lit.push(c);
                                        }
                                    }
                                    Some(c) => {
                                        dq_lit.push('\\');
                                        dq_lit.push(c);
                                        self.advance();
                                    }
                                    None => dq_lit.push('\\'),
                                }
                            }
                            Some('$') => {
                                // Inside double quotes, $' and $" are literal
                                if matches!(self.peek_at(1), Some('\'' | '"')) {
                                    dq_lit.push('$');
                                    self.advance();
                                } else {
                                    if !dq_lit.is_empty() {
                                        dq_parts
                                            .push(WordPart::Literal(std::mem::take(&mut dq_lit)));
                                    }
                                    self.advance();
                                    let input_clone = self.input.clone();
                                    let part = parse_dollar(&input_clone, &mut self.pos, true);
                                    dq_parts.push(part);
                                }
                            }
                            Some('`') => {
                                if !dq_lit.is_empty() {
                                    dq_parts.push(WordPart::Literal(std::mem::take(&mut dq_lit)));
                                }
                                self.advance();
                                let mut cmd = String::new();
                                loop {
                                    match self.peek() {
                                        None | Some('`') => {
                                            self.advance();
                                            break;
                                        }
                                        Some('\\') => {
                                            self.advance();
                                            match self.peek() {
                                                // Only these chars are special after \ in double-quoted backtick
                                                Some(c @ ('$' | '\\' | '`' | '"')) => {
                                                    cmd.push(c);
                                                    self.advance();
                                                }
                                                Some('\n') => {
                                                    // \<newline> line continuation
                                                    self.advance();
                                                }
                                                Some(c) => {
                                                    cmd.push('\\');
                                                    cmd.push(c);
                                                    self.advance();
                                                }
                                                None => cmd.push('\\'),
                                            }
                                        }
                                        Some(c) => {
                                            cmd.push(c);
                                            self.advance();
                                        }
                                    }
                                }
                                dq_parts.push(WordPart::BacktickSub(cmd));
                            }
                            Some(c) => {
                                dq_lit.push(c);
                                self.advance();
                            }
                        }
                    }
                    if !dq_lit.is_empty() {
                        dq_parts.push(WordPart::Literal(dq_lit));
                    }
                    parts.push(WordPart::DoubleQuoted(dq_parts));
                }
                '$' => {
                    if !literal.is_empty() {
                        parts.push(WordPart::Literal(std::mem::take(&mut literal)));
                    }
                    self.advance();
                    if self.peek() == Some('"') {
                        // $"..." locale-specific quoting (treated as regular double quoting)
                        self.advance(); // consume "
                        let mut dq_parts = Vec::new();
                        let mut dq_literal = String::new();
                        loop {
                            match self.peek() {
                                None | Some('"') => {
                                    self.advance();
                                    break;
                                }
                                Some('\\') => {
                                    self.advance();
                                    match self.advance() {
                                        Some(c @ ('$' | '`' | '"' | '\\')) => {
                                            dq_literal.push(c);
                                        }
                                        Some(c) => {
                                            dq_literal.push('\\');
                                            dq_literal.push(c);
                                        }
                                        None => dq_literal.push('\\'),
                                    }
                                }
                                Some('$') => {
                                    if !dq_literal.is_empty() {
                                        dq_parts.push(WordPart::Literal(std::mem::take(
                                            &mut dq_literal,
                                        )));
                                    }
                                    self.advance();
                                    let input_clone = self.input.clone();
                                    let part = parse_dollar(&input_clone, &mut self.pos, true);
                                    dq_parts.push(part);
                                }
                                Some('`') => {
                                    if !dq_literal.is_empty() {
                                        dq_parts.push(WordPart::Literal(std::mem::take(
                                            &mut dq_literal,
                                        )));
                                    }
                                    self.advance();
                                    let mut cmd = String::new();
                                    loop {
                                        match self.peek() {
                                            None | Some('`') => {
                                                self.advance();
                                                break;
                                            }
                                            Some(c) => {
                                                cmd.push(c);
                                                self.advance();
                                            }
                                        }
                                    }
                                    dq_parts.push(WordPart::BacktickSub(cmd));
                                }
                                Some(c) => {
                                    dq_literal.push(c);
                                    self.advance();
                                }
                            }
                        }
                        if !dq_literal.is_empty() {
                            dq_parts.push(WordPart::Literal(dq_literal));
                        }
                        parts.push(WordPart::DoubleQuoted(dq_parts));
                    } else if self.peek() == Some('\'') {
                        // $'...' ANSI-C quoting
                        self.advance();
                        let mut s = String::new();
                        let mut nul_terminated = false;
                        loop {
                            match self.advance() {
                                None | Some('\'') => break,
                                Some('\\') => match self.advance() {
                                    Some('n') => s.push('\n'),
                                    Some('t') => s.push('\t'),
                                    Some('r') => s.push('\r'),
                                    Some('\\') => s.push('\\'),
                                    Some('\'') => s.push('\''),
                                    Some('"') => s.push('"'),
                                    Some('a') => s.push('\x07'),
                                    Some('b') => s.push('\x08'),
                                    Some('c') => {
                                        // \cX — control character (X ^ 0x40), like bash
                                        // If next char is \, process the escape first
                                        if let Some(ch) = self.advance() {
                                            let target_char = if ch == '\\' {
                                                self.advance().unwrap_or('\\')
                                            } else {
                                                ch
                                            };
                                            let ctrl = (target_char as u8) ^ 0x40;
                                            if ctrl == 0 {
                                                nul_terminated = true;
                                                break;
                                            }
                                            s.push(ctrl as char);
                                        }
                                    }
                                    Some('e') | Some('E') => s.push('\x1b'),
                                    Some('f') => s.push('\x0c'),
                                    Some('v') => s.push('\x0b'),
                                    Some(oc @ '0'..='7') => {
                                        let mut val = oc as u8 - b'0';
                                        for _ in 0..2 {
                                            match self.peek() {
                                                Some(c @ '0'..='7') => {
                                                    val = val * 8 + (c as u8 - b'0');
                                                    self.advance();
                                                }
                                                _ => break,
                                            }
                                        }
                                        if val == 0 {
                                            nul_terminated = true;
                                            break; // NUL terminates string
                                        }
                                        s.push(val as char);
                                    }
                                    Some('x') => {
                                        let mut val = 0u32;
                                        let mut count = 0;
                                        let mut braced = false;
                                        // \x{NN} or \xNN (up to 2 hex digits without braces)
                                        if self.peek() == Some('{') {
                                            braced = true;
                                            self.advance(); // consume {
                                            while let Some(c) = self.peek() {
                                                if c == '}' {
                                                    self.advance();
                                                    break;
                                                }
                                                if c.is_ascii_hexdigit() {
                                                    val = val * 16 + c.to_digit(16).unwrap();
                                                    self.advance();
                                                    count += 1;
                                                } else {
                                                    break;
                                                }
                                            }
                                        } else {
                                            for _ in 0..2 {
                                                match self.peek() {
                                                    Some(c) if c.is_ascii_hexdigit() => {
                                                        val = val * 16 + c.to_digit(16).unwrap();
                                                        self.advance();
                                                        count += 1;
                                                    }
                                                    _ => break,
                                                }
                                            }
                                        }
                                        if count > 0 || braced {
                                            // \x produces single bytes (truncate to 0xFF)
                                            let byte_val = (val & 0xFF) as u8;
                                            if byte_val == 0 {
                                                nul_terminated = true;
                                                break;
                                            }
                                            s.push(byte_val as char);
                                        } else {
                                            s.push('\\');
                                            s.push('x');
                                        }
                                    }
                                    Some('u') => {
                                        let mut val = 0u32;
                                        let mut count = 0;
                                        if self.peek() == Some('{') {
                                            self.advance();
                                            while let Some(c) = self.peek() {
                                                if c == '}' {
                                                    self.advance();
                                                    break;
                                                }
                                                if c.is_ascii_hexdigit() {
                                                    val = val * 16 + c.to_digit(16).unwrap();
                                                    self.advance();
                                                    count += 1;
                                                } else {
                                                    break;
                                                }
                                            }
                                        } else {
                                            for _ in 0..4 {
                                                match self.peek() {
                                                    Some(c) if c.is_ascii_hexdigit() => {
                                                        val = val * 16 + c.to_digit(16).unwrap();
                                                        self.advance();
                                                        count += 1;
                                                    }
                                                    _ => break,
                                                }
                                            }
                                        }
                                        if count > 0 {
                                            if let Some(c) = char::from_u32(val) {
                                                s.push(c);
                                            }
                                        } else {
                                            s.push('\\');
                                            s.push('u');
                                        }
                                    }
                                    Some('U') => {
                                        let mut val = 0u32;
                                        let mut count = 0;
                                        if self.peek() == Some('{') {
                                            self.advance();
                                            while let Some(c) = self.peek() {
                                                if c == '}' {
                                                    self.advance();
                                                    break;
                                                }
                                                if c.is_ascii_hexdigit() {
                                                    val = val * 16 + c.to_digit(16).unwrap();
                                                    self.advance();
                                                    count += 1;
                                                } else {
                                                    break;
                                                }
                                            }
                                        } else {
                                            for _ in 0..8 {
                                                match self.peek() {
                                                    Some(c) if c.is_ascii_hexdigit() => {
                                                        val = val * 16 + c.to_digit(16).unwrap();
                                                        self.advance();
                                                        count += 1;
                                                    }
                                                    _ => break,
                                                }
                                            }
                                        }
                                        if count > 0 {
                                            if let Some(c) = char::from_u32(val) {
                                                s.push(c);
                                            }
                                        } else {
                                            s.push('\\');
                                            s.push('U');
                                        }
                                    }
                                    Some(c) => {
                                        s.push('\\');
                                        s.push(c);
                                    }
                                    None => s.push('\\'),
                                },
                                Some(c) => s.push(c),
                            }
                        }
                        // If NUL-terminated, skip to closing quote
                        if nul_terminated {
                            while let Some(c) = self.peek() {
                                self.advance();
                                if c == '\'' {
                                    break;
                                }
                            }
                        }
                        parts.push(WordPart::SingleQuoted(s));
                    } else {
                        let input_clone = self.input.clone();
                        let part = parse_dollar(&input_clone, &mut self.pos, false);
                        parts.push(part);
                    }
                }
                '`' => {
                    if !literal.is_empty() {
                        parts.push(WordPart::Literal(std::mem::take(&mut literal)));
                    }
                    self.advance();
                    let mut cmd = String::new();
                    loop {
                        match self.peek() {
                            None | Some('`') => {
                                self.advance();
                                break;
                            }
                            Some('\\') => {
                                self.advance();
                                if let Some(c) = self.advance() {
                                    if matches!(c, '$' | '`' | '\\') {
                                        cmd.push(c);
                                    } else if c == '\n' {
                                        // \<newline> is line continuation — remove both
                                    } else {
                                        cmd.push('\\');
                                        cmd.push(c);
                                    }
                                }
                            }
                            Some(c) => {
                                cmd.push(c);
                                self.advance();
                            }
                        }
                    }
                    parts.push(WordPart::BacktickSub(cmd));
                }
                '~' if parts.is_empty() && literal.is_empty() => {
                    let _tilde_pos = self.pos;
                    self.advance();
                    let mut user = String::new();
                    let mut valid_tilde = true;
                    // Check for ~+ and ~- first
                    if let Some(c) = self.peek() {
                        if (c == '+' || c == '-')
                            && !self
                                .input
                                .get(self.pos + 1)
                                .is_some_and(|&nc| nc.is_alphanumeric() || nc == '_')
                        {
                            user.push(c);
                            self.advance();
                        } else {
                            while let Some(c) = self.peek() {
                                if c.is_alphanumeric() || c == '_' || c == '-' || c == '.' {
                                    user.push(c);
                                    self.advance();
                                } else {
                                    break;
                                }
                            }
                        }
                    }
                    // Tilde prefix is only valid if followed by /, :, or end of word
                    if let Some(next) = self.peek()
                        && next != '/'
                        && next != ':'
                        && !next.is_whitespace()
                        && next != ';'
                        && next != '|'
                        && next != '&'
                        && next != ')'
                        && next != '}'
                        && next != '\n'
                    {
                        valid_tilde = false;
                    }
                    if valid_tilde {
                        parts.push(WordPart::Tilde(user));
                    } else {
                        // Revert: treat ~ and consumed chars as literal
                        literal.push('~');
                        literal.push_str(&user);
                    }
                }
                c => {
                    literal.push(c);
                    self.advance();
                }
            }
        }

        if !literal.is_empty() {
            parts.push(WordPart::Literal(literal));
        }

        if parts.is_empty() {
            Token::Eof
        } else {
            Token::Word(parts)
        }
    }

    /// Read raw text until `))` is found (for arithmetic commands).
    /// Skip a `$(...)` command substitution starting at the `(` after `$`.
    /// Handles case/esac, quotes, nested $(), and backticks.
    /// Returns the consumed text including the outer `(...)`.
    fn skip_comsub(&mut self) -> String {
        let mut s = String::new();
        // self.pos is at the '(' of '$('
        s.push(self.input[self.pos]); // '('
        self.pos += 1;
        let mut depth = 1i32;
        let mut case_depth = 0i32;
        while self.pos < self.input.len() && depth > 0 {
            let ch = self.input[self.pos];
            match ch {
                '\'' => {
                    // Single-quoted string — skip entirely
                    s.push(ch);
                    self.pos += 1;
                    while self.pos < self.input.len() && self.input[self.pos] != '\'' {
                        s.push(self.input[self.pos]);
                        self.pos += 1;
                    }
                    if self.pos < self.input.len() {
                        s.push(self.input[self.pos]);
                        self.pos += 1;
                    }
                    continue;
                }
                '"' => {
                    // Double-quoted string — skip but handle escapes
                    s.push(ch);
                    self.pos += 1;
                    while self.pos < self.input.len() && self.input[self.pos] != '"' {
                        if self.input[self.pos] == '\\' && self.pos + 1 < self.input.len() {
                            s.push(self.input[self.pos]);
                            self.pos += 1;
                        }
                        s.push(self.input[self.pos]);
                        self.pos += 1;
                    }
                    if self.pos < self.input.len() {
                        s.push(self.input[self.pos]);
                        self.pos += 1;
                    }
                    continue;
                }
                '`' => {
                    // Backtick command sub — skip
                    s.push(ch);
                    self.pos += 1;
                    while self.pos < self.input.len() && self.input[self.pos] != '`' {
                        if self.input[self.pos] == '\\' && self.pos + 1 < self.input.len() {
                            s.push(self.input[self.pos]);
                            self.pos += 1;
                        }
                        s.push(self.input[self.pos]);
                        self.pos += 1;
                    }
                    if self.pos < self.input.len() {
                        s.push(self.input[self.pos]);
                        self.pos += 1;
                    }
                    continue;
                }
                '(' => depth += 1,
                ')' => {
                    if case_depth <= 0 {
                        depth -= 1;
                        if depth == 0 {
                            s.push(ch);
                            self.pos += 1;
                            return s;
                        }
                    }
                    // Inside a case block, ) is a pattern delimiter — skip
                }
                _ => {}
            }
            // Track case/esac keywords
            if ch.is_alphabetic() {
                let mut word = String::new();
                while self.pos < self.input.len()
                    && (self.input[self.pos].is_alphanumeric() || self.input[self.pos] == '_')
                {
                    word.push(self.input[self.pos]);
                    self.pos += 1;
                }
                // Check for case/esac keywords, also through aliases
                let effective_word = if self.shopt_expand_aliases {
                    self.aliases
                        .get(word.as_str())
                        .map(|v| v.trim().to_string())
                        .unwrap_or_else(|| word.clone())
                } else {
                    word.clone()
                };
                if effective_word == "case" {
                    case_depth += 1;
                } else if effective_word == "esac" || word == "esac" {
                    case_depth -= 1;
                } else if effective_word == "(" {
                    depth += 1;
                }
                s.push_str(&word);
                continue;
            }
            s.push(ch);
            self.pos += 1;
        }
        s
    }

    /// Skip a `${ ... }` funsub starting at the `{` after `$`.
    /// Returns the consumed text including the outer `{...}`.
    fn skip_funsub(&mut self) -> String {
        let mut s = String::new();
        // self.pos is at the '{' of '${'
        s.push(self.input[self.pos]); // '{'
        self.pos += 1;
        // Skip whitespace after '{' to confirm it's a funsub (has space)
        let mut depth = 1i32;
        while self.pos < self.input.len() && depth > 0 {
            let ch = self.input[self.pos];
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        s.push(ch);
                        self.pos += 1;
                        return s;
                    }
                }
                '\'' => {
                    s.push(ch);
                    self.pos += 1;
                    while self.pos < self.input.len() && self.input[self.pos] != '\'' {
                        s.push(self.input[self.pos]);
                        self.pos += 1;
                    }
                    if self.pos < self.input.len() {
                        s.push(self.input[self.pos]);
                        self.pos += 1;
                    }
                    continue;
                }
                '"' => {
                    s.push(ch);
                    self.pos += 1;
                    while self.pos < self.input.len() && self.input[self.pos] != '"' {
                        if self.input[self.pos] == '\\' && self.pos + 1 < self.input.len() {
                            s.push(self.input[self.pos]);
                            self.pos += 1;
                        }
                        s.push(self.input[self.pos]);
                        self.pos += 1;
                    }
                    if self.pos < self.input.len() {
                        s.push(self.input[self.pos]);
                        self.pos += 1;
                    }
                    continue;
                }
                _ => {}
            }
            s.push(ch);
            self.pos += 1;
        }
        s
    }

    /// The `((` has already been consumed by the parser.
    pub fn read_until_double_paren(&mut self) -> Result<String, String> {
        let mut expr = String::new();
        let mut paren_depth = 0; // Track nested ( ) inside the expression
        while self.pos < self.input.len() {
            let ch = self.input[self.pos];
            // Handle $(...) and ${ ... } — use proper parsers
            if ch == '$' && self.pos + 1 < self.input.len() {
                if self.input[self.pos + 1] == '(' {
                    expr.push(ch);
                    self.pos += 1;
                    expr.push_str(&self.skip_comsub());
                    continue;
                }
                if self.input[self.pos + 1] == '{'
                    && self.pos + 2 < self.input.len()
                    && self.input[self.pos + 2] == ' '
                {
                    expr.push(ch);
                    self.pos += 1;
                    expr.push_str(&self.skip_funsub());
                    continue;
                }
            }
            if ch == '(' {
                paren_depth += 1;
                expr.push(ch);
                self.pos += 1;
            } else if ch == ')' {
                if paren_depth > 0 {
                    // Close an inner paren
                    paren_depth -= 1;
                    expr.push(ch);
                    self.pos += 1;
                } else {
                    // At top level — check if this starts the closing ))
                    // Skip whitespace after first ) to find second )
                    self.pos += 1;
                    let saved = self.pos;
                    while self.pos < self.input.len() && matches!(self.input[self.pos], ' ' | '\t')
                    {
                        self.pos += 1;
                    }
                    if self.pos < self.input.len() && self.input[self.pos] == ')' {
                        // Found )) (possibly with whitespace between)
                        self.pos += 1;
                        return Ok(expr.trim_start().to_string());
                    }
                    // Not )), restore position and treat as expression
                    self.pos = saved;
                    expr.push(')');
                }
            } else if ch == ';' && paren_depth == 0 {
                return Err("syntax error: `;' unexpected".to_string());
            } else {
                if ch == '\n' {
                    self.line += 1;
                }
                expr.push(ch);
                self.pos += 1;
            }
        }
        Err("unexpected EOF while looking for matching `)'".to_string())
    }

    /// Read raw text until the given character is found (for C-style for loops).
    pub fn read_until_char(&mut self, target: char) -> Result<String, String> {
        let mut s = String::new();
        let mut depth = 0i32;
        while self.pos < self.input.len() {
            let ch = self.input[self.pos];
            // Handle $((...)) and $(...) — use proper parsers
            if ch == '$' && self.pos + 1 < self.input.len() {
                if self.input[self.pos + 1] == '('
                    && self.pos + 2 < self.input.len()
                    && self.input[self.pos + 2] == '('
                {
                    // $((arith)) — skip arithmetic expansion
                    s.push_str("$((");
                    self.pos += 3;
                    let mut arith_depth = 1i32;
                    while self.pos < self.input.len() && arith_depth > 0 {
                        let c = self.input[self.pos];
                        if c == '$'
                            && self.pos + 1 < self.input.len()
                            && self.input[self.pos + 1] == '('
                        {
                            // Nested $( — skip comsub inside arithmetic
                            s.push('$');
                            self.pos += 1;
                            s.push_str(&self.skip_comsub());
                            continue;
                        } else if c == ')'
                            && self.pos + 1 < self.input.len()
                            && self.input[self.pos + 1] == ')'
                        {
                            arith_depth -= 1;
                            if arith_depth == 0 {
                                s.push_str("))");
                                self.pos += 2;
                                break;
                            }
                        }
                        s.push(c);
                        self.pos += 1;
                    }
                    continue;
                }
                if self.input[self.pos + 1] == '(' {
                    s.push(ch);
                    self.pos += 1;
                    s.push_str(&self.skip_comsub());
                    continue;
                }
                if self.input[self.pos + 1] == '{'
                    && self.pos + 2 < self.input.len()
                    && self.input[self.pos + 2] == ' '
                {
                    s.push(ch);
                    self.pos += 1;
                    s.push_str(&self.skip_funsub());
                    continue;
                }
            }
            if ch == '(' {
                depth += 1;
            } else if ch == ')' {
                depth -= 1;
            }
            if ch == target && depth == 0 {
                self.pos += 1; // consume the delimiter
                return Ok(s.trim_start().to_string());
            }
            s.push(ch);
            self.pos += 1;
        }
        Err(format!("unexpected EOF looking for '{}'", target))
    }

    /// Get the next heredoc body (called by the parser when processing heredoc redirections).
    pub fn take_heredoc_body(&mut self) -> Option<Word> {
        if self.heredoc_index < self.heredoc_bodies.len() {
            let body = self.heredoc_bodies[self.heredoc_index].clone();
            self.heredoc_index += 1;
            Some(body)
        } else {
            None
        }
    }

    pub fn take_heredoc_delimiter(&mut self) -> Option<String> {
        if !self.heredoc_delimiters.is_empty() {
            Some(self.heredoc_delimiters.remove(0))
        } else {
            None
        }
    }
}
