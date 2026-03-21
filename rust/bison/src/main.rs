#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::fmt;
use std::fs;
use std::path::Path;
use std::process;

// ============================================================================
// Constants
// ============================================================================

const VERSION: &str = env!("CARGO_PKG_VERSION");
const EOF_TOKEN: usize = 0;
const ERROR_TOKEN: usize = 256;
const ACCEPT_ACTION: i32 = 0;
const ERROR_ACTION: i32 = i32::MIN;

// ============================================================================
// Data Structures
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
enum Assoc {
    Left,
    Right,
    NonAssoc,
    None,
}

#[derive(Debug, Clone)]
struct TokenInfo {
    name: String,
    value: Option<i32>,
    tag: Option<String>,
    prec: i32,
    assoc: Assoc,
    is_literal: bool,
}

#[derive(Debug, Clone)]
struct Rule {
    lhs: usize,
    rhs: Vec<usize>,
    action: String,
    action_line: usize,
    prec_sym: Option<usize>,
    rule_number: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct LR0Item {
    rule: usize,
    dot: usize,
}

type ItemSet = BTreeSet<LR0Item>;

#[derive(Debug, Clone)]
struct Grammar {
    // Symbol tables
    symbols: Vec<String>,               // index -> name
    symbol_map: HashMap<String, usize>, // name -> index
    token_info: HashMap<usize, TokenInfo>,
    nonterminals: HashSet<usize>,
    terminals: HashSet<usize>,

    // Rules
    rules: Vec<Rule>,
    start_symbol: Option<usize>,
    augmented_start: usize,

    // Declarations
    union_code: String,
    prologue: String,
    epilogue: String,
    type_tags: HashMap<usize, String>, // symbol -> tag
    expect_conflicts: Option<i32>,
    defines: bool,

    // Precedence
    next_prec: i32,

    // Token value assignment
    next_token_value: i32,
}

impl Grammar {
    fn new() -> Self {
        let mut g = Grammar {
            symbols: Vec::new(),
            symbol_map: HashMap::new(),
            token_info: HashMap::new(),
            nonterminals: HashSet::new(),
            terminals: HashSet::new(),
            rules: Vec::new(),
            start_symbol: None,
            augmented_start: 0,
            union_code: String::new(),
            prologue: String::new(),
            epilogue: String::new(),
            type_tags: HashMap::new(),
            expect_conflicts: None,
            defines: false,
            next_prec: 1,
            next_token_value: 258,
        };
        // Reserve symbol 0 for $end
        g.intern_symbol("$end");
        g.terminals.insert(0);
        g.token_info.insert(
            0,
            TokenInfo {
                name: "$end".to_string(),
                value: Some(0),
                tag: None,
                prec: 0,
                assoc: Assoc::None,
                is_literal: false,
            },
        );
        // Reserve symbol for error
        let err_id = g.intern_symbol("error");
        g.terminals.insert(err_id);
        g.token_info.insert(
            err_id,
            TokenInfo {
                name: "error".to_string(),
                value: Some(ERROR_TOKEN as i32),
                tag: None,
                prec: 0,
                assoc: Assoc::None,
                is_literal: false,
            },
        );
        g
    }

    fn intern_symbol(&mut self, name: &str) -> usize {
        if let Some(&id) = self.symbol_map.get(name) {
            return id;
        }
        let id = self.symbols.len();
        self.symbols.push(name.to_string());
        self.symbol_map.insert(name.to_string(), id);
        id
    }

    fn is_terminal(&self, sym: usize) -> bool {
        self.terminals.contains(&sym)
    }

    fn is_nonterminal(&self, sym: usize) -> bool {
        self.nonterminals.contains(&sym)
    }

    fn symbol_name(&self, sym: usize) -> &str {
        &self.symbols[sym]
    }

    fn token_c_value(&self, sym: usize) -> i32 {
        if let Some(info) = self.token_info.get(&sym)
            && let Some(v) = info.value
        {
            return v;
        }
        // Single-char literal
        let name = &self.symbols[sym];
        if name.starts_with('\'') && name.ends_with('\'') && name.len() >= 3 {
            let inner = &name[1..name.len() - 1];
            if let Some(ch) = parse_char_literal(inner) {
                return ch as i32;
            }
        }
        sym as i32
    }

    fn rule_prec(&self, rule: &Rule) -> (i32, Assoc) {
        // If %prec is specified, use that
        if let Some(prec_sym) = rule.prec_sym
            && let Some(info) = self.token_info.get(&prec_sym)
        {
            return (info.prec, info.assoc);
        }
        // Otherwise use the rightmost terminal's precedence
        for &sym in rule.rhs.iter().rev() {
            if self.is_terminal(sym)
                && let Some(info) = self.token_info.get(&sym)
                && info.prec > 0
            {
                return (info.prec, info.assoc);
            }
        }
        (0, Assoc::None)
    }
}

fn parse_char_literal(s: &str) -> Option<char> {
    let mut chars = s.chars();
    match chars.next()? {
        '\\' => match chars.next()? {
            'n' => Some('\n'),
            't' => Some('\t'),
            'r' => Some('\r'),
            '\\' => Some('\\'),
            '\'' => Some('\''),
            '0' => Some('\0'),
            'a' => Some('\x07'),
            'b' => Some('\x08'),
            'f' => Some('\x0C'),
            'v' => Some('\x0B'),
            c => Some(c),
        },
        c => Some(c),
    }
}

// ============================================================================
// Lexer / Parser for .y files
// ============================================================================

struct YaccParser<'a> {
    input: &'a str,
    pos: usize,
    line: usize,
    grammar: Grammar,
}

impl<'a> YaccParser<'a> {
    fn new(input: &'a str) -> Self {
        YaccParser {
            input,
            pos: 0,
            line: 1,
            grammar: Grammar::new(),
        }
    }

    fn peek(&self) -> Option<char> {
        self.input[self.pos..].chars().next()
    }

    fn advance(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.pos += ch.len_utf8();
        if ch == '\n' {
            self.line += 1;
        }
        Some(ch)
    }

    fn skip_whitespace(&mut self) {
        while self.pos < self.input.len() {
            match self.peek() {
                Some(c) if c.is_ascii_whitespace() => {
                    self.advance();
                }
                Some('/') if self.input[self.pos..].starts_with("/*") => {
                    self.skip_c_comment();
                }
                Some('/') if self.input[self.pos..].starts_with("//") => {
                    self.skip_line_comment();
                }
                _ => break,
            }
        }
    }

    fn skip_c_comment(&mut self) {
        self.advance(); // /
        self.advance(); // *
        loop {
            match self.advance() {
                Some('*') if self.peek() == Some('/') => {
                    self.advance();
                    return;
                }
                None => return,
                _ => {}
            }
        }
    }

    fn skip_line_comment(&mut self) {
        while let Some(c) = self.advance() {
            if c == '\n' {
                return;
            }
        }
    }

