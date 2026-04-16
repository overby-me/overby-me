use std::collections::HashSet;

#[derive(Clone, Debug)]
pub enum Token {
    // Literals
    Integer(i64),
    Float(f64),
    StringLit(String),
    RegexLit(String, String), // pattern, flags
    QW(Vec<String>),
    InterpString(String), // double-quoted string needing variable interpolation

    // Variables
    ScalarVar(String), // $name
    ArrayVar(String),  // @name
    HashVar(String),   // %name
    ArrayLen(String),  // $#name

    // Keywords
    If,
    Else,
    Elsif,
    Unless,
    While,
    Until,
    For,
    Foreach,
    My,
    Our,
    Local,
    Sub,
    Return,
    Last,
    Next,
    Redo,
    Print,
    Say,
    Die,
    Warn,
    Begin,
    End,
    Use,
    Require,
    Package,
    Do,
    Eval,
    UndefKw,
    Defined,
    Not,
    And,
    Or,
    Chomp,
    Chop,
    Push,
    Pop,
    Shift,
    Unshift,
    Splice,
    Delete,
    Exists,
    Keys,
    Values,
    Each,
    Reverse,
    Sort,
    Join,
    Split,
    Grep,
    Map,
    Abs,
    Int,
    Length,
    Substr,
    Index,
    Rindex,
    Sprintf,
    Printf,
    Chr,
    Ord,
    Lc,
    Uc,
    Lcfirst,
    Ucfirst,
    Hex,
    Oct,
    Ref,
    Wantarray,
    Caller,
    Open,
    Close,
    Read,
    Eof,
    Binmode,
    Unlink,
    Rename,
    Mkdir,
    Rmdir,
    Chdir,
    Stat,

    // Operators
    Assign,           // =
    Plus,             // +
    Minus,            // -
    Star,             // *
    Slash,            // /
    Percent,          // %
    Power,            // **
    Dot,              // .
    DotDot,           // ..
    Eq,               // eq
    Ne,               // ne
    Lt,               // lt
    Gt,               // gt
    Le,               // le
    Ge,               // ge
    NumEq,            // ==
    NumNe,            // !=
    NumLt,            // <
    NumGt,            // >
    NumLe,            // <=
    NumGe,            // >=
    Spaceship,        // <=>
    Cmp,              // cmp
    LogAnd,           // &&
    LogOr,            // ||
    LogNot,           // !
    DefOr,            // //
    BitAnd,           // &
    BitOr,            // |
    BitXor,           // ^
    BitNot,           // ~
    ShiftLeft,        // <<
    ShiftRight,       // >>
    PlusPlus,         // ++
    MinusMinus,       // --
    PlusAssign,       // +=
    MinusAssign,      // -=
    StarAssign,       // *=
    SlashAssign,      // /=
    PercentAssign,    // %=
    DotAssign,        // .=
    PowerAssign,      // **=
    LogAndAssign,     // &&=
    LogOrAssign,      // ||=
    DefOrAssign,      // //=
    BitAndAssign,     // &=
    BitOrAssign,      // |=
    BitXorAssign,     // ^=
    ShiftLeftAssign,  // <<=
    ShiftRightAssign, // >>=
    Match,            // =~
    NotMatch,         // !~
    Arrow,            // ->
    FatComma,         // =>
    Question,         // ?
    Colon,            // :
    Comma,            // ,
    Semi,             // ;
    Backslash,        // \
    StringRepeat,     // x

    // Delimiters
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,

    // Regex operators
    Substitution(String, String, String), // s/pattern/replacement/flags
    Transliterate(String, String, String), // tr/from/to/flags or y/from/to/flags

    // Special
    Ident(String),
    Diamond(String), // <FH> or <>
    Newline,
    EOF,
}

impl Token {
    /// Whether this token can be followed by a regex literal (/)
    pub fn expects_operand(&self) -> bool {
        matches!(
            self,
            Token::Assign
                | Token::Plus
                | Token::Minus
                | Token::Star
                | Token::Slash
                | Token::Percent
                | Token::Power
                | Token::Dot
                | Token::DotDot
                | Token::NumEq
                | Token::NumNe
                | Token::NumLt
                | Token::NumGt
                | Token::NumLe
                | Token::NumGe
                | Token::Spaceship
                | Token::LogAnd
                | Token::LogOr
                | Token::LogNot
                | Token::DefOr
                | Token::BitAnd
                | Token::BitOr
                | Token::BitXor
                | Token::BitNot
                | Token::ShiftLeft
                | Token::ShiftRight
                | Token::PlusAssign
                | Token::MinusAssign
                | Token::StarAssign
                | Token::SlashAssign
                | Token::PercentAssign
                | Token::DotAssign
                | Token::PowerAssign
                | Token::Match
                | Token::NotMatch
                | Token::Arrow
                | Token::FatComma
                | Token::Question
                | Token::Colon
                | Token::Comma
                | Token::Semi
                | Token::Backslash
                | Token::LParen
                | Token::LBracket
                | Token::LBrace
                | Token::If
                | Token::Else
                | Token::Elsif
                | Token::Unless
                | Token::While
                | Token::Until
                | Token::For
                | Token::Foreach
                | Token::My
                | Token::Our
                | Token::Local
                | Token::Sub
                | Token::Return
                | Token::Last
                | Token::Next
                | Token::Print
                | Token::Say
                | Token::Die
                | Token::Warn
                | Token::Begin
                | Token::End
                | Token::Do
                | Token::Eval
                | Token::UndefKw
                | Token::Defined
                | Token::Not
                | Token::And
                | Token::Or
                | Token::Chomp
                | Token::Chop
                | Token::Push
                | Token::Pop
                | Token::Shift
                | Token::Unshift
                | Token::Splice
                | Token::Delete
                | Token::Exists
                | Token::Keys
                | Token::Values
                | Token::Each
                | Token::Reverse
                | Token::Sort
                | Token::Join
                | Token::Split
                | Token::Grep
                | Token::Map
                | Token::Abs
                | Token::Int
                | Token::Length
                | Token::Substr
                | Token::Index
                | Token::Rindex
                | Token::Sprintf
                | Token::Printf
                | Token::Chr
                | Token::Ord
                | Token::Lc
                | Token::Uc
                | Token::Hex
                | Token::Oct
                | Token::Ref
                | Token::Open
                | Token::Close
                | Token::Read
                | Token::Binmode
                | Token::Unlink
                | Token::Rename
                | Token::Mkdir
                | Token::Rmdir
                | Token::Chdir
                | Token::Stat
                | Token::Newline
                | Token::EOF
        )
    }
}

