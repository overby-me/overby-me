#[derive(Clone, Debug)]
pub enum Expr {
    // Literals
    IntLit(i64),
    FloatLit(f64),
    StringLit(String),
    RegexLit(String, String),
    ArrayLit(Vec<Expr>),
    HashLit(Vec<(Expr, Expr)>),
    QW(Vec<String>),
    Undef,

    // Variables
    ScalarVar(String),
    ArrayVar(String),
    HashVar(String),
    ArrayElement(String, Box<Expr>), // $array[expr]
    HashElement(String, Box<Expr>),  // $hash{expr}
    ArraySlice(String, Vec<Expr>),   // @array[list]
    HashSlice(String, Vec<Expr>),    // @hash{list}
    ArrayLen(String),                // $#array

    // String interpolation — a sequence of literal parts and embedded expressions
    Interp(Vec<InterpPart>),

    // Operations
    BinOp(BinOp, Box<Expr>, Box<Expr>),
    UnaryOp(UnaryOp, Box<Expr>),
    PostfixOp(PostfixOp, Box<Expr>),

    // Assignment
    Assign(Box<Expr>, Box<Expr>),
    OpAssign(BinOp, Box<Expr>, Box<Expr>),

    // Regex ops
    RegexMatch(Box<Expr>, String, String), // expr =~ /pat/flags
    RegexNotMatch(Box<Expr>, String, String), // expr !~ /pat/flags
    Substitution(Box<Expr>, String, String, String), // expr =~ s/pat/repl/flags

    // Ternary
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),

    // Function/method calls
    Call(String, Vec<Expr>),
    MethodCall(Box<Expr>, String, Vec<Expr>),

    // Special
    Diamond(String),            // <FH> or <>
    Backtick(String),           // `command` (literal)
    BacktickInterp(Box<Expr>),  // `command` with interpolation
    Ref(Box<Expr>),             // \expr
    Deref(Box<Expr>),           // $$ref, @$ref, %$ref
    ArrayRef(Vec<Expr>),        // [expr, ...]
    HashRef(Vec<(Expr, Expr)>), // {key => val, ...}

    // Range
    Range(Box<Expr>, Box<Expr>), // expr..expr

    // Defined
    Defined(Box<Expr>),

    // Wantarray
    Wantarray,

    // Do block/file
    DoBlock(Vec<Stmt>),
    DoFile(Box<Expr>),

    // Local/My as expression (for my $x = ...)
    MyVar(String),
    LocalVar(String),

    // File test operators (-e, -f, -d, etc.)
    FileTest(String, Box<Expr>),
}

#[derive(Clone, Debug)]
pub enum InterpPart {
    Lit(String),
    ScalarVar(String),
    ArrayVar(String),
    ArrayElement(String, Box<Expr>),
    HashElement(String, Box<Expr>),
    Expr(Box<Expr>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum BinOp {
    // Arithmetic
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,

    // String
    Concat,
    Repeat, // x

    // Numeric comparison
    NumEq,
    NumNe,
    NumLt,
    NumGt,
    NumLe,
    NumGe,
    Spaceship,

    // String comparison
    StrEq,
    StrNe,
    StrLt,
    StrGt,
    StrLe,
    StrGe,
    StrCmp,

    // Logical
    LogAnd,
    LogOr,
    DefOr,
    And,
    Or,

    // Bitwise
    BitAnd,
    BitOr,
    BitXor,
    ShiftLeft,
    ShiftRight,

    // Range
    Range,
}

#[derive(Clone, Debug)]
pub enum UnaryOp {
    Neg,
    Pos,
    LogNot,
    BitNot,
    Not,
    Ref,
    PreInc,
    PreDec,
}

#[derive(Clone, Debug)]
pub enum PostfixOp {
    Inc,
    Dec,
}

#[derive(Clone, Debug)]
pub enum Stmt {
    // Expression statement
    Expr(Expr),

    // Print/say
    Print(Option<Expr>, Vec<Expr>), // filehandle, args
    Say(Option<Expr>, Vec<Expr>),
    Printf(Option<Expr>, Vec<Expr>),

    // Control flow
    If {
        cond: Expr,
        then: Vec<Stmt>,
        elsifs: Vec<(Expr, Vec<Stmt>)>,
        else_block: Option<Vec<Stmt>>,
    },
    Unless {
        cond: Expr,
        then: Vec<Stmt>,
        else_block: Option<Vec<Stmt>>,
    },
    While {
        cond: Expr,
        body: Vec<Stmt>,
        label: Option<String>,
    },
    Until {
        cond: Expr,
        body: Vec<Stmt>,
        label: Option<String>,
    },
    DoWhile {
        body: Vec<Stmt>,
        cond: Expr,
    },
    For {
        init: Option<Box<Stmt>>,
        cond: Option<Expr>,
        step: Option<Expr>,
        body: Vec<Stmt>,
        label: Option<String>,
    },
    Foreach {
        var: String,
        is_my: bool,
        list: Expr,
        body: Vec<Stmt>,
        label: Option<String>,
    },
    Loop {
        body: Vec<Stmt>,
        label: Option<String>,
    },

    // Flow control
    Last(Option<String>),
    Next(Option<String>),
    Redo(Option<String>),
    Return(Option<Expr>),

    // Block
    Block(Vec<Stmt>),
    NamedBlock(String, Vec<Stmt>),

    // Declarations
    Sub {
        name: String,
        params: Vec<String>,
        body: Vec<Stmt>,
    },
    My(Vec<(String, Option<Expr>)>), // my ($a, $b) = ...
    Local(Vec<(String, Option<Expr>)>),
    Our(Vec<(String, Option<Expr>)>),

    // Package
    Package(String),

    // Use/require
    Use(String, Vec<Expr>),
    Require(Expr),

    // BEGIN/END
    Begin(Vec<Stmt>),
    End(Vec<Stmt>),

    // Die/warn
    Die(Vec<Expr>),
    Warn(Vec<Expr>),

    // Eval
    Eval(Box<EvalArg>),

    // Postfix conditions
    PostfixIf(Box<Stmt>, Expr),
    PostfixUnless(Box<Stmt>, Expr),
    PostfixWhile(Box<Stmt>, Expr),
    PostfixUntil(Box<Stmt>, Expr),
    PostfixFor(Box<Stmt>, Expr),

    // Bare block (for scoping)
    BareBlock(Vec<Stmt>),

    // No-op
    Nop,
}

#[derive(Clone, Debug)]
pub enum EvalArg {
    Block(Vec<Stmt>),
    Expr(Expr),
}
