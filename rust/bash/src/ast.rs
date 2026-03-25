//! AST node types for the bash shell.

/// A program is a sequence of complete commands.
pub type Program = Vec<CompleteCommand>;

/// A complete command with optional background execution.
#[derive(Debug, Clone)]
pub struct CompleteCommand {
    pub list: AndOrList,
    pub background: bool,
    pub line: usize,
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
    pub timed: bool,
    pub time_posix: bool,
    pub commands: Vec<Command>,
    /// For each pipe connection (between commands[i] and commands[i+1]),
    /// true means `|&` (redirect stderr to pipe too)
    pub pipe_stderr: Vec<bool>,
}

/// A single command: simple, compound, or function definition.
#[derive(Debug, Clone)]
pub enum Command {
    Simple(SimpleCommand),
    Compound(CompoundCommand, Vec<Redirection>),
    FunctionDef(String, Box<CompoundCommand>),
    Coproc(Option<String>, Box<Command>),
}

/// A simple command: assignments, words, and redirections.
#[derive(Debug, Clone)]
pub struct SimpleCommand {
    pub assignments: Vec<Assignment>,
    pub words: Vec<Word>,
    pub redirections: Vec<Redirection>,
}

/// A variable assignment (name=value, name+=value, or name=(array values)).
#[derive(Debug, Clone)]
pub struct Assignment {
    pub name: String,
    pub value: AssignValue,
    pub append: bool,
}

/// The right-hand side of an assignment.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum AssignValue {
    /// No value: `declare name`
    None,
    /// Scalar: `name=word`
    Scalar(Word),
    /// Array: `name=(word1 word2 ...)`
    Array(Vec<ArrayElement>),
}

/// An element in an array literal.
#[derive(Debug, Clone)]
pub struct ArrayElement {
    /// Optional explicit index: `[n]=word`
    pub index: Option<Word>,
    pub value: Word,
    /// Per-element append: `[n]+=word`
    pub append: bool,
}

/// Compound commands: control flow and grouping.
#[derive(Debug, Clone)]
pub enum CompoundCommand {
    BraceGroup(Program),
    Subshell(Program),
    If(IfClause),
    For(ForClause),
    ArithFor(ArithForClause),
    While(WhileClause),
    Until(WhileClause),
    Case(CaseClause),
    /// `[[ expression ]]`
    Conditional(CondExpr),
    /// `(( expression ))`
    Arithmetic(String),
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

/// C-style for loop: `for (( init; cond; step )) do body done`
#[derive(Debug, Clone)]
pub struct ArithForClause {
    pub init: String,
    pub cond: String,
    pub step: String,
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
    pub terminator: CaseTerminator,
}

/// How a case item terminates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CaseTerminator {
    /// `;;` — stop matching
    Break,
    /// `;&` — fall through to next body unconditionally
    FallThrough,
    /// `;;&` — continue testing next patterns
    TestNext,
}

