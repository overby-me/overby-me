#![allow(dead_code)]

use regex::Regex;
use std::collections::HashMap;
use std::env;
use std::fmt;
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::process::{Command, Stdio};

// ============================================================================
// Lexer
// ============================================================================

#[derive(Debug, Clone, PartialEq)]
enum Token {
    // Literals
    Number(f64),
    StringLit(String),
    Regex(String),

    // Identifiers and keywords
    Ident(String),
    Begin,
    End,
    If,
    Else,
    While,
    For,
    Do,
    In,
    Delete,
    Print,
    Printf,
    Getline,
    Next,
    Exit,
    Function,
    Return,
    Break,
    Continue,

    // Operators
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Caret,
    Assign,
    PlusAssign,
    MinusAssign,
    StarAssign,
    SlashAssign,
    PercentAssign,
    CaretAssign,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    And,
    Or,
    Not,
    Match,
    NotMatch,
    Increment,
    Decrement,
    Append,

    // Delimiters
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Semicolon,
    Comma,
    Dollar,
    Pipe,
    PipeBoth,
    Question,
    Colon,

    // Special
    Newline,
    Eof,
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Token::Number(n) => write!(f, "{n}"),
            Token::StringLit(s) => write!(f, "\"{s}\""),
            Token::Ident(s) => write!(f, "{s}"),
            _ => write!(f, "{self:?}"),
        }
    }
}

struct Lexer {
    input: Vec<char>,
    pos: usize,
    tokens: Vec<Token>,
}

impl Lexer {
    fn new(input: &str) -> Self {
        Lexer {
            input: input.chars().collect(),
            pos: 0,
            tokens: Vec::new(),
        }
    }

    fn peek(&self) -> Option<char> {
        self.input.get(self.pos).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let ch = self.input.get(self.pos).copied();
        if ch.is_some() {
            self.pos += 1;
        }
        ch
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        self.input.get(self.pos + offset).copied()
    }

    fn skip_whitespace(&mut self) {
        while let Some(ch) = self.peek() {
            if ch == ' ' || ch == '\t' || ch == '\r' {
                self.advance();
            } else if ch == '\\' && self.peek_at(1) == Some('\n') {
                self.advance();
                self.advance();
            } else if ch == '#' {
                while let Some(c) = self.peek() {
                    if c == '\n' {
                        break;
                    }
                    self.advance();
                }
            } else {
                break;
            }
        }
    }

    fn read_string(&mut self) -> String {
        let mut s = String::new();
        // skip opening quote
        self.advance();
        while let Some(ch) = self.advance() {
            if ch == '\\' {
                if let Some(esc) = self.advance() {
                    match esc {
                        'n' => s.push('\n'),
                        't' => s.push('\t'),
                        'r' => s.push('\r'),
                        '\\' => s.push('\\'),
                        '"' => s.push('"'),
                        'a' => s.push('\x07'),
                        'b' => s.push('\x08'),
                        'f' => s.push('\x0C'),
                        'v' => s.push('\x0B'),
                        '/' => s.push('/'),
                        _ => {
                            s.push('\\');
                            s.push(esc);
                        }
                    }
                }
            } else if ch == '"' {
                break;
            } else {
                s.push(ch);
            }
        }
        s
    }

    fn read_regex(&mut self) -> String {
        let mut s = String::new();
        // skip opening /
        self.advance();
        while let Some(ch) = self.advance() {
            if ch == '\\' {
                if let Some(next) = self.advance() {
                    if next == '/' {
                        s.push('/');
                    } else {
                        s.push('\\');
                        s.push(next);
                    }
                }
            } else if ch == '/' {
                break;
            } else {
                s.push(ch);
            }
        }
        s
    }

    fn read_number(&mut self) -> f64 {
        let mut s = String::new();
        if self.peek() == Some('0')
            && (self.peek_at(1) == Some('x') || self.peek_at(1) == Some('X'))
        {
            s.push(self.advance().unwrap());
            s.push(self.advance().unwrap());
            while let Some(ch) = self.peek() {
                if ch.is_ascii_hexdigit() {
                    s.push(self.advance().unwrap());
                } else {
                    break;
                }
            }
            return i64::from_str_radix(&s[2..], 16).unwrap_or(0) as f64;
        }
        while let Some(ch) = self.peek() {
            if ch.is_ascii_digit() || ch == '.' {
                s.push(self.advance().unwrap());
            } else if ch == 'e' || ch == 'E' {
                s.push(self.advance().unwrap());
                if self.peek() == Some('+') || self.peek() == Some('-') {
                    s.push(self.advance().unwrap());
                }
            } else {
                break;
            }
        }
        s.parse().unwrap_or(0.0)
    }

    fn read_ident(&mut self) -> String {
        let mut s = String::new();
        while let Some(ch) = self.peek() {
            if ch.is_alphanumeric() || ch == '_' {
                s.push(self.advance().unwrap());
            } else {
                break;
            }
        }
        s
    }

    fn can_be_regex(&self) -> bool {
        // A '/' is a regex if the previous token is one that can precede a regex
        if let Some(last) = self.tokens.last() {
            matches!(
                last,
                Token::Newline
                    | Token::Semicolon
                    | Token::LBrace
                    | Token::RBrace
                    | Token::LParen
                    | Token::Comma
                    | Token::Not
                    | Token::And
                    | Token::Or
                    | Token::Match
                    | Token::NotMatch
                    | Token::Print
                    | Token::Printf
                    | Token::Return
                    | Token::Pipe
            )
        } else {
            true // beginning of input
        }
    }

    fn tokenize(&mut self) -> Vec<Token> {
        loop {
            self.skip_whitespace();
            let ch = match self.peek() {
                Some(c) => c,
                None => {
                    self.tokens.push(Token::Eof);
                    break;
                }
            };

            let tok = match ch {
                '\n' => {
                    self.advance();
                    Token::Newline
                }
                '"' => Token::StringLit(self.read_string()),
                '/' if self.can_be_regex() => Token::Regex(self.read_regex()),
                '0'..='9' | '.' if ch.is_ascii_digit() || {
                    ch == '.'
                        && self
                            .peek_at(1)
                            .is_some_and(|c| c.is_ascii_digit())
                } =>
                {
                    Token::Number(self.read_number())
                }
                'a'..='z' | 'A'..='Z' | '_' => {
                    let ident = self.read_ident();
                    match ident.as_str() {
                        "BEGIN" => Token::Begin,
                        "END" => Token::End,
                        "if" => Token::If,
                        "else" => Token::Else,
                        "while" => Token::While,
                        "for" => Token::For,
                        "do" => Token::Do,
                        "in" => Token::In,
                        "delete" => Token::Delete,
                        "print" => Token::Print,
                        "printf" => Token::Printf,
                        "getline" => Token::Getline,
                        "next" => Token::Next,
                        "exit" => Token::Exit,
                        "function" => Token::Function,
                        "return" => Token::Return,
                        "break" => Token::Break,
                        "continue" => Token::Continue,
                        _ => Token::Ident(ident),
                    }
                }
                '+' => {
                    self.advance();
                    if self.peek() == Some('+') {
                        self.advance();
                        Token::Increment
                    } else if self.peek() == Some('=') {
                        self.advance();
                        Token::PlusAssign
                    } else {
                        Token::Plus
                    }
                }
                '-' => {
                    self.advance();
                    if self.peek() == Some('-') {
                        self.advance();
                        Token::Decrement
                    } else if self.peek() == Some('=') {
                        self.advance();
                        Token::MinusAssign
                    } else {
                        Token::Minus
                    }
                }
                '*' => {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        Token::StarAssign
                    } else {
                        Token::Star
                    }
                }
                '/' => {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        Token::SlashAssign
                    } else {
                        Token::Slash
                    }
                }
                '%' => {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        Token::PercentAssign
                    } else {
                        Token::Percent
                    }
                }
                '^' => {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        Token::CaretAssign
                    } else {
                        Token::Caret
                    }
                }
                '=' => {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        Token::Eq
                    } else {
                        Token::Assign
                    }
                }
                '!' => {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        Token::Ne
                    } else if self.peek() == Some('~') {
                        self.advance();
                        Token::NotMatch
                    } else {
                        Token::Not
                    }
                }
                '<' => {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        Token::Le
                    } else {
                        Token::Lt
                    }
                }
                '>' => {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        Token::Ge
                    } else if self.peek() == Some('>') {
                        self.advance();
                        Token::Append
                    } else {
                        Token::Gt
                    }
                }
                '&' => {
                    self.advance();
                    if self.peek() == Some('&') {
                        self.advance();
                        Token::And
                    } else {
                        // lone & not typically used in awk but handle gracefully
                        Token::Ident("&".to_string())
                    }
                }
                '|' => {
                    self.advance();
                    if self.peek() == Some('|') {
                        self.advance();
                        Token::Or
                    } else if self.peek() == Some('&') {
                        self.advance();
                        Token::PipeBoth
                    } else {
                        Token::Pipe
                    }
                }
                '~' => {
                    self.advance();
                    Token::Match
                }
                '(' => {
                    self.advance();
                    Token::LParen
                }
                ')' => {
                    self.advance();
                    Token::RParen
                }
                '{' => {
                    self.advance();
                    Token::LBrace
                }
                '}' => {
                    self.advance();
                    Token::RBrace
                }
                '[' => {
                    self.advance();
                    Token::LBracket
                }
                ']' => {
                    self.advance();
                    Token::RBracket
                }
                ';' => {
                    self.advance();
                    Token::Semicolon
                }
                ',' => {
                    self.advance();
                    Token::Comma
                }
                '$' => {
                    self.advance();
                    Token::Dollar
                }
                '?' => {
                    self.advance();
                    Token::Question
                }
                ':' => {
                    self.advance();
                    Token::Colon
                }
                _ => {
                    self.advance();
                    continue;
                }
            };

            self.tokens.push(tok);
        }

        self.tokens.clone()
    }
}

// ============================================================================
// AST
// ============================================================================

#[derive(Debug, Clone)]
enum Expr {
    Number(f64),
    StringLit(String),
    Regex(String),
    Var(String),
    FieldRef(Box<Expr>),
    ArrayRef(String, Vec<Expr>),
    Binop(Box<Expr>, BinOp, Box<Expr>),
    Unop(UnOp, Box<Expr>),
    PostIncrement(Box<Expr>),
    PostDecrement(Box<Expr>),
    Assign(Box<Expr>, Box<Expr>),
    OpAssign(Box<Expr>, BinOp, Box<Expr>),
    Match(Box<Expr>, Box<Expr>),
    NotMatch(Box<Expr>, Box<Expr>),
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),
    Concat(Box<Expr>, Box<Expr>),
    In(Box<Expr>, String),
    MultiIn(Vec<Expr>, String),
    FuncCall(String, Vec<Expr>),
    Getline(Option<Box<Expr>>, Option<Box<Expr>>, GetlineSource),
    Sprintf(Vec<Expr>),
    Pipe(Box<Expr>, Box<Expr>),
}

#[derive(Debug, Clone)]
enum GetlineSource {
    Stdin,
    File,
    Pipe,
}

#[derive(Debug, Clone, Copy)]
enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    And,
    Or,
}

#[derive(Debug, Clone, Copy)]
enum UnOp {
    Neg,
    Not,
    PreIncrement,
    PreDecrement,
}