    fn read_identifier(&mut self) -> String {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c.is_ascii_alphanumeric() || c == '_' || c == '.' {
                self.advance();
            } else {
                break;
            }
        }
        self.input[start..self.pos].to_string()
    }

    fn read_number(&mut self) -> i32 {
        let start = self.pos;
        // Handle 0x prefix
        if self.peek() == Some('0') {
            self.advance();
            if self.peek() == Some('x') || self.peek() == Some('X') {
                self.advance();
                while let Some(c) = self.peek() {
                    if c.is_ascii_hexdigit() {
                        self.advance();
                    } else {
                        break;
                    }
                }
                return i32::from_str_radix(&self.input[start + 2..self.pos], 16).unwrap_or(0);
            }
            // Could be octal, but just parse as decimal for simplicity
        }
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                self.advance();
            } else {
                break;
            }
        }
        self.input[start..self.pos].parse().unwrap_or(0)
    }

    /// Peek ahead to check if current identifier is followed by ':' (new rule definition).
    fn is_new_rule_ahead(&self) -> bool {
        let mut p = self.pos;
        // Skip identifier
        while p < self.input.len() {
            let c = self.input.as_bytes()[p] as char;
            if c.is_ascii_alphanumeric() || c == '_' || c == '.' {
                p += 1;
            } else {
                break;
            }
        }
        // Skip whitespace
        while p < self.input.len() {
            let c = self.input.as_bytes()[p] as char;
            if c == ' ' || c == '\t' || c == '\n' || c == '\r' {
                p += 1;
            } else {
                break;
            }
        }
        // Check for ':'
        p < self.input.len() && self.input.as_bytes()[p] == b':'
    }

    fn read_quoted_char(&mut self) -> String {
        let start = self.pos;
        self.advance(); // opening '
        while let Some(c) = self.advance() {
            if c == '\\' {
                self.advance(); // skip escaped char
            } else if c == '\'' {
                return self.input[start..self.pos].to_string();
            }
        }
        self.input[start..self.pos].to_string()
    }

    fn read_quoted_string(&mut self) -> String {
        let start = self.pos;
        self.advance(); // opening "
        while let Some(c) = self.advance() {
            if c == '\\' {
                self.advance();
            } else if c == '"' {
                return self.input[start..self.pos].to_string();
            }
        }
        self.input[start..self.pos].to_string()
    }

    fn read_tag(&mut self) -> Option<String> {
        self.skip_whitespace();
        if self.peek() == Some('<') {
            self.advance(); // <
            let start = self.pos;
            while let Some(c) = self.peek() {
                if c == '>' {
                    let tag = self.input[start..self.pos].to_string();
                    self.advance();
                    return Some(tag);
                }
                self.advance();
            }
        }
        None
    }

    fn read_braced_code(&mut self) -> String {
        let mut depth = 1;
        let start = self.pos;
        // We've already consumed the opening {
        while depth > 0 {
            match self.advance() {
                Some('{') => depth += 1,
                Some('}') => {
                    depth -= 1;
                    if depth == 0 {
                        return self.input[start..self.pos - 1].to_string();
                    }
                }
                Some('\'') => {
                    // skip char literal
                    while let Some(c) = self.advance() {
                        if c == '\\' {
                            self.advance();
                        } else if c == '\'' {
                            break;
                        }
                    }
                }
                Some('"') => {
                    // skip string literal
                    while let Some(c) = self.advance() {
                        if c == '\\' {
                            self.advance();
                        } else if c == '"' {
                            break;
                        }
                    }
                }
                Some('/') if self.peek() == Some('*') => {
                    self.advance();
                    loop {
                        match self.advance() {
                            Some('*') if self.peek() == Some('/') => {
                                self.advance();
                                break;
                            }
                            None => break,
                            _ => {}
                        }
                    }
                }
                Some('/') if self.peek() == Some('/') => {
                    while let Some(c) = self.advance() {
                        if c == '\n' {
                            break;
                        }
                    }
                }
                None => break,
                _ => {}
            }
        }
        self.input[start..self.pos].to_string()
    }

    fn read_prologue_block(&mut self) -> String {
        // %{ ... %} block
        let start = self.pos;
        loop {
            if self.input[self.pos..].starts_with("%}") {
                let code = self.input[start..self.pos].to_string();
                self.advance(); // %
                self.advance(); // }
                return code;
            }
            if self.advance().is_none() {
                return self.input[start..].to_string();
            }
        }
    }

    fn parse(&mut self) -> Result<(), String> {
        self.parse_declarations()?;
        self.expect_separator()?;
        self.parse_rules()?;
        self.parse_epilogue();
        self.finalize()?;
        Ok(())
    }

    fn expect_separator(&mut self) -> Result<(), String> {
        self.skip_whitespace();
        if self.input[self.pos..].starts_with("%%") {
            self.advance();
            self.advance();
            Ok(())
        } else {
            Err(format!("line {}: expected %%", self.line))
        }
    }

    fn parse_declarations(&mut self) -> Result<(), String> {
        loop {
            self.skip_whitespace();
            if self.pos >= self.input.len() {
                return Err("unexpected end of input in declarations".to_string());
            }
            if self.input[self.pos..].starts_with("%%") {
                return Ok(());
            }
            if self.input[self.pos..].starts_with("%{") {
                self.advance(); // %
                self.advance(); // {
                let code = self.read_prologue_block();
                self.grammar.prologue.push_str(&code);
                self.grammar.prologue.push('\n');
                continue;
            }
            if self.peek() == Some('%') {
                self.advance(); // %
                let directive = self.read_identifier();
                match directive.as_str() {
                    "token" => self.parse_token_decl(Assoc::None, 0)?,
                    "left" => {
                        let prec = self.grammar.next_prec;
                        self.grammar.next_prec += 1;
                        self.parse_token_decl(Assoc::Left, prec)?;
                    }
                    "right" => {
                        let prec = self.grammar.next_prec;
                        self.grammar.next_prec += 1;
                        self.parse_token_decl(Assoc::Right, prec)?;
                    }
                    "nonassoc" => {
                        let prec = self.grammar.next_prec;
                        self.grammar.next_prec += 1;
                        self.parse_token_decl(Assoc::NonAssoc, prec)?;
                    }
                    "union" => {
                        self.skip_whitespace();
                        if self.peek() == Some('{') {
                            self.advance();
                            self.grammar.union_code = self.read_braced_code();
                        }
                    }
                    "type" => self.parse_type_decl()?,
                    "start" => {
                        self.skip_whitespace();
                        let name = self.read_identifier();
                        let id = self.grammar.intern_symbol(&name);
                        self.grammar.start_symbol = Some(id);
                    }
                    "expect" => {
                        self.skip_whitespace();
                        let n = self.read_number();
                        self.grammar.expect_conflicts = Some(n);
                    }
                    "defines" => {
                        self.grammar.defines = true;
                        // Optionally followed by a filename in quotes
                        self.skip_whitespace();
                        if self.peek() == Some('"') {
                            self.read_quoted_string();
                        }
                    }
                    "define" => {
                        // %define directives - skip for now
                        self.skip_whitespace();
                        // Read key
                        while let Some(c) = self.peek() {
                            if c.is_ascii_whitespace() || c == '\n' {
                                break;
                            }
                            self.advance();
                        }
                        self.skip_whitespace();
                        // Read value if on same line
                        while let Some(c) = self.peek() {
                            if c == '\n' {
                                break;
                            }
                            self.advance();
                        }
                    }
                    "output" => {
                        self.skip_whitespace();
                        if self.peek() == Some('"') {
                            self.read_quoted_string();
                        }
                    }
                    "pure" | "pure-parser" | "pure_parser" => {
                        // skip optional "-parser"
                        self.skip_whitespace();
                    }
                    "error" | "verbose" => {
                        self.skip_whitespace();
                    }
                    "destructor" | "printer" => {
                        self.skip_whitespace();
                        if self.peek() == Some('{') {
                            self.advance();
                            self.read_braced_code();
                        }
                        // Skip symbol list
                        loop {
                            self.skip_whitespace();
                            match self.peek() {
                                Some(c)
                                    if c.is_ascii_alphabetic()
                                        || c == '_'
                                        || c == '<'
                                        || c == '\'' =>
                                {
                                    if c == '<' {
                                        self.read_tag();
                                    } else if c == '\'' {
                                        self.read_quoted_char();
                                    } else {
                                        self.read_identifier();
                                    }
                                }
                                _ => break,
                            }
                        }
                    }
                    "" => {
                        // Just a stray %, ignore
                    }
                    other => {
                        // Unknown directive, skip to end of line
                        eprintln!("warning: unknown directive %{}", other);
                        while let Some(c) = self.peek() {
                            if c == '\n' {
                                break;
                            }
                            self.advance();
                        }
                    }
                }
            } else {
                // Unknown content, skip line
                while let Some(c) = self.peek() {
                    if c == '\n' {
                        break;
                    }
                    self.advance();
                }
            }
        }
    }

    fn parse_token_decl(&mut self, assoc: Assoc, prec: i32) -> Result<(), String> {
        let tag = self.read_tag();
        loop {
            self.skip_whitespace();
            match self.peek() {
                Some(c) if c.is_ascii_alphabetic() || c == '_' => {
                    let name = self.read_identifier();
                    let id = self.grammar.intern_symbol(&name);
                    self.grammar.terminals.insert(id);

                    // Check for explicit value
                    self.skip_whitespace();
                    let value = if self.peek() == Some('{')
                        || self.peek().is_some_and(|c| c.is_ascii_digit())
                    {
                        let v = self.read_number();
                        if v >= self.grammar.next_token_value {
                            self.grammar.next_token_value = v + 1;
                        }
                        Some(v)
                    } else {
                        let v = self.grammar.next_token_value;
                        self.grammar.next_token_value += 1;
                        Some(v)
                    };

                    self.grammar.token_info.insert(
                        id,
                        TokenInfo {
                            name: name.clone(),
                            value,
                            tag: tag.clone(),
                            prec,
                            assoc,
                            is_literal: false,
                        },
                    );
                    if let Some(ref t) = tag {
                        self.grammar.type_tags.insert(id, t.clone());
                    }
                }
                Some('\'') => {
                    let lit = self.read_quoted_char();
                    let id = self.grammar.intern_symbol(&lit);
                    self.grammar.terminals.insert(id);
                    let ch_val = parse_char_literal(&lit[1..lit.len() - 1]).map(|c| c as i32);
                    self.grammar.token_info.insert(
                        id,
                        TokenInfo {
                            name: lit.clone(),
                            value: ch_val,
                            tag: tag.clone(),
                            prec,
                            assoc,
                            is_literal: true,
                        },
                    );
                    if let Some(ref t) = tag {
                        self.grammar.type_tags.insert(id, t.clone());
                    }
                }
                _ => break,
            }
        }
        Ok(())
    }

    fn parse_type_decl(&mut self) -> Result<(), String> {
        let tag = self
            .read_tag()
            .ok_or_else(|| format!("line {}: expected <tag> after %type", self.line))?;
        loop {
            self.skip_whitespace();
            match self.peek() {
                Some(c) if c.is_ascii_alphabetic() || c == '_' => {
                    let name = self.read_identifier();
                    let id = self.grammar.intern_symbol(&name);
                    self.grammar.type_tags.insert(id, tag.clone());
                }
                Some('\'') => {
                    let lit = self.read_quoted_char();
                    let id = self.grammar.intern_symbol(&lit);
                    self.grammar.type_tags.insert(id, tag.clone());
                }
                _ => break,
            }
        }
        Ok(())
    }

    fn parse_rules(&mut self) -> Result<(), String> {
        loop {
            self.skip_whitespace();
            if self.pos >= self.input.len() {
                return Ok(());
            }
            if self.input[self.pos..].starts_with("%%") {
                self.advance();
                self.advance();
                return Ok(());
            }
            self.parse_rule()?;
        }
    }

    fn parse_rule(&mut self) -> Result<(), String> {
        self.skip_whitespace();
        let lhs_name = self.read_identifier();
        if lhs_name.is_empty() {
            return Err(format!("line {}: expected rule name", self.line));
        }
        let lhs = self.grammar.intern_symbol(&lhs_name);
        self.grammar.nonterminals.insert(lhs);

        // Set start symbol to first rule's LHS if not specified
        if self.grammar.start_symbol.is_none() {
            self.grammar.start_symbol = Some(lhs);
        }

        self.skip_whitespace();
        // Expect ':'
        if self.peek() != Some(':') {
            return Err(format!(
                "line {}: expected ':' after '{}'",
                self.line, lhs_name
            ));
        }
        self.advance();

        // Parse alternatives
        loop {
            self.parse_alternative(lhs)?;
            self.skip_whitespace();
            match self.peek() {
                Some('|') => {
                    self.advance();
                }
                Some(';') => {
                    self.advance();
                    return Ok(());
                }
                _ => return Ok(()),
            }
        }
    }

    fn parse_alternative(&mut self, lhs: usize) -> Result<(), String> {
        let mut rhs = Vec::new();
        let mut action = String::new();
        let action_line = self.line;
        let mut prec_sym = None;

        loop {
            self.skip_whitespace();
            match self.peek() {
                None | Some(';') | Some('|') => break,
                Some('{') => {
                    self.advance();
                    action = self.read_braced_code();
                    self.skip_whitespace();
                    // Check if this is the end of the alternative:
                    // - End of input, ';', '|', '%%', or an identifier followed by ':'
                    //   (which starts a new rule)
                    let at_end = match self.peek() {
                        None | Some(';') | Some('|') => true,
                        Some('%') => true, // %% or %prec
                        Some(c) if c.is_ascii_alphabetic() || c == '_' => {
                            // Peek ahead: is this identifier followed by ':'?
                            self.is_new_rule_ahead()
                        }
                        _ => false,
                    };
                    if at_end {
                        break;
                    }
                    // Mid-rule action: create an anonymous nonterminal
                    let mid_name = format!("@{}", self.grammar.rules.len());
                    let mid_id = self.grammar.intern_symbol(&mid_name);
                    self.grammar.nonterminals.insert(mid_id);
                    let rule_number = self.grammar.rules.len();
                    self.grammar.rules.push(Rule {
                        lhs: mid_id,
                        rhs: Vec::new(),
                        action: action.clone(),
                        action_line,
                        prec_sym: None,
                        rule_number,
                    });
                    rhs.push(mid_id);
                    action = String::new();
                }
                Some('%') => {
                    // Check for %prec or end marker %%
                    if self.input[self.pos..].starts_with("%%") {
                        break;
                    }
                    if self.input[self.pos..].starts_with("%prec") {
                        self.advance(); // %
                        self.read_identifier(); // "prec"
                        self.skip_whitespace();
                        let sym_name = if self.peek() == Some('\'') {
                            self.read_quoted_char()
                        } else {
                            self.read_identifier()
                        };
                        let sym_id = self.grammar.intern_symbol(&sym_name);
                        prec_sym = Some(sym_id);
                    } else {
                        break;
                    }
                }
                Some('\'') => {
                    let lit = self.read_quoted_char();
                    let id = self.grammar.intern_symbol(&lit);
                    // Register as terminal if not already known
                    if !self.grammar.terminals.contains(&id)
                        && !self.grammar.nonterminals.contains(&id)
                    {
                        self.grammar.terminals.insert(id);
                        let ch_val = parse_char_literal(&lit[1..lit.len() - 1]).map(|c| c as i32);
                        self.grammar.token_info.insert(
                            id,
                            TokenInfo {
                                name: lit.clone(),
                                value: ch_val,
                                tag: None,
                                prec: 0,
                                assoc: Assoc::None,
                                is_literal: true,
                            },
                        );
                    }
                    rhs.push(id);
                }
                Some(c) if c.is_ascii_alphabetic() || c == '_' => {
                    let name = self.read_identifier();
                    let id = self.grammar.intern_symbol(&name);
                    rhs.push(id);
                }
                Some(c) => {
                    return Err(format!(
                        "line {}: unexpected character '{}' in rule",
                        self.line, c
                    ));
                }
            }
        }

        let rule_number = self.grammar.rules.len();
        self.grammar.rules.push(Rule {
            lhs,
            rhs,
            action,
            action_line,
            prec_sym,
            rule_number,
        });
        Ok(())
    }

    fn parse_epilogue(&mut self) {
        // Everything after the second %% is epilogue C code
        self.grammar.epilogue = self.input[self.pos..].to_string();
    }

    fn finalize(&mut self) -> Result<(), String> {
        // Ensure all symbols used in RHS are classified
        let rule_syms: Vec<Vec<usize>> = self.grammar.rules.iter().map(|r| r.rhs.clone()).collect();
        for rhs in &rule_syms {
            for &sym in rhs {
                if !self.grammar.terminals.contains(&sym)
                    && !self.grammar.nonterminals.contains(&sym)
                {
                    // Assume it's a terminal (undefined token used in grammar)
                    self.grammar.terminals.insert(sym);
                    let v = self.grammar.next_token_value;
                    self.grammar.next_token_value += 1;
                    self.grammar.token_info.insert(
                        sym,
                        TokenInfo {
                            name: self.grammar.symbols[sym].clone(),
                            value: Some(v),
                            tag: None,
                            prec: 0,
                            assoc: Assoc::None,
                            is_literal: false,
                        },
                    );
                }
            }
        }

        // Add augmented start rule: $accept -> start_symbol $end
        let start = self.grammar.start_symbol.ok_or("no start symbol defined")?;
        let accept_name = "$accept";
        let accept_id = self.grammar.intern_symbol(accept_name);
        self.grammar.nonterminals.insert(accept_id);
        self.grammar.augmented_start = accept_id;

        // Insert augmented rule at the beginning
        let aug_rule = Rule {
            lhs: accept_id,
            rhs: vec![start, EOF_TOKEN],
            action: String::new(),
            action_line: 0,
            prec_sym: None,
            rule_number: 0,
        };

        // Renumber rules: augmented rule is #0, shift others up
        for r in &mut self.grammar.rules {
            r.rule_number += 1;
        }
        self.grammar.rules.insert(0, aug_rule);

        Ok(())
    }
}

