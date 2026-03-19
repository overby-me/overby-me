//! AST node types for the bash shell.

/// A program is a sequence of complete commands.
pub type Program = Vec<CompleteCommand>;

/// A complete command with optional background execution.
#[derive(Debug, Clone)]
pub struct CompleteCommand {
    pub list: AndOrList,
    pub background: bool,
}

/// A chain of pipelines connected by && or ||.
#[derive(Debug, Clone)]
pub struct AndOrList {
    pub first: Pipeline,
    pub rest: Vec<(AndOr, Pipeline)>,
}

#[derive(Debug, Clone, Copy)]
pub enum AndOr {
    And, // &&
    Or,  // ||
}

/// A pipeline of commands connected by |.
#[derive(Debug, Clone)]
pub struct Pipeline {
    pub negated: bool,
    pub commands: Vec<Command>,
}

/// A single command: simple, compound, or function definition.
#[derive(Debug, Clone)]
pub enum Command {
    Simple(SimpleCommand),
    Compound(CompoundCommand, Vec<Redirection>),
    FunctionDef(String, Box<CompoundCommand>),
}

/// A simple command: assignments, words, and redirections.
#[derive(Debug, Clone)]
pub struct SimpleCommand {
    pub assignments: Vec<Assignment>,
    pub words: Vec<Word>,
    pub redirections: Vec<Redirection>,
}

/// A variable assignment (name=value or name+=value).
#[derive(Debug, Clone)]
pub struct Assignment {
    pub name: String,
    pub value: Option<Word>,
    pub append: bool,
}

/// Compound commands: control flow and grouping.
#[derive(Debug, Clone)]
pub enum CompoundCommand {
    BraceGroup(Program),
    Subshell(Program),
    If(IfClause),
    For(ForClause),
    While(WhileClause),
    Until(WhileClause),
    Case(CaseClause),
}

#[derive(Debug, Clone)]
pub struct IfClause {
    pub condition: Program,
    pub then_body: Program,
    pub elif_parts: Vec<(Program, Program)>,
    pub else_body: Option<Program>,
}

#[derive(Debug, Clone)]
pub struct ForClause {
    pub var: String,
    pub words: Option<Vec<Word>>,
    pub body: Program,
}

#[derive(Debug, Clone)]
pub struct WhileClause {
    pub condition: Program,
    pub body: Program,
}

#[derive(Debug, Clone)]
pub struct CaseClause {
    pub word: Word,
    pub items: Vec<CaseItem>,
}

#[derive(Debug, Clone)]
pub struct CaseItem {
    pub patterns: Vec<Word>,
    pub body: Program,
}

/// A word is a sequence of parts that get concatenated after expansion.
pub type Word = Vec<WordPart>;

#[derive(Debug, Clone, PartialEq)]
pub enum WordPart {
    Literal(String),
    SingleQuoted(String),
    DoubleQuoted(Vec<WordPart>),
    Tilde(String),
    Variable(String),
    Param(ParamExpr),
    CommandSub(String),
    BacktickSub(String),
    ArithSub(String),
}

/// Parameter expansion: ${name op word}
#[derive(Debug, Clone, PartialEq)]
pub struct ParamExpr {
    pub name: String,
    pub op: ParamOp,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParamOp {
    None,
    Length,
    Default(bool, Word),
    Assign(bool, Word),
    Error(bool, Word),
    Alt(bool, Word),
    TrimSmallLeft(Word),
    TrimLargeLeft(Word),
    TrimSmallRight(Word),
    TrimLargeRight(Word),
    Replace(Word, Word),
    ReplaceAll(Word, Word),
    Substring(String, Option<String>),
}

/// I/O redirection.
#[derive(Debug, Clone)]
pub struct Redirection {
    pub fd: Option<i32>,
    pub kind: RedirectKind,
    pub target: Word,
}

#[derive(Debug, Clone)]
pub enum RedirectKind {
    Input,
    Output,
    Append,
    Clobber,
    DupInput,
    DupOutput,
    ReadWrite,
    #[allow(dead_code)]
    HereDoc(bool),
    HereString,
}

/// Get the literal text of a word (without expansion).
pub fn word_to_string(word: &Word) -> String {
    let mut s = String::new();
    for part in word {
        match part {
            WordPart::Literal(t) | WordPart::SingleQuoted(t) | WordPart::Tilde(t) => {
                s.push_str(t);
            }
            WordPart::DoubleQuoted(parts) => {
                s.push_str(&word_to_string(parts));
            }
            WordPart::Variable(name) => {
                s.push('$');
                s.push_str(name);
            }
            _ => {}
        }
    }
    s
}