#[derive(Debug, Clone)]
enum Stmt {
    Expr(Expr),
    Print(Vec<Expr>, Option<OutputDest>),
    Printf(Vec<Expr>, Option<OutputDest>),
    If(Expr, Box<Stmt>, Option<Box<Stmt>>),
    While(Expr, Box<Stmt>),
    DoWhile(Box<Stmt>, Expr),
    For(Option<Box<Stmt>>, Option<Expr>, Option<Box<Stmt>>, Box<Stmt>),
    ForIn(String, String, Box<Stmt>),
    Block(Vec<Stmt>),
    Next,
    Exit(Option<Expr>),
    Delete(String, Vec<Expr>),
    Break,
    Continue,
    Return(Option<Expr>),
    Getline(Option<Box<Expr>>, Option<Box<Expr>>, GetlineSource),
}

#[derive(Debug, Clone)]
enum OutputDest {
    File(Expr),
    Append(Expr),
    Pipe(Expr),
}

#[derive(Debug, Clone)]
enum Pattern {
    Begin,
    End,
    Expression(Expr),
    Range(Expr, Expr),
}

#[derive(Debug, Clone)]
struct Rule {
    pattern: Option<Pattern>,
    action: Vec<Stmt>,
}

#[derive(Debug, Clone)]
struct FuncDef {
    name: String,
    params: Vec<String>,
    body: Vec<Stmt>,
}

#[derive(Debug, Clone)]
struct Program {
    rules: Vec<Rule>,
    functions: Vec<FuncDef>,
}