// ============================================================================
// LALR(1) Table Generation
// ============================================================================

struct TableGenerator<'a> {
    grammar: &'a Grammar,
    first_sets: HashMap<usize, BTreeSet<usize>>, // symbol -> set of terminal ids (EOF_TOKEN for epsilon)
    follow_sets: HashMap<usize, BTreeSet<usize>>,
    states: Vec<ItemSet>,
    state_map: BTreeMap<ItemSet, usize>,
    action_table: Vec<HashMap<usize, i32>>, // state -> terminal -> action
    goto_table: Vec<HashMap<usize, usize>>, // state -> nonterminal -> state
    sr_conflicts: i32,
    rr_conflicts: i32,
}

// Use a sentinel to represent epsilon in FIRST sets
const EPSILON: usize = usize::MAX;

impl<'a> TableGenerator<'a> {
    fn new(grammar: &'a Grammar) -> Self {
        TableGenerator {
            grammar,
            first_sets: HashMap::new(),
            follow_sets: HashMap::new(),
            states: Vec::new(),
            state_map: BTreeMap::new(),
            action_table: Vec::new(),
            goto_table: Vec::new(),
            sr_conflicts: 0,
            rr_conflicts: 0,
        }
    }

    fn generate(&mut self) {
        self.compute_first_sets();
        self.compute_follow_sets();
        self.build_states();
        self.build_tables();
    }

