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
    HashKVSlice(String, Vec<Expr>),  // %hash{list} — key/value slice
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
    /// Array dereference of a scalar reference: `@$name` / `@{$name}`.
    /// In list context yields the array's elements; in scalar context the length.
    ArrayDerefVar(String),
    /// Hash dereference of a scalar reference: `%$name` / `%{$name}`.
    HashDerefVar(String),
    /// Scalar dereference of a scalar reference: `$$name` / `${$name}`.
    ScalarDerefVar(String),
    /// Typeglob literal: `*NAME`. Evaluates to `Value::Glob(name)`.
    GlobVar(String),
    /// Arrow-element: `$ref->[i]` or `$ref->{k}`.
    ArrowElement(Box<Expr>, Box<Expr>, ArrowKind),

    // Range
    Range(Box<Expr>, Box<Expr>), // expr..expr

    // Defined
    Defined(Box<Expr>),

    // Wantarray
    Wantarray,

    // Do block/file
    DoBlock(Vec<Stmt>),
    DoFile(Box<Expr>),

    // Anonymous sub: `sub { ... }` → CodeRef
    AnonSub(Vec<String>, Vec<Stmt>),

    // `$coderef->(args)` — invoke a code reference
    CodeCall(Box<Expr>, Vec<Expr>),

    // Local/My as expression (for my $x = ...)
    MyVar(String),
    LocalVar(String),

    // File test operators (-e, -f, -d, etc.)
    FileTest(String, Box<Expr>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum ArrowKind {
    Array, // $ref->[i]
    Hash,  // $ref->{k}
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
    Xor,

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
        continue_body: Option<Vec<Stmt>>,
        label: Option<String>,
    },
    Until {
        cond: Expr,
        body: Vec<Stmt>,
        continue_body: Option<Vec<Stmt>>,
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
        continue_body: Option<Vec<Stmt>>,
        label: Option<String>,
    },
    /// A bare `{ BLOCK } continue { BLOCK }` construct — runs body once, then
    /// continue_body, unless `last` exits. `last`/`next`/`redo` here behave as
    /// in a one-shot loop.
    BlockWithContinue {
        body: Vec<Stmt>,
        continue_body: Vec<Stmt>,
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
    Goto(String),
    /// Statement label — marks a position that `goto LABEL` / `last LABEL` /
    /// `next LABEL` / `redo LABEL` can target. The label applies to the
    /// *following* statement; we emit it as its own no-op Stmt so the
    /// enclosing block can scan for the name on jump.
    Label(String),
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
    My(Vec<(String, Option<Expr>)>, bool), // my ($a, $b) = ...; bool = is list-context destructure (parens used)
    Local(Vec<(String, Option<Expr>)>, bool),
    /// `local $NAME{KEY} = VAL;` — save the old hash element, set the new
    /// value, and schedule restoration at scope exit. Required for the
    /// `local $SIG{__DIE__} = sub {…}` idiom (temporarily install a die
    /// handler for the enclosing block).
    LocalHashElem(String, Expr, Option<Expr>),
    /// `local @arr[i,j] = LIST` / `local %h{a,b} = LIST` — slice-form
    /// localisation. `name` is `@arr` or `%h` (with sigil prefix to mark
    /// array vs hash). The key list is the indices/keys; if `Some(rhs)`
    /// is given, its list-context evaluation is destructured 1-1 onto
    /// the slots, padding with undef. On scope exit each slot is restored
    /// to its prior value (or marked deleted if originally absent).
    LocalSlice(String, Vec<Expr>, Option<Expr>),
    Our(Vec<(String, Option<Expr>)>, bool),

    // Package
    Package(String),

    // Use/require
    Use(String, Vec<Expr>),
    Require(Expr),

    // BEGIN/END. The optional usize on Begin is the source line of the
    // closing `}` — used so `BEGIN failed--compilation aborted` can report
    // the right line (matching reference perl).
    Begin(Vec<Stmt>, usize),
    End(Vec<Stmt>),
    Check(Vec<Stmt>),
    Init(Vec<Stmt>),

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

    /// Emitted by the parser to mark the source line of the following stmt.
    /// The interpreter uses it to keep `current_line` up to date for `caller()`.
    LineMark(usize),

    /// Emitted by the parser to mark the source file of the following stmt.
    /// Pushed before a LineMark when a `# line N "FILE"` directive sets a file.
    FileMark(String),
}

#[derive(Clone, Debug)]
pub enum EvalArg {
    Block(Vec<Stmt>),
    Expr(Expr),
}