pub struct Lexer {
    input: Vec<char>,
    pos: usize,
    pub tokens: Vec<Token>,
    keywords: HashSet<&'static str>,
}

impl Lexer {
    pub fn new(input: &str) -> Self {
        let keywords: HashSet<&str> = [
            "if",
            "else",
            "elsif",
            "unless",
            "while",
            "until",
            "for",
            "foreach",
            "my",
            "our",
            "local",
            "sub",
            "return",
            "last",
            "next",
            "redo",
            "print",
            "say",
            "die",
            "warn",
            "BEGIN",
            "END",
            "use",
            "require",
            "package",
            "do",
            "eval",
            "undef",
            "defined",
            "not",
            "and",
            "or",
            "eq",
            "ne",
            "lt",
            "gt",
            "le",
            "ge",
            "cmp",
            "chomp",
            "chop",
            "push",
            "pop",
            "shift",
            "unshift",
            "splice",
            "delete",
            "exists",
            "keys",
            "values",
            "each",
            "reverse",
            "sort",
            "join",
            "split",
            "grep",
            "map",
            "abs",
            "int",
            "length",
            "substr",
            "index",
            "rindex",
            "sprintf",
            "printf",
            "chr",
            "ord",
            "lc",
            "uc",
            "lcfirst",
            "ucfirst",
            "hex",
            "oct",
            "ref",
            "wantarray",
            "caller",
            "open",
            "close",
            "read",
            "eof",
            "binmode",
            "unlink",
            "rename",
            "mkdir",
            "rmdir",
            "chdir",
            "stat",
            "x",
        ]
        .into_iter()
        .collect();

        Lexer {
            input: input.chars().collect(),
            pos: 0,
            tokens: Vec::new(),
            keywords,
        }
    }

    fn ch(&self) -> char {
        if self.pos < self.input.len() {
            self.input[self.pos]
        } else {
            '\0'
        }
    }

    fn peek(&self, offset: usize) -> char {
        let p = self.pos + offset;
        if p < self.input.len() {
            self.input[p]
        } else {
            '\0'
        }
    }

    fn advance(&mut self) -> char {
        let c = self.ch();
        self.pos += 1;
        c
    }