/// Conditional expression for `[[ ]]`.
#[derive(Debug, Clone)]
pub enum CondExpr {
    /// `-n str`, `-z str`, `-e file`, etc.
    Unary(String, Word),
    /// `str1 == str2`, `str1 =~ regex`, `-eq`, etc.
    Binary(Word, String, Word),
    /// Negation: `! expr`
    Not(Box<CondExpr>),
    /// `expr1 && expr2`
    And(Box<CondExpr>, Box<CondExpr>),
    /// `expr1 || expr2`
    Or(Box<CondExpr>, Box<CondExpr>),
    /// A single word (true if non-empty)
    Word(Word),
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
    /// `<(cmd)` or `>(cmd)` — process substitution
    ProcessSub(ProcessSubKind, String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ProcessSubKind {
    /// `<(cmd)` — provides input (read from)
    Input,
    /// `>(cmd)` — provides output (write to)
    Output,
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
    /// `${!prefix}` — indirect expansion
    Indirect,
    /// `${!prefix*}` or `${!prefix@}` — names matching prefix
    NamePrefix(char),
    /// `${!arr[@]}` or `${!arr[*]}` — array indices
    ArrayIndices(char),
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
    ReplacePrefix(Word, Word),
    ReplaceSuffix(Word, Word),
    Substring(String, Option<String>),
    /// `${var^pattern}` / `${var^^pattern}` — uppercase
    UpperFirst(Word),
    UpperAll(Word),
    /// `${var,pattern}` / `${var,,pattern}` — lowercase
    LowerFirst(Word),
    LowerAll(Word),
    /// `${var~pattern}` / `${var~~pattern}` — toggle case
    ToggleFirst(Word),
    ToggleAll(Word),
    /// `${var@E}` / `${var@Q}` / `${var@P}` / `${var@A}` / `${var@a}` / `${var@K}`
    Transform(char),
}

/// I/O redirection.
#[derive(Debug, Clone)]
pub struct Redirection {
    pub fd: Option<RedirFd>,
    pub kind: RedirectKind,
    pub target: Word,
}

/// File descriptor for redirections — either a number or {varname} for auto-allocation.
#[derive(Debug, Clone)]
pub enum RedirFd {
    Number(i32),
    /// `{varname}` — allocate a new fd and store in varname
    #[allow(dead_code)]
    Var(String),
}

#[derive(Debug, Clone)]
pub enum RedirectKind {
    Input,
    Output,
    Append,
    Clobber,
    /// `&>` — redirect both stdout and stderr to file
    OutputAll,
    /// `&>>` — append both stdout and stderr to file
    AppendAll,
    DupInput,
    DupOutput,
    ReadWrite,
    #[allow(dead_code)]
    HereDoc(bool, String),
    HereString,
    /// `<(cmd)` — process substitution (read)
    #[allow(dead_code)]
    ProcessSubIn,
    /// `>(cmd)` — process substitution (write)
    #[allow(dead_code)]
    ProcessSubOut,
}

/// Get the literal text of a word (without expansion).
/// Reconstruct word text for xtrace display — preserves $var syntax and escapes metacharacters
pub fn word_to_xtrace_string(word: &Word) -> String {
    let mut s = String::new();
    for part in word {
        match part {
            WordPart::Literal(t) => {
                // Escape shell metacharacters with backslash
                for ch in t.chars() {
                    if matches!(
                        ch,
                        '|' | '&'
                            | ';'
                            | '('
                            | ')'
                            | '<'
                            | '>'
                            | '\\'
                            | '!'
                            | '{'
                            | '}'
                            | '*'
                            | '?'
                            | '['
                            | ']'
                    ) {
                        s.push('\\');
                    }
                    s.push(ch);
                }
            }
            WordPart::SingleQuoted(t) => {
                // For xtrace: if single char metacharacter, use \char
                if t.len() == 1 {
                    let ch = t.chars().next().unwrap();
                    if matches!(
                        ch,
                        '|' | '&'
                            | ';'
                            | '('
                            | ')'
                            | '<'
                            | '>'
                            | '\\'
                            | '!'
                            | '{'
                            | '}'
                            | '*'
                            | '?'
                            | '['
                            | ']'
                    ) {
                        s.push('\\');
                        s.push(ch);
                    } else if ch == ' ' {
                        s.push_str("' '");
                    } else if ch == '\t' || ch == '\n' {
                        s.push('\'');
                        s.push(ch);
                        s.push('\'');
                    } else {
                        s.push('\'');
                        s.push_str(t);
                        s.push('\'');
                    }
                } else {
                    s.push('\'');
                    s.push_str(t);
                    s.push('\'');
                }
            }
            WordPart::Tilde(t) => s.push_str(t),
            WordPart::DoubleQuoted(parts) => {
                s.push_str(&word_to_xtrace_string(parts));
            }
            WordPart::Variable(name) => {
                s.push('$');
                s.push_str(name);
            }
            WordPart::Param(_) => {
                // Just show as raw text
                s.push_str(&word_to_string(word));
                return s;
            }
            _ => {
                let v = vec![part.clone()];
                s.push_str(&word_to_string(&v));
            }
        }
    }
    s
}

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
