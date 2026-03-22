/// AST node types for the Meson build language.

#[derive(Debug, Clone)]
pub struct SourceLocation {
    pub line: usize,
    pub col: usize,
}

#[derive(Debug, Clone)]
pub struct Program {
    pub statements: Vec<Statement>,
}

#[derive(Debug, Clone)]
pub enum Statement {
    Expression(Expression),
    Assignment(Assignment),
    PlusAssignment(Assignment),
    If(IfStatement),
    Foreach(ForeachStatement),
    Break(SourceLocation),
    Continue(SourceLocation),
}

#[derive(Debug, Clone)]
pub struct Assignment {
    pub name: String,
    pub value: Expression,
    pub loc: SourceLocation,
}

#[derive(Debug, Clone)]
pub struct IfStatement {
    pub condition: Expression,
    pub body: Vec<Statement>,
    pub elif_clauses: Vec<(Expression, Vec<Statement>)>,
    pub else_body: Option<Vec<Statement>>,
    pub loc: SourceLocation,
}

#[derive(Debug, Clone)]
pub struct ForeachStatement {
    pub varnames: Vec<String>,
    pub iterable: Expression,
    pub body: Vec<Statement>,
    pub loc: SourceLocation,
}

#[derive(Debug, Clone)]
pub enum Expression {
    StringLiteral(String, SourceLocation),
    MultilineStringLiteral(String, SourceLocation),
    FStringLiteral(String, SourceLocation),
    IntLiteral(i64, SourceLocation),
    BoolLiteral(bool, SourceLocation),
    Identifier(String, SourceLocation),
    Array(Vec<Expression>, SourceLocation),
    Dict(Vec<(Expression, Expression)>, SourceLocation),
    UnaryOp(UnaryOp, Box<Expression>, SourceLocation),
    BinaryOp(BinaryOp, Box<Expression>, Box<Expression>, SourceLocation),
    FunctionCall(Box<Expression>, Vec<Argument>, SourceLocation),
    MethodCall(Box<Expression>, String, Vec<Argument>, SourceLocation),
    Index(Box<Expression>, Box<Expression>, SourceLocation),
    Ternary(
        Box<Expression>,
        Box<Expression>,
        Box<Expression>,
        SourceLocation,
    ),
}

impl Expression {
    pub fn loc(&self) -> &SourceLocation {
        match self {
            Expression::StringLiteral(_, loc)
            | Expression::MultilineStringLiteral(_, loc)
            | Expression::FStringLiteral(_, loc)
            | Expression::IntLiteral(_, loc)
            | Expression::BoolLiteral(_, loc)
            | Expression::Identifier(_, loc)
            | Expression::Array(_, loc)
            | Expression::Dict(_, loc)
            | Expression::UnaryOp(_, _, loc)
            | Expression::BinaryOp(_, _, _, loc)
            | Expression::FunctionCall(_, _, loc)
            | Expression::MethodCall(_, _, _, loc)
            | Expression::Index(_, _, loc)
            | Expression::Ternary(_, _, _, loc) => loc,
        }
    }
}

#[derive(Debug, Clone)]
pub enum UnaryOp {
    Not,
    Negate,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Neq,
    Lt,
    Gt,
    Le,
    Ge,
    And,
    Or,
    In,
    NotIn,
}

#[derive(Debug, Clone)]
pub struct Argument {
    pub name: Option<String>,
    pub value: Expression,
}