// ============================================================================
// Parser
// ============================================================================

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Parser { tokens, pos: 0 }
    }

    fn peek(&self) -> &Token {
        self.tokens.get(self.pos).unwrap_or(&Token::Eof)
    }

    fn advance(&mut self) -> Token {
        let tok = self.tokens.get(self.pos).cloned().unwrap_or(Token::Eof);
        self.pos += 1;
        tok
    }

    fn expect(&mut self, expected: &Token) {
        let tok = self.advance();
        if std::mem::discriminant(&tok) != std::mem::discriminant(expected) {
            // silently skip for robustness
        }
    }

    fn skip_terminators(&mut self) {
        while matches!(self.peek(), Token::Newline | Token::Semicolon) {
            self.advance();
        }
    }

    fn parse(&mut self) -> Program {
        let mut rules = Vec::new();
        let mut functions = Vec::new();

        self.skip_terminators();

        while !matches!(self.peek(), Token::Eof) {
            if matches!(self.peek(), Token::Function) {
                functions.push(self.parse_function_def());
            } else {
                rules.push(self.parse_rule());
            }
            self.skip_terminators();
        }

        Program { rules, functions }
    }

    fn parse_function_def(&mut self) -> FuncDef {
        self.advance(); // 'function'
        let name = match self.advance() {
            Token::Ident(s) => s,
            _ => String::new(),
        };
        self.expect(&Token::LParen);
        let mut params = Vec::new();
        while !matches!(self.peek(), Token::RParen | Token::Eof) {
            if let Token::Ident(s) = self.advance() {
                params.push(s);
            }
            if matches!(self.peek(), Token::Comma) {
                self.advance();
            }
        }
        self.expect(&Token::RParen);
        self.skip_terminators();
        let body = self.parse_action();
        FuncDef { name, params, body }
    }

    fn parse_rule(&mut self) -> Rule {
        match self.peek().clone() {
            Token::Begin => {
                self.advance();
                self.skip_terminators();
                let action = self.parse_action();
                Rule {
                    pattern: Some(Pattern::Begin),
                    action,
                }
            }
            Token::End => {
                self.advance();
                self.skip_terminators();
                let action = self.parse_action();
                Rule {
                    pattern: Some(Pattern::End),
                    action,
                }
            }
            Token::LBrace => {
                let action = self.parse_action();
                Rule {
                    pattern: None,
                    action,
                }
            }
            _ => {
                let expr = self.parse_expr();
                if matches!(self.peek(), Token::Comma) {
                    self.advance();
                    self.skip_terminators();
                    let expr2 = self.parse_expr();
                    self.skip_terminators();
                    if matches!(self.peek(), Token::LBrace) {
                        let action = self.parse_action();
                        Rule {
                            pattern: Some(Pattern::Range(expr, expr2)),
                            action,
                        }
                    } else {
                        Rule {
                            pattern: Some(Pattern::Range(expr, expr2)),
                            action: vec![Stmt::Print(vec![], None)],
                        }
                    }
                } else {
                    self.skip_terminators();
                    if matches!(self.peek(), Token::LBrace) {
                        let action = self.parse_action();
                        Rule {
                            pattern: Some(Pattern::Expression(expr)),
                            action,
                        }
                    } else {
                        Rule {
                            pattern: Some(Pattern::Expression(expr)),
                            action: vec![Stmt::Print(vec![], None)],
                        }
                    }
                }
            }
        }
    }

    fn parse_action(&mut self) -> Vec<Stmt> {
        self.expect(&Token::LBrace);
        self.skip_terminators();
        let mut stmts = Vec::new();
        while !matches!(self.peek(), Token::RBrace | Token::Eof) {
            stmts.push(self.parse_stmt());
            self.skip_terminators();
        }
        self.expect(&Token::RBrace);
        stmts
    }

    fn parse_stmt(&mut self) -> Stmt {
        match self.peek().clone() {
            Token::If => self.parse_if(),
            Token::While => self.parse_while(),
            Token::For => self.parse_for(),
            Token::Do => self.parse_do_while(),
            Token::LBrace => {
                let stmts = self.parse_action();
                Stmt::Block(stmts)
            }
            Token::Print => self.parse_print(),
            Token::Printf => self.parse_printf(),
            Token::Next => {
                self.advance();
                Stmt::Next
            }
            Token::Exit => {
                self.advance();
                if matches!(
                    self.peek(),
                    Token::Newline | Token::Semicolon | Token::RBrace | Token::Eof
                ) {
                    Stmt::Exit(None)
                } else {
                    Stmt::Exit(Some(self.parse_expr()))
                }
            }
            Token::Delete => {
                self.advance();
                if let Token::Ident(name) = self.advance() {
                    if matches!(self.peek(), Token::LBracket) {
                        self.advance();
                        let mut indices = vec![self.parse_expr()];
                        while matches!(self.peek(), Token::Comma) {
                            self.advance();
                            indices.push(self.parse_expr());
                        }
                        self.expect(&Token::RBracket);
                        Stmt::Delete(name, indices)
                    } else {
                        Stmt::Delete(name, vec![])
                    }
                } else {
                    Stmt::Expr(Expr::Number(0.0))
                }
            }
            Token::Break => {
                self.advance();
                Stmt::Break
            }
            Token::Continue => {
                self.advance();
                Stmt::Continue
            }
            Token::Return => {
                self.advance();
                if matches!(
                    self.peek(),
                    Token::Newline | Token::Semicolon | Token::RBrace | Token::Eof
                ) {
                    Stmt::Return(None)
                } else {
                    Stmt::Return(Some(self.parse_expr()))
                }
            }
            _ => {
                let expr = self.parse_expr();
                Stmt::Expr(expr)
            }
        }
    }

    fn parse_if(&mut self) -> Stmt {
        self.advance(); // if
        self.expect(&Token::LParen);
        let cond = self.parse_expr();
        self.expect(&Token::RParen);
        self.skip_terminators();
        let then_branch = self.parse_stmt();
        self.skip_terminators();
        let else_branch = if matches!(self.peek(), Token::Else) {
            self.advance();
            self.skip_terminators();
            Some(Box::new(self.parse_stmt()))
        } else {
            None
        };
        Stmt::If(cond, Box::new(then_branch), else_branch)
    }

    fn parse_while(&mut self) -> Stmt {
        self.advance(); // while
        self.expect(&Token::LParen);
        let cond = self.parse_expr();
        self.expect(&Token::RParen);
        self.skip_terminators();
        let body = self.parse_stmt();
        Stmt::While(cond, Box::new(body))
    }

    fn parse_for(&mut self) -> Stmt {
        self.advance(); // for
        self.expect(&Token::LParen);

        // Check for for-in: for (var in array)
        let saved_pos = self.pos;
        if let Token::Ident(var_name) = self.peek().clone() {
            self.advance();
            if matches!(self.peek(), Token::In) {
                self.advance();
                if let Token::Ident(arr_name) = self.advance() {
                    self.expect(&Token::RParen);
                    self.skip_terminators();
                    let body = self.parse_stmt();
                    return Stmt::ForIn(var_name, arr_name, Box::new(body));
                }
            }
        }
        self.pos = saved_pos;

        // Regular for loop
        let init = if matches!(self.peek(), Token::Semicolon) {
            None
        } else {
            Some(Box::new(self.parse_stmt()))
        };
        self.expect(&Token::Semicolon);
        let cond = if matches!(self.peek(), Token::Semicolon) {
            None
        } else {
            Some(self.parse_expr())
        };
        self.expect(&Token::Semicolon);
        let update = if matches!(self.peek(), Token::RParen) {
            None
        } else {
            Some(Box::new(self.parse_stmt()))
        };
        self.expect(&Token::RParen);
        self.skip_terminators();
        let body = self.parse_stmt();
        Stmt::For(init, cond, update, Box::new(body))
    }

    fn parse_do_while(&mut self) -> Stmt {
        self.advance(); // do
        self.skip_terminators();
        let body = self.parse_stmt();
        self.skip_terminators();
        self.expect(&Token::While);
        self.expect(&Token::LParen);
        let cond = self.parse_expr();
        self.expect(&Token::RParen);
        Stmt::DoWhile(Box::new(body), cond)
    }

    fn parse_output_dest(&mut self) -> Option<OutputDest> {
        match self.peek() {
            Token::Gt => {
                self.advance();
                Some(OutputDest::File(self.parse_primary()))
            }
            Token::Append => {
                self.advance();
                Some(OutputDest::Append(self.parse_primary()))
            }
            Token::Pipe => {
                self.advance();
                Some(OutputDest::Pipe(self.parse_primary()))
            }
            _ => None,
        }
    }

    fn parse_print(&mut self) -> Stmt {
        self.advance(); // print
        let mut args = Vec::new();

        if !matches!(
            self.peek(),
            Token::Newline
                | Token::Semicolon
                | Token::RBrace
                | Token::Eof
                | Token::Gt
                | Token::Append
                | Token::Pipe
        ) {
            args.push(self.parse_non_assign_expr());
            while matches!(self.peek(), Token::Comma) {
                self.advance();
                args.push(self.parse_non_assign_expr());
            }
        }

        let dest = self.parse_output_dest();
        Stmt::Print(args, dest)
    }

    fn parse_printf(&mut self) -> Stmt {
        self.advance(); // printf
        let mut args = Vec::new();

        if !matches!(
            self.peek(),
            Token::Newline | Token::Semicolon | Token::RBrace | Token::Eof
        ) {
            args.push(self.parse_non_assign_expr());
            while matches!(self.peek(), Token::Comma) {
                self.advance();
                args.push(self.parse_non_assign_expr());
            }
        }

        let dest = self.parse_output_dest();
        Stmt::Printf(args, dest)
    }

    // Expression parsing with precedence climbing

    fn parse_expr(&mut self) -> Expr {
        self.parse_assignment()
    }

    fn parse_non_assign_expr(&mut self) -> Expr {
        self.parse_ternary()
    }

    fn parse_assignment(&mut self) -> Expr {
        let expr = self.parse_ternary();
        match self.peek() {
            Token::Assign => {
                self.advance();
                let rhs = self.parse_assignment();
                Expr::Assign(Box::new(expr), Box::new(rhs))
            }
            Token::PlusAssign => {
                self.advance();
                let rhs = self.parse_assignment();
                Expr::OpAssign(Box::new(expr), BinOp::Add, Box::new(rhs))
            }
            Token::MinusAssign => {
                self.advance();
                let rhs = self.parse_assignment();
                Expr::OpAssign(Box::new(expr), BinOp::Sub, Box::new(rhs))
            }
            Token::StarAssign => {
                self.advance();
                let rhs = self.parse_assignment();
                Expr::OpAssign(Box::new(expr), BinOp::Mul, Box::new(rhs))
            }
            Token::SlashAssign => {
                self.advance();
                let rhs = self.parse_assignment();
                Expr::OpAssign(Box::new(expr), BinOp::Div, Box::new(rhs))
            }
            Token::PercentAssign => {
                self.advance();
                let rhs = self.parse_assignment();
                Expr::OpAssign(Box::new(expr), BinOp::Mod, Box::new(rhs))
            }
            Token::CaretAssign => {
                self.advance();
                let rhs = self.parse_assignment();
                Expr::OpAssign(Box::new(expr), BinOp::Pow, Box::new(rhs))
            }
            _ => expr,
        }
    }

    fn parse_ternary(&mut self) -> Expr {
        let cond = self.parse_or();
        if matches!(self.peek(), Token::Question) {
            self.advance();
            let then_expr = self.parse_assignment();
            self.expect(&Token::Colon);
            let else_expr = self.parse_assignment();
            Expr::Ternary(Box::new(cond), Box::new(then_expr), Box::new(else_expr))
        } else {
            cond
        }
    }

    fn parse_or(&mut self) -> Expr {
        let mut left = self.parse_and();
        while matches!(self.peek(), Token::Or) {
            self.advance();
            let right = self.parse_and();
            left = Expr::Binop(Box::new(left), BinOp::Or, Box::new(right));
        }
        left
    }

    fn parse_and(&mut self) -> Expr {
        let mut left = self.parse_in_expr();
        while matches!(self.peek(), Token::And) {
            self.advance();
            let right = self.parse_in_expr();
            left = Expr::Binop(Box::new(left), BinOp::And, Box::new(right));
        }
        left
    }

    fn parse_in_expr(&mut self) -> Expr {
        let left = self.parse_match();
        if matches!(self.peek(), Token::In) {
            self.advance();
            if let Token::Ident(arr) = self.advance() {
                return Expr::In(Box::new(left), arr);
            }
        }
        left
    }

    fn parse_match(&mut self) -> Expr {
        let left = self.parse_comparison();
        match self.peek() {
            Token::Match => {
                self.advance();
                let right = self.parse_comparison();
                Expr::Match(Box::new(left), Box::new(right))
            }
            Token::NotMatch => {
                self.advance();
                let right = self.parse_comparison();
                Expr::NotMatch(Box::new(left), Box::new(right))
            }
            _ => left,
        }
    }

    fn parse_comparison(&mut self) -> Expr {
        let left = self.parse_concatenation();
        let op = match self.peek() {
            Token::Eq => BinOp::Eq,
            Token::Ne => BinOp::Ne,
            Token::Lt => BinOp::Lt,
            Token::Gt => BinOp::Gt,
            Token::Le => BinOp::Le,
            Token::Ge => BinOp::Ge,
            _ => return left,
        };
        self.advance();
        let right = self.parse_concatenation();
        Expr::Binop(Box::new(left), op, Box::new(right))
    }

    fn parse_concatenation(&mut self) -> Expr {
        let mut left = self.parse_addition();
        // Concatenation by juxtaposition: if the next token can start an expression
        // but is not an operator, it's concatenation
        loop {
            match self.peek() {
                Token::Number(_)
                | Token::StringLit(_)
                | Token::Ident(_)
                | Token::Dollar
                | Token::LParen
                | Token::Not
                | Token::Increment
                | Token::Decrement => {
                    let right = self.parse_addition();
                    left = Expr::Concat(Box::new(left), Box::new(right));
                }
                _ => break,
            }
        }
        left
    }

    fn parse_addition(&mut self) -> Expr {
        let mut left = self.parse_multiplication();
        loop {
            match self.peek() {
                Token::Plus => {
                    self.advance();
                    let right = self.parse_multiplication();
                    left = Expr::Binop(Box::new(left), BinOp::Add, Box::new(right));
                }
                Token::Minus => {
                    self.advance();
                    let right = self.parse_multiplication();
                    left = Expr::Binop(Box::new(left), BinOp::Sub, Box::new(right));
                }
                _ => break,
            }
        }
        left
    }

    fn parse_multiplication(&mut self) -> Expr {
        let mut left = self.parse_power();
        loop {
            match self.peek() {
                Token::Star => {
                    self.advance();
                    let right = self.parse_power();
                    left = Expr::Binop(Box::new(left), BinOp::Mul, Box::new(right));
                }
                Token::Slash => {
                    self.advance();
                    let right = self.parse_power();
                    left = Expr::Binop(Box::new(left), BinOp::Div, Box::new(right));
                }
                Token::Percent => {
                    self.advance();
                    let right = self.parse_power();
                    left = Expr::Binop(Box::new(left), BinOp::Mod, Box::new(right));
                }
                _ => break,
            }
        }
        left
    }

    fn parse_power(&mut self) -> Expr {
        let base = self.parse_unary();
        if matches!(self.peek(), Token::Caret) {
            self.advance();
            let exp = self.parse_power(); // right associative
            Expr::Binop(Box::new(base), BinOp::Pow, Box::new(exp))
        } else {
            base
        }
    }

    fn parse_unary(&mut self) -> Expr {
        match self.peek() {
            Token::Minus => {
                self.advance();
                let expr = self.parse_unary();
                Expr::Unop(UnOp::Neg, Box::new(expr))
            }
            Token::Not => {
                self.advance();
                let expr = self.parse_unary();
                Expr::Unop(UnOp::Not, Box::new(expr))
            }
            Token::Increment => {
                self.advance();
                let expr = self.parse_unary();
                Expr::Unop(UnOp::PreIncrement, Box::new(expr))
            }
            Token::Decrement => {
                self.advance();
                let expr = self.parse_unary();
                Expr::Unop(UnOp::PreDecrement, Box::new(expr))
            }
            _ => self.parse_postfix(),
        }
    }

    fn parse_postfix(&mut self) -> Expr {
        let mut expr = self.parse_primary();
        loop {
            match self.peek() {
                Token::Increment => {
                    self.advance();
                    expr = Expr::PostIncrement(Box::new(expr));
                }
                Token::Decrement => {
                    self.advance();
                    expr = Expr::PostDecrement(Box::new(expr));
                }
                Token::LBracket => {
                    if let Expr::Var(name) = expr {
                        self.advance();
                        let mut indices = vec![self.parse_expr()];
                        while matches!(self.peek(), Token::Comma) {
                            self.advance();
                            indices.push(self.parse_expr());
                        }
                        self.expect(&Token::RBracket);
                        expr = Expr::ArrayRef(name, indices);
                    } else {
                        break;
                    }
                }
                _ => break,
            }
        }
        expr
    }

    fn parse_primary(&mut self) -> Expr {
        match self.peek().clone() {
            Token::Number(n) => {
                self.advance();
                Expr::Number(n)
            }
            Token::StringLit(s) => {
                self.advance();
                Expr::StringLit(s)
            }
            Token::Regex(r) => {
                self.advance();
                // Bare regex matches against $0
                Expr::Match(
                    Box::new(Expr::FieldRef(Box::new(Expr::Number(0.0)))),
                    Box::new(Expr::Regex(r)),
                )
            }
            Token::Dollar => {
                self.advance();
                let expr = self.parse_primary();
                Expr::FieldRef(Box::new(expr))
            }
            Token::LParen => {
                self.advance();
                // Check for (expr) in array
                let expr = self.parse_expr();
                if matches!(self.peek(), Token::RParen) {
                    self.advance();
                    // Check for (expr) in array at higher level
                    if matches!(self.peek(), Token::In) {
                        self.advance();
                        if let Token::Ident(arr) = self.advance() {
                            return Expr::In(Box::new(expr), arr);
                        }
                    }
                    expr
                } else {
                    // This shouldn't normally happen
                    self.expect(&Token::RParen);
                    expr
                }
            }
            Token::Getline => {
                self.advance();
                // getline [var] [< file]
                let var = if matches!(self.peek(), Token::Ident(_)) {
                    let saved = self.pos;
                    if let Token::Ident(v) = self.advance() {
                        // Check if this looks like a variable for getline
                        if matches!(self.peek(), Token::Lt)
                            || matches!(
                                self.peek(),
                                Token::Newline
                                    | Token::Semicolon
                                    | Token::RBrace
                                    | Token::Eof
                                    | Token::RParen
                                    | Token::Pipe
                            )
                        {
                            Some(Box::new(Expr::Var(v)))
                        } else {
                            self.pos = saved;
                            None
                        }
                    } else {
                        self.pos = saved;
                        None
                    }
                } else {
                    None
                };

                if matches!(self.peek(), Token::Lt) {
                    self.advance();
                    let file = self.parse_primary();
                    Expr::Getline(var, Some(Box::new(file)), GetlineSource::File)
                } else {
                    Expr::Getline(var, None, GetlineSource::Stdin)
                }
            }
            Token::Ident(name) => {
                self.advance();
                if matches!(self.peek(), Token::LParen) {
                    // Function call
                    self.advance();
                    let mut args = Vec::new();
                    while !matches!(self.peek(), Token::RParen | Token::Eof) {
                        args.push(self.parse_expr());
                        if matches!(self.peek(), Token::Comma) {
                            self.advance();
                        }
                    }
                    self.expect(&Token::RParen);
                    Expr::FuncCall(name, args)
                } else if matches!(self.peek(), Token::LBracket) {
                    self.advance();
                    let mut indices = vec![self.parse_expr()];
                    while matches!(self.peek(), Token::Comma) {
                        self.advance();
                        indices.push(self.parse_expr());
                    }
                    self.expect(&Token::RBracket);
                    Expr::ArrayRef(name, indices)
                } else {
                    Expr::Var(name)
                }
            }
            _ => {
                self.advance();
                Expr::Number(0.0)
            }
        }
    }
}

// ============================================================================
// Interpreter
// ============================================================================

#[derive(Debug)]
enum ControlFlow {
    None,
    Next,
    Exit(i32),
    Break,
    Continue,
    Return(Value),
}

#[derive(Debug, Clone)]
enum Value {
    Num(f64),
    Str(String),
    Uninitialized,
}

impl Value {
    fn to_num(&self) -> f64 {
        match self {
            Value::Num(n) => *n,
            Value::Str(s) => parse_num(s),
            Value::Uninitialized => 0.0,
        }
    }

    fn to_string_val(&self) -> String {
        match self {
            Value::Num(n) => format_number(*n),
            Value::Str(s) => s.clone(),
            Value::Uninitialized => String::new(),
        }
    }

