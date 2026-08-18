//! AST types for Makefile parsing.

/// A parsed Makefile is a list of directives.
pub type Makefile = Vec<Directive>;

/// Top-level directives in a Makefile.
#[derive(Debug, Clone)]
pub enum Directive {
    Rule(Rule),
    Assignment(Assignment, String, usize), // (assignment, source_name, line_no)
    Include(Vec<String>, bool, String, usize), // (files, is_sinclude/-include, source_name, line_no)
    Conditional(Conditional),
    /// `export` (no args) → export all. `export VAR ...` → export the
    /// listed vars.
    Export(Option<Vec<String>>),
    ExportAssign(Box<Assignment>), // export VAR = value: assign + export
    Unexport(Option<Vec<String>>), // unexport VAR ... or unexport
    UnexportAssign(Box<Assignment>), // unexport VAR = value: assign + unexport
    #[allow(dead_code)]
    Vpath(Option<(String, String)>),
    Override(Box<Assignment>),
    Define(String, AssignOp, Vec<String>, String, usize), // multi-line variable (name, op, body, source, line_no)
    OverrideDefine(String, AssignOp, Vec<String>, String, usize), // override define NAME…endef
    Undefine(String, String, usize),                      // (name, source, line_no)
    OverrideUndefine(String, String, usize), // (name, source, line_no) // override undefine VAR — force-remove
    /// Target-specific variable assignment: `targets: VAR OP value`.
    /// `targets` is the raw (possibly space-separated, unexpanded) list;
    /// the engine expands it at load time.
    TargetVarAssign(String, Box<Assignment>),
    /// Global `private VAR = val` — assign and mark as private (not visible
    /// in recipe execution contexts).
    PrivateAssign(Box<Assignment>, String, usize),
    /// Global `private export VAR = val` — assign, export, and mark as private.
    PrivateExportAssign(Box<Assignment>),
    /// A bare expression line (e.g. `$(error msg)` / `$(info ...)` /
    /// `$(eval ...)`) evaluated at makefile-load time for its side effects.
    /// Carries `(expr, source_name, line_no)` so diagnostics from
    /// `$(error)` / `$(warning)` can include the source location.
    Expand(String, String, usize),
    /// A tab-prefixed recipe line appearing in a conditional body (or
    /// otherwise detached from a rule parse). Carries `(recipe_text,
    /// line_no)`. The engine appends it to the most recently emitted
    /// explicit rule's recipe when the enclosing conditional branch is
    /// taken — matching GNU make's behavior where recipe lines inside
    /// `ifeq…endif` blocks belong to the preceding rule.
    RecipeLine(String, usize),
}

/// A make rule: targets, prerequisites, order-only prereqs, and recipe lines.
#[derive(Debug, Clone)]
pub struct Rule {
    pub targets: Vec<String>,
    pub pattern: Option<PatternRule>,
    pub prerequisites: Vec<String>,
    pub order_only: Vec<String>,
    pub recipe: Vec<String>,
    /// 1-based source line number for each entry in `recipe`; parallel
    /// to the recipe vec. Used for `$(error)`/`$(warning)` diagnostics.
    pub recipe_lines: Vec<usize>,
    pub source_name: String,
    pub is_double_colon: bool,
    /// `&:` grouped-target rule. Each target is considered updated
    /// after the recipe runs once for any one of them.
    pub is_grouped: bool,
    /// 1-based line number of the rule definition in its source file.
    pub line_no: usize,
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
    Recursive,          // =
    Simple,             // :=  or ::=
    ImmediateRecursive, // :::= (expand RHS now, but store as recursive)
    Conditional,        // ?=
    Append,             // +=
    Shell,              // !=
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