    fn compute_first_sets(&mut self) {
        // Initialize: terminals have themselves as FIRST
        for &t in &self.grammar.terminals {
            let mut s = BTreeSet::new();
            s.insert(t);
            self.first_sets.insert(t, s);
        }

        // Initialize nonterminals to empty
        for &nt in &self.grammar.nonterminals {
            self.first_sets.entry(nt).or_default();
        }

        // Fixed-point iteration
        let mut changed = true;
        while changed {
            changed = false;
            for rule in &self.grammar.rules {
                let before_size = self.first_sets.get(&rule.lhs).map_or(0, |s| s.len());

                if rule.rhs.is_empty() {
                    // Epsilon production
                    let set = self
                        .first_sets
                        .entry(rule.lhs)
                        .or_default();
                    if set.insert(EPSILON) {
                        changed = true;
                    }
                    continue;
                }

                let mut all_nullable = true;
                for &sym in &rule.rhs {
                    let sym_first: BTreeSet<usize> =
                        self.first_sets.get(&sym).cloned().unwrap_or_default();

                    let non_eps: Vec<usize> = sym_first
                        .iter()
                        .filter(|&&x| x != EPSILON)
                        .cloned()
                        .collect();

                    let set = self
                        .first_sets
                        .entry(rule.lhs)
                        .or_default();
                    for t in non_eps {
                        if set.insert(t) {
                            changed = true;
                        }
                    }

                    if !sym_first.contains(&EPSILON) {
                        all_nullable = false;
                        break;
                    }
                }

                if all_nullable {
                    let set = self
                        .first_sets
                        .entry(rule.lhs)
                        .or_default();
                    if set.insert(EPSILON) {
                        changed = true;
                    }
                }

                let after_size = self.first_sets.get(&rule.lhs).map_or(0, |s| s.len());
                if after_size != before_size {
                    changed = true;
                }
            }
        }
    }

    fn first_of_string(&self, symbols: &[usize]) -> BTreeSet<usize> {
        let mut result = BTreeSet::new();
        if symbols.is_empty() {
            result.insert(EPSILON);
            return result;
        }
        for &sym in symbols {
            let sym_first = self.first_sets.get(&sym).cloned().unwrap_or_default();
            for &t in &sym_first {
                if t != EPSILON {
                    result.insert(t);
                }
            }
            if !sym_first.contains(&EPSILON) {
                return result;
            }
        }
        result.insert(EPSILON);
        result
    }

    fn compute_follow_sets(&mut self) {
        // Initialize: FOLLOW(start) = {$end}
        for &nt in &self.grammar.nonterminals {
            self.follow_sets.entry(nt).or_default();
        }

        if let Some(start) = self.grammar.start_symbol {
            let set = self.follow_sets.entry(start).or_default();
            set.insert(EOF_TOKEN);
        }

        // Also add $end to FOLLOW($accept)
        let set = self
            .follow_sets
            .entry(self.grammar.augmented_start)
            .or_default();
        set.insert(EOF_TOKEN);

        let mut changed = true;
        while changed {
            changed = false;
            for rule in &self.grammar.rules {
                for (i, &sym) in rule.rhs.iter().enumerate() {
                    if !self.grammar.is_nonterminal(sym) {
                        continue;
                    }
                    let rest = &rule.rhs[i + 1..];
                    let first_rest = self.first_of_string(rest);

                    // Add FIRST(rest) - epsilon to FOLLOW(sym)
                    let non_eps: Vec<usize> = first_rest
                        .iter()
                        .filter(|&&x| x != EPSILON)
                        .cloned()
                        .collect();
                    let set = self.follow_sets.entry(sym).or_default();
                    for t in non_eps {
                        if set.insert(t) {
                            changed = true;
                        }
                    }

                    // If rest can derive epsilon, add FOLLOW(lhs) to FOLLOW(sym)
                    if first_rest.contains(&EPSILON) {
                        let follow_lhs: Vec<usize> = self
                            .follow_sets
                            .get(&rule.lhs)
                            .cloned()
                            .unwrap_or_default()
                            .into_iter()
                            .collect();
                        let set = self.follow_sets.entry(sym).or_default();
                        for t in follow_lhs {
                            if set.insert(t) {
                                changed = true;
                            }
                        }
                    }
                }
            }
        }
    }

    fn closure(&self, items: &ItemSet) -> ItemSet {
        let mut result = items.clone();
        let mut worklist: VecDeque<LR0Item> = items.iter().cloned().collect();

        while let Some(item) = worklist.pop_front() {
            let rule = &self.grammar.rules[item.rule];
            if item.dot < rule.rhs.len() {
                let next_sym = rule.rhs[item.dot];
                if self.grammar.is_nonterminal(next_sym) {
                    for (i, r) in self.grammar.rules.iter().enumerate() {
                        if r.lhs == next_sym {
                            let new_item = LR0Item { rule: i, dot: 0 };
                            if result.insert(new_item.clone()) {
                                worklist.push_back(new_item);
                            }
                        }
                    }
                }
            }
        }
        result
    }

    fn goto_set(&self, items: &ItemSet, symbol: usize) -> ItemSet {
        let mut moved = ItemSet::new();
        for item in items {
            let rule = &self.grammar.rules[item.rule];
            if item.dot < rule.rhs.len() && rule.rhs[item.dot] == symbol {
                moved.insert(LR0Item {
                    rule: item.rule,
                    dot: item.dot + 1,
                });
            }
        }
        self.closure(&moved)
    }