    fn to_bool(&self) -> bool {
        match self {
            Value::Num(n) => *n != 0.0,
            Value::Str(s) => !s.is_empty(),
            Value::Uninitialized => false,
        }
    }

    fn is_numeric_string(&self) -> bool {
        match self {
            Value::Num(_) => true,
            Value::Str(s) => {
                let s = s.trim();
                if s.is_empty() {
                    return false;
                }
                s.parse::<f64>().is_ok()
            }
            Value::Uninitialized => false,
        }
    }
}

fn parse_num(s: &str) -> f64 {
    let s = s.trim();
    if s.is_empty() {
        return 0.0;
    }
    // Parse as much of the leading part as possible
    let mut end = 0;
    let chars: Vec<char> = s.chars().collect();
    if end < chars.len() && (chars[end] == '+' || chars[end] == '-') {
        end += 1;
    }
    let mut has_digits = false;
    while end < chars.len() && chars[end].is_ascii_digit() {
        end += 1;
        has_digits = true;
    }
    if end < chars.len() && chars[end] == '.' {
        end += 1;
        while end < chars.len() && chars[end].is_ascii_digit() {
            end += 1;
            has_digits = true;
        }
    }
    if has_digits && end < chars.len() && (chars[end] == 'e' || chars[end] == 'E') {
        end += 1;
        if end < chars.len() && (chars[end] == '+' || chars[end] == '-') {
            end += 1;
        }
        while end < chars.len() && chars[end].is_ascii_digit() {
            end += 1;
        }
    }
    if !has_digits {
        return 0.0;
    }
    let num_str: String = chars[..end].iter().collect();
    num_str.parse().unwrap_or(0.0)
}

fn format_number(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e16 && !n.is_nan() && !n.is_infinite() {
        format!("{}", n as i64)
    } else {
        // Use %.6g style formatting like awk
        let s = format!("{:.6}", n);
        // Trim trailing zeros after decimal point
        if s.contains('.') {
            let s = s.trim_end_matches('0');
            let s = s.trim_end_matches('.');
            s.to_string()
        } else {
            s
        }
    }
}

fn compare_values(a: &Value, b: &Value) -> std::cmp::Ordering {
    // If both are numeric or numeric strings, compare as numbers
    if a.is_numeric_string() && b.is_numeric_string() {
        let na = a.to_num();
        let nb = b.to_num();
        na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal)
    } else {
        let sa = a.to_string_val();
        let sb = b.to_string_val();
        sa.cmp(&sb)
    }
}

struct Interpreter {
    globals: HashMap<String, Value>,
    arrays: HashMap<String, HashMap<String, Value>>,
    fields: Vec<String>,
    functions: HashMap<String, FuncDef>,
    open_files: HashMap<String, Box<dyn Write>>,
    open_read_files: HashMap<String, Box<dyn BufRead>>,
    open_pipes: HashMap<String, Box<dyn Write>>,
    open_read_pipes: HashMap<String, Box<dyn BufRead>>,
    rng_state: u64,
    range_active: HashMap<usize, bool>,
    nr: i64,
    fnr: i64,
    exit_code: i32,
}

impl Interpreter {
    fn new() -> Self {
        let mut globals = HashMap::new();
        globals.insert("FS".to_string(), Value::Str(" ".to_string()));
        globals.insert("RS".to_string(), Value::Str("\n".to_string()));
        globals.insert("OFS".to_string(), Value::Str(" ".to_string()));
        globals.insert("ORS".to_string(), Value::Str("\n".to_string()));
        globals.insert("NR".to_string(), Value::Num(0.0));
        globals.insert("NF".to_string(), Value::Num(0.0));
        globals.insert("FNR".to_string(), Value::Num(0.0));
        globals.insert("FILENAME".to_string(), Value::Str(String::new()));
        globals.insert("SUBSEP".to_string(), Value::Str("\x1c".to_string()));
        globals.insert("RSTART".to_string(), Value::Num(0.0));
        globals.insert("RLENGTH".to_string(), Value::Num(-1.0));
        globals.insert("OFMT".to_string(), Value::Str("%.6g".to_string()));
        globals.insert("CONVFMT".to_string(), Value::Str("%.6g".to_string()));

        Interpreter {
            globals,
            arrays: HashMap::new(),
            fields: vec![String::new()],
            functions: HashMap::new(),
            open_files: HashMap::new(),
            open_read_files: HashMap::new(),
            open_pipes: HashMap::new(),
            open_read_pipes: HashMap::new(),
            rng_state: 0,
            range_active: HashMap::new(),
            nr: 0,
            fnr: 0,
            exit_code: 0,
        }
    }

    fn set_record(&mut self, line: &str) {
        let fs = self.globals.get("FS").map(|v| v.to_string_val()).unwrap_or(" ".to_string());
        self.fields = vec![line.to_string()];
        let parts: Vec<String> = if fs == " " {
            line.split_whitespace().map(|s| s.to_string()).collect()
        } else if fs.len() == 1 {
            line.split(fs.chars().next().unwrap())
                .map(|s| s.to_string())
                .collect()
        } else {
            match Regex::new(&fs) {
                Ok(re) => re.split(line).map(|s| s.to_string()).collect(),
                Err(_) => vec![line.to_string()],
            }
        };
        self.fields.extend(parts);
        let nf = (self.fields.len() - 1) as f64;
        self.globals.insert("NF".to_string(), Value::Num(nf));
    }

    fn rebuild_record(&mut self) {
        let ofs = self
            .globals
            .get("OFS")
            .map(|v| v.to_string_val())
            .unwrap_or(" ".to_string());
        if self.fields.len() > 1 {
            self.fields[0] = self.fields[1..].join(&ofs);
        }
    }

    fn get_field(&self, idx: usize) -> String {
        if idx < self.fields.len() {
            self.fields[idx].clone()
        } else {
            String::new()
        }
    }

    fn set_field(&mut self, idx: usize, val: String) {
        while self.fields.len() <= idx {
            self.fields.push(String::new());
        }
        self.fields[idx] = val;
        let nf = (self.fields.len() - 1) as f64;
        self.globals.insert("NF".to_string(), Value::Num(nf));
        if idx > 0 {
            self.rebuild_record();
        } else {
            // Re-split if $0 was assigned
            let line = self.fields[0].clone();
            self.set_record(&line);
        }
    }

    fn get_var(&self, name: &str) -> Value {
        match name {
            "NR" => Value::Num(self.nr as f64),
            "FNR" => Value::Num(self.fnr as f64),
            _ => self.globals.get(name).cloned().unwrap_or(Value::Uninitialized),
        }
    }

    fn set_var(&mut self, name: &str, val: Value) {
        match name {
            "NR" => self.nr = val.to_num() as i64,
            "FNR" => self.fnr = val.to_num() as i64,
            "NF" => {
                let nf = val.to_num() as usize;
                while self.fields.len() <= nf {
                    self.fields.push(String::new());
                }
                self.fields.truncate(nf + 1);
                self.globals.insert("NF".to_string(), val);
                self.rebuild_record();
            }
            "$0" => {
                // handled elsewhere
            }
            _ => {
                self.globals.insert(name.to_string(), val);
            }
        }
    }

    fn get_array(&self, name: &str, key: &str) -> Value {
        self.arrays
            .get(name)
            .and_then(|a| a.get(key))
            .cloned()
            .unwrap_or(Value::Uninitialized)
    }

    fn set_array(&mut self, name: &str, key: &str, val: Value) {
        self.arrays
            .entry(name.to_string())
            .or_default()
            .insert(key.to_string(), val);
    }

    fn array_key(&self, indices: &[Value]) -> String {
        let subsep = self
            .globals
            .get("SUBSEP")
            .map(|v| v.to_string_val())
            .unwrap_or("\x1c".to_string());
        indices
            .iter()
            .map(|v| v.to_string_val())
            .collect::<Vec<_>>()
            .join(&subsep)
    }

    fn run(&mut self, program: &Program, files: &[String]) {
        // Register functions
        for func in &program.functions {
            self.functions.insert(func.name.clone(), func.clone());
        }

        // Run BEGIN blocks
        for rule in &program.rules {
            if matches!(rule.pattern, Some(Pattern::Begin)) {
                match self.exec_stmts(&rule.action) {
                    ControlFlow::Exit(code) => {
                        self.exit_code = code;
                        self.run_end_blocks(program);
                        return;
                    }
                    _ => {}
                }
            }
        }

        // Process input
        if files.is_empty() {
            self.process_stream(program, &mut io::stdin().lock(), "-");
        } else {
            for file in files {
                if file == "-" {
                    self.process_stream(program, &mut io::stdin().lock(), "-");
                } else {
                    match fs::File::open(file) {
                        Ok(f) => {
                            let mut reader = BufReader::new(f);
                            self.process_stream(program, &mut reader, file);
                        }
                        Err(e) => {
                            eprintln!("awk: can't open file {file}: {e}");
                        }
                    }
                }
                self.fnr = 0;
            }
        }

        self.run_end_blocks(program);
    }

    fn run_end_blocks(&mut self, program: &Program) {
        for rule in &program.rules {
            if matches!(rule.pattern, Some(Pattern::End)) {
                match self.exec_stmts(&rule.action) {
                    ControlFlow::Exit(code) => {
                        self.exit_code = code;
                        return;
                    }
                    _ => {}
                }
            }
        }
    }

    fn process_stream(&mut self, program: &Program, reader: &mut dyn BufRead, filename: &str) {
        self.globals
            .insert("FILENAME".to_string(), Value::Str(filename.to_string()));

        let rs = self
            .globals
            .get("RS")
            .map(|v| v.to_string_val())
            .unwrap_or("\n".to_string());

        if rs == "\n" || rs.len() == 1 {
            // Line-by-line reading
            let sep = if rs == "\n" { '\n' } else { rs.chars().next().unwrap_or('\n') };
            let mut buf = String::new();
            loop {
                buf.clear();
                match reader.read_line(&mut buf) {
                    Ok(0) => break,
                    Ok(_) => {
                        // Remove the trailing RS
                        if buf.ends_with(sep) {
                            buf.pop();
                        }
                        if sep == '\n' && buf.ends_with('\r') {
                            buf.pop();
                        }
                        self.nr += 1;
                        self.fnr += 1;
                        self.globals
                            .insert("NR".to_string(), Value::Num(self.nr as f64));
                        self.globals
                            .insert("FNR".to_string(), Value::Num(self.fnr as f64));
                        self.set_record(&buf);

                        match self.process_rules(program) {
                            ControlFlow::Exit(code) => {
                                self.exit_code = code;
                                return;
                            }
                            _ => {}
                        }
                    }
                    Err(_) => break,
                }
            }
        } else if rs.is_empty() {
            // Paragraph mode: empty RS means split on blank lines
            let mut all = String::new();
            reader.read_to_string(&mut all).ok();
            // Split on one or more blank lines
            let paragraphs: Vec<&str> = all.split("\n\n").collect();
            for para in paragraphs {
                let para = para.trim_matches('\n');
                if para.is_empty() {
                    continue;
                }
                self.nr += 1;
                self.fnr += 1;
                self.globals
                    .insert("NR".to_string(), Value::Num(self.nr as f64));
                self.globals
                    .insert("FNR".to_string(), Value::Num(self.fnr as f64));
                self.set_record(para);

                match self.process_rules(program) {
                    ControlFlow::Exit(code) => {
                        self.exit_code = code;
                        return;
                    }
                    _ => {}
                }
            }
        } else {
            // Multi-char RS: use as regex
            let mut all = String::new();
            reader.read_to_string(&mut all).ok();
            let records: Vec<&str> = match Regex::new(&rs) {
                Ok(re) => re.split(&all).collect(),
                Err(_) => all.split(&rs).collect(),
            };
            let records_len = records.len();
            for rec in records {
                if rec.is_empty() && records_len > 1 {
                    continue;
                }
                self.nr += 1;
                self.fnr += 1;
                self.globals
                    .insert("NR".to_string(), Value::Num(self.nr as f64));
                self.globals
                    .insert("FNR".to_string(), Value::Num(self.fnr as f64));
                self.set_record(rec);

                match self.process_rules(program) {
                    ControlFlow::Exit(code) => {
                        self.exit_code = code;
                        return;
                    }
                    _ => {}
                }
            }
        }
    }

