use crate::ast::*;
use crate::lexer::{Lexer, Token};
use std::collections::HashMap;

/// Result of parsing a command substitution with full recursive parse.
/// Like C bash's `parse_comsub`/`xparse_dolparen`, this runs the real parser
/// on the comsub contents so that case/esac, done/while, etc. are handled
/// correctly instead of relying on a character-level state machine.
pub struct ComsubParseResult {
    /// The text of the command substitution (between `$(` and `)`)
    pub text: String,
    /// Total characters consumed from the input (including the closing `)`)
    pub chars_consumed: usize,
    /// Whether the parse encountered a syntax error inside the comsub
    pub syntax_error: Option<String>,
    /// Heredoc EOF warnings from the comsub parse
    pub heredoc_eof_warnings: Vec<(usize, usize, String)>,
    /// Whether the comsub was incomplete (no closing `)` found)
    pub incomplete: bool,
    /// Number of unterminated here-documents inside the comsub (for warning)
    pub unterminated_heredoc_count: usize,
}

/// Hybrid comsub parser: uses a character-level scan to find the `$(...)` boundary
/// (handling quotes, case/esac, heredocs, nested constructs), then runs the real
/// parser on the bounded text to validate it and detect syntax errors — just like
/// C bash's `parse_comsub`/`xparse_dolparen` which calls `yyparse()`.
///
/// `input` is the text starting AFTER the `$(` — i.e. the first char is the
/// beginning of the command.  `in_dquote` indicates whether the `$(` appeared
/// inside double quotes (needed for `}` handling inside `${...}`).
///
/// `aliases` and flags are forwarded to the validation sub-parser.
pub fn parse_comsub(
    input: &str,
    aliases: HashMap<String, String>,
    _expand_aliases: bool,
    _posix_mode: bool,
    in_dquote: bool,
    line_offset: usize,
) -> ComsubParseResult {
    // Phase 1: character-level scan to find the comsub boundary.
    // This is the old proven approach that correctly handles quotes, case/esac,
    // heredocs, nested $(), `}` inside ${...}, etc.
    let chars: Vec<char> = input.chars().collect();
    let boundary = find_comsub_boundary(&chars, in_dquote, &aliases, line_offset);

    match boundary {
        ComsubBoundary::Closed {
            text,
            chars_consumed,
            heredoc_eof_warnings,
            unterminated_heredoc_count,
        } => {
            // Phase 2: validate the bounded text with a full recursive parse.
            // This catches errors like `done` in case body, `in` at command
            // position, etc. that the character scanner cannot detect.
            let syntax_error = validate_comsub_text(&text, &aliases);

            ComsubParseResult {
                text,
                chars_consumed,
                syntax_error,
                heredoc_eof_warnings,
                incomplete: false,
                unterminated_heredoc_count,
            }
        }
        ComsubBoundary::Incomplete {
            heredoc_eof_warnings,
        } => {
            let text = input.to_string();
            ComsubParseResult {
                text,
                chars_consumed: chars.len(),
                syntax_error: None, // incomplete is signalled via INCOMPLETE_COMSUB marker
                heredoc_eof_warnings,
                incomplete: true,
                unterminated_heredoc_count: 0,
            }
        }
        ComsubBoundary::SilentClose { chars_scanned } => {
            // `}` at comsub depth 1 in dquote — the enclosing ${...} closes.
            // chars_scanned tells the caller how far to advance past the
            // comsub text (up to but not including the `}`).
            ComsubParseResult {
                text: "\x00SILENT_COMSUB".to_string(),
                chars_consumed: chars_scanned,
                syntax_error: None,
                heredoc_eof_warnings: Vec::new(),
                incomplete: false,
                unterminated_heredoc_count: 0,
            }
        }
    }
}

enum ComsubBoundary {
    /// Found closing `)` — comsub text and total chars consumed (including `)`)
    Closed {
        text: String,
        chars_consumed: usize,
        heredoc_eof_warnings: Vec<(usize, usize, String)>,
        /// How many unterminated here-documents were in the comsub
        unterminated_heredoc_count: usize,
    },
    /// No closing `)` found — incomplete comsub
    Incomplete {
        heredoc_eof_warnings: Vec<(usize, usize, String)>,
    },
    /// `}` in dquote at depth 1 — silent close for enclosing `${...}`
    /// `chars_scanned` is how many chars were consumed before hitting `}`.
    SilentClose { chars_scanned: usize },
}