    fn build_states(&mut self) {
        let initial_item = LR0Item { rule: 0, dot: 0 };
        let mut initial = ItemSet::new();
        initial.insert(initial_item);
        let initial_closure = self.closure(&initial);

        self.states.push(initial_closure.clone());
        self.state_map.insert(initial_closure, 0);

        let mut queue = VecDeque::new();
        queue.push_back(0usize);

        while let Some(state_idx) = queue.pop_front() {
            // Collect all symbols after dot
            let state = self.states[state_idx].clone();
            let mut symbols = BTreeSet::new();
            for item in &state {
                let rule = &self.grammar.rules[item.rule];
                if item.dot < rule.rhs.len() {
                    symbols.insert(rule.rhs[item.dot]);
                }
            }

            for sym in symbols {
                let next = self.goto_set(&state, sym);
                if next.is_empty() {
                    continue;
                }
                if !self.state_map.contains_key(&next) {
                    let idx = self.states.len();
                    self.state_map.insert(next.clone(), idx);
                    self.states.push(next);
                    queue.push_back(idx);
                }
            }
        }
    }

    fn build_tables(&mut self) {
        let num_states = self.states.len();
        self.action_table = vec![HashMap::new(); num_states];
        self.goto_table = vec![HashMap::new(); num_states];

        for state_idx in 0..num_states {
            let state = self.states[state_idx].clone();

            for item in &state {
                let rule = &self.grammar.rules[item.rule];

                if item.dot < rule.rhs.len() {
                    // Shift
                    let sym = rule.rhs[item.dot];
                    if self.grammar.is_terminal(sym) {
                        let next_state = self.goto_set(&state, sym);
                        if let Some(&target) = self.state_map.get(&next_state) {
                            let shift_action = target as i32 + 1; // positive = shift

                            if let Some(&existing) = self.action_table[state_idx].get(&sym) {
                                if existing < 0 && existing != ERROR_ACTION {
                                    // Shift/reduce conflict
                                    let reduce_rule = (-existing - 1) as usize;
                                    let resolved =
                                        self.resolve_sr_conflict(sym, reduce_rule, shift_action);
                                    self.action_table[state_idx].insert(sym, resolved);
                                }
                                // If existing == shift_action, no conflict
                            } else {
                                self.action_table[state_idx].insert(sym, shift_action);
                            }
                        }
                    }
                } else {
                    // Reduce
                    if item.rule == 0 {
                        // Accept
                        self.action_table[state_idx].insert(EOF_TOKEN, ACCEPT_ACTION);
                        continue;
                    }

                    let reduce_action = -(item.rule as i32) - 1; // negative = reduce

                    // Use SLR(1) lookaheads via FOLLOW set
                    let follow = self.follow_sets.get(&rule.lhs).cloned().unwrap_or_default();
                    for &la in &follow {
                        if let Some(&existing) = self.action_table[state_idx].get(&la) {
                            if existing == ACCEPT_ACTION && la == EOF_TOKEN {
                                // Accept takes precedence
                                continue;
                            }
                            if existing > 0 {
                                // Shift/reduce conflict: existing is shift
                                let resolved = self.resolve_sr_conflict(la, item.rule, existing);
                                self.action_table[state_idx].insert(la, resolved);
                            } else if existing < 0 && existing != ERROR_ACTION {
                                // Reduce/reduce conflict: keep lower-numbered rule
                                let other_rule = (-existing - 1) as usize;
                                self.rr_conflicts += 1;
                                if item.rule < other_rule {
                                    self.action_table[state_idx].insert(la, reduce_action);
                                }
                                // else keep existing (lower numbered)
                            }
                        } else {
                            self.action_table[state_idx].insert(la, reduce_action);
                        }
                    }
                }
            }

            // Build GOTO table for nonterminals
            let mut nt_syms = BTreeSet::new();
            for item in &state {
                let rule = &self.grammar.rules[item.rule];
                if item.dot < rule.rhs.len() {
                    let sym = rule.rhs[item.dot];
                    if self.grammar.is_nonterminal(sym) {
                        nt_syms.insert(sym);
                    }
                }
            }

            for nt in nt_syms {
                let next = self.goto_set(&state, nt);
                if let Some(&target) = self.state_map.get(&next) {
                    self.goto_table[state_idx].insert(nt, target);
                }
            }
        }
    }

    fn resolve_sr_conflict(&mut self, token: usize, reduce_rule: usize, shift_action: i32) -> i32 {
        let (rule_prec, rule_assoc) = self.grammar.rule_prec(&self.grammar.rules[reduce_rule]);
        let (tok_prec, _tok_assoc) = if let Some(info) = self.grammar.token_info.get(&token) {
            (info.prec, info.assoc)
        } else {
            (0, Assoc::None)
        };

        if rule_prec > 0 && tok_prec > 0 {
            if rule_prec > tok_prec {
                return -(reduce_rule as i32) - 1; // reduce
            } else if tok_prec > rule_prec {
                return shift_action; // shift
            } else {
                // Same precedence: use associativity
                match rule_assoc {
                    Assoc::Left => return -(reduce_rule as i32) - 1, // reduce
                    Assoc::Right => return shift_action,             // shift
                    Assoc::NonAssoc => return ERROR_ACTION,          // error
                    Assoc::None => {}
                }
            }
        }

        // Default: shift wins
        self.sr_conflicts += 1;
        shift_action
    }
}

// ============================================================================
// C Code Generator
// ============================================================================

struct CodeGenerator<'a> {
    grammar: &'a Grammar,
    table: &'a TableGenerator<'a>,
    prefix: String,
}

impl<'a> CodeGenerator<'a> {
    fn new(grammar: &'a Grammar, table: &'a TableGenerator<'a>, prefix: String) -> Self {
        CodeGenerator {
            grammar,
            table,
            prefix,
        }
    }

    fn generate_source(&self) -> String {
        let mut out = String::new();
        self.emit_header(&mut out);
        self.emit_prologue(&mut out);
        self.emit_token_defines(&mut out);
        self.emit_yystype(&mut out);
        self.emit_tables(&mut out);
        self.emit_parser(&mut out);
        self.emit_epilogue(&mut out);
        out
    }

    fn generate_header(&self) -> String {
        let mut out = String::new();
        let guard = format!("{}_TAB_H", self.prefix.to_uppercase());
        out.push_str(&format!("#ifndef {}\n#define {}\n\n", guard, guard));
        self.emit_token_defines(&mut out);
        self.emit_yystype(&mut out);
        out.push_str(&format!("extern YYSTYPE {}lval;\n", self.prefix));
        out.push_str(&format!("\n#endif /* {} */\n", guard));
        out
    }

    fn emit_header(&self, out: &mut String) {
        out.push_str("/* A Bison-like parser, generated by rust-bison.  */\n\n");
        out.push_str("#include <stdio.h>\n");
        out.push_str("#include <stdlib.h>\n");
        out.push_str("#include <string.h>\n\n");
    }

    fn emit_prologue(&self, out: &mut String) {
        if !self.grammar.prologue.is_empty() {
            out.push_str("/* Prologue */\n");
            out.push_str(&self.grammar.prologue);
            out.push('\n');
        }
    }

    fn emit_token_defines(&self, out: &mut String) {
        out.push_str("/* Token definitions */\n");
        // Collect tokens and sort by value for deterministic output
        let mut tokens: Vec<(String, i32)> = Vec::new();
        for info in self.grammar.token_info.values() {
            if info.is_literal || info.name == "$end" || info.name == "error" {
                continue;
            }
            if let Some(v) = info.value {
                tokens.push((info.name.clone(), v));
            }
        }
        tokens.sort_by_key(|(_, v)| *v);
        tokens.dedup();

        // Emit as enum
        if !tokens.is_empty() {
            out.push_str("#ifndef YYTOKENTYPE\n");
            out.push_str("#define YYTOKENTYPE\n");
            out.push_str("enum yytokentype {\n");
            for (name, val) in &tokens {
                out.push_str(&format!("  {} = {},\n", name, val));
            }
            out.push_str("};\n");
            out.push_str("#endif\n\n");

            // Also emit #defines for compatibility
            for (name, val) in &tokens {
                out.push_str(&format!("#define {} {}\n", name, val));
            }
            out.push('\n');
        }
    }

    fn emit_yystype(&self, out: &mut String) {
        if !self.grammar.union_code.is_empty() {
            out.push_str("#ifndef YYSTYPE\n");
            out.push_str("typedef union YYSTYPE {\n");
            out.push_str(&self.grammar.union_code);
            out.push_str("\n} YYSTYPE;\n");
            out.push_str("#define YYSTYPE YYSTYPE\n");
            out.push_str("#endif\n\n");
        } else {
            out.push_str("#ifndef YYSTYPE\n");
            out.push_str("typedef int YYSTYPE;\n");
            out.push_str("#define YYSTYPE YYSTYPE\n");
            out.push_str("#endif\n\n");
        }
    }