    fn process_rules(&mut self, program: &Program) -> ControlFlow {
        for (idx, rule) in program.rules.iter().enumerate() {
            let should_run = match &rule.pattern {
                None => true,
                Some(Pattern::Begin) | Some(Pattern::End) => false,
                Some(Pattern::Expression(expr)) => {
                    let val = self.eval_expr(expr);
                    val.to_bool()
                }
                Some(Pattern::Range(start, end)) => {
                    let active = self.range_active.get(&idx).copied().unwrap_or(false);
                    if active {
                        let end_val = self.eval_expr(end);
                        if end_val.to_bool() {
                            self.range_active.insert(idx, false);
                        }
                        true
                    } else {
                        let start_val = self.eval_expr(start);
                        if start_val.to_bool() {
                            self.range_active.insert(idx, true);
                            true
                        } else {
                            false
                        }
                    }
                }
            };

            if should_run {
                match self.exec_stmts(&rule.action) {
                    ControlFlow::Next => return ControlFlow::None,
                    ControlFlow::Exit(code) => return ControlFlow::Exit(code),
                    _ => {}
                }
            }
        }
        ControlFlow::None
    }

    fn exec_stmts(&mut self, stmts: &[Stmt]) -> ControlFlow {
        for stmt in stmts {
            match self.exec_stmt(stmt) {
                ControlFlow::None => {}
                cf => return cf,
            }
        }
        ControlFlow::None
    }

    fn exec_stmt(&mut self, stmt: &Stmt) -> ControlFlow {
        match stmt {
            Stmt::Expr(expr) => {
                self.eval_expr(expr);
            }
            Stmt::Print(args, dest) => {
                let ofs = self
                    .globals
                    .get("OFS")
                    .map(|v| v.to_string_val())
                    .unwrap_or(" ".to_string());
                let ors = self
                    .globals
                    .get("ORS")
                    .map(|v| v.to_string_val())
                    .unwrap_or("\n".to_string());

                let output = if args.is_empty() {
                    self.get_field(0)
                } else {
                    args.iter()
                        .map(|a| self.eval_expr(a).to_string_val())
                        .collect::<Vec<_>>()
                        .join(&ofs)
                };

                let output = format!("{output}{ors}");
                self.write_output(&output, dest);
            }
            Stmt::Printf(args, dest) => {
                if args.is_empty() {
                    return ControlFlow::None;
                }
                let vals: Vec<Value> = args.iter().map(|a| self.eval_expr(a)).collect();
                let output = self.sprintf_impl(&vals);
                self.write_output(&output, dest);
            }
            Stmt::If(cond, then_branch, else_branch) => {
                let val = self.eval_expr(cond);
                if val.to_bool() {
                    return self.exec_stmt(then_branch);
                } else if let Some(else_b) = else_branch {
                    return self.exec_stmt(else_b);
                }
            }
            Stmt::While(cond, body) => loop {
                let val = self.eval_expr(cond);
                if !val.to_bool() {
                    break;
                }
                match self.exec_stmt(body) {
                    ControlFlow::Break => break,
                    ControlFlow::Continue => continue,
                    ControlFlow::None => {}
                    cf => return cf,
                }
            },
            Stmt::DoWhile(body, cond) => loop {
                match self.exec_stmt(body) {
                    ControlFlow::Break => break,
                    ControlFlow::Continue => {}
                    ControlFlow::None => {}
                    cf => return cf,
                }
                let val = self.eval_expr(cond);
                if !val.to_bool() {
                    break;
                }
            },
            Stmt::For(init, cond, update, body) => {
                if let Some(init) = init {
                    self.exec_stmt(init);
                }
                loop {
                    if let Some(cond) = cond {
                        let val = self.eval_expr(cond);
                        if !val.to_bool() {
                            break;
                        }
                    }
                    match self.exec_stmt(body) {
                        ControlFlow::Break => break,
                        ControlFlow::Continue => {}
                        ControlFlow::None => {}
                        cf => return cf,
                    }
                    if let Some(update) = update {
                        self.exec_stmt(update);
                    }
                }
            }
            Stmt::ForIn(var, array, body) => {
                let keys: Vec<String> = self
                    .arrays
                    .get(array)
                    .map(|a| a.keys().cloned().collect())
                    .unwrap_or_default();
                for key in keys {
                    self.set_var(var, Value::Str(key));
                    match self.exec_stmt(body) {
                        ControlFlow::Break => break,
                        ControlFlow::Continue => continue,
                        ControlFlow::None => {}
                        cf => return cf,
                    }
                }
            }
            Stmt::Block(stmts) => {
                return self.exec_stmts(stmts);
            }
            Stmt::Next => return ControlFlow::Next,
            Stmt::Exit(expr) => {
                let code = expr
                    .as_ref()
                    .map(|e| self.eval_expr(e).to_num() as i32)
                    .unwrap_or(0);
                return ControlFlow::Exit(code);
            }
            Stmt::Delete(name, indices) => {
                if indices.is_empty() {
                    self.arrays.remove(name);
                } else {
                    let vals: Vec<Value> = indices.iter().map(|i| self.eval_expr(i)).collect();
                    let key = self.array_key(&vals);
                    if let Some(arr) = self.arrays.get_mut(name) {
                        arr.remove(&key);
                    }
                }
            }
            Stmt::Break => return ControlFlow::Break,
            Stmt::Continue => return ControlFlow::Continue,
            Stmt::Return(expr) => {
                let val = expr
                    .as_ref()
                    .map(|e| self.eval_expr(e))
                    .unwrap_or(Value::Uninitialized);
                return ControlFlow::Return(val);
            }
            Stmt::Getline(var, file, source) => {
                self.eval_getline(var.as_ref().map(|e| e.as_ref()), file.as_ref().map(|e| e.as_ref()), source);
            }
        }
        ControlFlow::None
    }