/// Character-level scan to find the comsub boundary.  This is the battle-tested
/// scanner that handles all the tricky edge cases (quotes, case/esac, heredocs,
/// `}` inside `${...}`, nested `$()`, backticks, comments, etc.).
///
/// `line_offset` is the number of newlines before the `$(` in the original input,
/// so that heredoc warnings report correct line numbers relative to the script.
fn find_comsub_boundary(
    chars: &[char],
    in_dquote: bool,
    aliases: &HashMap<String, String>,
    line_offset: usize,
) -> ComsubBoundary {
    let mut i = 0;
    let mut depth = 1i32;
    let mut brace_depth = 0i32;
    let mut cmd = String::new();
    let mut case_depth = 0i32;
    let mut case_paren_depth = Vec::<i32>::new();
    let mut case_action_stack = Vec::<bool>::new();
    let mut in_case_action = false;
    let mut compound_depth = 0i32;
    let mut heredoc_eof_warnings = Vec::new();
    let mut pending_heredocs: Vec<(String, bool, bool, usize)> = Vec::new();

    while i < chars.len() && depth > 0 {
        match chars[i] {
            '\'' => {
                cmd.push(chars[i]);
                i += 1;
                while i < chars.len() && chars[i] != '\'' {
                    cmd.push(chars[i]);
                    i += 1;
                }
                if i < chars.len() {
                    cmd.push(chars[i]);
                    i += 1;
                }
                continue;
            }
            '"' => {
                cmd.push(chars[i]);
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        cmd.push(chars[i]);
                        i += 1;
                        cmd.push(chars[i]);
                        i += 1;
                        continue;
                    }
                    if chars[i] == '$' && i + 1 < chars.len() {
                        if chars[i + 1] == '(' {
                            cmd.push(chars[i]);
                            i += 1;
                            cmd.push(chars[i]);
                            i += 1;
                            let mut inner_depth = 1i32;
                            while i < chars.len() && inner_depth > 0 {
                                match chars[i] {
                                    '(' => inner_depth += 1,
                                    ')' => {
                                        inner_depth -= 1;
                                        if inner_depth == 0 {
                                            cmd.push(chars[i]);
                                            i += 1;
                                            break;
                                        }
                                    }
                                    '"' => {
                                        cmd.push(chars[i]);
                                        i += 1;
                                        while i < chars.len() && chars[i] != '"' {
                                            if chars[i] == '\\' && i + 1 < chars.len() {
                                                cmd.push(chars[i]);
                                                i += 1;
                                            }
                                            cmd.push(chars[i]);
                                            i += 1;
                                        }
                                        if i < chars.len() {
                                            cmd.push(chars[i]);
                                            i += 1;
                                        }
                                        continue;
                                    }
                                    '\'' => {
                                        cmd.push(chars[i]);
                                        i += 1;
                                        while i < chars.len() && chars[i] != '\'' {
                                            cmd.push(chars[i]);
                                            i += 1;
                                        }
                                        if i < chars.len() {
                                            cmd.push(chars[i]);
                                            i += 1;
                                        }
                                        continue;
                                    }
                                    _ => {}
                                }
                                if i < chars.len() && inner_depth > 0 {
                                    cmd.push(chars[i]);
                                    i += 1;
                                }
                            }
                            continue;
                        } else if chars[i + 1] == '{' {
                            cmd.push(chars[i]);
                            i += 1;
                            cmd.push(chars[i]);
                            i += 1;
                            let mut inner_depth = 1i32;
                            while i < chars.len() && inner_depth > 0 {
                                match chars[i] {
                                    '{' => inner_depth += 1,
                                    '}' => {
                                        inner_depth -= 1;
                                        if inner_depth == 0 {
                                            cmd.push(chars[i]);
                                            i += 1;
                                            break;
                                        }
                                    }
                                    _ => {}
                                }
                                if i < chars.len() && inner_depth > 0 {
                                    cmd.push(chars[i]);
                                    i += 1;
                                }
                            }
                            continue;
                        }
                    }
                    cmd.push(chars[i]);
                    i += 1;
                }
                if i < chars.len() {
                    cmd.push(chars[i]);
                    i += 1;
                }
                continue;
            }
            '`' => {
                cmd.push(chars[i]);
                i += 1;
                while i < chars.len() && chars[i] != '`' {
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        cmd.push(chars[i]);
                        i += 1;
                    }
                    cmd.push(chars[i]);
                    i += 1;
                }
                if i < chars.len() {
                    cmd.push(chars[i]);
                    i += 1;
                }
                continue;
            }
            '(' => {
                let preceded_by_dollar = cmd.ends_with('$') || cmd.ends_with("$(");
                if case_depth > 0 && !in_case_action && !preceded_by_dollar {
                    // ( in case pattern context — pattern delimiter, skip
                } else {
                    depth += 1;
                }
            }
            ')' => {
                if case_depth > 0 && !in_case_action {
                    in_case_action = true;
                } else if compound_depth > 0 && depth <= 1 {
                    depth = 0;
                    break;
                } else {
                    depth -= 1;
                    if depth == 0 {
                        i += 1;
                        break;
                    }
                }
            }
            '}' if brace_depth > 0 => {
                brace_depth -= 1;
            }
            '}' if in_dquote && depth == 1 => {
                return ComsubBoundary::SilentClose { chars_scanned: i };
            }
            '$' if i + 1 < chars.len() && chars[i + 1] == '{' => {
                cmd.push(chars[i]);
                i += 1;
                cmd.push(chars[i]);
                i += 1;
                brace_depth += 1;
                continue;
            }
            '#' if cmd.is_empty()
                || cmd.ends_with('\n')
                || (cmd.ends_with(";;") || (cmd.ends_with(';') && !cmd.ends_with("\\;"))) =>
            {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            '\\' if i + 1 < chars.len() && chars[i + 1] == '\n' => {
                i += 2;
                continue;
            }
            '\\' if i + 1 < chars.len() => {
                cmd.push(chars[i]);
                i += 1;
                cmd.push(chars[i]);
                i += 1;
                continue;
            }
            '<' if i + 2 < chars.len() && chars[i + 1] == '<' && chars[i + 2] == '<' => {
                cmd.push(chars[i]);
                i += 1;
                cmd.push(chars[i]);
                i += 1;
                cmd.push(chars[i]);
                i += 1;
                continue;
            }
            '<' if i + 1 < chars.len()
                && chars[i + 1] == '<'
                && (i + 2 >= chars.len() || chars[i + 2] != '<') =>
            {
                let hd_start = i;
                cmd.push(chars[i]);
                i += 1;
                cmd.push(chars[i]);
                i += 1;
                let strip_tabs = i < chars.len() && chars[i] == '-';
                if strip_tabs {
                    cmd.push(chars[i]);
                    i += 1;
                }
                while i < chars.len() && (chars[i] == ' ' || chars[i] == '\t') {
                    cmd.push(chars[i]);
                    i += 1;
                }
                let mut delim = String::new();
                let mut _heredoc_quoted = false;
                while i < chars.len()
                    && chars[i] != '\n'
                    && chars[i] != ' '
                    && chars[i] != '\t'
                    && chars[i] != ';'
                    && chars[i] != '&'
                    && chars[i] != ')'
                    && chars[i] != '|'
                {
                    let ch = chars[i];
                    if ch == '\'' {
                        _heredoc_quoted = true;
                        cmd.push(ch);
                        i += 1;
                        while i < chars.len() && chars[i] != '\'' {
                            delim.push(chars[i]);
                            cmd.push(chars[i]);
                            i += 1;
                        }
                        if i < chars.len() {
                            cmd.push(chars[i]);
                            i += 1;
                        }
                    } else if ch == '"' {
                        _heredoc_quoted = true;
                        cmd.push(ch);
                        i += 1;
                        while i < chars.len() && chars[i] != '"' {
                            delim.push(chars[i]);
                            cmd.push(chars[i]);
                            i += 1;
                        }
                        if i < chars.len() {
                            cmd.push(chars[i]);
                            i += 1;
                        }
                    } else if ch == '\\' {
                        _heredoc_quoted = true;
                        cmd.push(ch);
                        i += 1;
                        if i < chars.len() && chars[i] == '\n' {
                            cmd.push(chars[i]);
                            i += 1;
                        } else if i < chars.len() {
                            delim.push(chars[i]);
                            cmd.push(chars[i]);
                            i += 1;
                        }
                    } else {
                        delim.push(ch);
                        cmd.push(ch);
                        i += 1;
                    }
                }
                // Read remaining tokens on the same line after the heredoc
                // delimiter, but watch for `)` that closes the comsub.
                let mut comsub_closed_by_paren = false;
                while i < chars.len() && chars[i] != '\n' {
                    if chars[i] == ')' {
                        // Check if this `)` closes the comsub (depth 1)
                        if depth == 1 && case_depth == 0 {
                            // The `)` closes the comsub — but the heredoc
                            // body follows on subsequent lines in the outer
                            // input.  Read the body and embed it in the
                            // comsub text so the inner parser sees a
                            // complete `cat << EOF\nbody\nEOF`.
                            //
                            // Record unterminated-heredoc warning (bash
                            // warns about this even though it works).
                            pending_heredocs.push((
                                delim.clone(),
                                strip_tabs,
                                _heredoc_quoted,
                                0, // unused
                            ));

                            // Consume the `)` — it closes the comsub
                            i += 1;
                            depth = 0;

                            // Skip the rest of the line after `)` in the
                            // outer input (consumed but NOT part of comsub)
                            while i < chars.len() && chars[i] != '\n' {
                                i += 1;
                            }
                            // Skip the newline
                            if i < chars.len() && chars[i] == '\n' {
                                i += 1;
                            }

                            // Now read the heredoc body from the outer
                            // input and append it to the comsub text.
                            cmd.push('\n');
                            if !delim.is_empty() {
                                loop {
                                    if i >= chars.len() {
                                        // EOF before delimiter found
                                        let current_line =
                                            chars[..i].iter().filter(|&&c| c == '\n').count();
                                        let heredoc_start_line = chars[..hd_start]
                                            .iter()
                                            .filter(|&&c| c == '\n')
                                            .count()
                                            + 1;
                                        heredoc_eof_warnings.push((
                                            current_line,
                                            heredoc_start_line,
                                            delim.clone(),
                                        ));
                                        break;
                                    }
                                    let line_start = i;
                                    while i < chars.len() && chars[i] != '\n' {
                                        i += 1;
                                    }
                                    let line: String = chars[line_start..i].iter().collect();
                                    let check = if strip_tabs {
                                        line.trim_start_matches('\t').to_string()
                                    } else {
                                        line.clone()
                                    };
                                    let check_delim = if strip_tabs {
                                        delim.trim_start_matches('\t')
                                    } else {
                                        &delim
                                    };
                                    if check == check_delim {
                                        cmd.push_str(&line);
                                        // Do NOT consume the \n after the
                                        // delimiter — it belongs to the outer
                                        // context and the lexer needs it to
                                        // properly separate tokens.
                                        break;
                                    }
                                    cmd.push_str(&line);
                                    cmd.push('\n');
                                    if i < chars.len() {
                                        i += 1; // skip \n
                                    }
                                }
                            }
                            comsub_closed_by_paren = true;
                            break;
                        }
                    }
                    cmd.push(chars[i]);
                    i += 1;
                }
                if comsub_closed_by_paren {
                    // Comsub is closed and heredoc body is embedded in cmd.
                    // Break out of the main while loop.
                    break;
                }
                if i < chars.len() {
                    cmd.push(chars[i]);
                    i += 1;
                }
                if !delim.is_empty() {
                    loop {
                        if i >= chars.len() {
                            let current_line = chars.iter().filter(|&&c| c == '\n').count();
                            let heredoc_start_line =
                                chars[..hd_start].iter().filter(|&&c| c == '\n').count() + 1;
                            heredoc_eof_warnings.push((
                                current_line,
                                heredoc_start_line,
                                delim.clone(),
                            ));
                            break;
                        }
                        let line_start = i;
                        while i < chars.len() && chars[i] != '\n' {
                            i += 1;
                        }
                        let line: String = chars[line_start..i].iter().collect();
                        let check = if strip_tabs {
                            line.trim_start_matches('\t').to_string()
                        } else {
                            line.clone()
                        };
                        // When strip_tabs is set (<<-), also strip leading
                        // tabs from the delimiter for matching, matching
                        // bash's behavior with tab-indented delimiters.
                        let check_delim = if strip_tabs {
                            delim.trim_start_matches('\t')
                        } else {
                            &delim
                        };
                        if check == check_delim {
                            cmd.push_str(&line);
                            if i < chars.len() {
                                cmd.push('\n');
                                i += 1;
                            }
                            break;
                        }
                        if check.starts_with(check_delim)
                            && check[check_delim.len()..].trim_start().starts_with(')')
                        {
                            let current_line =
                                chars[..i].iter().filter(|&&c| c == '\n').count() + 1;
                            let heredoc_start_line =
                                chars[..hd_start].iter().filter(|&&c| c == '\n').count() + 1;
                            heredoc_eof_warnings.push((
                                current_line,
                                heredoc_start_line,
                                delim.clone(),
                            ));
                            cmd.push_str(&delim);
                            cmd.push('\n');
                            i = line_start + delim.len();
                            break;
                        }
                        cmd.push_str(&line);
                        if i < chars.len() {
                            cmd.push('\n');
                            i += 1;
                        }
                    }
                }
                continue;
            }
            _ => {}
        }
        // Detect ;; to reset case action context
        if chars[i] == ';' && i + 1 < chars.len() && chars[i + 1] == ';' && case_depth > 0 {
            in_case_action = false;
            cmd.push(';');
            i += 1;
            cmd.push(';');
            i += 1;
            continue;
        }
        // Handle # comments
        if chars[i] == '#' {
            let prev = if cmd.is_empty() {
                '\n'
            } else {
                cmd.chars().last().unwrap_or('\n')
            };
            let prev_escaped = cmd.len() >= 2 && cmd.as_bytes()[cmd.len() - 2] == b'\\';
            if !prev_escaped && matches!(prev, '\n' | ';' | '&' | '|' | '(' | ' ' | '\t') {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                continue;
            }
        }
        // Track case/esac keywords (resolve aliases like `switch=case`)
        if chars[i].is_alphabetic() {
            let mut word = String::new();
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                word.push(chars[i]);
                i += 1;
            }
            // Resolve aliases so that e.g. `alias switch=case` is tracked
            let effective = aliases.get(word.as_str()).map(|v| v.trim().to_string());
            let kw = effective.as_deref().unwrap_or(word.as_str());
            if kw == "case" {
                case_depth += 1;
                case_paren_depth.push(depth);
                case_action_stack.push(in_case_action);
                in_case_action = false;
            } else if (kw == "esac" || word == "esac") && case_depth > 0 {
                let case_open_depth = case_paren_depth.last().copied().unwrap_or(depth);
                if depth == case_open_depth {
                    let trimmed = cmd.trim_end();
                    let prev_ch = trimmed.chars().last().unwrap_or('\n');
                    let after_in = trimmed.ends_with(" in")
                        || trimmed.ends_with("\tin")
                        || trimmed.ends_with("\nin");
                    if in_case_action || prev_ch == ';' || prev_ch == '\n' || after_in {
                        case_depth -= 1;
                        case_paren_depth.pop();
                        in_case_action = case_action_stack.pop().unwrap_or(false);
                    }
                }
            }
            if matches!(kw, "do" | "then") {
                compound_depth += 1;
            } else if (matches!(kw, "done" | "fi") || matches!(word.as_str(), "done" | "fi"))
                && compound_depth > 0
            {
                compound_depth -= 1;
            }
            // Count ( and ) in alias expansion to adjust depth — handles
            // aliases like `alias nest='('` and `alias short='echo ok )'`.
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
                    i += 0; // already past the word
                    break;
                }
            }
            cmd.push_str(&word);
            continue;
        }
        cmd.push(chars[i]);
        i += 1;
    }

    if depth > 0 {
        ComsubBoundary::Incomplete {
            heredoc_eof_warnings: heredoc_eof_warnings
                .into_iter()
                .map(|(eof_line, start_line, delim)| {
                    (eof_line + line_offset, start_line + line_offset, delim)
                })
                .collect(),
        }
    } else {
        ComsubBoundary::Closed {
            text: cmd,
            chars_consumed: i,
            heredoc_eof_warnings: heredoc_eof_warnings
                .into_iter()
                .map(|(eof_line, start_line, delim)| {
                    (eof_line + line_offset, start_line + line_offset, delim)
                })
                .collect(),
            unterminated_heredoc_count: pending_heredocs.len(),
        }
    }
}

/// Validate comsub text with a full recursive parse.  Returns `Some(error)` if
/// the text contains a syntax error (e.g. `done` in a case body, `in` at
/// command position, incomplete compound commands).  Like C bash's `yyparse()`
/// call inside `parse_comsub`.
fn validate_comsub_text(text: &str, aliases: &HashMap<String, String>) -> Option<String> {
    if text.trim().is_empty() {
        return None;
    }

    let mut lexer = Lexer::new(text);
    lexer.aliases = aliases.clone();
    // Enable alias expansion if aliases are present — matches C bash's
    // parse_comsub which expands aliases in posix mode.
    lexer.shopt_expand_aliases = !aliases.is_empty();
    lexer.comsub_eof = false; // no special `)` handling — text is already bounded

    let first_token = lexer.next_token();
    let mut parser = Parser {
        lexer,
        current: first_token,
        compound_cmd_stack: Vec::new(),
    };

    match parser.parse_program() {
        Ok(_program) => {
            // Check if the parser consumed everything
            if parser.current != Token::Eof {
                let token = parser.current_token_str();
                Some(format!("syntax error near unexpected token `{}'", token))
            } else {
                None
            }
        }
        Err(e) => {
            // In comsub context, the "EOF" token is really the closing `)`.
            // But since we're validating bounded text, EOF means end of the
            // comsub — replace with `)` for better error messages.
            Some(e.replace("`EOF'", "`)'"))
        }
    }
}

pub struct Parser {
    lexer: Lexer,
    current: Token,
    /// Stack of compound command starts for EOF error reporting
    compound_cmd_stack: Vec<(String, usize)>, // (keyword, line_number)
}

impl Parser {
    pub fn new(input: &str) -> Self {
        let mut lexer = Lexer::new(input);
        let current = lexer.next_token();
        Self {
            lexer,
            current,
            compound_cmd_stack: Vec::new(),
        }
    }

    pub fn new_with_aliases(
        input: &str,
        aliases: HashMap<String, String>,
        expand_aliases: bool,
        posix_mode: bool,
    ) -> Self {
        let mut lexer = Lexer::new(input);
        lexer.aliases = aliases;
        lexer.shopt_expand_aliases = expand_aliases;
        lexer.posix_mode = posix_mode;
        let current = lexer.next_token();
        Self {
            lexer,
            current,
            compound_cmd_stack: Vec::new(),
        }
    }

    /// Update the alias table (called between parse-execute cycles)
    pub fn update_aliases(
        &mut self,
        aliases: HashMap<String, String>,
        expand_aliases: bool,
        posix_mode: bool,
    ) {
        self.lexer.aliases = aliases;
        self.lexer.shopt_expand_aliases = expand_aliases;
        self.lexer.posix_mode = posix_mode;
    }

    fn advance(&mut self) -> Token {
        std::mem::replace(&mut self.current, self.lexer.next_token())
    }

    fn eat(&mut self, expected: &Token) -> bool {
        if std::mem::discriminant(&self.current) == std::mem::discriminant(expected) {
            self.advance();
            true
        } else {
            false
        }
    }

    pub fn heredoc_overflow_line(&self) -> Option<usize> {
        self.lexer.heredoc_overflow_line
    }

    pub fn take_heredoc_eof_warnings(&mut self) -> Vec<(usize, usize, String)> {
        std::mem::take(&mut self.lexer.heredoc_eof_warnings)
    }

    fn skip_newlines(&mut self) {
        while self.current == Token::Newline {
            self.advance();
        }
    }

    fn is_keyword(&self, kw: &str) -> bool {
        if let Token::Word(parts) = &self.current
            && parts.len() == 1
            && let WordPart::Literal(s) = &parts[0]
        {
            return s == kw;
        }
        false
    }