    fn emit_tables(&self, out: &mut String) {
        let num_states = self.table.states.len();
        let num_rules = self.grammar.rules.len();
        let p = &self.prefix;

        // Rule lengths (yyr2)
        out.push_str(&format!("static const short {}r2[] = {{\n  ", p));
        for (i, rule) in self.grammar.rules.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            out.push_str(&format!("{}", rule.rhs.len()));
        }
        out.push_str("\n};\n\n");

        // Rule LHS symbol internal index (yyr1) - map nonterminals to dense ids
        let mut nt_map: BTreeMap<usize, usize> = BTreeMap::new();
        let mut nt_idx = 0;
        // Assign indices to nonterminals in order of first appearance
        for rule in &self.grammar.rules {
            if let std::collections::btree_map::Entry::Vacant(e) = nt_map.entry(rule.lhs) {
                e.insert(nt_idx);
                nt_idx += 1;
            }
        }

        out.push_str(&format!("static const short {}r1[] = {{\n  ", p));
        for (i, rule) in self.grammar.rules.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            out.push_str(&format!("{}", nt_map.get(&rule.lhs).unwrap()));
        }
        out.push_str("\n};\n\n");

        // Collect all terminal symbols that appear in the action table
        let mut all_terminals: BTreeSet<usize> = BTreeSet::new();
        for state_actions in &self.table.action_table {
            for &tok in state_actions.keys() {
                all_terminals.insert(tok);
            }
        }
        let term_list: Vec<usize> = all_terminals.iter().cloned().collect();
        let mut term_idx_map: HashMap<usize, usize> = HashMap::new();
        for (i, &t) in term_list.iter().enumerate() {
            term_idx_map.insert(t, i);
        }

        // Number of terminals and nonterminals
        let num_terms = term_list.len();
        let num_nonterms = nt_map.len();

        out.push_str(&format!(
            "#define {}NTOKENS {}\n",
            p.to_uppercase(),
            num_terms
        ));
        out.push_str(&format!(
            "#define {}NSTATES {}\n",
            p.to_uppercase(),
            num_states
        ));
        out.push_str(&format!(
            "#define {}NRULES {}\n",
            p.to_uppercase(),
            num_rules
        ));
        out.push_str(&format!(
            "#define {}NNONTERMS {}\n\n",
            p.to_uppercase(),
            num_nonterms
        ));

        // Terminal value to index mapping table
        out.push_str("/* Terminal symbol token values */\n");
        out.push_str(&format!("static const int {}token_values[] = {{\n  ", p));
        for (i, &t) in term_list.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            out.push_str(&format!("{}", self.grammar.token_c_value(t)));
        }
        out.push_str("\n};\n\n");

        // Action table: state x terminal -> action
        // Encode: 0 = accept, positive = shift to state N-1, negative = reduce by rule -(N+1)
        // ERROR_ACTION = error
        out.push_str(&format!(
            "static const int {}action[][{}] = {{\n",
            p, num_terms
        ));
        for state_idx in 0..num_states {
            out.push_str("  { ");
            for (j, &t) in term_list.iter().enumerate() {
                if j > 0 {
                    out.push_str(", ");
                }
                let action = self.table.action_table[state_idx]
                    .get(&t)
                    .copied()
                    .unwrap_or(ERROR_ACTION);
                out.push_str(&format!("{}", action));
            }
            out.push_str(" },\n");
        }
        out.push_str("};\n\n");

        // Nonterminal list for goto
        let mut nt_by_idx: Vec<usize> = vec![0; num_nonterms];
        for (&nt, &idx) in &nt_map {
            nt_by_idx[idx] = nt;
        }

        // GOTO table: state x nonterminal -> state (-1 = error)
        out.push_str(&format!(
            "static const int {}goto[][{}] = {{\n",
            p, num_nonterms
        ));
        for state_idx in 0..num_states {
            out.push_str("  { ");
            for (j, &nt) in nt_by_idx.iter().enumerate() {
                if j > 0 {
                    out.push_str(", ");
                }
                let target = self.table.goto_table[state_idx]
                    .get(&nt)
                    .map(|&s| s as i32)
                    .unwrap_or(-1);
                out.push_str(&format!("{}", target));
            }
            out.push_str(" },\n");
        }
        out.push_str("};\n\n");
    }

    fn emit_parser(&self, out: &mut String) {
        let p = &self.prefix;
        let pu = p.to_uppercase();
        // (terminal info already emitted in tables)

        out.push_str(&format!("YYSTYPE {}lval;\n\n", p));

        // Location type stub
        out.push_str("typedef struct YYLTYPE {\n");
        out.push_str("  int first_line;\n");
        out.push_str("  int first_column;\n");
        out.push_str("  int last_line;\n");
        out.push_str("  int last_column;\n");
        out.push_str("} YYLTYPE;\n\n");

        // Forward declarations
        out.push_str(&format!("int {}lex(void);\n", p));
        out.push_str(&format!("void {}error(const char *s);\n\n", p));

        // Token to table index function
        out.push_str(&format!("static int {p}token_to_index(int token) {{\n"));
        out.push_str("  int i;\n");
        out.push_str(&format!("  for (i = 0; i < {}NTOKENS; i++) {{\n", pu));
        out.push_str(&format!(
            "    if ({}token_values[i] == token) return i;\n",
            p
        ));
        out.push_str("  }\n");
        out.push_str("  return -1;\n");
        out.push_str("}\n\n");

        // Stack size
        out.push_str("#ifndef YYMAXDEPTH\n");
        out.push_str("#define YYMAXDEPTH 10000\n");
        out.push_str("#endif\n");
        out.push_str("#ifndef YYINITDEPTH\n");
        out.push_str("#define YYINITDEPTH 200\n");
        out.push_str("#endif\n\n");

        // yyparse function
        out.push_str(&format!("int {}parse(void) {{\n", p));
        out.push_str("  int yystate = 0;\n");
        out.push_str("  int yychar = -1; /* empty */\n");
        out.push_str("  int yynerrs = 0;\n");
        out.push_str("  int yyn, yylen, yyidx;\n");
        out.push_str("  int yyss[YYMAXDEPTH];\n");
        out.push_str("  YYSTYPE yyvs[YYMAXDEPTH];\n");
        out.push_str("  int yyssp = 0;\n");
        out.push_str("  YYSTYPE yyval;\n\n");
        out.push_str("  yyss[0] = 0;\n");
        out.push_str("  memset(&yyvs[0], 0, sizeof(YYSTYPE));\n\n");
        out.push_str("  for (;;) {\n");

        // Get lookahead if needed
        out.push_str("    if (yychar < 0) {\n");
        out.push_str(&format!("      yychar = {}lex();\n", p));
        out.push_str("      if (yychar < 0) yychar = 0; /* EOF */\n");
        out.push_str("    }\n\n");

        // Look up action
        out.push_str(&format!("    yyidx = {p}token_to_index(yychar);\n"));
        out.push_str("    if (yyidx < 0) {\n");
        out.push_str("      goto yyerrlab;\n");
        out.push_str("    }\n\n");
        out.push_str(&format!("    yyn = {}action[yystate][yyidx];\n\n", p));

        // Accept
        out.push_str(&format!("    if (yyn == {}) {{\n", ACCEPT_ACTION));
        out.push_str("      return 0; /* accept */\n");
        out.push_str("    }\n\n");

        // Error
        out.push_str(&format!("    if (yyn == {}) {{\n", ERROR_ACTION));
        out.push_str("      goto yyerrlab;\n");
        out.push_str("    }\n\n");

        // Shift
        out.push_str("    if (yyn > 0) {\n");
        out.push_str("      /* Shift */\n");
        out.push_str("      yyssp++;\n");
        out.push_str("      if (yyssp >= YYMAXDEPTH) {\n");
        out.push_str(&format!("        {}error(\"parser stack overflow\");\n", p));
        out.push_str("        return 2;\n");
        out.push_str("      }\n");
        out.push_str("      yystate = yyn - 1;\n");
        out.push_str("      yyss[yyssp] = yystate;\n");
        out.push_str(&format!("      yyvs[yyssp] = {}lval;\n", p));
        out.push_str("      yychar = -1; /* consumed */\n");
        out.push_str("      continue;\n");
        out.push_str("    }\n\n");

        // Reduce
        out.push_str("    /* Reduce */\n");
        out.push_str("    yyn = -(yyn + 1); /* rule number */\n");
        out.push_str(&format!("    yylen = {}r2[yyn];\n", p));
        out.push_str("    memset(&yyval, 0, sizeof(YYSTYPE));\n\n");

        // Semantic actions via switch
        out.push_str("    switch (yyn) {\n");
        for (i, rule) in self.grammar.rules.iter().enumerate() {
            if i == 0 {
                continue;
            } // Skip augmented rule
            if rule.action.is_empty() {
                // Default action: $$ = $1
                if !rule.rhs.is_empty() {
                    out.push_str(&format!("    case {}:\n", i));
                    out.push_str("      yyval = yyvs[yyssp - yylen + 1];\n");
                    out.push_str("      break;\n");
                }
                continue;
            }
            out.push_str(&format!("    case {}:\n", i));
            out.push_str(&format!("#line {} \"{}\"\n", rule.action_line, "input.y")); // TODO: actual filename

            // Transform action: replace $$, $N, @$, @N
            let transformed = self.transform_action(&rule.action, rule.rhs.len());
            out.push_str("      { ");
            out.push_str(&transformed);
            out.push_str(" }\n");
            out.push_str("      break;\n");
        }
        out.push_str("    default: break;\n");
        out.push_str("    }\n\n");

        // Pop stack and push result
        out.push_str("    yyssp -= yylen;\n");
        out.push_str("    yystate = yyss[yyssp];\n");
        out.push_str(&format!("    {{ int nt = {}r1[yyn];\n", p));
        out.push_str(&format!("      int newstate = {}goto[yystate][nt];\n", p));
        out.push_str("      if (newstate < 0) goto yyerrlab;\n");
        out.push_str("      yyssp++;\n");
        out.push_str("      yystate = newstate;\n");
        out.push_str("      yyss[yyssp] = yystate;\n");
        out.push_str("      yyvs[yyssp] = yyval;\n");
        out.push_str("    }\n");
        out.push_str("  }\n\n");

        // Error handling
        out.push_str("yyerrlab:\n");
        out.push_str(&format!("  {}error(\"syntax error\");\n", p));
        out.push_str("  yynerrs++;\n");
        out.push_str("  return 1;\n");
        out.push_str("}\n");
    }

    fn transform_action(&self, action: &str, _rhs_len: usize) -> String {
        let mut result = String::with_capacity(action.len());
        let chars: Vec<char> = action.chars().collect();
        let len = chars.len();
        let mut i = 0;

        while i < len {
            if chars[i] == '$' && i + 1 < len {
                if chars[i + 1] == '$' {
                    // $$ -> yyval
                    // Check for type tag: $<tag>$
                    result.push_str("yyval");
                    i += 2;
                } else if chars[i + 1].is_ascii_digit()
                    || (chars[i + 1] == '-' && i + 2 < len && chars[i + 2].is_ascii_digit())
                {
                    // $N -> yyvs[yyssp - yylen + N]
                    i += 1;
                    let neg = chars[i] == '-';
                    if neg {
                        i += 1;
                    }
                    let start = i;
                    while i < len && chars[i].is_ascii_digit() {
                        i += 1;
                    }
                    let num: i32 = chars[start..i]
                        .iter()
                        .collect::<String>()
                        .parse()
                        .unwrap_or(0);
                    let num = if neg { -num } else { num };
                    result.push_str(&format!("yyvs[yyssp - yylen + {}]", num));
                } else if chars[i + 1] == '<' {
                    // $<tag>$ or $<tag>N - typed reference
                    i += 2;
                    // skip tag
                    while i < len && chars[i] != '>' {
                        i += 1;
                    }
                    if i < len {
                        i += 1;
                    } // skip >
                    if i < len && chars[i] == '$' {
                        result.push_str("yyval");
                        i += 1;
                    } else if i < len && (chars[i].is_ascii_digit() || chars[i] == '-') {
                        let neg = chars[i] == '-';
                        if neg {
                            i += 1;
                        }
                        let start = i;
                        while i < len && chars[i].is_ascii_digit() {
                            i += 1;
                        }
                        let num: i32 = chars[start..i]
                            .iter()
                            .collect::<String>()
                            .parse()
                            .unwrap_or(0);
                        let num = if neg { -num } else { num };
                        result.push_str(&format!("yyvs[yyssp - yylen + {}]", num));
                    }
                } else {
                    result.push(chars[i]);
                    i += 1;
                }
            } else if chars[i] == '@' && i + 1 < len {
                if chars[i + 1] == '$' {
                    // @$ -> yyloc (stub)
                    result.push_str("yyloc");
                    i += 2;
                } else if chars[i + 1].is_ascii_digit() {
                    // @N -> location stub
                    i += 1;
                    let start = i;
                    while i < len && chars[i].is_ascii_digit() {
                        i += 1;
                    }
                    let num: i32 = chars[start..i]
                        .iter()
                        .collect::<String>()
                        .parse()
                        .unwrap_or(0);
                    result.push_str(&format!("yylsp[yyssp - yylen + {}]", num));
                } else {
                    result.push(chars[i]);
                    i += 1;
                }
            } else {
                result.push(chars[i]);
                i += 1;
            }
        }

        result
    }

    fn emit_epilogue(&self, out: &mut String) {
        if !self.grammar.epilogue.is_empty() {
            out.push_str("\n/* Epilogue */\n");
            out.push_str(&self.grammar.epilogue);
        }
    }
}