    fn write_output(&mut self, output: &str, dest: &Option<OutputDest>) {
        match dest {
            None => {
                print!("{output}");
                io::stdout().flush().ok();
            }
            Some(OutputDest::File(expr)) => {
                let filename = self.eval_expr(expr).to_string_val();
                if !self.open_files.contains_key(&filename) {
                    match fs::File::create(&filename) {
                        Ok(f) => {
                            self.open_files.insert(filename.clone(), Box::new(f));
                        }
                        Err(e) => {
                            eprintln!("awk: can't redirect to {filename}: {e}");
                            return;
                        }
                    }
                }
                if let Some(f) = self.open_files.get_mut(&filename) {
                    f.write_all(output.as_bytes()).ok();
                    f.flush().ok();
                }
            }
            Some(OutputDest::Append(expr)) => {
                let filename = self.eval_expr(expr).to_string_val();
                if !self.open_files.contains_key(&filename) {
                    match fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&filename)
                    {
                        Ok(f) => {
                            self.open_files.insert(filename.clone(), Box::new(f));
                        }
                        Err(e) => {
                            eprintln!("awk: can't redirect to {filename}: {e}");
                            return;
                        }
                    }
                }
                if let Some(f) = self.open_files.get_mut(&filename) {
                    f.write_all(output.as_bytes()).ok();
                    f.flush().ok();
                }
            }
            Some(OutputDest::Pipe(expr)) => {
                let cmd = self.eval_expr(expr).to_string_val();
                if !self.open_pipes.contains_key(&cmd) {
                    match Command::new("sh")
                        .arg("-c")
                        .arg(&cmd)
                        .stdin(Stdio::piped())
                        .spawn()
                    {
                        Ok(mut child) => {
                            if let Some(stdin) = child.stdin.take() {
                                self.open_pipes.insert(cmd.clone(), Box::new(stdin));
                            }
                        }
                        Err(e) => {
                            eprintln!("awk: can't open pipe to {cmd}: {e}");
                            return;
                        }
                    }
                }
                if let Some(p) = self.open_pipes.get_mut(&cmd) {
                    p.write_all(output.as_bytes()).ok();
                    p.flush().ok();
                }
            }
        }
    }

    fn eval_getline(
        &mut self,
        var: Option<&Expr>,
        file: Option<&Expr>,
        source: &GetlineSource,
    ) -> Value {
        match source {
            GetlineSource::Stdin => {
                let mut line = String::new();
                match io::stdin().lock().read_line(&mut line) {
                    Ok(0) => Value::Num(0.0),
                    Ok(_) => {
                        if line.ends_with('\n') {
                            line.pop();
                        }
                        if line.ends_with('\r') {
                            line.pop();
                        }
                        if let Some(var_expr) = var {
                            self.assign_to(var_expr, Value::Str(line.clone()));
                        } else {
                            self.set_record(&line);
                        }
                        self.nr += 1;
                        self.globals
                            .insert("NR".to_string(), Value::Num(self.nr as f64));
                        Value::Num(1.0)
                    }
                    Err(_) => Value::Num(-1.0),
                }
            }
            GetlineSource::File => {
                if let Some(file_expr) = file {
                    let filename = self.eval_expr(file_expr).to_string_val();
                    if !self.open_read_files.contains_key(&filename) {
                        match fs::File::open(&filename) {
                            Ok(f) => {
                                self.open_read_files
                                    .insert(filename.clone(), Box::new(BufReader::new(f)));
                            }
                            Err(_) => return Value::Num(-1.0),
                        }
                    }
                    let mut line = String::new();
                    let result = if let Some(reader) = self.open_read_files.get_mut(&filename) {
                        reader.read_line(&mut line)
                    } else {
                        return Value::Num(-1.0);
                    };
                    match result {
                        Ok(0) => Value::Num(0.0),
                        Ok(_) => {
                            if line.ends_with('\n') {
                                line.pop();
                            }
                            if line.ends_with('\r') {
                                line.pop();
                            }
                            if let Some(var_expr) = var {
                                self.assign_to(var_expr, Value::Str(line.clone()));
                            } else {
                                self.set_record(&line);
                            }
                            Value::Num(1.0)
                        }
                        Err(_) => Value::Num(-1.0),
                    }
                } else {
                    Value::Num(-1.0)
                }
            }
            GetlineSource::Pipe => {
                if let Some(cmd_expr) = file {
                    let cmd = self.eval_expr(cmd_expr).to_string_val();
                    if !self.open_read_pipes.contains_key(&cmd) {
                        match Command::new("sh")
                            .arg("-c")
                            .arg(&cmd)
                            .stdout(Stdio::piped())
                            .spawn()
                        {
                            Ok(mut child) => {
                                if let Some(stdout) = child.stdout.take() {
                                    self.open_read_pipes
                                        .insert(cmd.clone(), Box::new(BufReader::new(stdout)));
                                }
                            }
                            Err(_) => return Value::Num(-1.0),
                        }
                    }
                    let mut line = String::new();
                    let result = if let Some(reader) = self.open_read_pipes.get_mut(&cmd) {
                        reader.read_line(&mut line)
                    } else {
                        return Value::Num(-1.0);
                    };
                    match result {
                        Ok(0) => Value::Num(0.0),
                        Ok(_) => {
                            if line.ends_with('\n') {
                                line.pop();
                            }
                            if line.ends_with('\r') {
                                line.pop();
                            }
                            if let Some(var_expr) = var {
                                self.assign_to(var_expr, Value::Str(line.clone()));
                            } else {
                                self.set_record(&line);
                                self.nr += 1;
                                self.globals
                                    .insert("NR".to_string(), Value::Num(self.nr as f64));
                            }
                            Value::Num(1.0)
                        }
                        Err(_) => Value::Num(-1.0),
                    }
                } else {
                    Value::Num(-1.0)
                }
            }
        }
    }

    fn assign_to(&mut self, expr: &Expr, val: Value) {
        match expr {
            Expr::Var(name) => self.set_var(name, val),
            Expr::FieldRef(idx_expr) => {
                let idx = self.eval_expr(idx_expr).to_num() as usize;
                self.set_field(idx, val.to_string_val());
            }
            Expr::ArrayRef(name, indices) => {
                let vals: Vec<Value> = indices.iter().map(|i| self.eval_expr(i)).collect();
                let key = self.array_key(&vals);
                self.set_array(name, &key, val);
            }
            _ => {}
        }
    }

    fn eval_expr(&mut self, expr: &Expr) -> Value {
        match expr {
            Expr::Number(n) => Value::Num(*n),
            Expr::StringLit(s) => Value::Str(s.clone()),
            Expr::Regex(_) => {
                // Bare regex - shouldn't appear standalone normally
                Value::Str(String::new())
            }
            Expr::Var(name) => self.get_var(name),
            Expr::FieldRef(idx_expr) => {
                let idx = self.eval_expr(idx_expr).to_num() as usize;
                let field = self.get_field(idx);
                // Return as string that may be numeric
                Value::Str(field)
            }
            Expr::ArrayRef(name, indices) => {
                let vals: Vec<Value> = indices.iter().map(|i| self.eval_expr(i)).collect();
                let key = self.array_key(&vals);
                self.get_array(name, &key)
            }
            Expr::Binop(left, op, right) => {
                match op {
                    BinOp::And => {
                        let lv = self.eval_expr(left);
                        if !lv.to_bool() {
                            return Value::Num(0.0);
                        }
                        let rv = self.eval_expr(right);
                        Value::Num(if rv.to_bool() { 1.0 } else { 0.0 })
                    }
                    BinOp::Or => {
                        let lv = self.eval_expr(left);
                        if lv.to_bool() {
                            return Value::Num(1.0);
                        }
                        let rv = self.eval_expr(right);
                        Value::Num(if rv.to_bool() { 1.0 } else { 0.0 })
                    }
                    _ => {
                        let lv = self.eval_expr(left);
                        let rv = self.eval_expr(right);
                        match op {
                            BinOp::Add => Value::Num(lv.to_num() + rv.to_num()),
                            BinOp::Sub => Value::Num(lv.to_num() - rv.to_num()),
                            BinOp::Mul => Value::Num(lv.to_num() * rv.to_num()),
                            BinOp::Div => {
                                let d = rv.to_num();
                                if d == 0.0 {
                                    eprintln!("awk: division by zero");
                                    Value::Num(0.0)
                                } else {
                                    Value::Num(lv.to_num() / d)
                                }
                            }
                            BinOp::Mod => {
                                let d = rv.to_num();
                                if d == 0.0 {
                                    eprintln!("awk: division by zero");
                                    Value::Num(0.0)
                                } else {
                                    Value::Num(lv.to_num() % d)
                                }
                            }
                            BinOp::Pow => Value::Num(lv.to_num().powf(rv.to_num())),
                            BinOp::Eq => {
                                let ord = compare_values(&lv, &rv);
                                Value::Num(if ord == std::cmp::Ordering::Equal {
                                    1.0
                                } else {
                                    0.0
                                })
                            }
                            BinOp::Ne => {
                                let ord = compare_values(&lv, &rv);
                                Value::Num(if ord != std::cmp::Ordering::Equal {
                                    1.0
                                } else {
                                    0.0
                                })
                            }
                            BinOp::Lt => {
                                let ord = compare_values(&lv, &rv);
                                Value::Num(if ord == std::cmp::Ordering::Less {
                                    1.0
                                } else {
                                    0.0
                                })
                            }
                            BinOp::Gt => {
                                let ord = compare_values(&lv, &rv);
                                Value::Num(if ord == std::cmp::Ordering::Greater {
                                    1.0
                                } else {
                                    0.0
                                })
                            }
                            BinOp::Le => {
                                let ord = compare_values(&lv, &rv);
                                Value::Num(if ord != std::cmp::Ordering::Greater {
                                    1.0
                                } else {
                                    0.0
                                })
                            }
                            BinOp::Ge => {
                                let ord = compare_values(&lv, &rv);
                                Value::Num(if ord != std::cmp::Ordering::Less {
                                    1.0
                                } else {
                                    0.0
                                })
                            }
                            BinOp::And | BinOp::Or => unreachable!(),
                        }
                    }
                }
            }
            Expr::Unop(op, operand) => match op {
                UnOp::Neg => {
                    let v = self.eval_expr(operand);
                    Value::Num(-v.to_num())
                }
                UnOp::Not => {
                    let v = self.eval_expr(operand);
                    Value::Num(if v.to_bool() { 0.0 } else { 1.0 })
                }
                UnOp::PreIncrement => {
                    let v = self.eval_expr(operand).to_num() + 1.0;
                    let new_val = Value::Num(v);
                    self.assign_to(operand, new_val.clone());
                    new_val
                }
                UnOp::PreDecrement => {
                    let v = self.eval_expr(operand).to_num() - 1.0;
                    let new_val = Value::Num(v);
                    self.assign_to(operand, new_val.clone());
                    new_val
                }
            },
            Expr::PostIncrement(operand) => {
                let v = self.eval_expr(operand).to_num();
                self.assign_to(operand, Value::Num(v + 1.0));
                Value::Num(v)
            }
            Expr::PostDecrement(operand) => {
                let v = self.eval_expr(operand).to_num();
                self.assign_to(operand, Value::Num(v - 1.0));
                Value::Num(v)
            }
            Expr::Assign(lhs, rhs) => {
                let val = self.eval_expr(rhs);
                self.assign_to(lhs, val.clone());
                val
            }
            Expr::OpAssign(lhs, op, rhs) => {
                let lv = self.eval_expr(lhs).to_num();
                let rv = self.eval_expr(rhs).to_num();
                let result = match op {
                    BinOp::Add => lv + rv,
                    BinOp::Sub => lv - rv,
                    BinOp::Mul => lv * rv,
                    BinOp::Div => {
                        if rv == 0.0 {
                            eprintln!("awk: division by zero");
                            0.0
                        } else {
                            lv / rv
                        }
                    }
                    BinOp::Mod => {
                        if rv == 0.0 {
                            eprintln!("awk: division by zero");
                            0.0
                        } else {
                            lv % rv
                        }
                    }
                    BinOp::Pow => lv.powf(rv),
                    _ => lv,
                };
                let val = Value::Num(result);
                self.assign_to(lhs, val.clone());
                val
            }
            Expr::Match(left, right) => {
                let s = self.eval_expr(left).to_string_val();
                let pattern = match right.as_ref() {
                    Expr::Regex(r) => r.clone(),
                    _ => self.eval_expr(right).to_string_val(),
                };
                match Regex::new(&pattern) {
                    Ok(re) => Value::Num(if re.is_match(&s) { 1.0 } else { 0.0 }),
                    Err(_) => Value::Num(0.0),
                }
            }
            Expr::NotMatch(left, right) => {
                let s = self.eval_expr(left).to_string_val();
                let pattern = match right.as_ref() {
                    Expr::Regex(r) => r.clone(),
                    _ => self.eval_expr(right).to_string_val(),
                };
                match Regex::new(&pattern) {
                    Ok(re) => Value::Num(if re.is_match(&s) { 0.0 } else { 1.0 }),
                    Err(_) => Value::Num(1.0),
                }
            }
            Expr::Ternary(cond, then_expr, else_expr) => {
                let val = self.eval_expr(cond);
                if val.to_bool() {
                    self.eval_expr(then_expr)
                } else {
                    self.eval_expr(else_expr)
                }
            }
            Expr::Concat(left, right) => {
                let ls = self.eval_expr(left).to_string_val();
                let rs = self.eval_expr(right).to_string_val();
                Value::Str(format!("{ls}{rs}"))
            }
            Expr::In(expr, array) => {
                let key = self.eval_expr(expr).to_string_val();
                let exists = self
                    .arrays
                    .get(array)
                    .is_some_and(|a| a.contains_key(&key));
                Value::Num(if exists { 1.0 } else { 0.0 })
            }
            Expr::MultiIn(indices, array) => {
                let vals: Vec<Value> = indices.iter().map(|i| self.eval_expr(i)).collect();
                let key = self.array_key(&vals);
                let exists = self
                    .arrays
                    .get(array)
                    .is_some_and(|a| a.contains_key(&key));
                Value::Num(if exists { 1.0 } else { 0.0 })
            }
            Expr::FuncCall(name, args) => self.call_function(name, args),
            Expr::Getline(var, file, source) => {
                self.eval_getline(var.as_ref().map(|e| e.as_ref()), file.as_ref().map(|e| e.as_ref()), source)
            }
            Expr::Sprintf(args) => {
                let vals: Vec<Value> = args.iter().map(|a| self.eval_expr(a)).collect();
                Value::Str(self.sprintf_impl(&vals))
            }
            Expr::Pipe(cmd, getline_expr) => {
                // cmd | getline [var]
                let cmd_str = self.eval_expr(cmd).to_string_val();
                let var = match getline_expr.as_ref() {
                    Expr::Getline(v, _, _) => v.as_ref().map(|e| e.as_ref()),
                    _ => None,
                };
                let cmd_expr = Expr::StringLit(cmd_str);
                self.eval_getline(var, Some(&cmd_expr), &GetlineSource::Pipe)
            }
        }
    }

    fn call_function(&mut self, name: &str, args: &[Expr]) -> Value {
        // Built-in functions
        match name {
            "length" => {
                if args.is_empty() {
                    return Value::Num(self.get_field(0).len() as f64);
                }
                // Check if argument is an array name
                if let Some(Expr::Var(arr_name)) = args.first() {
                    if self.arrays.contains_key(arr_name) {
                        return Value::Num(self.arrays[arr_name].len() as f64);
                    }
                }
                let v = self.eval_expr(&args[0]);
                Value::Num(v.to_string_val().len() as f64)
            }
            "substr" => {
                if args.len() < 2 {
                    return Value::Str(String::new());
                }
                let s = self.eval_expr(&args[0]).to_string_val();
                let start = (self.eval_expr(&args[1]).to_num() as i64).max(1) as usize;
                let chars: Vec<char> = s.chars().collect();
                if start > chars.len() {
                    return Value::Str(String::new());
                }
                let start_idx = start - 1;
                if args.len() >= 3 {
                    let len = self.eval_expr(&args[2]).to_num().max(0.0) as usize;
                    let end = (start_idx + len).min(chars.len());
                    Value::Str(chars[start_idx..end].iter().collect())
                } else {
                    Value::Str(chars[start_idx..].iter().collect())
                }
            }
            "index" => {
                if args.len() < 2 {
                    return Value::Num(0.0);
                }
                let s = self.eval_expr(&args[0]).to_string_val();
                let t = self.eval_expr(&args[1]).to_string_val();
                match s.find(&t) {
                    Some(pos) => {
                        // Convert byte offset to char position
                        let char_pos = s[..pos].chars().count() + 1;
                        Value::Num(char_pos as f64)
                    }
                    None => Value::Num(0.0),
                }
            }
            "split" => {
                if args.len() < 2 {
                    return Value::Num(0.0);
                }
                let s = self.eval_expr(&args[0]).to_string_val();
                let arr_name = match &args[1] {
                    Expr::Var(n) => n.clone(),
                    Expr::ArrayRef(n, _) => n.clone(),
                    _ => return Value::Num(0.0),
                };
                let fs = if args.len() >= 3 {
                    self.eval_expr(&args[2]).to_string_val()
                } else {
                    self.globals
                        .get("FS")
                        .map(|v| v.to_string_val())
                        .unwrap_or(" ".to_string())
                };

                // Clear the array
                self.arrays.remove(&arr_name);

                let parts: Vec<String> = if fs == " " {
                    s.split_whitespace().map(|p| p.to_string()).collect()
                } else if fs.len() == 1 {
                    s.split(fs.chars().next().unwrap())
                        .map(|p| p.to_string())
                        .collect()
                } else {
                    match Regex::new(&fs) {
                        Ok(re) => re.split(&s).map(|p| p.to_string()).collect(),
                        Err(_) => s.split(&fs).map(|p| p.to_string()).collect(),
                    }
                };

                let count = parts.len();
                for (i, part) in parts.into_iter().enumerate() {
                    self.set_array(&arr_name, &(i + 1).to_string(), Value::Str(part));
                }
                Value::Num(count as f64)
            }
            "sub" | "gsub" => {
                if args.len() < 2 {
                    return Value::Num(0.0);
                }
                let pattern = match &args[0] {
                    Expr::Regex(r) => r.clone(),
                    _ => {
                        let v = self.eval_expr(&args[0]);
                        regex::escape(&v.to_string_val())
                    }
                };
                let replacement = self.eval_expr(&args[1]).to_string_val();

                // Target: either specified or $0
                let target_expr = if args.len() >= 3 {
                    args[2].clone()
                } else {
                    Expr::FieldRef(Box::new(Expr::Number(0.0)))
                };

                let target_val = self.eval_expr(&target_expr).to_string_val();
                let is_global = name == "gsub";

                match Regex::new(&pattern) {
                    Ok(re) => {
                        let mut count = 0;
                        let result = if is_global {
                            let r = re.replace_all(&target_val, |caps: &regex::Captures| {
                                count += 1;
                                awk_replace(&replacement, caps)
                            });
                            r.to_string()
                        } else {
                            let r = re.replace(&target_val, |caps: &regex::Captures| {
                                count += 1;
                                awk_replace(&replacement, caps)
                            });
                            r.to_string()
                        };
                        self.assign_to(&target_expr, Value::Str(result));
                        Value::Num(count as f64)
                    }
                    Err(_) => Value::Num(0.0),
                }
            }
            "gensub" => {
                if args.len() < 3 {
                    return Value::Str(String::new());
                }
                let pattern = match &args[0] {
                    Expr::Regex(r) => r.clone(),
                    _ => self.eval_expr(&args[0]).to_string_val(),
                };
                let replacement = self.eval_expr(&args[1]).to_string_val();
                let how = self.eval_expr(&args[2]).to_string_val();

                let target = if args.len() >= 4 {
                    self.eval_expr(&args[3]).to_string_val()
                } else {
                    self.get_field(0)
                };

                let is_global = how == "g" || how == "G";

                match Regex::new(&pattern) {
                    Ok(re) => {
                        let result = if is_global {
                            re.replace_all(&target, |caps: &regex::Captures| {
                                awk_replace(&replacement, caps)
                            })
                            .to_string()
                        } else {
                            let n = how.parse::<usize>().unwrap_or(1);
                            let mut count = 0;
                            re.replace_all(&target, |caps: &regex::Captures| {
                                count += 1;
                                if count == n {
                                    awk_replace(&replacement, caps)
                                } else {
                                    caps[0].to_string()
                                }
                            })
                            .to_string()
                        };
                        Value::Str(result)
                    }
                    Err(_) => Value::Str(target),
                }
            }
            "match" => {
                if args.len() < 2 {
                    return Value::Num(0.0);
                }
                let s = self.eval_expr(&args[0]).to_string_val();
                let pattern = match &args[1] {
                    Expr::Regex(r) => r.clone(),
                    _ => self.eval_expr(&args[1]).to_string_val(),
                };
                match Regex::new(&pattern) {
                    Ok(re) => {
                        if let Some(m) = re.find(&s) {
                            let start = s[..m.start()].chars().count() + 1;
                            let length = m.as_str().chars().count();
                            self.set_var("RSTART", Value::Num(start as f64));
                            self.set_var("RLENGTH", Value::Num(length as f64));
                            Value::Num(start as f64)
                        } else {
                            self.set_var("RSTART", Value::Num(0.0));
                            self.set_var("RLENGTH", Value::Num(-1.0));
                            Value::Num(0.0)
                        }
                    }
                    Err(_) => {
                        self.set_var("RSTART", Value::Num(0.0));
                        self.set_var("RLENGTH", Value::Num(-1.0));
                        Value::Num(0.0)
                    }
                }
            }
            "sprintf" => {
                let vals: Vec<Value> = args.iter().map(|a| self.eval_expr(a)).collect();
                Value::Str(self.sprintf_impl(&vals))
            }
            "tolower" => {
                if args.is_empty() {
                    return Value::Str(String::new());
                }
                let s = self.eval_expr(&args[0]).to_string_val();
                Value::Str(s.to_lowercase())
            }
            "toupper" => {
                if args.is_empty() {
                    return Value::Str(String::new());
                }
                let s = self.eval_expr(&args[0]).to_string_val();
                Value::Str(s.to_uppercase())
            }
            // Math functions
            "sin" => {
                let n = if args.is_empty() {
                    0.0
                } else {
                    self.eval_expr(&args[0]).to_num()
                };
                Value::Num(n.sin())
            }
            "cos" => {
                let n = if args.is_empty() {
                    0.0
                } else {
                    self.eval_expr(&args[0]).to_num()
                };
                Value::Num(n.cos())
            }
            "atan2" => {
                if args.len() < 2 {
                    return Value::Num(0.0);
                }
                let y = self.eval_expr(&args[0]).to_num();
                let x = self.eval_expr(&args[1]).to_num();
                Value::Num(y.atan2(x))
            }
            "exp" => {
                let n = if args.is_empty() {
                    0.0
                } else {
                    self.eval_expr(&args[0]).to_num()
                };
                Value::Num(n.exp())
            }
            "log" => {
                let n = if args.is_empty() {
                    0.0
                } else {
                    self.eval_expr(&args[0]).to_num()
                };
                Value::Num(n.ln())
            }
            "sqrt" => {
                let n = if args.is_empty() {
                    0.0
                } else {
                    self.eval_expr(&args[0]).to_num()
                };
                Value::Num(n.sqrt())
            }
            "int" => {
                let n = if args.is_empty() {
                    0.0
                } else {
                    self.eval_expr(&args[0]).to_num()
                };
                Value::Num(n.trunc())
            }
            "rand" => {
                // Simple LCG random
                self.rng_state = self
                    .rng_state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                let val = (self.rng_state >> 33) as f64 / (1u64 << 31) as f64;
                Value::Num(val)
            }
            "srand" => {
                let old = self.rng_state;
                if args.is_empty() {
                    self.rng_state = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_nanos() as u64)
                        .unwrap_or(0);
                } else {
                    self.rng_state = self.eval_expr(&args[0]).to_num() as u64;
                }
                Value::Num(old as f64)
            }
            "system" => {
                if args.is_empty() {
                    return Value::Num(0.0);
                }
                let cmd = self.eval_expr(&args[0]).to_string_val();
                // Flush stdout before running system command
                io::stdout().flush().ok();
                match Command::new("sh").arg("-c").arg(&cmd).status() {
                    Ok(status) => Value::Num(status.code().unwrap_or(-1) as f64),
                    Err(_) => Value::Num(-1.0),
                }
            }
            "close" => {
                if args.is_empty() {
                    return Value::Num(-1.0);
                }
                let name = self.eval_expr(&args[0]).to_string_val();
                let mut found = false;
                if self.open_files.remove(&name).is_some() {
                    found = true;
                }
                if self.open_pipes.remove(&name).is_some() {
                    found = true;
                }
                if self.open_read_files.remove(&name).is_some() {
                    found = true;
                }
                if self.open_read_pipes.remove(&name).is_some() {
                    found = true;
                }
                Value::Num(if found { 0.0 } else { -1.0 })
            }
            "mktime" | "systime" => {
                // systime returns current epoch
                if name == "systime" {
                    let epoch = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    return Value::Num(epoch as f64);
                }
                Value::Num(0.0)
            }
            "strftime" => {
                // Minimal strftime
                Value::Str(String::new())
            }
            _ => {
                // User-defined function
                let func = self.functions.get(name).cloned();
                if let Some(func) = func {
                    // Save and set up local scope
                    let mut saved_vars: Vec<(String, Option<Value>)> = Vec::new();
                    let mut saved_arrays: Vec<(String, Option<HashMap<String, Value>>)> =
                        Vec::new();

                    // Evaluate args before modifying scope
                    let arg_vals: Vec<Value> = args.iter().map(|a| self.eval_expr(a)).collect();

                    for (i, param) in func.params.iter().enumerate() {
                        // Save old value
                        saved_vars.push((param.clone(), self.globals.remove(param)));
                        saved_arrays.push((param.clone(), self.arrays.remove(param)));

                        if i < arg_vals.len() {
                            self.set_var(param, arg_vals[i].clone());
                        } else {
                            // Extra params are local variables, initialized to 0/""
                            self.set_var(param, Value::Uninitialized);
                        }
                    }

                    let result = match self.exec_stmts(&func.body) {
                        ControlFlow::Return(val) => val,
                        _ => Value::Uninitialized,
                    };

                    // Restore scope
                    for (name, old_val) in saved_vars {
                        self.globals.remove(&name);
                        if let Some(val) = old_val {
                            self.globals.insert(name, val);
                        }
                    }
                    for (name, old_arr) in saved_arrays {
                        self.arrays.remove(&name);
                        if let Some(arr) = old_arr {
                            self.arrays.insert(name, arr);
                        }
                    }

                    result
                } else {
                    eprintln!("awk: unknown function {name}");
                    Value::Uninitialized
                }
            }
        }
    }

    fn sprintf_impl(&self, vals: &[Value]) -> String {
        if vals.is_empty() {
            return String::new();
        }
        let fmt = vals[0].to_string_val();
        let mut result = String::new();
        let chars: Vec<char> = fmt.chars().collect();
        let mut i = 0;
        let mut arg_idx = 1;

        while i < chars.len() {
            if chars[i] == '%' {
                i += 1;
                if i >= chars.len() {
                    result.push('%');
                    break;
                }
                if chars[i] == '%' {
                    result.push('%');
                    i += 1;
                    continue;
                }

                // Parse format spec
                let mut flags = String::new();
                while i < chars.len() && "-+ #0".contains(chars[i]) {
                    flags.push(chars[i]);
                    i += 1;
                }

                let mut width = String::new();
                if i < chars.len() && chars[i] == '*' {
                    if arg_idx < vals.len() {
                        width = format!("{}", vals[arg_idx].to_num() as i64);
                        arg_idx += 1;
                    }
                    i += 1;
                } else {
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        width.push(chars[i]);
                        i += 1;
                    }
                }

                let mut precision = String::new();
                let has_precision = if i < chars.len() && chars[i] == '.' {
                    i += 1;
                    if i < chars.len() && chars[i] == '*' {
                        if arg_idx < vals.len() {
                            precision = format!("{}", vals[arg_idx].to_num() as i64);
                            arg_idx += 1;
                        }
                        i += 1;
                    } else {
                        while i < chars.len() && chars[i].is_ascii_digit() {
                            precision.push(chars[i]);
                            i += 1;
                        }
                    }
                    true
                } else {
                    false
                };

                if i >= chars.len() {
                    break;
                }

                let conv = chars[i];
                i += 1;

                let val = if arg_idx < vals.len() {
                    &vals[arg_idx]
                } else {
                    &Value::Uninitialized
                };
                arg_idx += 1;

                let width_num: usize = width.parse().unwrap_or(0);
                let prec_num: usize = precision.parse().unwrap_or(6);
                let left_align = flags.contains('-');
                let zero_pad = flags.contains('0') && !left_align;
                let plus_sign = flags.contains('+');
                let space_sign = flags.contains(' ');

                let formatted = match conv {
                    'd' | 'i' => {
                        let n = val.to_num() as i64;
                        let s = if plus_sign && n >= 0 {
                            format!("+{n}")
                        } else if space_sign && n >= 0 {
                            format!(" {n}")
                        } else {
                            format!("{n}")
                        };
                        s
                    }
                    'o' => format!("{:o}", val.to_num() as u64),
                    'x' => format!("{:x}", val.to_num() as u64),
                    'X' => format!("{:X}", val.to_num() as u64),
                    'u' => format!("{}", val.to_num() as u64),
                    'c' => {
                        let n = val.to_num() as u32;
                        if let Some(c) = char::from_u32(n) {
                            c.to_string()
                        } else {
                            let s = val.to_string_val();
                            if let Some(c) = s.chars().next() {
                                c.to_string()
                            } else {
                                "\0".to_string()
                            }
                        }
                    }
                    's' => {
                        let s = val.to_string_val();
                        if has_precision {
                            let chars: Vec<char> = s.chars().collect();
                            chars[..chars.len().min(prec_num)].iter().collect()
                        } else {
                            s
                        }
                    }
                    'f' => {
                        let n = val.to_num();
                        let p = if has_precision { prec_num } else { 6 };
                        let s = format!("{n:.prec$}", prec = p);
                        if plus_sign && n >= 0.0 {
                            format!("+{s}")
                        } else if space_sign && n >= 0.0 {
                            format!(" {s}")
                        } else {
                            s
                        }
                    }
                    'e' => {
                        let n = val.to_num();
                        let p = if has_precision { prec_num } else { 6 };
                        format_scientific(n, p, false)
                    }
                    'E' => {
                        let n = val.to_num();
                        let p = if has_precision { prec_num } else { 6 };
                        format_scientific(n, p, true)
                    }
                    'g' | 'G' => {
                        let n = val.to_num();
                        let p = if has_precision {
                            prec_num.max(1)
                        } else {
                            6
                        };
                        format_g(n, p, conv == 'G')
                    }
                    _ => format!("%{conv}"),
                };

                // Apply width padding
                if width_num > 0 && formatted.len() < width_num {
                    let pad = width_num - formatted.len();
                    if left_align {
                        result.push_str(&formatted);
                        for _ in 0..pad {
                            result.push(' ');
                        }
                    } else if zero_pad
                        && matches!(conv, 'd' | 'i' | 'f' | 'e' | 'E' | 'g' | 'G')
                    {
                        // Put sign before zeros
                        if formatted.starts_with('-') || formatted.starts_with('+') {
                            result.push(formatted.chars().next().unwrap());
                            for _ in 0..pad {
                                result.push('0');
                            }
                            result.push_str(&formatted[1..]);
                        } else {
                            for _ in 0..pad {
                                result.push('0');
                            }
                            result.push_str(&formatted);
                        }
                    } else {
                        for _ in 0..pad {
                            result.push(' ');
                        }
                        result.push_str(&formatted);
                    }
                } else {
                    result.push_str(&formatted);
                }
            } else if chars[i] == '\\' {
                i += 1;
                if i < chars.len() {
                    match chars[i] {
                        'n' => result.push('\n'),
                        't' => result.push('\t'),
                        'r' => result.push('\r'),
                        '\\' => result.push('\\'),
                        '"' => result.push('"'),
                        'a' => result.push('\x07'),
                        'b' => result.push('\x08'),
                        'f' => result.push('\x0C'),
                        '/' => result.push('/'),
                        _ => {
                            result.push('\\');
                            result.push(chars[i]);
                        }
                    }
                    i += 1;
                }
            } else {
                result.push(chars[i]);
                i += 1;
            }
        }
        result
    }
}

