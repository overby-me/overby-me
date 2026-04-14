#[derive(Debug, Clone)]
pub enum Expr {
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
pub enum GetlineSource {
    Stdin,
    File,
    Pipe,
}

#[derive(Debug, Clone, Copy)]
pub enum BinOp {
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
pub enum UnOp {
    Neg,
    Not,
    PreIncrement,
    PreDecrement,
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Expr(Expr),
    Print(Vec<Expr>, Option<OutputDest>),
    Printf(Vec<Expr>, Option<OutputDest>),
    If(Expr, Box<Stmt>, Option<Box<Stmt>>),
    While(Expr, Box<Stmt>),
    DoWhile(Box<Stmt>, Expr),
    For(
        Option<Box<Stmt>>,
        Option<Expr>,
        Option<Box<Stmt>>,
        Box<Stmt>,
    ),
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
pub enum OutputDest {
    File(Expr),
    Append(Expr),
    Pipe(Expr),
}

#[derive(Debug, Clone)]
pub enum Pattern {
    Begin,
    End,
    Expression(Expr),
    Range(Expr, Expr),
}

#[derive(Debug, Clone)]
pub struct Rule {
    pub pattern: Option<Pattern>,
    pub action: Vec<Stmt>,
}

#[derive(Debug, Clone)]
pub struct FuncDef {
    pub name: String,
    pub params: Vec<String>,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone)]
pub struct Program {
    pub rules: Vec<Rule>,
    pub functions: Vec<FuncDef>,
}