// ============================================================================
// Verbose output (.output file)
// ============================================================================

fn generate_verbose_output(grammar: &Grammar, table: &TableGenerator) -> String {
    let mut out = String::new();

    // Grammar rules
    out.push_str("Grammar\n\n");
    for (i, rule) in grammar.rules.iter().enumerate() {
        out.push_str(&format!("  {:>4} {}: ", i, grammar.symbol_name(rule.lhs)));
        if rule.rhs.is_empty() {
            out.push_str("/* empty */");
        } else {
            for (j, &sym) in rule.rhs.iter().enumerate() {
                if j > 0 {
                    out.push(' ');
                }
                out.push_str(grammar.symbol_name(sym));
            }
        }
        out.push('\n');
    }
    out.push('\n');

    // Terminals
    out.push_str("Terminals, with rules where they appear\n\n");
    let mut term_rules: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (i, rule) in grammar.rules.iter().enumerate() {
        for &sym in &rule.rhs {
            if grammar.is_terminal(sym) {
                term_rules.entry(sym).or_default().push(i);
            }
        }
    }
    for (&t, rules) in &term_rules {
        out.push_str(&format!(
            "  {} ({})",
            grammar.symbol_name(t),
            grammar.token_c_value(t)
        ));
        for r in rules {
            out.push_str(&format!(" {}", r));
        }
        out.push('\n');
    }
    out.push('\n');

    // Nonterminals
    out.push_str("Nonterminals, with rules where they appear\n\n");
    let mut nt_rules: BTreeMap<usize, (Vec<usize>, Vec<usize>)> = BTreeMap::new();
    for (i, rule) in grammar.rules.iter().enumerate() {
        nt_rules.entry(rule.lhs).or_default().0.push(i);
        for &sym in &rule.rhs {
            if grammar.is_nonterminal(sym) {
                nt_rules.entry(sym).or_default().1.push(i);
            }
        }
    }
    for (&nt, (lhs_rules, rhs_rules)) in &nt_rules {
        out.push_str(&format!("  {} (on left:", grammar.symbol_name(nt)));
        for r in lhs_rules {
            out.push_str(&format!(" {}", r));
        }
        out.push_str(", on right:");
        for r in rhs_rules {
            out.push_str(&format!(" {}", r));
        }
        out.push_str(")\n");
    }
    out.push('\n');

    // States
    for (state_idx, state) in table.states.iter().enumerate() {
        out.push_str(&format!("\nState {}\n\n", state_idx));
        for item in state {
            let rule = &grammar.rules[item.rule];
            out.push_str(&format!(
                "  {:>4} {}: ",
                item.rule,
                grammar.symbol_name(rule.lhs)
            ));
            for (j, &sym) in rule.rhs.iter().enumerate() {
                if j == item.dot {
                    out.push_str(". ");
                }
                out.push_str(grammar.symbol_name(sym));
                if j < rule.rhs.len() - 1 {
                    out.push(' ');
                }
            }
            if item.dot == rule.rhs.len() {
                out.push_str(" .");
            }
            out.push('\n');
        }
        out.push('\n');

        // Actions
        for (&tok, &action) in &table.action_table[state_idx] {
            let tok_name = grammar.symbol_name(tok);
            if action == ACCEPT_ACTION {
                out.push_str(&format!("    {}  accept\n", tok_name));
            } else if action == ERROR_ACTION {
                out.push_str(&format!("    {}  error\n", tok_name));
            } else if action > 0 {
                out.push_str(&format!(
                    "    {}  shift, and go to state {}\n",
                    tok_name,
                    action - 1
                ));
            } else {
                out.push_str(&format!(
                    "    {}  reduce using rule {}\n",
                    tok_name,
                    -(action + 1)
                ));
            }
        }

        // Gotos
        for (&nt, &target) in &table.goto_table[state_idx] {
            out.push_str(&format!(
                "    {}  go to state {}\n",
                grammar.symbol_name(nt),
                target
            ));
        }
    }

    out
}