fn awk_replace(replacement: &str, caps: &regex::Captures) -> String {
    let mut result = String::new();
    let chars: Vec<char> = replacement.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' {
            i += 1;
            if i < chars.len() {
                if chars[i] == '&' {
                    result.push('&');
                } else if chars[i] == '\\' {
                    result.push('\\');
                } else if chars[i].is_ascii_digit() {
                    let n = (chars[i] as u32 - '0' as u32) as usize;
                    if let Some(m) = caps.get(n) {
                        result.push_str(m.as_str());
                    }
                } else {
                    result.push('\\');
                    result.push(chars[i]);
                }
            }
            i += 1;
        } else if chars[i] == '&' {
            result.push_str(&caps[0]);
            i += 1;
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    result
}

fn format_scientific(n: f64, prec: usize, upper: bool) -> String {
    if n == 0.0 {
        let e_char = if upper { 'E' } else { 'e' };
        return format!("0.{:0>width$}{e_char}+00", "", width = prec);
    }
    let sign = if n < 0.0 { "-" } else { "" };
    let n = n.abs();
    let exp = n.log10().floor() as i32;
    let mantissa = n / 10f64.powi(exp);
    let e_char = if upper { 'E' } else { 'e' };
    let exp_sign = if exp >= 0 { '+' } else { '-' };
    let exp_abs = exp.unsigned_abs();
    format!(
        "{sign}{mantissa:.prec$}{e_char}{exp_sign}{exp_abs:02}",
        prec = prec
    )
}

