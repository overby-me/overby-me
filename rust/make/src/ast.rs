//! AST types for Makefile parsing.

/// A parsed Makefile is a list of directives.
pub type Makefile = Vec<Directive>;

/// Top-level directives in a Makefile.
#[derive(Debug, Clone)]
pub enum Directive {
    Rule(Rule),
    Assignment(Assignment),
    Include(Vec<String>, bool), // (files, is_sinclude/-include)
    Conditional(Conditional),
    Export(Option<String>),   // export VAR or export (all)
    Unexport(Option<String>), // unexport VAR or unexport (all)
    #[allow(dead_code)]
    Vpath(Option<(String, String)>),
    Override(Box<Assignment>),
    Define(String, AssignOp, Vec<String>), // multi-line variable
    Undefine(String),
}

/// A make rule: targets, prerequisites, order-only prereqs, and recipe lines.
#[derive(Debug, Clone)]
pub struct Rule {
    pub targets: Vec<String>,
    pub pattern: Option<PatternRule>,
    pub prerequisites: Vec<String>,
    pub order_only: Vec<String>,
    pub recipe: Vec<String>,
    pub is_double_colon: bool,
}

/// Pattern rule info (e.g., %.o: %.c).
#[derive(Debug, Clone)]
pub struct PatternRule {
    #[allow(dead_code)]
    pub target_pattern: String,
    pub prereq_patterns: Vec<String>,
}

/// Variable assignment.
#[derive(Debug, Clone)]
pub struct Assignment {
    pub name: String,
    pub op: AssignOp,
    pub value: String,
}

/// Assignment operators.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AssignOp {
    Recursive,   // =
    Simple,      // :=  or ::=
    Conditional, // ?=
    Append,      // +=
    Shell,       // !=
}

/// Conditional directives.
#[derive(Debug, Clone)]
pub struct Conditional {
    pub kind: CondKind,
    pub then_body: Makefile,
    pub else_body: Option<Makefile>,
}

#[derive(Debug, Clone)]
pub enum CondKind {
    Ifdef(String),
    Ifndef(String),
    Ifeq(String, String),
    Ifneq(String, String),
}