    fn skip_whitespace_and_comments(&mut self) {
        loop {
            // Skip whitespace (but not newlines — we track those)
            while self.pos < self.input.len()
                && self.ch().is_ascii_whitespace()
                && self.ch() != '\n'
            {
                self.pos += 1;
            }
            // Skip comments
            if self.ch() == '#' {
                while self.pos < self.input.len() && self.ch() != '\n' {
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    fn skip_pod(&mut self) {
        // Skip =pod / =head1 / =cut blocks
        while self.pos < self.input.len() {
            if self.ch() == '\n' {
                self.pos += 1;
                if self.pos < self.input.len() && self.ch() == '=' {
                    // Check for =cut
                    let rest: String = self.input[self.pos..].iter().take(4).collect();
                    if rest == "=cut" || rest.starts_with("=cut") {
                        // Skip to end of =cut line
                        while self.pos < self.input.len() && self.ch() != '\n' {
                            self.pos += 1;
                        }
                        if self.pos < self.input.len() {
                            self.pos += 1; // skip newline
                        }
                        return;
                    }
                }
            } else {
                self.pos += 1;
            }
        }
    }

    pub fn tokenize(&mut self) -> Vec<Token> {
        let mut tokens = Vec::new();

        loop {
            self.skip_whitespace_and_comments();

            if self.pos >= self.input.len() {
                tokens.push(Token::EOF);
                break;
            }

            let c = self.ch();

            // Check for POD at start of line
            if c == '='
                && (tokens.is_empty()
                    || matches!(tokens.last(), Some(Token::Newline) | Some(Token::Semi)))
            {
                let rest: String = self.input[self.pos..].iter().take(5).collect();
                if rest.starts_with("=pod")
                    || rest.starts_with("=head")
                    || rest.starts_with("=over")
                    || rest.starts_with("=item")
                    || rest.starts_with("=begi")
                    || rest.starts_with("=for")
                    || rest.starts_with("=cut")
                    || rest.starts_with("=enc")
                {
                    self.skip_pod();
                    continue;
                }
            }

            match c {
                '\n' => {
                    self.pos += 1;
                    // Check for POD after newline
                    if self.pos < self.input.len() && self.ch() == '=' {
                        let rest: String = self.input[self.pos..].iter().take(5).collect();
                        if rest.starts_with("=pod")
                            || rest.starts_with("=head")
                            || rest.starts_with("=over")
                            || rest.starts_with("=item")
                            || rest.starts_with("=begi")
                            || rest.starts_with("=for")
                            || rest.starts_with("=enc")
                        {
                            self.skip_pod();
                            continue;
                        }
                    }
                    // Collapse multiple newlines, don't emit if last token handles it
                    if !matches!(
                        tokens.last(),
                        Some(Token::Newline) | Some(Token::Semi) | Some(Token::LBrace) | None
                    ) {
                        tokens.push(Token::Newline);
                    }
                }

                '$' => {
                    self.pos += 1;
                    if self.ch() == '#' {
                        // $#array or $#{ ... }
                        self.pos += 1;
                        let name = self.read_ident();
                        tokens.push(Token::ArrayLen(name));
                    } else if self.ch() == '{' {
                        // ${expr} or ${^NAME}
                        self.pos += 1;
                        if self.ch() == '^' {
                            self.pos += 1;
                            let name = self.read_ident();
                            // Skip closing brace
                            if self.ch() == '}' {
                                self.pos += 1;
                            }
                            tokens.push(Token::ScalarVar(format!("^{name}")));
                        } else {
                            let name = self.read_ident();
                            if self.ch() == '}' {
                                self.pos += 1;
                            }
                            tokens.push(Token::ScalarVar(name));
                        }
                    } else if self.ch() == '^' {
                        // $^X style special variable
                        self.pos += 1;
                        let c = self.advance();
                        tokens.push(Token::ScalarVar(format!("^{c}")));
                    } else if self.ch() == '_' || self.ch().is_ascii_alphabetic() {
                        let name = self.read_ident();
                        // Check for $name::name
                        while self.ch() == ':' && self.peek(1) == ':' {
                            let mut full = name.clone();
                            full.push_str("::");
                            self.pos += 2;
                            let next = self.read_ident();
                            full.push_str(&next);
                            tokens.push(Token::ScalarVar(full));
                            continue;
                        }
                        tokens.push(Token::ScalarVar(name));
                    } else if self.ch() == '0' {
                        self.pos += 1;
                        tokens.push(Token::ScalarVar("0".to_string()));
                    } else if self.ch() == '/' {
                        self.pos += 1;
                        tokens.push(Token::ScalarVar("/".to_string()));
                    } else if self.ch() == '\\' {
                        self.pos += 1;
                        tokens.push(Token::ScalarVar("\\".to_string()));
                    } else if self.ch() == ',' {
                        self.pos += 1;
                        tokens.push(Token::ScalarVar(",".to_string()));
                    } else if self.ch() == '@' {
                        self.pos += 1;
                        tokens.push(Token::ScalarVar("@".to_string()));
                    } else if self.ch() == '_' {
                        self.pos += 1;
                        tokens.push(Token::ScalarVar("_".to_string()));
                    } else if self.ch() == '!' {
                        self.pos += 1;
                        tokens.push(Token::ScalarVar("!".to_string()));
                    } else if self.ch() == '"' {
                        self.pos += 1;
                        tokens.push(Token::ScalarVar("\"".to_string()));
                    } else if self.ch() == ';' {
                        self.pos += 1;
                        tokens.push(Token::ScalarVar(";".to_string()));
                    } else if self.ch() == '|' {
                        self.pos += 1;
                        tokens.push(Token::ScalarVar("|".to_string()));
                    } else {
                        // Unknown special var, just treat as $_
                        tokens.push(Token::ScalarVar("_".to_string()));
                    }
                }

                '@' => {
                    self.pos += 1;
                    if self.ch() == '_' || self.ch().is_ascii_alphabetic() {
                        let name = self.read_ident();
                        while self.ch() == ':' && self.peek(1) == ':' {
                            // @Pkg::name not needed yet, skip for simplicity
                            break;
                        }
                        tokens.push(Token::ArrayVar(name));
                    } else if self.ch() == '{' {
                        self.pos += 1;
                        if self.ch() == '^' {
                            self.pos += 1;
                            let name = self.read_ident();
                            if self.ch() == '}' {
                                self.pos += 1;
                            }
                            tokens.push(Token::ArrayVar(format!("^{name}")));
                        } else {
                            let name = self.read_ident();
                            if self.ch() == '}' {
                                self.pos += 1;
                            }
                            tokens.push(Token::ArrayVar(name));
                        }
                    } else {
                        tokens.push(Token::ArrayVar(String::new()));
                    }
                }

                '%' => {
                    self.pos += 1;
                    if self.ch() == '=' {
                        self.pos += 1;
                        tokens.push(Token::PercentAssign);
                    } else if self.ch() == '{'
                        || self.ch() == '_'
                        || self.ch().is_ascii_alphabetic()
                    {
                        // Could be hash variable or modulo
                        // Check context: if last token expects operand, it's a hash var
                        let is_hash = tokens.last().map(|t| t.expects_operand()).unwrap_or(true);
                        if is_hash && (self.ch() == '_' || self.ch().is_ascii_alphabetic()) {
                            let name = self.read_ident();
                            tokens.push(Token::HashVar(name));
                        } else {
                            tokens.push(Token::Percent);
                        }
                    } else {
                        tokens.push(Token::Percent);
                    }
                }

                '\'' => {
                    self.pos += 1;
                    let s = self.read_single_quoted_string();
                    tokens.push(Token::StringLit(s));
                }

                '"' => {
                    self.pos += 1;
                    let s = self.read_double_quoted_string('"');
                    // Check if string needs interpolation
                    if s.contains('$') || s.contains('@') {
                        tokens.push(Token::InterpString(s));
                    } else {
                        tokens.push(Token::StringLit(s));
                    }
                }

                '0'..='9' => {
                    tokens.push(self.read_number());
                }

                'a'..='z' | 'A'..='Z' | '_' => {
                    let ident = self.read_ident();

                    // Check for => (fat comma) - the ident is auto-quoted
                    self.skip_whitespace_and_comments();
                    if self.ch() == '=' && self.peek(1) == '>' {
                        self.pos += 2;
                        tokens.push(Token::StringLit(ident));
                        tokens.push(Token::FatComma);
                        continue;
                    }

                    // Check for q//, qq//, qw//, qr// quoting operators
                    match ident.as_str() {
                        "q" if !self.ch().is_alphanumeric() && self.ch() != '_' => {
                            let s = self.read_q_string();
                            tokens.push(Token::StringLit(s));
                            continue;
                        }
                        "qq" if !self.ch().is_alphanumeric() && self.ch() != '_' => {
                            let s = self.read_qq_string();
                            tokens.push(Token::StringLit(s));
                            continue;
                        }
                        "qw" if !self.ch().is_alphanumeric() && self.ch() != '_' => {
                            let words = self.read_qw();
                            tokens.push(Token::QW(words));
                            continue;
                        }
                        "qr" if !self.ch().is_alphanumeric() && self.ch() != '_' => {
                            let (pat, flags) = self.read_qr();
                            tokens.push(Token::RegexLit(pat, flags));
                            continue;
                        }
                        "s" if !self.ch().is_alphanumeric() && self.ch() != '_' => {
                            let (pat, repl, flags) = self.read_substitution();
                            tokens.push(Token::Substitution(pat, repl, flags));
                            continue;
                        }
                        "tr" | "y" if !self.ch().is_alphanumeric() && self.ch() != '_' => {
                            let (from, to, flags) = self.read_transliterate();
                            tokens.push(Token::Transliterate(from, to, flags));
                            continue;
                        }
                        _ => {}
                    }

                    // Map keywords
                    let tok = match ident.as_str() {
                        "if" => Token::If,
                        "else" => Token::Else,
                        "elsif" => Token::Elsif,
                        "unless" => Token::Unless,
                        "while" => Token::While,
                        "until" => Token::Until,
                        "for" => Token::For,
                        "foreach" => Token::Foreach,
                        "my" => Token::My,
                        "our" => Token::Our,
                        "local" => Token::Local,
                        "sub" => Token::Sub,
                        "return" => Token::Return,
                        "last" => Token::Last,
                        "next" => Token::Next,
                        "redo" => Token::Redo,
                        "print" => Token::Print,
                        "say" => Token::Say,
                        "die" => Token::Die,
                        "warn" => Token::Warn,
                        "BEGIN" => Token::Begin,
                        "END" => Token::End,
                        "use" => Token::Use,
                        "require" => Token::Require,
                        "package" => Token::Package,
                        "do" => Token::Do,
                        "eval" => Token::Eval,
                        "undef" => Token::UndefKw,
                        "defined" => Token::Defined,
                        "not" => Token::Not,
                        "and" => Token::And,
                        "or" => Token::Or,
                        "eq" => Token::Eq,
                        "ne" => Token::Ne,
                        "lt" => Token::Lt,
                        "gt" => Token::Gt,
                        "le" => Token::Le,
                        "ge" => Token::Ge,
                        "cmp" => Token::Cmp,
                        "chomp" => Token::Chomp,
                        "chop" => Token::Chop,
                        "push" => Token::Push,
                        "pop" => Token::Pop,
                        "shift" => Token::Shift,
                        "unshift" => Token::Unshift,
                        "splice" => Token::Splice,
                        "delete" => Token::Delete,
                        "exists" => Token::Exists,
                        "keys" => Token::Keys,
                        "values" => Token::Values,
                        "each" => Token::Each,
                        "reverse" => Token::Reverse,
                        "sort" => Token::Sort,
                        "join" => Token::Join,
                        "split" => Token::Split,
                        "grep" => Token::Grep,
                        "map" => Token::Map,
                        "abs" => Token::Abs,
                        "int" => Token::Int,
                        "length" => Token::Length,
                        "substr" => Token::Substr,
                        "index" => Token::Index,
                        "rindex" => Token::Rindex,
                        "sprintf" => Token::Sprintf,
                        "printf" => Token::Printf,
                        "chr" => Token::Chr,
                        "ord" => Token::Ord,
                        "lc" => Token::Lc,
                        "uc" => Token::Uc,
                        "lcfirst" => Token::Lcfirst,
                        "ucfirst" => Token::Ucfirst,
                        "hex" => Token::Hex,
                        "oct" => Token::Oct,
                        "ref" => Token::Ref,
                        "wantarray" => Token::Wantarray,
                        "caller" => Token::Caller,
                        "open" => Token::Open,
                        "close" => Token::Close,
                        "read" => Token::Read,
                        "eof" => Token::Eof,
                        "binmode" => Token::Binmode,
                        "unlink" => Token::Unlink,
                        "rename" => Token::Rename,
                        "mkdir" => Token::Mkdir,
                        "rmdir" => Token::Rmdir,
                        "chdir" => Token::Chdir,
                        "stat" => Token::Stat,
                        "x" => {
                            // 'x' is the string repeat operator, but only in operator context
                            if !tokens.last().map(|t| t.expects_operand()).unwrap_or(true) {
                                Token::StringRepeat
                            } else {
                                Token::Ident("x".to_string())
                            }
                        }
                        _ => Token::Ident(ident),
                    };
                    tokens.push(tok);
                }

                '(' => {
                    self.pos += 1;
                    tokens.push(Token::LParen);
                }
                ')' => {
                    self.pos += 1;
                    tokens.push(Token::RParen);
                }
                '[' => {
                    self.pos += 1;
                    tokens.push(Token::LBracket);
                }
                ']' => {
                    self.pos += 1;
                    tokens.push(Token::RBracket);
                }
                '{' => {
                    self.pos += 1;
                    tokens.push(Token::LBrace);
                }
                '}' => {
                    self.pos += 1;
                    tokens.push(Token::RBrace);
                }
                ';' => {
                    self.pos += 1;
                    tokens.push(Token::Semi);
                }
                ',' => {
                    self.pos += 1;
                    tokens.push(Token::Comma);
                }
                '?' => {
                    self.pos += 1;
                    tokens.push(Token::Question);
                }
                ':' => {
                    self.pos += 1;
                    if self.ch() == ':' {
                        self.pos += 1;
                        // :: is package separator, but we handle it in ident reading
                        tokens.push(Token::Ident("::".to_string()));
                    } else {
                        tokens.push(Token::Colon);
                    }
                }
                '\\' => {
                    self.pos += 1;
                    tokens.push(Token::Backslash);
                }

                '+' => {
                    self.pos += 1;
                    if self.ch() == '+' {
                        self.pos += 1;
                        tokens.push(Token::PlusPlus);
                    } else if self.ch() == '=' {
                        self.pos += 1;
                        tokens.push(Token::PlusAssign);
                    } else {
                        tokens.push(Token::Plus);
                    }
                }
                '-' => {
                    self.pos += 1;
                    if self.ch() == '-' {
                        self.pos += 1;
                        tokens.push(Token::MinusMinus);
                    } else if self.ch() == '=' {
                        self.pos += 1;
                        tokens.push(Token::MinusAssign);
                    } else if self.ch() == '>' {
                        self.pos += 1;
                        tokens.push(Token::Arrow);
                    } else if self.ch().is_ascii_alphabetic()
                        && tokens.last().map(|t| t.expects_operand()).unwrap_or(true)
                    {
                        // File test operator like -d, -f, -e, etc.
                        let op = self.advance();
                        tokens.push(Token::Ident(format!("-{op}")));
                    } else {
                        tokens.push(Token::Minus);
                    }
                }
                '*' => {
                    self.pos += 1;
                    if self.ch() == '*' {
                        self.pos += 1;
                        if self.ch() == '=' {
                            self.pos += 1;
                            tokens.push(Token::PowerAssign);
                        } else {
                            tokens.push(Token::Power);
                        }
                    } else if self.ch() == '=' {
                        self.pos += 1;
                        tokens.push(Token::StarAssign);
                    } else if (self.ch() == '_' || self.ch().is_ascii_alphabetic())
                        && tokens.last().map(|t| t.expects_operand()).unwrap_or(true)
                    {
                        // Glob like *FH
                        let name = self.read_ident();
                        tokens.push(Token::Ident(format!("*{name}")));
                    } else {
                        tokens.push(Token::Star);
                    }
                }

                '/' => {
                    // Division or regex?
                    let is_regex = tokens.last().map(|t| t.expects_operand()).unwrap_or(true);
                    if is_regex {
                        self.pos += 1;
                        let (pat, flags) = self.read_regex('/');
                        tokens.push(Token::RegexLit(pat, flags));
                    } else {
                        self.pos += 1;
                        if self.ch() == '=' {
                            self.pos += 1;
                            tokens.push(Token::SlashAssign);
                        } else if self.ch() == '/' {
                            self.pos += 1;
                            if self.ch() == '=' {
                                self.pos += 1;
                                tokens.push(Token::DefOrAssign);
                            } else {
                                tokens.push(Token::DefOr);
                            }
                        } else {
                            tokens.push(Token::Slash);
                        }
                    }
                }

                '.' => {
                    self.pos += 1;
                    if self.ch() == '.' {
                        self.pos += 1;
                        if self.ch() == '.' {
                            self.pos += 1;
                            tokens.push(Token::Ident("...".to_string()));
                        } else {
                            tokens.push(Token::DotDot);
                        }
                    } else if self.ch() == '=' {
                        self.pos += 1;
                        tokens.push(Token::DotAssign);
                    } else if self.ch().is_ascii_digit() {
                        // .5 style number
                        self.pos -= 1; // back up to include the dot
                        tokens.push(self.read_number());
                    } else {
                        tokens.push(Token::Dot);
                    }
                }

                '=' => {
                    self.pos += 1;
                    if self.ch() == '=' {
                        self.pos += 1;
                        tokens.push(Token::NumEq);
                    } else if self.ch() == '~' {
                        self.pos += 1;
                        tokens.push(Token::Match);
                    } else if self.ch() == '>' {
                        self.pos += 1;
                        tokens.push(Token::FatComma);
                    } else {
                        tokens.push(Token::Assign);
                    }
                }

                '!' => {
                    self.pos += 1;
                    if self.ch() == '=' {
                        self.pos += 1;
                        tokens.push(Token::NumNe);
                    } else if self.ch() == '~' {
                        self.pos += 1;
                        tokens.push(Token::NotMatch);
                    } else {
                        tokens.push(Token::LogNot);
                    }
                }

                '<' => {
                    self.pos += 1;
                    if self.ch() == '=' {
                        self.pos += 1;
                        if self.ch() == '>' {
                            self.pos += 1;
                            tokens.push(Token::Spaceship);
                        } else {
                            tokens.push(Token::NumLe);
                        }
                    } else if self.ch() == '<' {
                        self.pos += 1;
                        if self.ch() == '=' {
                            self.pos += 1;
                            tokens.push(Token::ShiftLeftAssign);
                        } else {
                            // Could be heredoc <<EOF or shift <<
                            let last_expects =
                                tokens.last().map(|t| t.expects_operand()).unwrap_or(true);
                            if last_expects {
                                // Heredoc
                                let s = self.read_heredoc();
                                tokens.push(Token::StringLit(s));
                            } else {
                                tokens.push(Token::ShiftLeft);
                            }
                        }
                    } else if self.ch() == '>'
                        || self.ch() == '$'
                        || self.ch() == '_'
                        || self.ch().is_ascii_alphabetic()
                    {
                        // Diamond operator <FH> or <>
                        let mut name = String::new();
                        while self.ch() != '>' && self.pos < self.input.len() {
                            name.push(self.advance());
                        }
                        if self.ch() == '>' {
                            self.pos += 1;
                        }
                        tokens.push(Token::Diamond(name));
                    } else {
                        tokens.push(Token::NumLt);
                    }
                }

                '>' => {
                    self.pos += 1;
                    if self.ch() == '=' {
                        self.pos += 1;
                        tokens.push(Token::NumGe);
                    } else if self.ch() == '>' {
                        self.pos += 1;
                        if self.ch() == '=' {
                            self.pos += 1;
                            tokens.push(Token::ShiftRightAssign);
                        } else {
                            tokens.push(Token::ShiftRight);
                        }
                    } else {
                        tokens.push(Token::NumGt);
                    }
                }

                '&' => {
                    self.pos += 1;
                    if self.ch() == '&' {
                        self.pos += 1;
                        if self.ch() == '=' {
                            self.pos += 1;
                            tokens.push(Token::LogAndAssign);
                        } else {
                            tokens.push(Token::LogAnd);
                        }
                    } else if self.ch() == '=' {
                        self.pos += 1;
                        tokens.push(Token::BitAndAssign);
                    } else {
                        tokens.push(Token::BitAnd);
                    }
                }

                '|' => {
                    self.pos += 1;
                    if self.ch() == '|' {
                        self.pos += 1;
                        if self.ch() == '=' {
                            self.pos += 1;
                            tokens.push(Token::LogOrAssign);
                        } else {
                            tokens.push(Token::LogOr);
                        }
                    } else if self.ch() == '=' {
                        self.pos += 1;
                        tokens.push(Token::BitOrAssign);
                    } else {
                        tokens.push(Token::BitOr);
                    }
                }

                '^' => {
                    self.pos += 1;
                    if self.ch() == '=' {
                        self.pos += 1;
                        tokens.push(Token::BitXorAssign);
                    } else {
                        tokens.push(Token::BitXor);
                    }
                }

                '~' => {
                    self.pos += 1;
                    tokens.push(Token::BitNot);
                }

                '`' => {
                    self.pos += 1;
                    let s = self.read_double_quoted_string('`');
                    tokens.push(Token::Ident("backtick".to_string()));
                    if s.contains('$') || s.contains('@') {
                        tokens.push(Token::InterpString(s));
                    } else {
                        tokens.push(Token::StringLit(s));
                    }
                }

                _ => {
                    // Skip unknown characters
                    self.pos += 1;
                }
            }
        }

        // Filter out newlines (they're not significant in our grammar)
        tokens.retain(|t| !matches!(t, Token::Newline));

        self.tokens = tokens.clone();
        tokens
    }

    fn read_ident(&mut self) -> String {
        let mut s = String::new();
        while self.pos < self.input.len() && (self.ch().is_ascii_alphanumeric() || self.ch() == '_')
        {
            s.push(self.advance());
        }
        // Handle :: package separator
        while self.ch() == ':' && self.peek(1) == ':' {
            s.push_str("::");
            self.pos += 2;
            while self.pos < self.input.len()
                && (self.ch().is_ascii_alphanumeric() || self.ch() == '_')
            {
                s.push(self.advance());
            }
        }
        s
    }

    fn read_number(&mut self) -> Token {
        let start = self.pos;
        let mut is_float = false;

        // Check for 0x, 0b, 0o prefixes
        if self.ch() == '0' && self.pos + 1 < self.input.len() {
            match self.peek(1) {
                'x' | 'X' => {
                    self.pos += 2;
                    let hex_start = self.pos;
                    while self.pos < self.input.len()
                        && (self.ch().is_ascii_hexdigit() || self.ch() == '_')
                    {
                        self.pos += 1;
                    }
                    let s: String = self.input[hex_start..self.pos]
                        .iter()
                        .filter(|c| **c != '_')
                        .collect();
                    let v = i64::from_str_radix(&s, 16).unwrap_or(0);
                    return Token::Integer(v);
                }
                'b' | 'B' => {
                    self.pos += 2;
                    let bin_start = self.pos;
                    while self.pos < self.input.len()
                        && (self.ch() == '0' || self.ch() == '1' || self.ch() == '_')
                    {
                        self.pos += 1;
                    }
                    let s: String = self.input[bin_start..self.pos]
                        .iter()
                        .filter(|c| **c != '_')
                        .collect();
                    let v = i64::from_str_radix(&s, 2).unwrap_or(0);
                    return Token::Integer(v);
                }
                'o' | 'O' => {
                    self.pos += 2;
                    let oct_start = self.pos;
                    while self.pos < self.input.len()
                        && ((self.ch() >= '0' && self.ch() <= '7') || self.ch() == '_')
                    {
                        self.pos += 1;
                    }
                    let s: String = self.input[oct_start..self.pos]
                        .iter()
                        .filter(|c| **c != '_')
                        .collect();
                    let v = i64::from_str_radix(&s, 8).unwrap_or(0);
                    return Token::Integer(v);
                }
                '0'..='7' => {
                    // Octal without 'o' prefix: 0777
                    self.pos += 1;
                    let oct_start = self.pos - 1;
                    while self.pos < self.input.len()
                        && ((self.ch() >= '0' && self.ch() <= '7') || self.ch() == '_')
                    {
                        self.pos += 1;
                    }
                    // Check it's not actually a float like 0.1
                    if self.ch() == '.' || self.ch() == 'e' || self.ch() == 'E' {
                        // It's a float, re-parse
                        self.pos = start;
                    } else {
                        let s: String = self.input[oct_start..self.pos]
                            .iter()
                            .filter(|c| **c != '_')
                            .collect();
                        let v = i64::from_str_radix(&s, 8).unwrap_or(0);
                        return Token::Integer(v);
                    }
                }
                _ => {}
            }
        }

        // Regular decimal number
        // Integer part
        while self.pos < self.input.len() && (self.ch().is_ascii_digit() || self.ch() == '_') {
            self.pos += 1;
        }

        // Decimal point
        if self.ch() == '.' && self.peek(1) != '.' {
            is_float = true;
            self.pos += 1;
            while self.pos < self.input.len() && (self.ch().is_ascii_digit() || self.ch() == '_') {
                self.pos += 1;
            }
        }

        // Exponent
        if self.ch() == 'e' || self.ch() == 'E' {
            is_float = true;
            self.pos += 1;
            if self.ch() == '+' || self.ch() == '-' {
                self.pos += 1;
            }
            while self.pos < self.input.len() && self.ch().is_ascii_digit() {
                self.pos += 1;
            }
        }

        let s: String = self.input[start..self.pos]
            .iter()
            .filter(|c| **c != '_')
            .collect();

        if is_float {
            Token::Float(s.parse::<f64>().unwrap_or(0.0))
        } else {
            match s.parse::<i64>() {
                Ok(v) => Token::Integer(v),
                Err(_) => Token::Float(s.parse::<f64>().unwrap_or(0.0)),
            }
        }
    }

    fn read_single_quoted_string(&mut self) -> String {
        let mut s = String::new();
        while self.pos < self.input.len() && self.ch() != '\'' {
            if self.ch() == '\\' {
                self.pos += 1;
                match self.ch() {
                    '\'' => {
                        s.push('\'');
                        self.pos += 1;
                    }
                    '\\' => {
                        s.push('\\');
                        self.pos += 1;
                    }
                    _ => {
                        s.push('\\');
                        // Don't consume the next char — it's literal
                    }
                }
            } else {
                s.push(self.advance());
            }
        }
        if self.ch() == '\'' {
            self.pos += 1;
        }
        s
    }

    fn read_double_quoted_string(&mut self, delim: char) -> String {
        let mut s = String::new();
        let mut has_interp = false;
        while self.pos < self.input.len() && self.ch() != delim {
            if self.ch() == '\\' {
                self.pos += 1;
                match self.ch() {
                    'n' => {
                        s.push('\n');
                        self.pos += 1;
                    }
                    't' => {
                        s.push('\t');
                        self.pos += 1;
                    }
                    'r' => {
                        s.push('\r');
                        self.pos += 1;
                    }
                    '\\' => {
                        s.push('\\');
                        self.pos += 1;
                    }
                    '"' => {
                        s.push('"');
                        self.pos += 1;
                    }
                    '$' => {
                        // Escaped $ — use a placeholder that won't conflict
                        s.push('\x01'); // placeholder for literal $
                        self.pos += 1;
                    }
                    '@' => {
                        s.push('\x02'); // placeholder for literal @
                        self.pos += 1;
                    }
                    '0' => {
                        self.pos += 1;
                        if self.ch().is_ascii_digit() {
                            let mut oct = String::new();
                            oct.push('0');
                            while self.pos < self.input.len()
                                && self.ch().is_ascii_digit()
                                && oct.len() < 3
                            {
                                oct.push(self.advance());
                            }
                            let v = u8::from_str_radix(&oct, 8).unwrap_or(0);
                            s.push(v as char);
                        } else {
                            s.push('\0');
                        }
                    }
                    'x' => {
                        self.pos += 1;
                        let mut hex = String::new();
                        if self.ch() == '{' {
                            self.pos += 1;
                            while self.pos < self.input.len() && self.ch() != '}' {
                                hex.push(self.advance());
                            }
                            if self.ch() == '}' {
                                self.pos += 1;
                            }
                        } else {
                            for _ in 0..2 {
                                if self.pos < self.input.len() && self.ch().is_ascii_hexdigit() {
                                    hex.push(self.advance());
                                }
                            }
                        }
                        let v = u32::from_str_radix(&hex, 16).unwrap_or(0);
                        if let Some(c) = char::from_u32(v) {
                            s.push(c);
                        }
                    }
                    'a' => {
                        s.push('\x07');
                        self.pos += 1;
                    }
                    'b' => {
                        s.push('\x08');
                        self.pos += 1;
                    }
                    'f' => {
                        s.push('\x0C');
                        self.pos += 1;
                    }
                    'e' => {
                        s.push('\x1B');
                        self.pos += 1;
                    }
                    c if c == delim => {
                        s.push(c);
                        self.pos += 1;
                    }
                    _ => {
                        s.push('\\');
                        s.push(self.ch());
                        self.pos += 1;
                    }
                }
            } else if self.ch() == '$' || self.ch() == '@' {
                has_interp = true;
                s.push(self.advance());
            } else {
                s.push(self.advance());
            }
        }
        if self.pos < self.input.len() && self.ch() == delim {
            self.pos += 1;
        }
        // After processing, replace placeholders back
        // \x01 → $, \x02 → @  (these were escaped in the source)
        if !has_interp {
            s = s.replace('\x01', "$").replace('\x02', "@");
        }
        s
    }

    fn find_matching_delim(open: char) -> char {
        match open {
            '(' => ')',
            '[' => ']',
            '{' => '}',
            '<' => '>',
            _ => open,
        }
    }

    fn read_delimited_string(&mut self) -> (char, char, String) {
        // Skip whitespace/comments before delimiter
        self.skip_whitespace_and_comments();
        while self.ch() == '\n' {
            self.pos += 1;
            self.skip_whitespace_and_comments();
        }

        let open = self.advance();
        let close = Self::find_matching_delim(open);
        let is_paired = open != close;
        let mut s = String::new();
        let mut depth = 1;

        while self.pos < self.input.len() {
            if is_paired && self.ch() == open {
                depth += 1;
                s.push(self.advance());
            } else if self.ch() == close {
                depth -= 1;
                if depth == 0 {
                    self.pos += 1;
                    break;
                }
                s.push(self.advance());
            } else if self.ch() == '\\' && !is_paired {
                self.pos += 1;
                if self.ch() == close {
                    s.push(self.advance());
                } else {
                    s.push('\\');
                }
            } else {
                s.push(self.advance());
            }
        }
        (open, close, s)
    }

    fn read_q_string(&mut self) -> String {
        let (_, _, s) = self.read_delimited_string();
        // q// is like single quotes — minimal escaping
        s
    }

    fn read_qq_string(&mut self) -> String {
        let (_, _, s) = self.read_delimited_string();
        // qq// is like double quotes — process escape sequences
        process_escapes(&s)
    }

    fn read_qw(&mut self) -> Vec<String> {
        let (_, _, s) = self.read_delimited_string();
        s.split_whitespace().map(|w| w.to_string()).collect()
    }

    fn read_qr(&mut self) -> (String, String) {
        let (_, _, pat) = self.read_delimited_string();
        let flags = self.read_regex_flags();
        (pat, flags)
    }

    fn read_substitution(&mut self) -> (String, String, String) {
        // s/pattern/replacement/flags
        // The delimiter can be any non-alphanumeric character
        let open = self.advance();
        let close = Self::find_matching_delim(open);
        let is_paired = open != close;

        // Read pattern
        let mut pat = String::new();
        let mut depth = 1;
        while self.pos < self.input.len() {
            if is_paired && self.ch() == open {
                depth += 1;
                pat.push(self.advance());
            } else if self.ch() == close {
                depth -= 1;
                if depth == 0 {
                    self.pos += 1; // skip closing delimiter
                    break;
                }
                pat.push(self.advance());
            } else if self.ch() == '\\' {
                pat.push(self.advance());
                if self.pos < self.input.len() {
                    pat.push(self.advance());
                }
            } else {
                pat.push(self.advance());
            }
        }

        // For paired delimiters like s{pat}{repl}, skip whitespace before second part
        if is_paired {
            self.skip_whitespace_and_comments();
            while self.ch() == '\n' {
                self.pos += 1;
                self.skip_whitespace_and_comments();
            }
        }

        // Read replacement
        let repl_open = if is_paired { self.advance() } else { open };
        let repl_close = Self::find_matching_delim(repl_open);
        let repl_is_paired = repl_open != repl_close;

        let mut repl = String::new();
        let mut depth = 1;
        while self.pos < self.input.len() {
            if repl_is_paired && self.ch() == repl_open {
                depth += 1;
                repl.push(self.advance());
            } else if self.ch() == repl_close {
                depth -= 1;
                if depth == 0 {
                    self.pos += 1;
                    break;
                }
                repl.push(self.advance());
            } else if self.ch() == '\\' {
                repl.push(self.advance());
                if self.pos < self.input.len() {
                    repl.push(self.advance());
                }
            } else {
                repl.push(self.advance());
            }
        }

        let flags = self.read_regex_flags();
        (pat, repl, flags)
    }

    fn read_transliterate(&mut self) -> (String, String, String) {
        // tr/from/to/flags or y/from/to/flags
        // Reuse the same logic as substitution
        self.read_substitution()
    }

    fn read_regex(&mut self, delim: char) -> (String, String) {
        let mut pat = String::new();
        while self.pos < self.input.len() && self.ch() != delim {
            if self.ch() == '\\' {
                pat.push(self.advance());
                if self.pos < self.input.len() {
                    pat.push(self.advance());
                }
            } else {
                pat.push(self.advance());
            }
        }
        if self.pos < self.input.len() {
            self.pos += 1; // skip closing delimiter
        }
        let flags = self.read_regex_flags();
        (pat, flags)
    }

    fn read_regex_flags(&mut self) -> String {
        let mut flags = String::new();
        while self.pos < self.input.len()
            && matches!(self.ch(), 'g' | 'i' | 'm' | 's' | 'x' | 'e' | 'r' | 'n')
        {
            flags.push(self.advance());
        }
        flags
    }

    fn read_heredoc(&mut self) -> String {
        // We're after <<
        // Can be <<EOF, <<'EOF', <<"EOF", <<~EOF, <<~'EOF', <<~"EOF"
        let mut indent = false;
        let mut interpolate = true;

        // Skip whitespace
        while self.ch() == ' ' || self.ch() == '\t' {
            self.pos += 1;
        }

        if self.ch() == '~' {
            indent = true;
            self.pos += 1;
        }

        // Skip \
        if self.ch() == '\\' {
            self.pos += 1;
            interpolate = false;
        }

        let quote = if self.ch() == '\'' || self.ch() == '"' {
            let q = self.ch();
            if q == '\'' {
                interpolate = false;
            }
            self.pos += 1;
            Some(q)
        } else {
            None
        };

        // Read the tag
        let mut tag = String::new();
        while self.pos < self.input.len()
            && self.ch() != '\n'
            && self.ch() != ';'
            && self.ch() != ','
            && Some(self.ch()) != quote.map(|_| quote.unwrap())
        {
            if quote.is_some() && self.ch() == quote.unwrap() {
                break;
            }
            tag.push(self.advance());
        }

        // Skip closing quote
        if let Some(q) = quote {
            if self.ch() == q {
                self.pos += 1;
            }
        }

        // Skip to end of line (there may be more code on this line after the heredoc marker)
        // For simplicity, skip to newline
        while self.pos < self.input.len() && self.ch() != '\n' {
            self.pos += 1;
        }
        if self.ch() == '\n' {
            self.pos += 1;
        }

        // Read the body until we find the tag on its own line
        let mut body = String::new();
        loop {
            if self.pos >= self.input.len() {
                break;
            }

            // Check if this line is the terminator
            let line_start = self.pos;
            let mut line = String::new();
            while self.pos < self.input.len() && self.ch() != '\n' {
                line.push(self.advance());
            }
            if self.pos < self.input.len() {
                self.pos += 1; // skip newline
            }

            let trimmed = if indent {
                line.trim().to_string()
            } else {
                line.clone()
            };
            if trimmed == tag {
                break;
            }

            body.push_str(&line);
            body.push('\n');
        }

        if indent {
            // Remove common leading whitespace
            // TODO: implement proper indented heredoc
        }

        if interpolate {
            process_escapes(&body)
        } else {
            body
        }
    }
}

fn process_escapes(s: &str) -> String {
    let mut result = String::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() {
            i += 1;
            match chars[i] {
                'n' => result.push('\n'),
                't' => result.push('\t'),
                'r' => result.push('\r'),
                '\\' => result.push('\\'),
                '"' => result.push('"'),
                '$' => result.push('$'),
                '@' => result.push('@'),
                '0' => result.push('\0'),
                'a' => result.push('\x07'),
                'b' => result.push('\x08'),
                'f' => result.push('\x0C'),
                'e' => result.push('\x1B'),
                'x' => {
                    i += 1;
                    let mut hex = String::new();
                    if i < chars.len() && chars[i] == '{' {
                        i += 1;
                        while i < chars.len() && chars[i] != '}' {
                            hex.push(chars[i]);
                            i += 1;
                        }
                        // skip }
                    } else {
                        for _ in 0..2 {
                            if i < chars.len() && chars[i].is_ascii_hexdigit() {
                                hex.push(chars[i]);
                                i += 1;
                            }
                        }
                    }
                    let v = u32::from_str_radix(&hex, 16).unwrap_or(0);
                    if let Some(c) = char::from_u32(v) {
                        result.push(c);
                    }
                    continue;
                }
                _ => {
                    result.push('\\');
                    result.push(chars[i]);
                }
            }
        } else {
            result.push(chars[i]);
        }
        i += 1;
    }
    result
}