fn format_g(n: f64, prec: usize, upper: bool) -> String {
    if n == 0.0 {
        return "0".to_string();
    }
    let exp = if n == 0.0 {
        0
    } else {
        n.abs().log10().floor() as i32
    };
    if exp >= -(prec as i32) && exp < prec as i32 {
        // Use fixed notation
        let decimal_places = if prec as i32 - 1 - exp > 0 {
            (prec as i32 - 1 - exp) as usize
        } else {
            0
        };
        let s = format!("{n:.prec$}", prec = decimal_places);
        // Trim trailing zeros
        if s.contains('.') {
            let s = s.trim_end_matches('0');
            let s = s.trim_end_matches('.');
            s.to_string()
        } else {
            s
        }
    } else {
        let p = if prec > 0 { prec - 1 } else { 0 };
        format_scientific(n, p, upper)
    }
}

// ============================================================================
// Main
// ============================================================================

fn main() {
    let args: Vec<String> = env::args().collect();
    let mut program_text = String::new();
    let mut input_files: Vec<String> = Vec::new();
    let mut var_assignments: Vec<(String, String)> = Vec::new();
    let mut field_sep: Option<String> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--version" | "-V" => {
                println!(
                    "awk (rust-awk) {}",
                    env!("CARGO_PKG_VERSION")
                );
                std::process::exit(0);
            }
            "-F" => {
                i += 1;
                if i < args.len() {
                    field_sep = Some(args[i].clone());
                }
            }
            s if s.starts_with("-F") => {
                field_sep = Some(s[2..].to_string());
            }
            "-v" => {
                i += 1;
                if i < args.len() {
                    if let Some(eq_pos) = args[i].find('=') {
                        let name = args[i][..eq_pos].to_string();
                        let val = args[i][eq_pos + 1..].to_string();
                        var_assignments.push((name, val));
                    }
                }
            }
            s if s.starts_with("-v") => {
                let rest = &s[2..];
                if let Some(eq_pos) = rest.find('=') {
                    let name = rest[..eq_pos].to_string();
                    let val = rest[eq_pos + 1..].to_string();
                    var_assignments.push((name, val));
                }
            }
            "-f" => {
                i += 1;
                if i < args.len() {
                    match fs::read_to_string(&args[i]) {
                        Ok(content) => {
                            if !program_text.is_empty() {
                                program_text.push('\n');
                            }
                            program_text.push_str(&content);
                        }
                        Err(e) => {
                            eprintln!("awk: can't open source file {}: {}", args[i], e);
                            std::process::exit(2);
                        }
                    }
                }
            }
            "--" => {
                i += 1;
                while i < args.len() {
                    input_files.push(args[i].clone());
                    i += 1;
                }
                break;
            }
            s if s.starts_with('-') && s.len() > 1 && program_text.is_empty() && input_files.is_empty() => {
                // Unknown flag, skip
                eprintln!("awk: unknown option: {s}");
            }
            _ => {
                if program_text.is_empty() && !args[i].contains('=') {
                    program_text = args[i].clone();
                } else if args[i].contains('=') && program_text.is_empty() {
                    // Could be assignment or program, treat as program if no program yet
                    program_text = args[i].clone();
                } else {
                    input_files.push(args[i].clone());
                }
            }
        }
        i += 1;
    }

    if program_text.is_empty() {
        eprintln!("usage: awk [-F fs] [-v var=value] [-f progfile] 'program' [file ...]");
        std::process::exit(1);
    }

    // Tokenize and parse
    let mut lexer = Lexer::new(&program_text);
    let tokens = lexer.tokenize();
    let mut parser = Parser::new(tokens);
    let program = parser.parse();

    // Set up interpreter
    let mut interp = Interpreter::new();

    // Apply field separator
    if let Some(fs) = field_sep {
        let fs = if fs == "\\t" || fs == "\t" {
            "\t".to_string()
        } else {
            fs
        };
        interp.set_var("FS", Value::Str(fs));
    }

    // Apply variable assignments
    for (name, val) in &var_assignments {
        interp.set_var(name, Value::Str(val.clone()));
    }

    // Seed RNG
    interp.rng_state = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(12345);

    // Run
    interp.run(&program, &input_files);

    std::process::exit(interp.exit_code);
}