// ============================================================================
// Display for Grammar (debug)
// ============================================================================

impl fmt::Display for Grammar {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(
            f,
            "Grammar ({} symbols, {} rules):",
            self.symbols.len(),
            self.rules.len()
        )?;
        for (i, rule) in self.rules.iter().enumerate() {
            write!(f, "  R{}: {} ->", i, self.symbol_name(rule.lhs))?;
            if rule.rhs.is_empty() {
                write!(f, " (empty)")?;
            }
            for &sym in &rule.rhs {
                write!(f, " {}", self.symbol_name(sym))?;
            }
            writeln!(f)?;
        }
        Ok(())
    }
}

// ============================================================================
// CLI and main
// ============================================================================

struct Options {
    input_file: String,
    output_file: Option<String>,
    header_file: Option<String>,
    verbose: bool,
    defines: bool,
    prefix: String,
}

fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut opts = Options {
        input_file: String::new(),
        output_file: None,
        header_file: None,
        verbose: false,
        defines: false,
        prefix: "yy".to_string(),
    };

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--version" | "-V" => {
                println!("bison (rust-bison) {}", VERSION);
                process::exit(0);
            }
            "--help" | "-h" => {
                print_usage();
                process::exit(0);
            }
            "-d" | "--defines" => {
                opts.defines = true;
                // Optional: --defines=FILE
                if args[i].contains('=') {
                    let parts: Vec<&str> = args[i].splitn(2, '=').collect();
                    if parts.len() == 2 {
                        opts.header_file = Some(parts[1].to_string());
                    }
                }
            }
            "-v" | "--verbose" => {
                opts.verbose = true;
            }
            "-o" | "--output" => {
                i += 1;
                if i >= args.len() {
                    return Err("-o requires an argument".to_string());
                }
                opts.output_file = Some(args[i].clone());
            }
            "-p" | "--name-prefix" => {
                i += 1;
                if i >= args.len() {
                    return Err("-p requires an argument".to_string());
                }
                opts.prefix = args[i].clone();
            }
            "-b" | "--file-prefix" => {
                i += 1;
                // Consume but don't use for now
            }
            "-y" | "--yacc" => {
                // POSIX yacc compatibility mode (default behavior)
            }
            arg if arg.starts_with("-o") => {
                opts.output_file = Some(arg[2..].to_string());
            }
            arg if arg.starts_with("-p") => {
                opts.prefix = arg[2..].to_string();
            }
            arg if arg.starts_with("--output=") => {
                opts.output_file = Some(arg["--output=".len()..].to_string());
            }
            arg if arg.starts_with("--name-prefix=") => {
                opts.prefix = arg["--name-prefix=".len()..].to_string();
            }
            arg if arg.starts_with("--defines=") => {
                opts.defines = true;
                opts.header_file = Some(arg["--defines=".len()..].to_string());
            }
            arg if !arg.starts_with('-') => {
                opts.input_file = arg.to_string();
            }
            arg => {
                return Err(format!("unrecognized option: {}", arg));
            }
        }
        i += 1;
    }

    if opts.input_file.is_empty() {
        return Err("no input file".to_string());
    }

    Ok(opts)
}

fn print_usage() {
    eprintln!("Usage: bison [OPTIONS] FILE");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  -d, --defines[=FILE]       generate header file");
    eprintln!("  -v, --verbose              generate verbose .output file");
    eprintln!("  -o, --output=FILE          output to FILE");
    eprintln!("  -p, --name-prefix=PREFIX   prepend PREFIX to external symbols");
    eprintln!("  -V, --version              print version");
    eprintln!("  -h, --help                 print this help");
}

fn default_output_file(input: &str) -> String {
    let stem = Path::new(input)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("y");
    format!("{}.tab.c", stem)
}

fn default_header_file(output: &str) -> String {
    if output.ends_with(".c") {
        format!("{}h", &output[..output.len() - 1])
    } else {
        format!("{}.h", output)
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let opts = match parse_args(&args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("bison: {}", e);
            process::exit(1);
        }
    };

    let input = match fs::read_to_string(&opts.input_file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("bison: {}: {}", opts.input_file, e);
            process::exit(1);
        }
    };

    // Parse grammar
    let mut parser = YaccParser::new(&input);
    if let Err(e) = parser.parse() {
        eprintln!("bison: {}: {}", opts.input_file, e);
        process::exit(1);
    }

    let grammar = parser.grammar;

    // Check for -d from grammar %defines directive
    let wants_defines = opts.defines || grammar.defines;

    // Generate tables
    let mut table_gen = TableGenerator::new(&grammar);
    table_gen.generate();

    // Report conflicts
    let sr = table_gen.sr_conflicts;
    let rr = table_gen.rr_conflicts;
    if let Some(expected) = grammar.expect_conflicts {
        if sr != expected {
            eprintln!(
                "bison: {}: warning: {} shift/reduce conflicts (expected {})",
                opts.input_file, sr, expected
            );
        }
    } else if sr > 0 {
        eprintln!(
            "bison: {}: warning: {} shift/reduce conflict{}",
            opts.input_file,
            sr,
            if sr == 1 { "" } else { "s" }
        );
    }
    if rr > 0 {
        eprintln!(
            "bison: {}: warning: {} reduce/reduce conflict{}",
            opts.input_file,
            rr,
            if rr == 1 { "" } else { "s" }
        );
    }

    // Generate C code
    let code_gen = CodeGenerator::new(&grammar, &table_gen, opts.prefix.clone());
    let output_file = opts
        .output_file
        .unwrap_or_else(|| default_output_file(&opts.input_file));

    let source = code_gen.generate_source();
    if let Err(e) = fs::write(&output_file, &source) {
        eprintln!("bison: {}: {}", output_file, e);
        process::exit(1);
    }

    // Generate header if requested
    if wants_defines {
        let header_file = opts
            .header_file
            .unwrap_or_else(|| default_header_file(&output_file));
        let header = code_gen.generate_header();
        if let Err(e) = fs::write(&header_file, &header) {
            eprintln!("bison: {}: {}", header_file, e);
            process::exit(1);
        }
    }

    // Generate verbose output if requested
    if opts.verbose {
        let verbose_file = format!(
            "{}.output",
            Path::new(&opts.input_file)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("y")
        );
        let verbose = generate_verbose_output(&grammar, &table_gen);
        if let Err(e) = fs::write(&verbose_file, &verbose) {
            eprintln!("bison: {}: {}", verbose_file, e);
            process::exit(1);
        }
    }
}