    fn eat_keyword(&mut self, kw: &str) -> bool {
        if self.is_keyword(kw) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn token_to_str(&self) -> String {
        match &self.current {
            Token::Word(parts) => parts
                .iter()
                .map(|p| match p {
                    WordPart::Literal(s) => s.clone(),
                    _ => String::new(),
                })
                .collect(),
            Token::Newline => "newline".to_string(),
            Token::Pipe => "|".to_string(),
            Token::AndIf => "&&".to_string(),
            Token::OrIf => "||".to_string(),
            Token::Semi => ";".to_string(),
            Token::Amp => "&".to_string(),
            Token::DSemi => ";;".to_string(),
            Token::SemiAmp => ";&".to_string(),
            Token::DSemiAmp => ";;&".to_string(),
            Token::LParen => "(".to_string(),
            Token::RParen => ")".to_string(),
            Token::Less => "<".to_string(),
            Token::Great => ">".to_string(),
            Token::DGreat => ">>".to_string(),
            Token::LessAnd => "<&".to_string(),
            Token::GreatAnd => ">&".to_string(),
            Token::LessGreat => "<>".to_string(),
            Token::Clobber => ">|".to_string(),
            Token::DLess => "<<".to_string(),
            Token::DLessDash => "<<-".to_string(),
            Token::TripleLess => "<<<".to_string(),
            Token::Eof => "EOF".to_string(),
            _ => format!("{:?}", self.current),
        }
    }

    fn expect_keyword(&mut self, kw: &str) -> Result<(), String> {
        if self.eat_keyword(kw) {
            Ok(())
        } else {
            let token_str = self.token_to_str();
            Err(format!(
                "syntax error near unexpected token `{}'",
                token_str
            ))
        }
    }

    fn word_text(&self) -> Option<String> {
        if let Token::Word(parts) = &self.current {
            Some(word_to_string(parts))
        } else {
            None
        }
    }

    /// Check if the current word token contains a `SyntaxError` part (e.g. from
    /// a comsub that failed to parse).  Returns `Some(err)` if found.
    fn check_word_syntax_error(&self) -> Option<String> {
        if let Token::Word(parts) = &self.current {
            for part in parts {
                if let WordPart::SyntaxError(msg) = part {
                    return Some(msg.clone());
                }
                // Also check inside DoubleQuoted parts
                if let WordPart::DoubleQuoted(inner) = part {
                    for p in inner {
                        if let WordPart::SyntaxError(msg) = p {
                            return Some(msg.clone());
                        }
                    }
                }
            }
        }
        None
    }

    fn take_word(&mut self) -> Option<Word> {
        if let Token::Word(_) = &self.current {
            if let Token::Word(w) = self.advance() {
                Some(w)
            } else {
                None
            }
        } else {
            None
        }
    }

    /// Like `take_word` but returns `Result` — propagates `SyntaxError` parts
    /// as parse errors immediately, like C bash's `jump_to_top_level(FORCE_EOF)`
    /// when `parse_comsub` detects a syntax error.
    fn take_word_checked(&mut self) -> Result<Option<Word>, String> {
        if let Some(err) = self.check_word_syntax_error() {
            // Capture the lexer line BEFORE advance(), because advance()
            // reads the next token which may cross a newline boundary,
            // giving a wrong line number for the error.
            let error_line = self.lexer.line;
            // Consume the word so the parser doesn't get stuck
            self.advance();
            // For COMSUB errors, embed the accurate line number so that
            // run_string can use it instead of the (possibly advanced)
            // parser.current_line().
            if let Some(inner) = err.strip_prefix("COMSUB:") {
                return Err(format!("COMSUB_LINE:{}:{}", error_line, inner));
            }
            return Err(err);
        }
        Ok(self.take_word())
    }

    pub fn is_at_eof(&self) -> bool {
        self.current == Token::Eof
    }

    /// Get current lexer position (for stuck detection)
    /// Get the innermost compound command context (for EOF error messages)
    pub fn compound_cmd_context(&self) -> Option<(&str, usize)> {
        self.compound_cmd_stack
            .last()
            .map(|(cmd, line)| (cmd.as_str(), *line))
    }

    /// Set a line offset for the lexer (used by eval to inherit source file line numbers)
    pub fn set_line_offset(&mut self, offset: usize) {
        self.lexer.line += offset;
    }

    /// Set the lexer line to an absolute 1-based line number, discarding any
    /// newlines already consumed during parser construction.
    ///
    /// Used by command substitution execution: `target_line` is the 1-based
    /// script line where `$(` appeared.  The constructor's `next_token()` call
    /// may have consumed a leading `\n` (incrementing `lexer.line` from 1 to
    /// 2), but that newline is merely the line break between `$(` and the
    /// comsub body — bash does not count it as a LINENO increment.  By
    /// setting `lexer.line` to `target_line` unconditionally, the first real
    /// content reports the same LINENO as the `$(` line (matching bash).
    pub fn set_line_number(&mut self, target_line: usize) {
        self.lexer.line = target_line;
    }

    pub fn current_pos(&self) -> usize {
        self.lexer.current_pos()
    }

    /// Skip newlines and semicolons (for incremental parsing)
    pub fn skip_newlines_and_semis(&mut self) {
        while matches!(self.current, Token::Newline | Token::Semi) {
            self.advance();
        }
    }

    /// Parse a single complete command (public wrapper for incremental loop).
    /// Does NOT consume the trailing terminator, so the caller can sync aliases
    /// before the next token is read.
    pub fn parse_complete_command_pub(&mut self) -> Result<CompleteCommand, String> {
        self.parse_complete_command_inner(false)
    }

    /// Skip tokens until the next command boundary (newline or semicolon)
    pub fn skip_to_next_command(&mut self) {
        let mut limit = 1000; // safety limit
        while !matches!(self.current, Token::Eof | Token::Newline) && limit > 0 {
            self.advance();
            limit -= 1;
        }
        // Consume the newline
        if self.current == Token::Newline {
            self.advance();
        }
    }

    pub fn current_line(&self) -> usize {
        self.lexer.line
    }

    pub fn current_token_str(&self) -> String {
        match &self.current {
            Token::Word(parts) => parts
                .iter()
                .map(|p| match p {
                    WordPart::Literal(s) => s.clone(),
                    WordPart::SingleQuoted(s) => format!("'{}'", s),
                    _ => String::new(),
                })
                .collect(),
            Token::RParen => ")".to_string(),
            Token::LParen => "(".to_string(),
            Token::Semi => ";".to_string(),
            Token::DSemi => ";;".to_string(),
            Token::Pipe => "|".to_string(),
            Token::Amp => "&".to_string(),
            Token::AndIf => "&&".to_string(),
            Token::OrIf => "||".to_string(),
            Token::Newline => "newline".to_string(),
            Token::Eof => "EOF".to_string(),
            Token::Less => "<".to_string(),
            Token::Great => ">".to_string(),
            Token::DGreat => ">>".to_string(),
            Token::DLess => "<<".to_string(),
            Token::SemiAmp => ";&".to_string(),
            Token::DSemiAmp => ";;&".to_string(),
            _ => "unknown".to_string(),
        }
    }

    fn current_token_desc(&self) -> String {
        self.current_token_str()
    }

    /// Print a conditional expression error directly to stderr (like bash's cond_error)
    /// and return a special error marker that tells the caller not to re-print.
    fn cond_error(&self, msg: &str) -> String {
        // The error prefix will be added by the caller (interpreter)
        // Just mark it as a cond error that should be printed as-is
        format!("\x00COND_ERROR{}", msg)
    }

    pub fn parse_program(&mut self) -> Result<Program, String> {
        let mut commands = Vec::new();
        self.skip_newlines();

        while self.current != Token::Eof {
            if self.is_keyword("}")
                || self.is_keyword("fi")
                || self.is_keyword("done")
                || self.is_keyword("esac")
                || self.is_keyword("then")
                || self.is_keyword("else")
                || self.is_keyword("elif")
                || self.is_keyword("do")
                || self.current == Token::RParen
                || self.current == Token::DSemi
                || self.current == Token::SemiAmp
                || self.current == Token::DSemiAmp
            {
                break;
            }

            let cmd = self.parse_complete_command()?;
            commands.push(cmd);
            self.skip_newlines();
        }

        Ok(commands)
    }

    fn parse_complete_command(&mut self) -> Result<CompleteCommand, String> {
        self.parse_complete_command_inner(true)
    }

    /// Parse a single complete command.
    /// If `consume_terminator` is true, advance past the trailing newline/semi.
    /// If false, leave the terminator in `self.current` so the caller can handle it
    /// (used by incremental parse-execute loop to sync aliases before reading the next token).
    fn parse_complete_command_inner(
        &mut self,
        consume_terminator: bool,
    ) -> Result<CompleteCommand, String> {
        let line = self.current_line();
        let mut list = self.parse_and_or_list()?;

        // RParen at top level is a syntax error (not inside subshell/case)
        if self.current == Token::RParen
            && !self
                .compound_cmd_stack
                .iter()
                .any(|(kw, _)| kw == "(" || kw == "case")
        {
            return Err("syntax error near unexpected token `)'".to_string());
        }

        let background = match self.current {
            Token::Amp => {
                self.advance();
                true
            }
            Token::Semi | Token::Newline => {
                if consume_terminator {
                    self.advance();
                }
                false
            }
            _ => {
                // Check for missing command separator: if the next token could start
                // a new command (brace group, keyword, word) without a terminator,
                // it's a syntax error
                if self.is_keyword("{") || self.is_keyword("(") {
                    return Err(format!(
                        "syntax error near unexpected token `{}'",
                        if self.is_keyword("{") { "{" } else { "(" }
                    ));
                }
                false
            }
        };

        // If the command was terminated by `&` before the newline, the lexer
        // hasn't reached the newline yet and pending heredoc bodies haven't
        // been read.  Force-read them now so resolve_heredoc_bodies can
        // fill them into the AST.
        if self.lexer.has_pending_heredocs() {
            self.lexer.force_read_pending_heredocs();
        }

        // Resolve any deferred heredoc bodies (for pipeline heredocs like cmd <<EOF | cmd2)
        self.resolve_heredoc_bodies(&mut list);

        let end_line = self.current_line();
        Ok(CompleteCommand {
            list,
            background,
            line,
            end_line,
        })
    }

    /// Fill in empty heredoc bodies after the full command has been parsed.
    /// All heredoc redirections use empty placeholders during parsing; this
    /// method assigns the actual bodies from the lexer queue in order.
    fn resolve_heredoc_bodies(&mut self, list: &mut AndOrList) {
        self.resolve_heredoc_in_pipeline(&mut list.first);
        for (_, pipeline) in &mut list.rest {
            self.resolve_heredoc_in_pipeline(pipeline);
        }
    }

    fn resolve_heredoc_in_redirections(&mut self, redirections: &mut [Redirection]) {
        for redir in redirections {
            if matches!(redir.kind, RedirectKind::HereDoc(_, _))
                && (redir.target.is_empty()
                    || (redir.target.len() == 1
                        && matches!(&redir.target[0], WordPart::Literal(s) if s.is_empty())))
                && let Some(body) = self.lexer.take_heredoc_body()
            {
                redir.target = body;
            }
        }
    }

    fn resolve_heredoc_in_program(&mut self, program: &mut Program) {
        for cc in program.iter_mut() {
            self.resolve_heredoc_in_complete_command(cc);
        }
    }

    fn resolve_heredoc_in_complete_command(&mut self, cc: &mut CompleteCommand) {
        self.resolve_heredoc_in_pipeline(&mut cc.list.first);
        for (_, pipeline) in &mut cc.list.rest {
            self.resolve_heredoc_in_pipeline(pipeline);
        }
    }

    fn resolve_heredoc_in_command(&mut self, cmd: &mut Command) {
        match cmd {
            Command::Simple(sc) => {
                self.resolve_heredoc_in_redirections(&mut sc.redirections);
            }
            Command::Compound(compound, redirections) => {
                // First resolve heredocs inside the compound command's sub-commands
                match compound {
                    CompoundCommand::While(wc) | CompoundCommand::Until(wc) => {
                        self.resolve_heredoc_in_program(&mut wc.condition);
                        self.resolve_heredoc_in_program(&mut wc.body);
                    }
                    CompoundCommand::If(ic) => {
                        self.resolve_heredoc_in_program(&mut ic.condition);
                        self.resolve_heredoc_in_program(&mut ic.then_body);
                        for (cond, body) in &mut ic.elif_parts {
                            self.resolve_heredoc_in_program(cond);
                            self.resolve_heredoc_in_program(body);
                        }
                        if let Some(eb) = &mut ic.else_body {
                            self.resolve_heredoc_in_program(eb);
                        }
                    }
                    CompoundCommand::For(fc) => {
                        self.resolve_heredoc_in_program(&mut fc.body);
                    }
                    CompoundCommand::ArithFor(afc) => {
                        self.resolve_heredoc_in_program(&mut afc.body);
                    }
                    CompoundCommand::Case(cc) => {
                        for item in &mut cc.items {
                            self.resolve_heredoc_in_program(&mut item.body);
                        }
                    }
                    CompoundCommand::BraceGroup(prog) | CompoundCommand::Subshell(prog) => {
                        self.resolve_heredoc_in_program(prog);
                    }
                    CompoundCommand::Conditional(_) | CompoundCommand::Arithmetic(_) => {}
                }
                // Then resolve heredocs on the compound command's own redirections
                self.resolve_heredoc_in_redirections(redirections);
            }
            Command::Coproc(_, inner_cmd) => {
                self.resolve_heredoc_in_command(inner_cmd);
            }
            Command::FunctionDef {
                body, redirections, ..
            } => {
                // Resolve inside the function body (a CompoundCommand)
                // We wrap it temporarily to reuse resolve logic
                let mut tmp_cmd = Command::Compound(*body.clone(), redirections.clone());
                self.resolve_heredoc_in_command(&mut tmp_cmd);
                if let Command::Compound(new_body, new_redirs) = tmp_cmd {
                    *body = Box::new(new_body);
                    *redirections = new_redirs;
                }
            }
        }
    }

    fn resolve_heredoc_in_pipeline(&mut self, pipeline: &mut Pipeline) {
        for cmd in &mut pipeline.commands {
            self.resolve_heredoc_in_command(cmd);
        }
    }

    fn parse_and_or_list(&mut self) -> Result<AndOrList, String> {
        let first = self.parse_pipeline()?;
        let mut rest = Vec::new();

        loop {
            let op = match self.current {
                Token::AndIf => {
                    self.advance();
                    AndOr::And
                }
                Token::OrIf => {
                    self.advance();
                    AndOr::Or
                }
                _ => break,
            };
            self.skip_newlines();
            let pipeline = self.parse_pipeline()?;
            rest.push((op, pipeline));
        }

        Ok(AndOrList { first, rest })
    }

    fn parse_pipeline(&mut self) -> Result<Pipeline, String> {
        let mut timed = self.eat_keyword("time");
        let mut negated = self.eat_keyword("!");
        // time can also come after !
        if !timed {
            timed = self.eat_keyword("time");
        }
        // Handle multiple ! (each additional one toggles negation)
        while self.eat_keyword("!") {
            negated = !negated;
        }
        // Consume additional time keywords (time time ... is valid)
        while self.eat_keyword("time") {
            timed = true;
        }
        // Consume `time -p` flag (POSIX time format) and `--`
        let mut time_posix = false;
        if timed {
            loop {
                if let Token::Word(ref w) = self.current {
                    let s: String = w
                        .iter()
                        .map(|p| match p {
                            WordPart::Literal(s) => s.as_str(),
                            _ => "",
                        })
                        .collect();
                    if s == "-p" {
                        time_posix = true;
                        self.advance();
                        continue;
                    }
                    if s == "--" {
                        self.advance();
                        continue;
                    }
                }
                // Check for another time keyword after -p
                if self.eat_keyword("time") {
                    continue;
                }
                break;
            }
        }

        let first = self.parse_command()?;
        let mut commands = vec![first];
        let mut pipe_stderr = Vec::new();

        while self.current == Token::Pipe || self.current == Token::PipeAmp {
            let is_pipe_amp = self.current == Token::PipeAmp;
            pipe_stderr.push(is_pipe_amp);
            self.advance();
            self.skip_newlines();
            commands.push(self.parse_command()?);
        }

        Ok(Pipeline {
            negated,
            timed,
            time_posix,
            commands,
            pipe_stderr,
        })
    }

    fn parse_command(&mut self) -> Result<Command, String> {
        // Check for function definition: name () compound_command
        if let Some(name) = self.word_text()
            && !is_reserved_word(&name)
            && !name.contains('=')
        {
            // Look ahead for () - save full state for backtrack
            let saved_lexer_pos = self.lexer.save_position();
            let saved_current = self.current.clone();

            self.advance();
            if self.current == Token::LParen {
                let saved2_lexer_pos = self.lexer.save_position();
                let saved2_current = self.current.clone();
                self.advance();
                if self.current == Token::RParen {
                    self.advance();
                    self.skip_newlines();
                    let body_start = self.current_line();
                    let body = self.parse_compound_command()?;
                    let end_line = self.current_line().saturating_sub(1).max(body_start);
                    let redirections = self.parse_redirections()?;
                    return Ok(Command::FunctionDef {
                        name,
                        body: Box::new(body),
                        body_line: body_start,
                        end_line,
                        has_function_keyword: false,
                        redirections,
                    });
                }
                // Backtrack from inner lookahead
                self.lexer.restore_position(saved2_lexer_pos);
                self.current = saved2_current;
            }

            // Backtrack
            self.lexer.restore_position(saved_lexer_pos);
            self.current = saved_current;
        }

        // Check for 'function' keyword
        if self.is_keyword("function") {
            self.advance();
            let name = self
                .word_text()
                .ok_or_else(|| "expected function name".to_string())?;
            self.advance();
            // Optional ()
            if self.current == Token::LParen {
                self.advance();
                if self.current == Token::RParen {
                    self.advance();
                }
            }
            self.skip_newlines();
            let body_start = self.current_line();
            let body = self.parse_compound_command()?;
            let end_line = self.current_line().saturating_sub(1).max(body_start);
            let redirections = self.parse_redirections()?;
            return Ok(Command::FunctionDef {
                name,
                body: Box::new(body),
                body_line: body_start,
                end_line,
                has_function_keyword: true,
                redirections,
            });
        }

        // Check for coproc
        if self.is_keyword("coproc") {
            self.advance();
            // Check if next token is a name (not a command keyword)
            let name = if let Some(n) = self.word_text()
                && !is_reserved_word(&n)
                && !n.contains('=')
                && !matches!(self.current, Token::LParen)
            {
                // Peek ahead to see if this is a name followed by a command
                let saved_pos = self.lexer.save_position();
                let saved_cur = self.current.clone();
                self.advance();
                let is_name = self.is_keyword("{")
                    || self.is_keyword("if")
                    || self.is_keyword("for")
                    || self.is_keyword("while")
                    || self.is_keyword("until")
                    || self.is_keyword("case")
                    || self.current == Token::LParen;
                if is_name {
                    Some(n)
                } else {
                    // Not a name — restore and parse as command
                    self.lexer.restore_position(saved_pos);
                    self.current = saved_cur;
                    None
                }
            } else {
                None
            };
            let name = name.or_else(|| Some("COPROC".to_string()));
            self.skip_newlines();
            let inner = self.parse_command()?;
            return Ok(Command::Coproc(name, Box::new(inner)));
        }

        // Check for compound command
        if self.is_keyword("{")
            || self.is_keyword("if")
            || self.is_keyword("for")
            || self.is_keyword("select")
            || self.is_keyword("while")
            || self.is_keyword("until")
            || self.is_keyword("case")
            || self.is_keyword("[[")
            || self.current == Token::LParen
        {
            let compound = self.parse_compound_command()?;
            let redirections = self.parse_redirections()?;
            return Ok(Command::Compound(compound, redirections));
        }

        // Reject `in` at command position — it's a reserved word that can only
        // appear after `case WORD` or `for VAR`, never as a standalone command.
        if self.is_keyword("in") {
            return Err("syntax error near unexpected token `in'".to_string());
        }

        // Simple command
        let cmd = self.parse_simple_command()?;
        Ok(Command::Simple(cmd))
    }

    fn parse_compound_command(&mut self) -> Result<CompoundCommand, String> {
        if self.is_keyword("{") {
            self.parse_brace_group()
        } else if self.current == Token::LParen {
            // Check for (( — arithmetic command or C-style for
            let saved_pos = self.lexer.save_position();
            let saved_tok = self.current.clone();
            self.advance(); // consume first (
            if self.current == Token::LParen && !self.lexer.had_whitespace_before_token {
                // Try (( expression )) first, but fall back to nested subshell
                // if the content contains command separators (;, |, &)
                let arith_saved_pos = self.lexer.save_position();
                match self.read_arith_command() {
                    Ok(expr) => {
                        self.current = self.lexer.next_token();
                        return Ok(CompoundCommand::Arithmetic(expr));
                    }
                    Err(e) if e.contains("`;'") => {
                        // Content has command separators — treat as nested subshell
                        self.lexer.restore_position(arith_saved_pos);
                        // Restore to the first ( and parse as subshell
                        self.lexer.restore_position(saved_pos);
                        self.current = saved_tok;
                        return self.parse_subshell();
                    }
                    Err(e) => return Err(e),
                }
            }
            // Not ((, backtrack to regular subshell
            self.lexer.restore_position(saved_pos);
            self.current = saved_tok;
            self.parse_subshell()
        } else if self.is_keyword("if") {
            self.parse_if()
        } else if self.is_keyword("for") || self.is_keyword("select") {
            self.parse_for()
        } else if self.is_keyword("while") {
            self.parse_while()
        } else if self.is_keyword("until") {
            self.parse_until()
        } else if self.is_keyword("case") {
            self.parse_case()
        } else if self.is_keyword("[[") {
            self.parse_conditional()
        } else {
            Err(format!("expected compound command, got {:?}", self.current))
        }
    }

    /// Read the body of `(( expr ))` — collect until matching `))`.
    fn read_arith_command(&mut self) -> Result<String, String> {
        // We've already consumed `((`; now read raw chars until `))`.
        let expr = self.lexer.read_until_double_paren()?;
        Ok(expr)
    }

    fn parse_brace_group(&mut self) -> Result<CompoundCommand, String> {
        let start_line = self.current_line();
        self.compound_cmd_stack.push(("{".to_string(), start_line));
        self.expect_keyword("{")?;
        self.skip_newlines();
        let body = self.parse_program()?;
        let result = self.expect_keyword("}");
        if result.is_ok() {
            self.compound_cmd_stack.pop();
        }
        result?;
        Ok(CompoundCommand::BraceGroup(body))
    }

    fn parse_subshell(&mut self) -> Result<CompoundCommand, String> {
        let start_line = self.current_line();
        self.compound_cmd_stack.push(("(".to_string(), start_line));
        assert!(self.eat(&Token::LParen));
        self.skip_newlines();
        let body = self.parse_program()?;
        if !self.eat(&Token::RParen) {
            // If at EOF, report "unexpected end of file" with compound command context
            if matches!(self.current, Token::Eof) {
                // Don't pop the stack — leave context for heredoc EOF warning emission
                return Err(format!(
                    "syntax error: unexpected end of file from `(' command on line {}",
                    start_line
                ));
            }
            self.compound_cmd_stack.pop();
            // If the current token is a reserved word, report it as unexpected
            if let Some(text) = self.word_text()
                && is_reserved_word(&text)
            {
                return Err(format!("syntax error near unexpected token `{}'", text));
            }
            return Err("expected ')'".to_string());
        }
        self.compound_cmd_stack.pop();
        Ok(CompoundCommand::Subshell(body))
    }

    fn parse_if(&mut self) -> Result<CompoundCommand, String> {
        self.expect_keyword("if")?;
        self.skip_newlines();
        let condition = self.parse_compound_list()?;
        self.expect_keyword("then")?;
        self.skip_newlines();
        let then_body = self.parse_program()?;

        let mut elif_parts = Vec::new();
        while self.is_keyword("elif") {
            self.advance();
            self.skip_newlines();
            let elif_cond = self.parse_compound_list()?;
            self.expect_keyword("then")?;
            self.skip_newlines();
            let elif_body = self.parse_program()?;
            elif_parts.push((elif_cond, elif_body));
        }

        let else_body = if self.is_keyword("else") {
            self.advance();
            self.skip_newlines();
            Some(self.parse_program()?)
        } else {
            None
        };

        self.expect_keyword("fi")?;
        Ok(CompoundCommand::If(IfClause {
            condition,
            then_body,
            elif_parts,
            else_body,
        }))
    }

    fn parse_for(&mut self) -> Result<CompoundCommand, String> {
        // Capture the line number of the `for`/`select` keyword for LINENO
        // reset per iteration (matching bash's execute_for_command behavior).
        let for_line = self.lexer.line;
        // Accept both 'for' and 'select'
        if !self.eat_keyword("for") {
            self.expect_keyword("select")?;
        }

        // Check for C-style: for (( init; cond; step ))
        if self.current == Token::LParen {
            let saved_pos = self.lexer.save_position();
            let saved_tok = self.current.clone();
            self.advance(); // consume first (
            if self.current == Token::LParen {
                // Don't consume second ( as token — use raw lexer
                // Lexer position is right after the second ( was read
                return self.parse_arith_for();
            }
            self.lexer.restore_position(saved_pos);
            self.current = saved_tok;
        }

        let (var, var_raw) = if let Token::Word(parts) = &self.current {
            let text = word_to_string(parts);
            let raw = word_to_raw_string(parts);
            (text.clone(), if raw != text { Some(raw) } else { None })
        } else {
            let token = self.token_to_str();
            return Err(format!("syntax error near unexpected token `{}'", token));
        };
        self.advance();

        self.skip_newlines();

        let words = if self.is_keyword("in") {
            self.advance();
            let mut words = Vec::new();
            while !matches!(self.current, Token::Semi | Token::Newline | Token::Eof) {
                if self.is_keyword("do") {
                    break;
                }
                if let Some(w) = self.take_word() {
                    words.push(w);
                } else {
                    break;
                }
            }
            if self.current == Token::Semi || self.current == Token::Newline {
                self.advance();
            }
            Some(words)
        } else {
            if self.current == Token::Semi || self.current == Token::Newline {
                self.advance();
            }
            None
        };

        self.skip_newlines();
        self.expect_keyword("do")?;
        self.skip_newlines();
        let body = self.parse_program()?;
        self.expect_keyword("done")?;

        Ok(CompoundCommand::For(ForClause {
            var,
            var_raw,
            words,
            body,
            line: for_line,
        }))
    }

    /// Parse `for (( init; cond; step )) do body done` — already consumed `((`
    fn parse_arith_for(&mut self) -> Result<CompoundCommand, String> {
        let init = self
            .lexer
            .read_until_char(';')
            .map_err(|_| "syntax error: arithmetic expression required".to_string())?;
        let cond = self
            .lexer
            .read_until_char(';')
            .map_err(|_| "syntax error: arithmetic expression required".to_string())?;
        let step_raw = self.lexer.read_until_double_paren()?;
        let step = step_raw.trim_start().to_string();
        // Sync parser — skip ; or newline before 'do'
        self.current = self.lexer.next_token();
        if matches!(self.current, Token::Semi | Token::Newline) {
            self.current = self.lexer.next_token();
        }
        self.skip_newlines();
        // Accept either do...done or { ... } for arith-for body
        let body = if self.eat_keyword("do") {
            self.skip_newlines();
            let body = self.parse_program()?;
            self.expect_keyword("done")?;
            body
        } else if let Token::Word(ref w) = self.current
            && w.len() == 1
            && matches!(&w[0], WordPart::Literal(s) if s == "{")
        {
            self.advance();
            self.skip_newlines();
            let body = self.parse_program()?;
            // Expect closing }
            if let Token::Word(ref w) = self.current
                && w.len() == 1
                && matches!(&w[0], WordPart::Literal(s) if s == "}")
            {
                self.advance();
            } else {
                return Err("syntax error near unexpected token".to_string());
            }
            body
        } else {
            return Err(format!(
                "syntax error near unexpected token `{}'",
                self.token_to_str()
            ));
        };
        Ok(CompoundCommand::ArithFor(ArithForClause {
            init,
            cond,
            step,
            body,
        }))
    }

    fn parse_while(&mut self) -> Result<CompoundCommand, String> {
        self.expect_keyword("while")?;
        self.skip_newlines();
        let condition = self.parse_compound_list()?;
        self.expect_keyword("do")?;
        self.skip_newlines();
        let body = self.parse_program()?;
        self.expect_keyword("done")?;
        Ok(CompoundCommand::While(WhileClause { condition, body }))
    }

    fn parse_until(&mut self) -> Result<CompoundCommand, String> {
        self.expect_keyword("until")?;
        self.skip_newlines();
        let condition = self.parse_compound_list()?;
        self.expect_keyword("do")?;
        self.skip_newlines();
        let body = self.parse_program()?;
        self.expect_keyword("done")?;
        Ok(CompoundCommand::Until(WhileClause { condition, body }))
    }

    fn parse_case(&mut self) -> Result<CompoundCommand, String> {
        self.expect_keyword("case")?;
        let word = self
            .take_word()
            .ok_or_else(|| "expected word after 'case'".to_string())?;
        self.skip_newlines();
        self.expect_keyword("in")?;
        // Suppress alias expansion in case patterns
        self.lexer.in_case_pattern = true;
        self.skip_newlines();

        let mut items = Vec::new();
        while !self.is_keyword("esac") && self.current != Token::Eof {
            // Optional leading (
            if self.current == Token::LParen {
                self.advance();
            }
            let mut patterns = Vec::new();
            while let Some(w) = self.take_word_checked()? {
                patterns.push(w);
                if self.current == Token::Pipe {
                    self.advance();
                } else {
                    break;
                }
            }
            self.lexer.in_case_pattern = false;

            if !self.eat(&Token::RParen) {
                // Try to recover
                if patterns.is_empty() {
                    break;
                }
            }

            self.skip_newlines();
            let body = self.parse_program()?;

            // Detect misplaced reserved words in case body.
            // If parse_program stopped on a reserved word like `done`, `fi`,
            // `then`, `else`, `elif`, or `do` that has no matching opening
            // construct, it means the token is unexpected here.
            if let Some(text) = self.word_text()
                && matches!(
                    text.as_str(),
                    "done" | "fi" | "then" | "else" | "elif" | "do"
                )
            {
                return Err(format!("syntax error near unexpected token `{}'", text));
            }

            let terminator = if self.current == Token::DSemi {
                self.advance();
                CaseTerminator::Break
            } else if self.current == Token::SemiAmp {
                self.advance();
                CaseTerminator::FallThrough
            } else if self.current == Token::DSemiAmp {
                self.advance();
                CaseTerminator::TestNext
            } else {
                CaseTerminator::Break
            };
            // Re-suppress alias expansion for next pattern
            self.lexer.in_case_pattern = true;
            self.skip_newlines();

            items.push(CaseItem {
                patterns,
                body,
                terminator,
            });
        }

        self.lexer.in_case_pattern = false;
        self.expect_keyword("esac")?;
        Ok(CompoundCommand::Case(CaseClause { word, items }))
    }

    /// Parse `[[ expression ]]`
    fn parse_conditional(&mut self) -> Result<CompoundCommand, String> {
        let start_line = self.current_line();
        self.compound_cmd_stack.push(("[[".to_string(), start_line));
        self.expect_keyword("[[")?;
        let expr = self.parse_cond_or()?;
        // Check for ]] — if not found, produce a conditional-specific error
        if !self.is_keyword("]]") {
            if self.current == Token::Eof {
                return Err(self.cond_error("unexpected EOF while looking for `]]'"));
            }
            let tok = self.current_token_desc();
            return Err(self.cond_error(&format!(
                "syntax error in conditional expression: unexpected token `{}'",
                tok
            )));
        }
        self.compound_cmd_stack.pop();
        self.advance(); // consume ]]
        Ok(CompoundCommand::Conditional(expr))
    }

    fn parse_cond_or(&mut self) -> Result<CondExpr, String> {
        let mut left = self.parse_cond_and()?;
        while self.current == Token::OrIf {
            self.advance();
            self.skip_newlines();
            let right = self.parse_cond_and()?;
            left = CondExpr::Or(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_cond_and(&mut self) -> Result<CondExpr, String> {
        let mut left = self.parse_cond_primary()?;
        while self.current == Token::AndIf {
            self.advance();
            self.skip_newlines();
            let right = self.parse_cond_primary()?;
            left = CondExpr::And(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_cond_primary(&mut self) -> Result<CondExpr, String> {
        // Handle ! (negation)
        if self.is_keyword("!") {
            self.advance();
            let expr = self.parse_cond_primary()?;
            return Ok(CondExpr::Not(Box::new(expr)));
        }

        // Handle ( expr )
        if self.current == Token::LParen {
            self.advance();
            let expr = self.parse_cond_or()?;
            if !self.eat(&Token::RParen) {
                let tok = self.current_token_desc();
                return Err(self.cond_error(&format!("unexpected token `{}', expected `)'", tok)));
            }
            return Ok(expr);
        }

        // Check for unary operators: -n, -z, -e, -f, -d, etc.
        if let Some(text) = self.word_text()
            && is_cond_unary_op(&text)
        {
            let op = text;
            self.advance();
            let operand = self.take_word().ok_or_else(|| {
                let tok = self.current_token_desc();
                self.cond_error(&format!(
                    "unexpected argument `{}' to conditional unary operator",
                    tok
                ))
            })?;
            return Ok(CondExpr::Unary(op, operand));
        }

        // Must be a word — check for binary operator after it
        let left = self.take_word().ok_or_else(|| {
            let tok = self.current_token_desc();
            self.cond_error(&format!(
                "unexpected token `{}' in conditional command",
                tok
            ))
        })?;

        // Check for ]]
        if self.is_keyword("]]") || self.current == Token::Eof {
            return Ok(CondExpr::Word(left));
        }

        // Check for && or || (handled by caller)
        if matches!(self.current, Token::AndIf | Token::OrIf) {
            return Ok(CondExpr::Word(left));
        }

        // Check for binary operator (as word token)
        if let Some(text) = self.word_text()
            && is_cond_binary_op(&text)
        {
            let op = text;
            self.advance();
            // For =~ (regex match), read the pattern as raw text.
            // For == != = (glob match), try normal word first but handle
            // extglob patterns by backtracking if LParen follows.
            let right = if op == "=~" {
                self.read_cond_pattern()?
            } else if op == "==" || op == "!=" || op == "=" {
                // Save state to backtrack if extglob
                let saved_lexer = self.lexer.save_position();
                let saved_tok = self.current.clone();
                if let Some(word) = self.take_word() {
                    if self.current == Token::LParen {
                        // This is an extglob pattern like +(foo) — backtrack
                        self.lexer.restore_position(saved_lexer);
                        self.current = saved_tok;
                        self.read_cond_pattern()?
                    } else {
                        word
                    }
                } else if matches!(self.current, Token::LParen) {
                    self.read_cond_pattern()?
                } else {
                    let tok = self.current_token_desc();
                    return Err(self.cond_error(&format!(
                        "unexpected argument `{}' to conditional binary operator",
                        tok
                    )));
                }
            } else {
                self.take_word().ok_or_else(|| {
                    let tok = self.current_token_desc();
                    self.cond_error(&format!(
                        "unexpected argument `{}' to conditional binary operator",
                        tok
                    ))
                })?
            };
            return Ok(CondExpr::Binary(left, op, right));
        }

        // Check for < and > which are Token::Less/Token::Great inside [[ ]]
        if matches!(self.current, Token::Less | Token::Great) {
            let op = if self.current == Token::Less {
                "<".to_string()
            } else {
                ">".to_string()
            };
            self.advance();
            let right = self.take_word().ok_or_else(|| {
                let tok = self.current_token_desc();
                self.cond_error(&format!(
                    "unexpected argument `{}' to conditional binary operator",
                    tok
                ))
            })?;
            return Ok(CondExpr::Binary(left, op, right));
        }

        // If next token is not ]], &&, ||, or EOF, then it's an unexpected token
        // where a binary operator was expected
        if !self.is_keyword("]]")
            && !matches!(
                self.current,
                Token::AndIf | Token::OrIf | Token::Eof | Token::RParen
            )
        {
            let tok = self.current_token_desc();
            return Err(self.cond_error(&format!(
                "unexpected token `{}', conditional binary operator expected",
                tok
            )));
        }
        Ok(CondExpr::Word(left))
    }

    /// Read a regex pattern for `[[ x =~ pattern ]]`.
    /// Regex patterns can contain ( ) | which are normally special tokens,
    /// so we read raw text from the lexer until we hit ]], &&, or ||.
    fn read_cond_pattern(&mut self) -> Result<Word, String> {
        let mut parts: Word = Vec::new();
        // Consume tokens and raw text until ]], &&, ||
        // This handles extglob patterns like +(foo|bar) and regex patterns
        // Preserve word parts for proper variable expansion
        let mut first = true;
        loop {
            if self.is_keyword("]]") || self.current == Token::Eof {
                break;
            }
            if matches!(self.current, Token::AndIf | Token::OrIf) {
                break;
            }
            // Add space between tokens if the lexer had whitespace before this token
            if !first && self.lexer.had_whitespace_before_token {
                parts.push(WordPart::Literal(" ".to_string()));
            }
            first = false;
            match &self.current {
                Token::Word(word_parts) => {
                    parts.extend(word_parts.clone());
                    self.advance();
                }
                Token::LParen => {
                    parts.push(WordPart::Literal("(".to_string()));
                    self.advance();
                }
                Token::RParen => {
                    parts.push(WordPart::Literal(")".to_string()));
                    self.advance();
                }
                Token::Pipe => {
                    parts.push(WordPart::Literal("|".to_string()));
                    self.advance();
                }
                Token::Less => {
                    parts.push(WordPart::Literal("<".to_string()));
                    self.advance();
                }
                Token::Great => {
                    parts.push(WordPart::Literal(">".to_string()));
                    self.advance();
                }
                _ => break,
            }
        }
        if parts.is_empty() {
            return Err("expected pattern in conditional".to_string());
        }
        Ok(parts)
    }

    fn parse_compound_list(&mut self) -> Result<Program, String> {
        self.skip_newlines();
        let mut commands = Vec::new();

        loop {
            if self.current == Token::Eof
                || self.is_keyword("then")
                || self.is_keyword("do")
                || self.is_keyword("done")
                || self.is_keyword("fi")
                || self.is_keyword("else")
                || self.is_keyword("elif")
                || self.is_keyword("esac")
            {
                break;
            }

            let cmd = self.parse_complete_command()?;
            commands.push(cmd);
            self.skip_newlines();
        }

        Ok(commands)
    }

    fn parse_simple_command(&mut self) -> Result<SimpleCommand, String> {
        let mut assignments = Vec::new();
        let mut words = Vec::new();
        let mut redirections = Vec::new();

        // Parse leading assignments
        while words.is_empty() {
            match self.try_parse_assignment() {
                Some(Ok(assign)) => assignments.push(assign),
                Some(Err(e)) => return Err(e),
                None => break,
            }
        }

        // Parse words and redirections
        loop {
            // Check for redirections
            if let Some(redir) = self.try_parse_redirection()? {
                redirections.push(redir);
                continue;
            }

            // Assignments can appear interspersed with redirections before command words
            if words.is_empty() {
                match self.try_parse_assignment() {
                    Some(Ok(assign)) => {
                        assignments.push(assign);
                        continue;
                    }
                    Some(Err(e)) => return Err(e),
                    None => {}
                }
            }

            // Check for inline array assignment: if we see word ending with = followed by (
            // This handles `declare -a arr=(one two three)` etc.
            if self.current == Token::LParen && !words.is_empty() {
                // Check if last word ends with =
                let last_ends_with_eq = {
                    let last: &Word = &words[words.len() - 1];
                    if let Some(WordPart::Literal(s)) = last.last() {
                        s.ends_with('=') || s.ends_with("+=")
                    } else {
                        false
                    }
                };
                // Only allow inline compound assignment name=(...) as arguments
                // to assignment builtins (declare/local/export/readonly/typeset).
                // Bash rejects `cmd a=(x y)` as a syntax error — compound
                // assignments are not valid in arbitrary command arguments.
                // `let` is special: `let a=(5+3)` is arithmetic grouping, not
                // compound assignment — consume balanced parens as part of the word.
                let first_word_owned: Option<String> =
                    if let Some(WordPart::Literal(cmd)) = words.first().and_then(|w| w.first()) {
                        Some(cmd.clone())
                    } else {
                        None
                    };
                let first_word_str = first_word_owned.as_deref();
                let first_is_assign_builtin = matches!(
                    first_word_str,
                    Some("declare" | "typeset" | "local" | "export" | "readonly")
                );
                let first_is_paren_word = matches!(first_word_str, Some("let" | "eval"));
                if last_ends_with_eq && first_is_paren_word {
                    // `let a=(5+3)` — arithmetic grouping, not compound assignment.
                    // `eval a=(1 2 3)` — pass as literal word; eval re-parses it.
                    // Consume balanced parens as part of the current word.
                    self.advance(); // consume (
                    let last = words.last_mut().unwrap();
                    last.push(WordPart::Literal("(".to_string()));
                    let mut depth = 1u32;
                    let mut need_space = false;
                    loop {
                        match &self.current {
                            Token::RParen => {
                                depth -= 1;
                                if depth == 0 {
                                    last.push(WordPart::Literal(")".to_string()));
                                    self.advance(); // consume )
                                    break;
                                }
                                last.push(WordPart::Literal(")".to_string()));
                                self.advance();
                                need_space = true;
                            }
                            Token::LParen => {
                                if need_space {
                                    last.push(WordPart::Literal(" ".to_string()));
                                }
                                depth += 1;
                                last.push(WordPart::Literal("(".to_string()));
                                self.advance();
                            }
                            Token::Word(parts) => {
                                if need_space {
                                    last.push(WordPart::Literal(" ".to_string()));
                                }
                                for p in parts {
                                    last.push(p.clone());
                                }
                                self.advance();
                                need_space = true;
                            }
                            Token::Eof | Token::Newline => {
                                return Err(
                                    "\x01RECOVERABLE\x01syntax error near unexpected token `('"
                                        .to_string(),
                                );
                            }
                            _ => {
                                if need_space {
                                    last.push(WordPart::Literal(" ".to_string()));
                                }
                                last.push(WordPart::Literal(self.token_to_str()));
                                self.advance();
                                need_space = true;
                            }
                        }
                    }
                    // For `let`, after the closing `)` there may be a
                    // continuation like `/2` in `let a=(4*3)/2`.  Keep
                    // appending adjacent Word tokens (no intervening
                    // whitespace in the source — the lexer produces a
                    // new Word token for `/2`).  For `eval` this is a
                    // no-op because `eval a=(1 2 3)` has `)` at end.
                    if first_word_str == Some("let") {
                        while let Token::Word(parts) = &self.current {
                            // Check if the next word could re-enter
                            // balanced-paren mode (another `name=(...)`).
                            // If so, break and let the outer loop handle it.
                            let starts_new_assign = if let Some(WordPart::Literal(s)) = parts.last()
                            {
                                s.ends_with('=') || s.ends_with("+=")
                            } else {
                                false
                            };
                            if starts_new_assign {
                                break;
                            }
                            let last = words.last_mut().unwrap();
                            for p in parts {
                                last.push(p.clone());
                            }
                            self.advance();
                        }
                    }
                    continue;
                }
                if last_ends_with_eq && !first_is_assign_builtin {
                    // Bash rejects `cmd a=(x y)` as a syntax error.
                    // Compound array assignments are only valid as arguments
                    // to declare/local/export/readonly/typeset (and `let` handled above).
                    return Err(
                        "\x01RECOVERABLE\x01syntax error near unexpected token `('".to_string()
                    );
                }
                if last_ends_with_eq && first_is_assign_builtin {
                    self.advance(); // consume (
                    let elements = self.parse_array_elements()?;
                    // For array assignments in command args (declare/local),
                    // expand each element individually and join with \x1F separator.
                    // This preserves the structure for the builtin to split.
                    let last = words.last_mut().unwrap();
                    last.push(WordPart::Literal("(".to_string()));
                    for (i, elem) in elements.iter().enumerate() {
                        if i > 0 {
                            last.push(WordPart::Literal("\x1F".to_string()));
                        }
                        // Include [key]= prefix for associative array elements
                        if let Some(idx) = &elem.index {
                            last.push(WordPart::Literal("[".to_string()));
                            for part in idx {
                                last.push(part.clone());
                            }
                            last.push(WordPart::Literal("]=".to_string()));
                        }
                        for part in &elem.value {
                            last.push(part.clone());
                        }
                    }
                    last.push(WordPart::Literal(")".to_string()));
                    continue;
                }
            }

            // Check for word
            if let Token::Word(_) = &self.current {
                // Don't consume keywords that end compound commands,
                // but only at command position (first word).
                // In argument position, these are regular words.
                if words.is_empty()
                    && let Some(text) = self.word_text()
                    && is_compound_end(&text)
                {
                    break;
                }
                if let Some(w) = self.take_word_checked()? {
                    words.push(w);
                }
            } else {
                break;
            }
        }

        Ok(SimpleCommand {
            assignments,
            words,
            redirections,
        })
    }

    fn try_parse_assignment(&mut self) -> Option<Result<Assignment, String>> {
        // Extract all info from the current token without holding a borrow
        let info = if let Token::Word(parts) = &self.current {
            if parts.is_empty() {
                return None;
            }
            if let WordPart::Literal(s) = &parts[0] {
                if let Some(eq_pos) = s.find('=') {
                    let before_eq = &s[..eq_pos];
                    let (name_str, append) = if let Some(stripped) = before_eq.strip_suffix('+') {
                        (stripped, true)
                    } else {
                        (before_eq, false)
                    };
                    let base_name = if let Some(bracket) = name_str.find('[') {
                        &name_str[..bracket]
                    } else {
                        name_str
                    };
                    if !base_name.is_empty()
                        && base_name.chars().all(|c| c.is_alphanumeric() || c == '_')
                        && !base_name.chars().next().unwrap().is_ascii_digit()
                    {
                        let full_name = name_str.to_string();
                        let after_eq = s[eq_pos + 1..].to_string();
                        let num_parts = parts.len();
                        // Build value parts eagerly
                        let mut value_parts = Vec::new();
                        if !after_eq.is_empty() {
                            value_parts.push(WordPart::Literal(after_eq.clone()));
                        }
                        for part in &parts[1..] {
                            value_parts.push(part.clone());
                        }
                        Some((full_name, append, after_eq, num_parts, value_parts))
                    } else {
                        None
                    }
                } else if s.ends_with('[') || s.contains('[') {
                    // Handle array assignment with quoted subscript: name[quoted_key]=value
                    // The = is in a later part (e.g., BASH_ALIASES['\$']=xx)
                    // Also handles spaced subscripts: chaff[hello world]=flip
                    // where the lexer splits at the space into separate tokens.
                    let base_end = s.find('[').unwrap();
                    let base_name = &s[..base_end];
                    if !base_name.is_empty()
                        && base_name.chars().all(|c| c.is_alphanumeric() || c == '_')
                        && !base_name.chars().next().unwrap().is_ascii_digit()
                    {
                        // Find ]=  or ]+=  in a later Literal part of the CURRENT token
                        let mut found_eq = false;
                        let mut name_text = s.to_string();
                        let mut value_parts = Vec::new();
                        let mut eq_part_idx = 0;
                        let mut needs_multi_token = false;
                        for (idx, part) in parts[1..].iter().enumerate() {
                            if found_eq {
                                value_parts.push(part.clone());
                                continue;
                            }
                            match part {
                                WordPart::Literal(lit) => {
                                    if let Some(eq_pos) = lit.find("]+=") {
                                        // Append assignment: arr[sub]+=value
                                        // Must check before "]=" since "]+" contains "]"
                                        let before = &lit[..eq_pos + 1]; // include ]
                                        name_text.push_str(before);
                                        name_text.push('+'); // mark as append (name ends with ]+)
                                        let after_eq = &lit[eq_pos + 3..]; // skip ]+=
                                        if !after_eq.is_empty() {
                                            value_parts
                                                .push(WordPart::Literal(after_eq.to_string()));
                                        }
                                        found_eq = true;
                                        eq_part_idx = idx + 1;
                                    } else if let Some(eq_pos) = lit.find("]=") {
                                        let before = &lit[..eq_pos + 1]; // include ]
                                        name_text.push_str(before);
                                        let after_eq = &lit[eq_pos + 2..];
                                        if !after_eq.is_empty() {
                                            value_parts
                                                .push(WordPart::Literal(after_eq.to_string()));
                                        }
                                        found_eq = true;
                                        eq_part_idx = idx + 1;
                                    } else {
                                        name_text.push_str(lit);
                                    }
                                }
                                WordPart::SingleQuoted(sq) => {
                                    // Preserve single-quote markers so that
                                    // expand_assoc_subscript can detect literal
                                    // subscripts (e.g. A['$(echo %)']=val).
                                    // If the content itself contains a single
                                    // quote (from \' in unquoted context), use
                                    // double-quote wrapping with inner escaping
                                    // so expand_assoc_subscript doesn't misparse.
                                    if sq.contains('\'') {
                                        name_text.push('"');
                                        for ch in sq.chars() {
                                            if ch == '"' || ch == '\\' || ch == '$' || ch == '`' {
                                                name_text.push('\\');
                                            }
                                            name_text.push(ch);
                                        }
                                        name_text.push('"');
                                    } else {
                                        name_text.push('\'');
                                        name_text.push_str(sq);
                                        name_text.push('\'');
                                    }
                                }
                                WordPart::DoubleQuoted(dq) => {
                                    // Preserve double-quote markers so that
                                    // expand_assoc_subscript sees them.
                                    name_text.push('"');
                                    for dp in dq {
                                        match dp {
                                            WordPart::Literal(l) => {
                                                // Escape literal `"` and `\` characters
                                                // that came from `\"` or `\\` escapes
                                                // inside double quotes, so they don't
                                                // prematurely close the reconstructed
                                                // double-quote region when processed
                                                // by expand_assoc_subscript.
                                                for ch in l.chars() {
                                                    if ch == '"' || ch == '\\' {
                                                        name_text.push('\\');
                                                    }
                                                    name_text.push(ch);
                                                }
                                            }
                                            other => {
                                                name_text.push_str(&crate::ast::word_to_string(
                                                    &vec![other.clone()],
                                                ));
                                            }
                                        }
                                    }
                                    name_text.push('"');
                                }
                                other => {
                                    // Include raw text of variable expansions ($i, ${i}, etc.)
                                    name_text.push_str(&crate::ast::word_to_string(&vec![
                                        other.clone(),
                                    ]));
                                }
                            }
                        }
                        if !found_eq {
                            // The bracket is unclosed within this token — check if the
                            // subscript has spaces and spans multiple tokens, e.g.
                            // chaff[hello world]=flip → tokens: "chaff[hello" "world]=flip"
                            // We flag this for multi-token merging below.
                            needs_multi_token = true;
                        }
                        if found_eq {
                            let append = name_text.ends_with('+');
                            // Strip trailing '+' from name when it's an append assignment
                            // (the '+' was added by the ]+= detection above)
                            if append {
                                name_text.pop();
                            }
                            let _ = eq_part_idx;
                            Some((name_text, append, String::new(), parts.len(), value_parts))
                        } else if needs_multi_token {
                            // Save state for multi-token bracket merge
                            // We pass a sentinel to trigger the merge logic below
                            Some((
                                name_text,
                                false,
                                "\x02MERGE_BRACKET".to_string(),
                                parts.len(),
                                value_parts,
                            ))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        let (full_name, append, after_eq, num_parts, value_parts) = info?;

        // Case 0: Multi-token bracket merge for spaced subscripts
        // e.g., chaff[hello world]=flip → tokens: "chaff[hello" "world]=flip"
        if after_eq == "\x02MERGE_BRACKET" {
            let saved_pos = self.lexer.save_position();
            let saved_tok = self.current.clone();
            self.advance();

            let mut merged_name = full_name.clone();
            let mut found = false;
            let mut final_value_parts: Vec<WordPart> = Vec::new();
            let mut final_append = false;
            let mut tokens_consumed = 0;

            // Try merging up to ~20 subsequent tokens (generous limit for long keys)
            for _ in 0..20 {
                if matches!(self.current, Token::Eof | Token::Newline) {
                    break;
                }
                if let Token::Word(ref parts) = self.current {
                    // Check each part for ]+= or ]=
                    let mut part_name = String::new();
                    let mut part_found = false;
                    let mut pval_parts: Vec<WordPart> = Vec::new();
                    for (pi, part) in parts.iter().enumerate() {
                        if part_found {
                            pval_parts.push(part.clone());
                            continue;
                        }
                        if let WordPart::Literal(lit) = part {
                            if let Some(pos) = lit.find("]+=") {
                                part_name.push_str(&lit[..pos + 1]);
                                let after = &lit[pos + 3..];
                                if !after.is_empty() {
                                    pval_parts.push(WordPart::Literal(after.to_string()));
                                }
                                pval_parts.extend(parts[pi + 1..].iter().cloned());
                                part_found = true;
                                final_append = true;
                                break;
                            } else if let Some(pos) = lit.find("]=") {
                                part_name.push_str(&lit[..pos + 1]);
                                let after = &lit[pos + 2..];
                                if !after.is_empty() {
                                    pval_parts.push(WordPart::Literal(after.to_string()));
                                }
                                pval_parts.extend(parts[pi + 1..].iter().cloned());
                                part_found = true;
                                break;
                            } else {
                                part_name.push_str(lit);
                            }
                        } else if let WordPart::SingleQuoted(sq) = part {
                            part_name.push_str(sq);
                        } else if let WordPart::DoubleQuoted(dq) = part {
                            for dp in dq {
                                if let WordPart::Literal(l) = dp {
                                    part_name.push_str(l);
                                }
                            }
                        } else {
                            part_name.push_str(&crate::ast::word_to_string(&vec![part.clone()]));
                        }
                    }
                    // Merge with a space separator (the lexer split at the space)
                    merged_name.push(' ');
                    merged_name.push_str(&part_name);
                    tokens_consumed += 1;
                    self.advance();

                    if part_found {
                        final_value_parts = pval_parts;
                        found = true;
                        break;
                    }
                } else {
                    break;
                }
            }

            if found {
                let value = AssignValue::Scalar(final_value_parts);
                return Some(Ok(Assignment {
                    name: merged_name,
                    value,
                    append: final_append,
                }));
            }
            // Failed to find ]= — backtrack
            let _ = tokens_consumed;
            self.lexer.restore_position(saved_pos);
            self.current = saved_tok;
            return None;
        }

        // Case 1: "name=(" all in one token
        if after_eq == "(" && num_parts == 1 {
            self.advance();
            let elements = self.parse_array_elements();
            match elements {
                Ok(elems) => {
                    return Some(Ok(Assignment {
                        name: full_name,
                        value: AssignValue::Array(elems),
                        append,
                    }));
                }
                Err(e) => return Some(Err(e)),
            }
        }

        // Case 2: "name=" as one token, then LParen as next token
        if after_eq.is_empty() && num_parts == 1 {
            let saved_pos = self.lexer.save_position();
            let saved_tok = self.current.clone();
            self.advance();
            if self.current == Token::LParen {
                self.advance();
                let elements = self.parse_array_elements();
                match elements {
                    Ok(elems) => {
                        // Check if ) is immediately followed by a word
                        // token (e.g. /2 in a=(4*3)/2).  In bash, when
                        // content follows the closing ), it's a scalar
                        // assignment — not a compound array.  Reconstruct
                        // the original text as a scalar value.
                        if let Token::Word(trail_parts) = &self.current {
                            // Adjacent word after ) — this is a scalar
                            // like a=(4*3)/2.  Reconstruct the value from
                            // the parsed elements + trailing text.
                            let mut scalar_parts: Vec<WordPart> = Vec::new();
                            scalar_parts.push(WordPart::Literal("(".to_string()));
                            for (ei, elem) in elems.iter().enumerate() {
                                if ei > 0 {
                                    scalar_parts.push(WordPart::Literal(" ".to_string()));
                                }
                                if let Some(idx) = &elem.index {
                                    scalar_parts.push(WordPart::Literal("[".to_string()));
                                    scalar_parts.extend(idx.iter().cloned());
                                    scalar_parts.push(WordPart::Literal("]=".to_string()));
                                }
                                scalar_parts.extend(elem.value.iter().cloned());
                            }
                            scalar_parts.push(WordPart::Literal(")".to_string()));
                            // Append the trailing word parts (e.g. "/2")
                            for p in trail_parts {
                                scalar_parts.push(p.clone());
                            }
                            self.advance(); // consume trailing word
                            return Some(Ok(Assignment {
                                name: full_name,
                                value: AssignValue::Scalar(scalar_parts),
                                append,
                            }));
                        }
                        return Some(Ok(Assignment {
                            name: full_name,
                            value: AssignValue::Array(elems),
                            append,
                        }));
                    }
                    Err(e) => return Some(Err(e)),
                }
            }
            // Not an array — backtrack
            self.lexer.restore_position(saved_pos);
            self.current = saved_tok;
        }

        // Scalar assignment — even empty value is a Scalar (a= sets to "")
        self.advance();
        let value = AssignValue::Scalar(value_parts);

        Some(Ok(Assignment {
            name: full_name,
            value,
            append,
        }))
    }

    /// Convert a literal string to word parts, recognizing leading `~` as a
    /// `Tilde` part so that tilde expansion works in compound array element
    /// values like `[key]=~/Desktop`.
    /// Also expands `~` after `:` separators (assignment-context tilde
    /// expansion), so `~/Desktop:~/Documents:~/Applications` correctly
    /// expands all three tildes, matching bash behavior for PATH-like values.
    fn literal_to_parts_with_tilde(s: &str) -> Vec<WordPart> {
        if s.is_empty() {
            return vec![];
        }

        // Split on ':' to handle assignment-context tilde expansion
        // (tildes after ':' are expanded just like the leading tilde).
        // We only do this when the string actually contains ':' followed
        // by '~' to avoid unnecessary splitting.
        if s.contains(":~") || s.starts_with('~') {
            let mut parts: Vec<WordPart> = Vec::new();
            let segments: Vec<&str> = s.split(':').collect();
            for (seg_idx, seg) in segments.iter().enumerate() {
                if seg_idx > 0 {
                    // Re-insert the ':' separator as literal text.
                    // Append to previous Literal part if possible, otherwise create new one.
                    if let Some(WordPart::Literal(prev)) = parts.last_mut() {
                        prev.push(':');
                    } else {
                        parts.push(WordPart::Literal(":".to_string()));
                    }
                }
                if let Some(rest) = seg.strip_prefix('~') {
                    if let Some(slash_pos) = rest.find('/') {
                        let user = rest[..slash_pos].to_string();
                        let after_tilde = &rest[slash_pos..]; // includes the '/'
                        parts.push(WordPart::Tilde(user));
                        if !after_tilde.is_empty() {
                            parts.push(WordPart::Literal(after_tilde.to_string()));
                        }
                    } else {
                        // ~ or ~user with nothing after (within this segment)
                        parts.push(WordPart::Tilde(rest.to_string()));
                    }
                } else if !seg.is_empty() {
                    // Append to previous Literal part if possible
                    if let Some(WordPart::Literal(prev)) = parts.last_mut() {
                        prev.push_str(seg);
                    } else {
                        parts.push(WordPart::Literal(seg.to_string()));
                    }
                }
            }
            parts
        } else {
            vec![WordPart::Literal(s.to_string())]
        }
    }

    /// Extract `[index]=value` or `[index]+=value` from a word's parts.
    /// Returns `(index_parts, value_parts, is_append)` if found, else `None`.
    fn extract_array_index(parts: &[WordPart]) -> Option<(Vec<WordPart>, Vec<WordPart>, bool)> {
        // The first literal must start with '['
        let first_lit = match parts.first() {
            Some(WordPart::Literal(s)) if s.starts_with('[') => s,
            _ => return None,
        };

        // Fast path: everything in the first literal, e.g. [key]=value or [key]+=value
        if let Some(close) = first_lit.find("]+=") {
            let idx_str = &first_lit[1..close];
            let after = &first_lit[close + 3..];
            let rest: Vec<WordPart> = parts[1..].to_vec();
            let idx_parts = Self::literal_to_parts_with_tilde(idx_str);
            let mut value_parts = Self::literal_to_parts_with_tilde(after);
            value_parts.extend(rest);
            return Some((idx_parts, value_parts, true));
        }
        if let Some(close) = first_lit.find("]=") {
            let idx_str = &first_lit[1..close];
            let after = &first_lit[close + 2..];
            let rest: Vec<WordPart> = parts[1..].to_vec();
            let idx_parts = Self::literal_to_parts_with_tilde(idx_str);
            let mut value_parts = Self::literal_to_parts_with_tilde(after);
            value_parts.extend(rest);
            return Some((idx_parts, value_parts, false));
        }

        // Slow path: key spans multiple parts, e.g. ["key"]=value
        // Walk parts looking for a Literal containing "]+=" or "]="
        let mut idx_parts: Vec<WordPart> = Vec::new();
        // Strip the leading '[' from the first literal
        let stripped = first_lit[1..].to_string();
        if !stripped.is_empty() {
            idx_parts.push(WordPart::Literal(stripped));
        }

        for (i, part) in parts.iter().enumerate().skip(1) {
            if let WordPart::Literal(s) = part {
                // Check for ]+=
                if let Some(pos) = s.find("]+=") {
                    let before = &s[..pos];
                    if !before.is_empty() {
                        idx_parts.push(WordPart::Literal(before.to_string()));
                    }
                    let after = &s[pos + 3..];
                    let mut value_parts = Self::literal_to_parts_with_tilde(after);
                    value_parts.extend(parts[i + 1..].to_vec());
                    return Some((idx_parts, value_parts, true));
                }
                // Check for ]=
                if let Some(pos) = s.find("]=") {
                    let before = &s[..pos];
                    if !before.is_empty() {
                        idx_parts.push(WordPart::Literal(before.to_string()));
                    }
                    let after = &s[pos + 2..];
                    let mut value_parts = Self::literal_to_parts_with_tilde(after);
                    value_parts.extend(parts[i + 1..].to_vec());
                    return Some((idx_parts, value_parts, false));
                }
                // No close bracket in this literal — accumulate
                idx_parts.push(part.clone());
            } else {
                // Non-literal part (quoted string, etc.) — accumulate as part of key
                idx_parts.push(part.clone());
            }
        }

        // Never found ']=' — not an indexed element
        None
    }

    /// Find `]=` or `]+=` in a token's parts, splitting into before-close and after-close parts.
    /// Returns `(key_tail_parts, value_parts, is_append)` if found.
    fn find_bracket_close_in_parts(
        parts: &[WordPart],
    ) -> Option<(Vec<WordPart>, Vec<WordPart>, bool)> {
        for (i, part) in parts.iter().enumerate() {
            if let WordPart::Literal(s) = part {
                // Check for ]+=
                if let Some(pos) = s.find("]+=") {
                    let before = &s[..pos];
                    let after = &s[pos + 3..];
                    let mut key_tail = Vec::new();
                    if !before.is_empty() {
                        key_tail.push(WordPart::Literal(before.to_string()));
                    }
                    let mut value_parts = Self::literal_to_parts_with_tilde(after);
                    value_parts.extend(parts[i + 1..].iter().cloned());
                    return Some((key_tail, value_parts, true));
                }
                // Check for ]=
                if let Some(pos) = s.find("]=") {
                    let before = &s[..pos];
                    let after = &s[pos + 2..];
                    let mut key_tail = Vec::new();
                    if !before.is_empty() {
                        key_tail.push(WordPart::Literal(before.to_string()));
                    }
                    let mut value_parts = Self::literal_to_parts_with_tilde(after);
                    value_parts.extend(parts[i + 1..].iter().cloned());
                    return Some((key_tail, value_parts, false));
                }
            }
        }
        None
    }

    /// Parse array elements: `word1 [n]=word2 word3 ...` until `)`
    fn parse_array_elements(&mut self) -> Result<Vec<ArrayElement>, String> {
        let mut elements = Vec::new();
        self.skip_newlines();

        while self.current != Token::RParen && self.current != Token::Eof {
            // Check for [index]=value syntax by examining all word parts.
            // The key may span multiple parts, e.g. ["key"]="value" produces:
            //   [Literal("["), DoubleQuoted(...), Literal("]="), DoubleQuoted(...)]
            // Or simple: [key]=value produces:
            //   [Literal("[key]=value")]
            let indexed_info = if let Token::Word(parts) = &self.current {
                Self::extract_array_index(parts)
            } else {
                None
            };

            if let Some((idx_parts, value_parts, elem_append)) = indexed_info {
                self.advance();
                elements.push(ArrayElement {
                    index: Some(idx_parts),
                    value: value_parts,
                    append: elem_append,
                });
                self.skip_newlines();
                continue;
            }

            // Check if current token starts with '[' but doesn't contain ']='.
            // This means the key has spaces and was split by the lexer, e.g.
            // [foo bar]="qux qix" → tokens: [foo, bar]="qux qix"
            // We need to merge tokens until we find one containing ']=' or ']+='.
            let starts_with_bracket = if let Token::Word(parts) = &self.current {
                if let Some(WordPart::Literal(s)) = parts.first() {
                    s.starts_with('[')
                } else {
                    false
                }
            } else {
                false
            };

            if starts_with_bracket {
                // Collect parts from the current token (strip leading '[')
                let mut merged_parts: Vec<WordPart> = Vec::new();
                if let Token::Word(parts) = &self.current {
                    for (pi, part) in parts.iter().enumerate() {
                        if pi == 0 {
                            if let WordPart::Literal(s) = part {
                                let stripped = &s[1..]; // remove '['
                                if !stripped.is_empty() {
                                    merged_parts.push(WordPart::Literal(stripped.to_string()));
                                }
                            } else {
                                merged_parts.push(part.clone());
                            }
                        } else {
                            merged_parts.push(part.clone());
                        }
                    }
                }
                self.advance();

                // Keep merging subsequent tokens, inserting space between them,
                // until we find one containing ']=' or ']+='
                let mut found_close = false;
                while self.current != Token::RParen && self.current != Token::Eof {
                    if let Token::Word(parts) = &self.current {
                        // Check if this token contains ']=' or ']+='
                        let close_info = Self::find_bracket_close_in_parts(parts);
                        if let Some((before_parts, after_parts, is_append)) = close_info {
                            // Add space separator + before-close parts to the key
                            merged_parts.push(WordPart::Literal(" ".to_string()));
                            merged_parts.extend(before_parts);
                            self.advance();
                            elements.push(ArrayElement {
                                index: Some(merged_parts.clone()),
                                value: after_parts,
                                append: is_append,
                            });
                            found_close = true;
                            break;
                        } else {
                            // No close bracket — accumulate as part of the key
                            merged_parts.push(WordPart::Literal(" ".to_string()));
                            merged_parts.extend(parts.iter().cloned());
                            self.advance();
                        }
                    } else {
                        break;
                    }
                }
                if found_close {
                    self.skip_newlines();
                    continue;
                }
                // If we never found ']=' — treat the accumulated parts as a bare word
                // (re-add the '[' we stripped)
                let mut restored = vec![WordPart::Literal("[".to_string())];
                restored.extend(merged_parts);
                elements.push(ArrayElement {
                    index: None,
                    value: restored,
                    append: false,
                });
                self.skip_newlines();
                continue;
            }

            if let Token::Word(_) = &self.current {
                if let Some(w) = self.take_word() {
                    elements.push(ArrayElement {
                        index: None,
                        value: w,
                        append: false,
                    });
                }
            } else {
                // Unexpected token inside array compound assignment (e.g. & | ;)
                let token_str = self.token_to_str();
                // Skip to closing ) or end of line to recover parser state
                while !matches!(self.current, Token::RParen | Token::Newline | Token::Eof) {
                    self.advance();
                }
                self.eat(&Token::RParen);
                // Mark as recoverable so run_string doesn't exit the shell
                return Err(format!(
                    "\x01RECOVERABLE\x01syntax error near unexpected token `{}'",
                    token_str
                ));
            }
            self.skip_newlines();
        }

        // Consume the closing )
        self.eat(&Token::RParen);
        Ok(elements)
    }

    fn try_parse_redirection(&mut self) -> Result<Option<Redirection>, String> {
        // Check for {varname}> style redirections
        let fd = self.try_parse_redir_fd();

        let kind = match &self.current {
            Token::Less => Some(RedirectKind::Input),
            Token::Great => Some(RedirectKind::Output),
            Token::DGreat => Some(RedirectKind::Append),
            Token::Clobber => Some(RedirectKind::Clobber),
            Token::LessAnd => Some(RedirectKind::DupInput),
            Token::GreatAnd => Some(RedirectKind::DupOutput),
            Token::LessGreat => Some(RedirectKind::ReadWrite),
            Token::DLess => Some(RedirectKind::HereDoc(false, String::new())),
            Token::DLessDash => Some(RedirectKind::HereDoc(true, String::new())),
            Token::TripleLess => Some(RedirectKind::HereString),
            Token::AmpGreat => Some(RedirectKind::OutputAll),
            Token::AmpDGreat => Some(RedirectKind::AppendAll),
            _ => {
                if fd.is_some() {
                    // We consumed an IO number but there's no redirect operator.
                }
                None
            }
        };

        if let Some(kind) = kind {
            self.advance();
            match &kind {
                RedirectKind::HereDoc(_, _) => {
                    // Get the delimiter for this heredoc
                    let delim = self.lexer.take_heredoc_delimiter().unwrap_or_default();
                    let kind = match kind {
                        RedirectKind::HereDoc(strip, _) => RedirectKind::HereDoc(strip, delim),
                        _ => kind,
                    };
                    // Always use empty placeholder for the body here.
                    // resolve_heredoc_bodies() fills in all bodies after the
                    // full command is parsed.  This avoids misordering when
                    // multiple heredocs appear on one line (e.g. <<EOF1 3<<EOF2):
                    // the first heredoc's body may not be available yet while the
                    // second's advance triggers read_heredoc_bodies, which would
                    // hand body[0] to the wrong redirection.
                    let target = vec![WordPart::Literal(String::new())];
                    Ok(Some(Redirection { fd, kind, target }))
                }
                _ => {
                    let target = self
                        .take_word()
                        .ok_or_else(|| "expected word after redirection".to_string())?;
                    Ok(Some(Redirection { fd, kind, target }))
                }
            }
        } else {
            Ok(None)
        }
    }

    fn try_parse_redir_fd(&mut self) -> Option<RedirFd> {
        // Extract info from current token without holding borrow
        let token_info = if let Token::Word(parts) = &self.current {
            if parts.len() == 1 {
                if let WordPart::Literal(s) = &parts[0] {
                    Some(s.clone())
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        let s = token_info?;

        // Check for {varname} before redirect operator
        if s.starts_with('{') && s.ends_with('}') && s.len() > 2 {
            let varname = s[1..s.len() - 1].to_string();
            // Allow plain identifiers (e.g. {v}) and array subscripts (e.g. {fd[0]})
            let is_valid_var_redir = if let Some(bracket) = varname.find('[') {
                varname.ends_with(']')
                    && varname[..bracket]
                        .chars()
                        .all(|c| c.is_alphanumeric() || c == '_')
                    && !varname[..bracket].is_empty()
            } else {
                varname.chars().all(|c| c.is_alphanumeric() || c == '_')
            };
            if is_valid_var_redir {
                let saved_pos = self.lexer.save_position();
                let saved_tok = self.current.clone();
                self.advance();
                if matches!(
                    self.current,
                    Token::Less
                        | Token::Great
                        | Token::DGreat
                        | Token::LessAnd
                        | Token::GreatAnd
                        | Token::LessGreat
                        | Token::DLess
                        | Token::DLessDash
                ) {
                    return Some(RedirFd::Var(varname));
                }
                self.lexer.restore_position(saved_pos);
                self.current = saved_tok;
            }
        }

        // Check for numeric fd — only if the next token is a redirect operator
        // and there was no whitespace between the digit word and the redirect.
        // The lexer records whether whitespace preceded the current token.
        if !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()) {
            let saved_pos = self.lexer.save_position();
            let saved_tok = self.current.clone();
            self.advance();
            let is_redir = !self.lexer.had_whitespace_before_token
                && matches!(
                    self.current,
                    Token::Less
                        | Token::Great
                        | Token::DGreat
                        | Token::LessAnd
                        | Token::GreatAnd
                        | Token::LessGreat
                        | Token::Clobber
                        | Token::AmpGreat
                        | Token::AmpDGreat
                        | Token::DLess
                        | Token::DLessDash
                        | Token::TripleLess
                );
            if is_redir {
                if let Ok(n) = s.parse::<i32>() {
                    return Some(RedirFd::Number(n));
                }
                // Number too large for fd — backtrack and treat as word
                self.lexer.restore_position(saved_pos);
                self.current = saved_tok;
                return None;
            }
            // Not a redirect — backtrack
            self.lexer.restore_position(saved_pos);
            self.current = saved_tok;
        }

        None
    }

    fn parse_redirections(&mut self) -> Result<Vec<Redirection>, String> {
        let mut redirections = Vec::new();
        while let Some(redir) = self.try_parse_redirection()? {
            redirections.push(redir);
        }
        Ok(redirections)
    }
}

fn is_reserved_word(s: &str) -> bool {
    matches!(
        s,
        "if" | "then"
            | "else"
            | "elif"
            | "fi"
            | "for"
            | "while"
            | "until"
            | "do"
            | "done"
            | "case"
            | "esac"
            | "in"
            | "function"
            | "{"
            | "}"
            | "!"
            | "[["
            | "]]"
            | "select"
    )
}

fn is_compound_end(s: &str) -> bool {
    matches!(
        s,
        "}" | "fi" | "done" | "esac" | "then" | "else" | "elif" | "do" | "]]"
    )
}

fn is_cond_unary_op(s: &str) -> bool {
    matches!(
        s,
        "-n" | "-z"
            | "-e"
            | "-f"
            | "-d"
            | "-r"
            | "-w"
            | "-x"
            | "-s"
            | "-L"
            | "-h"
            | "-a"
            | "-b"
            | "-c"
            | "-g"
            | "-k"
            | "-p"
            | "-t"
            | "-u"
            | "-G"
            | "-N"
            | "-O"
            | "-S"
            | "-o"
            | "-v"
            | "-R"
    )
}

fn is_cond_binary_op(s: &str) -> bool {
    matches!(
        s,
        "=" | "=="
            | "!="
            | "<"
            | ">"
            | "-eq"
            | "-ne"
            | "-lt"
            | "-le"
            | "-gt"
            | "-ge"
            | "-nt"
            | "-ot"
            | "-ef"
            | "=~"
    )
}
