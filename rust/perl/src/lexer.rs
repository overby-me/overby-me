use std::collections::HashSet;

#[derive(Clone, Debug)]
pub enum Token {
    // Literals
    Integer(i64),
    Float(f64),
    StringLit(String),
    RegexLit(String, String), // pattern, flags (from m// or bare /pat/)
    QrLit(String, String),    // pattern, flags (from qr// — produces a regex *value*)
    QW(Vec<String>),
    InterpString(String), // double-quoted string needing variable interpolation

    // Variables
    ScalarVar(String),   // $name
    ArrayVar(String),    // @name
    HashVar(String),     // %name
    ArrayDeref(String),  // @$name or @{$name} — dereference a scalar ref as array
    HashDeref(String),   // %$name or %{$name} — dereference a scalar ref as hash
    ScalarDeref(String), // $$name or ${$name} — dereference a scalar ref as scalar
    Glob(String),        // *NAME — typeglob reference into the symbol table
    ArrayLen(String),    // $#name

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
    Goto,
    Continue,
    Print,
    Say,
    Die,
    Warn,
    Begin,
    End,
    Check,
    Init,
    Bless,
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
    Xor,
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
    Tell,
    Binmode,
    Unlink,
    Rename,
    Mkdir,
    Rmdir,
    Chdir,
    Stat,

    // Operators
    Assign,             // =
    Plus,               // +
    Minus,              // -
    Star,               // *
    Slash,              // /
    Percent,            // %
    Power,              // **
    Dot,                // .
    DotDot,             // ..
    Eq,                 // eq
    Ne,                 // ne
    Lt,                 // lt
    Gt,                 // gt
    Le,                 // le
    Ge,                 // ge
    NumEq,              // ==
    NumNe,              // !=
    NumLt,              // <
    NumGt,              // >
    NumLe,              // <=
    NumGe,              // >=
    Spaceship,          // <=>
    Cmp,                // cmp
    LogAnd,             // &&
    LogOr,              // ||
    LogNot,             // !
    DefOr,              // //
    BitAnd,             // &
    BitOr,              // |
    BitXor,             // ^
    BitNot,             // ~
    ShiftLeft,          // <<
    ShiftRight,         // >>
    PlusPlus,           // ++
    MinusMinus,         // --
    PlusAssign,         // +=
    MinusAssign,        // -=
    StarAssign,         // *=
    SlashAssign,        // /=
    PercentAssign,      // %=
    DotAssign,          // .=
    PowerAssign,        // **=
    LogAndAssign,       // &&=
    LogOrAssign,        // ||=
    DefOrAssign,        // //=
    BitAndAssign,       // &=
    BitOrAssign,        // |=
    BitXorAssign,       // ^=
    ShiftLeftAssign,    // <<=
    ShiftRightAssign,   // >>=
    Match,              // =~
    NotMatch,           // !~
    Arrow,              // ->
    FatComma,           // =>
    Question,           // ?
    Colon,              // :
    Comma,              // ,
    Semi,               // ;
    Backslash,          // \
    StringRepeat,       // x
    StringRepeatAssign, // x=
    /// `@{ EXPR }` — block-form array deref. Parser treats the next `{`
    /// as opening an expression block whose value is dereferenced as an
    /// array.
    ArrayBlockDerefOpen,
    /// `${ EXPR }` — block-form scalar deref. The inner expression is
    /// evaluated and its result is dereferenced as a scalar.
    ScalarBlockDerefOpen,
    /// `%{ EXPR }` — block-form hash deref. The inner expression is
    /// evaluated; its result (a hash ref) is dereferenced as a hash.
    HashBlockDerefOpen,

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
                | Token::Xor
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
    /// Parallel to `tokens`: the 1-based line number each token was lexed at.
    /// Populated by `tokenize()`. Used so `caller()` can report file:line.
    pub token_lines: Vec<usize>,
    /// 1-based current line. Newlines seen during lex advance it.
    current_line: usize,
    keywords: HashSet<&'static str>,
    /// Heredocs queued up by `<<TAG` markers whose body hasn't been scanned
    /// yet. When we hit end-of-line during tokenization we drain these in
    /// order, backfilling the placeholder StringLit tokens with their body.
    pending_heredocs: Vec<PendingHeredoc>,
    /// Lex-time fatal error, e.g. an unterminated heredoc. main.rs checks
    /// this after tokenize() and exits with reference perl's diagnostic.
    pub error: Option<String>,
    /// Records `# line N "FILE"` directives. Each entry is
    /// (token_idx, file). Drained by parser to emit Stmt::FileMark.
    pub file_overrides: Vec<(usize, String)>,
    /// Captured contents after a `__DATA__` token, ready to be exposed as
    /// the `DATA` filehandle. `None` if the source had no `__DATA__` (or
    /// only had `__END__`, which discards the trailing data).
    pub data_section: Option<String>,
    /// Updated by `tokenize()` before each `skip_whitespace_and_comments`
    /// call so `try_handle_line_directive` knows the index of the next token.
    cur_token_count: usize,
    /// Added to `current_line` after recompute to honour `# line N` directives.
    line_offset: isize,
    /// Monotonic counter for unique heredoc-marker placeholders inserted
    /// into Substitution token strings — see `try_register_heredoc_in_subst`.
    subst_marker_counter: usize,
}

/// Where a queued heredoc's body should be spliced in once it's read.
enum HeredocTarget {
    /// Replace the placeholder StringLit/InterpString token at this index.
    /// This is the standard case for `print <<EOF;` etc.
    Token(usize),
    /// Replace a unique marker substring inside a Substitution token's
    /// captured pattern or replacement string. Used when `<<TAG` appears
    /// inside `s/PAT/REPL/e` — the heredoc body lives on the next line(s)
    /// in the source, but we capture REPL as a raw string ahead of time
    /// and need to splice the body back in once it's read.
    SubstReplMarker {
        token_idx: usize,
        marker: String,
    },
    SubstPatMarker {
        token_idx: usize,
        marker: String,
    },
    /// Replace a unique marker substring inside an InterpString token's
    /// captured body. Used when `<<TAG` appears inside a `qq|${\<<TAG}|`
    /// — the qq body is captured on the same line but the heredoc body
    /// arrives on subsequent lines.
    InterpMarker {
        token_idx: usize,
        marker: String,
    },
}

struct PendingHeredoc {
    tag: String,
    indent: bool,
    interpolate: bool,
    target: HeredocTarget,
    /// Line number of the `<<TAG` opening marker. Reference perl reports
    /// unterminated heredocs at this line.
    start_line: usize,
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
            "goto",
            "continue",
            "print",
            "say",
            "die",
            "warn",
            "BEGIN",
            "END",
            "CHECK",
            "INIT",
            "bless",
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
            "tell",
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
            token_lines: Vec::new(),
            current_line: 1,
            keywords,
            pending_heredocs: Vec::new(),
            error: None,
            file_overrides: Vec::new(),
            data_section: None,
            cur_token_count: 0,
            line_offset: 0,
            subst_marker_counter: 0,
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

    /// True if the upcoming chars are `IDENT}` (optionally with one or
    /// more `::IDENT` package segments). Used to disambiguate
    /// `@{name}` (simple deref) from `@{ EXPR }` (block deref).
    fn lookahead_close_brace_after_ident(&self) -> bool {
        let mut i = self.pos;
        // Allow an initial `$` when peeking into `@{$ref}`.
        if i < self.input.len() && self.input[i] == '$' {
            i += 1;
        }
        let start = i;
        while i < self.input.len()
            && (self.input[i].is_ascii_alphanumeric() || self.input[i] == '_')
        {
            i += 1;
        }
        if i == start {
            return false;
        }
        while i + 1 < self.input.len() && self.input[i] == ':' && self.input[i + 1] == ':' {
            i += 2;
            while i < self.input.len()
                && (self.input[i].is_ascii_alphanumeric() || self.input[i] == '_')
            {
                i += 1;
            }
        }
        i < self.input.len() && self.input[i] == '}'
    }

    fn advance(&mut self) -> char {
        let c = self.ch();
        self.pos += 1;
        if c == '\n' {
            self.current_line += 1;
        }
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
            // Skip comments (to end-of-line, not including the newline)
            if self.ch() == '#' {
                let comment_start = self.pos;
                while self.pos < self.input.len() && self.ch() != '\n' {
                    self.pos += 1;
                }
                self.try_handle_line_directive(comment_start);
            } else {
                break;
            }
        }
    }

    /// If the comment between `[start, self.pos)` matches `# line N` or
    /// `# line N "FILE"` AND the comment starts at start-of-line (only
    /// whitespace between the previous newline and `#`), record the file
    /// override (if any) and adjust `current_line` so the *next* line is N.
    fn try_handle_line_directive(&mut self, start: usize) {
        // Verify start-of-line: walk back from `start` over spaces/tabs
        let mut k = start;
        while k > 0 {
            let c = self.input[k - 1];
            if c == '\n' {
                break;
            }
            if c == ' ' || c == '\t' {
                k -= 1;
                continue;
            }
            return;
        }
        // self.input[start] == '#'
        let mut i = start + 1;
        while i < self.pos && (self.input[i] == ' ' || self.input[i] == '\t') {
            i += 1;
        }
        let word = ['l', 'i', 'n', 'e'];
        if i + 4 > self.pos {
            return;
        }
        for (j, w) in word.iter().enumerate() {
            if self.input[i + j] != *w {
                return;
            }
        }
        i += 4;
        if i >= self.pos || (self.input[i] != ' ' && self.input[i] != '\t') {
            return;
        }
        while i < self.pos && (self.input[i] == ' ' || self.input[i] == '\t') {
            i += 1;
        }
        let num_start = i;
        while i < self.pos && self.input[i].is_ascii_digit() {
            i += 1;
        }
        if i == num_start {
            return;
        }
        let num_str: String = self.input[num_start..i].iter().collect();
        let n: usize = match num_str.parse() {
            Ok(v) => v,
            Err(_) => return,
        };
        let mut file: Option<String> = None;
        let mut j = i;
        while j < self.pos && (self.input[j] == ' ' || self.input[j] == '\t') {
            j += 1;
        }
        if j < self.pos && self.input[j] == '"' {
            j += 1;
            let fs = j;
            while j < self.pos && self.input[j] != '"' {
                j += 1;
            }
            if j >= self.pos {
                return;
            }
            file = Some(self.input[fs..j].iter().collect());
            j += 1;
        } else if j < self.pos && self.input[j] != ' ' && self.input[j] != '\t' {
            // Bare-word filename form: `#line 3 warn.t`. Reference perl
            // accepts an unquoted word up to end-of-line.
            let fs = j;
            while j < self.pos && self.input[j] != ' ' && self.input[j] != '\t' {
                j += 1;
            }
            file = Some(self.input[fs..j].iter().collect());
        }
        while j < self.pos && (self.input[j] == ' ' || self.input[j] == '\t') {
            j += 1;
        }
        if j != self.pos {
            return;
        }

        // Count raw newlines from start of input to `start` to get the
        // 1-based physical line of the directive itself (before offset).
        let mut raw_line: isize = 1;
        for i in 0..start.min(self.input.len()) {
            if self.input[i] == '\n' {
                raw_line += 1;
            }
        }
        // We want the *next* physical line (raw_line + 1) to render as `n`.
        self.line_offset = (n as isize) - (raw_line + 1);
        self.current_line = if (raw_line + self.line_offset) < 1 {
            1
        } else {
            (raw_line + self.line_offset) as usize
        };
        if let Some(f) = file {
            self.file_overrides.push((self.cur_token_count, f));
        }
    }

    /// Recount `current_line` from the start of input up to `self.pos`.
    /// O(N) but N is small per call; amortized still fine since we call once
    /// per token. Keeps line tracking correct even when helpers consume
    /// newlines without updating `current_line`.
    fn recompute_line(&mut self) {
        let mut line: isize = 1;
        for i in 0..self.pos.min(self.input.len()) {
            if self.input[i] == '\n' {
                line += 1;
            }
        }
        let adj = line + self.line_offset;
        self.current_line = if adj < 1 { 1 } else { adj as usize };
    }

    fn skip_pod(&mut self) {
        // Skip =pod / =head1 / =cut blocks. POD ends only at a line that
        // starts with `=cut` followed by whitespace or end-of-line —
        // `=cute`, `=cut2`, `=cut_` keep us in POD (per perlpod and
        // base/lex tests 53–55).
        while self.pos < self.input.len() {
            if self.ch() == '\n' {
                self.pos += 1;
                if self.pos < self.input.len() && self.ch() == '=' {
                    let rest: String = self.input[self.pos..].iter().take(5).collect();
                    let after_cut = rest.chars().nth(4);
                    let ends_pod = rest.starts_with("=cut")
                        && match after_cut {
                            None => true,
                            Some(c) => c.is_whitespace() || c == '\r',
                        };
                    if ends_pod {
                        while self.pos < self.input.len() && self.ch() != '\n' {
                            self.pos += 1;
                        }
                        if self.pos < self.input.len() {
                            self.pos += 1;
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
        // Track line numbers: increment on every `\n` we consume so the
        // position of each emitted token reflects its source line.
        // We use a local `tokens` Vec for the existing logic, then record
        // line numbers in parallel via a closure over `self.current_line`.
        // To avoid rewriting 107 push sites, the loop captures the line at
        // the start of each iteration and backfills `self.token_lines` for
        // any new tokens pushed during that iteration.
        let mut tokens = Vec::new();

        loop {
            self.cur_token_count = tokens.len();
            self.skip_whitespace_and_comments();
            // Recompute line from absolute position to catch any newlines
            // consumed by internal helpers (heredocs, POD, multi-line strings)
            // that didn't update `current_line` themselves.
            self.recompute_line();
            let line_at_token_start = self.current_line;
            let token_count_before = tokens.len();

            if self.pos >= self.input.len() {
                // Drain pending heredocs at EOF too — programs without a
                // trailing newline (`map d<<<<""` with no `\n`) reach end-
                // of-input before the newline branch fires, but their body
                // attempt should still trigger the standard `Can't find
                // string terminator` diagnostic. With `pos` already at
                // EOF, `read_heredoc_body` sees no input and falls into
                // its "unterminated" branch, setting `self.error`.
                if !self.pending_heredocs.is_empty() {
                    let pending = std::mem::take(&mut self.pending_heredocs);
                    for ph in pending {
                        let _ = self.read_heredoc_body(&ph);
                    }
                }
                tokens.push(Token::EOF);
                while self.token_lines.len() < tokens.len() {
                    self.token_lines.push(line_at_token_start);
                }
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
                    self.current_line += 1;
                    // Drain any pending heredocs: their body starts immediately
                    // after this newline, and we backfill the placeholder we
                    // stashed when `<<TAG` was first scanned.
                    if !self.pending_heredocs.is_empty() {
                        let pending = std::mem::take(&mut self.pending_heredocs);
                        for ph in pending {
                            let body = self.read_heredoc_body(&ph);
                            match &ph.target {
                                HeredocTarget::Token(idx) => match tokens.get_mut(*idx) {
                                    Some(Token::StringLit(s)) | Some(Token::InterpString(s)) => {
                                        *s = body;
                                    }
                                    _ => {}
                                },
                                HeredocTarget::SubstReplMarker { token_idx, marker } => {
                                    if let Some(Token::Substitution(_, repl, _)) =
                                        tokens.get_mut(*token_idx)
                                    {
                                        let lit =
                                            heredoc_body_as_perl_literal(&body, ph.interpolate);
                                        *repl = repl.replace(marker, &lit);
                                    }
                                }
                                HeredocTarget::SubstPatMarker { token_idx, marker } => {
                                    if let Some(Token::Substitution(pat, _, _)) =
                                        tokens.get_mut(*token_idx)
                                    {
                                        let lit =
                                            heredoc_body_as_perl_literal(&body, ph.interpolate);
                                        *pat = pat.replace(marker, &lit);
                                    }
                                }
                                HeredocTarget::InterpMarker { token_idx, marker } => {
                                    let lit = heredoc_body_as_perl_literal(&body, ph.interpolate);
                                    match tokens.get_mut(*token_idx) {
                                        Some(Token::InterpString(s))
                                        | Some(Token::StringLit(s)) => {
                                            *s = s.replace(marker, &lit);
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                    }
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
                    if self.ch() == '#' && self.peek(1) != '[' {
                        // $#array, $#$ref, or $#{ ... }
                        self.pos += 1;
                        if self.ch() == '$' {
                            // `$#$ref` — last index of @$ref. Mark with
                            // leading `$` so parser can emit ArrayLenDeref.
                            self.pos += 1;
                            let name = self.read_ident();
                            tokens.push(Token::ArrayLen(format!("${name}")));
                        } else if self.ch() == '{' {
                            // `$#{ EXPR }` — block-form last-index. Mark
                            // with a leading `{` sentinel; the parser
                            // sees this and reads the inner expression.
                            tokens.push(Token::ArrayLen("{".to_string()));
                        } else {
                            let name = self.read_ident();
                            tokens.push(Token::ArrayLen(name));
                        }
                    } else if self.ch() == '$' {
                        // `$$` alone (no following ident/{/$) is the
                        // process-id special var; otherwise scalar deref.
                        // Multiple `$`s compose: `$$$foo` is two derefs —
                        // emit a ScalarDeref whose name has one leading `$`
                        // for each extra level. The interpreter strips
                        // leading `$`s, looking up / dereffing N times.
                        self.pos += 1;
                        let mut extra = 0usize;
                        while self.ch() == '$'
                            && (self.peek(1).is_ascii_alphabetic()
                                || self.peek(1) == '_'
                                || self.peek(1) == '$'
                                || self.peek(1) == '{')
                        {
                            extra += 1;
                            self.pos += 1;
                        }
                        let name = self.read_ident();
                        if name.is_empty() && self.ch() != '{' {
                            if extra > 0 {
                                // `$$$` with no ident — unusual; fall back.
                                tokens.push(Token::ScalarVar("$".to_string()));
                            } else {
                                tokens.push(Token::ScalarVar("$".to_string()));
                            }
                        } else {
                            let prefix: String = "$".repeat(extra);
                            tokens.push(Token::ScalarDeref(format!("{prefix}{name}")));
                        }
                    } else if self.ch() == ':' && self.peek(1) == ':' {
                        // `$::name` — shorthand for `$main::name`. Keep the
                        // `::` in the name so the interpreter can look it up
                        // as a package-qualified scalar.
                        self.pos += 2;
                        let rest = self.read_ident();
                        tokens.push(Token::ScalarVar(format!("::{rest}")));
                    } else if self.ch() == '{'
                        || (self.ch() == ' ' && {
                            // Perl allows whitespace between $ and { in code:
                            // `$ {name}` is the same as `${name}`.
                            let mut look = self.pos;
                            while look < self.input.len() && self.input[look] == ' ' {
                                look += 1;
                            }
                            look < self.input.len() && self.input[look] == '{'
                        })
                    {
                        // ${expr} or ${^NAME} or ${$ref} or ${name}
                        // Skip optional whitespace before the brace.
                        while self.pos < self.input.len() && self.ch() == ' ' {
                            self.pos += 1;
                        }
                        self.pos += 1;
                        if self.ch() == '^' {
                            self.pos += 1;
                            let name = self.read_ident();
                            // Skip closing brace
                            if self.ch() == '}' {
                                self.pos += 1;
                            }
                            tokens.push(Token::ScalarVar(format!("^{name}")));
                        } else if self.ch() == '$' && self.lookahead_close_brace_after_ident() {
                            // ${$ref} — same as $$ref
                            self.pos += 1;
                            let n = self.read_ident();
                            if self.ch() == '}' {
                                self.pos += 1;
                            }
                            tokens.push(Token::ScalarDeref(n));
                        } else if (self.ch() == '_'
                            || self.ch().is_ascii_alphabetic()
                            || (!self.ch().is_ascii() && self.ch().is_alphabetic()))
                            && self.lookahead_close_brace_after_ident()
                        {
                            let name = self.read_ident();
                            if self.ch() == '}' {
                                self.pos += 1;
                            }
                            tokens.push(Token::ScalarVar(name));
                        } else {
                            // `${ EXPR }` — block-scalar-deref. Sentinel.
                            tokens.push(Token::ScalarBlockDerefOpen);
                        }
                    } else if self.ch() == '^' {
                        // $^X style special variable
                        self.pos += 1;
                        let c = self.advance();
                        tokens.push(Token::ScalarVar(format!("^{c}")));
                    } else if self.ch() == '_'
                        || self.ch().is_ascii_alphabetic()
                        || (!self.ch().is_ascii() && self.ch().is_alphabetic())
                    {
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
                    } else if self.ch().is_ascii_digit() {
                        // $0, $1, $2, ... (capture variables and $0 program name)
                        let mut name = String::new();
                        while self.pos < self.input.len() && self.ch().is_ascii_digit() {
                            name.push(self.advance());
                        }
                        tokens.push(Token::ScalarVar(name));
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
                    } else if self.ch() == '.' && !self.peek(1).is_ascii_digit() {
                        // $. — current line number for last read filehandle.
                        // Don't consume `$.5` as `$.` to keep `\$. 5` etc safe.
                        self.pos += 1;
                        tokens.push(Token::ScalarVar(".".to_string()));
                    } else if self.ch() == '"' {
                        self.pos += 1;
                        tokens.push(Token::ScalarVar("\"".to_string()));
                    } else if self.ch() == ';' {
                        self.pos += 1;
                        tokens.push(Token::ScalarVar(";".to_string()));
                    } else if self.ch() == '|' {
                        self.pos += 1;
                        tokens.push(Token::ScalarVar("|".to_string()));
                    } else if self.ch() == '?' {
                        self.pos += 1;
                        tokens.push(Token::ScalarVar("?".to_string()));
                    } else if self.ch() == '&' {
                        self.pos += 1;
                        tokens.push(Token::ScalarVar("&".to_string()));
                    } else if self.ch() == '`' {
                        self.pos += 1;
                        tokens.push(Token::ScalarVar("`".to_string()));
                    } else if self.ch() == '\'' {
                        self.pos += 1;
                        tokens.push(Token::ScalarVar("'".to_string()));
                    } else if self.ch() == '-' {
                        self.pos += 1;
                        tokens.push(Token::ScalarVar("-".to_string()));
                    } else if self.ch() == '+' {
                        self.pos += 1;
                        tokens.push(Token::ScalarVar("+".to_string()));
                    } else if self.ch() == '#' {
                        // `$#` followed by `[` / `{` is `$` + name `#` —
                        // i.e. element/key access on `@#` / `%#` (the
                        // anonymous-name array/hash). Treat the `#` as the
                        // var name; the [/{ is consumed by parse_postfix.
                        self.pos += 1;
                        tokens.push(Token::ScalarVar("#".to_string()));
                    } else if self.ch() == '*' {
                        // `$*` — only meaningful as the postfix-deref form
                        // `EXPR->$*`. Emit a distinct ScalarVar so the parser
                        // can match it after `->`.
                        self.pos += 1;
                        tokens.push(Token::ScalarVar("*".to_string()));
                    } else {
                        // Unknown special var, just treat as $_
                        tokens.push(Token::ScalarVar("_".to_string()));
                    }
                }

                '@' => {
                    self.pos += 1;
                    if self.ch().is_ascii_digit() {
                        // `@4`, `@123` — digit-named array (Perl
                        // treats this as a symbolic ref to a global
                        // array with the given numeric name).
                        let mut name = String::new();
                        while self.pos < self.input.len() && self.ch().is_ascii_digit() {
                            name.push(self.advance());
                        }
                        tokens.push(Token::ArrayVar(name));
                    } else if self.ch() == '_'
                        || self.ch().is_ascii_alphabetic()
                        || (!self.ch().is_ascii() && self.ch().is_alphabetic())
                    {
                        let name = self.read_ident();
                        while self.ch() == ':' && self.peek(1) == ':' {
                            // @Pkg::name not needed yet, skip for simplicity
                            break;
                        }
                        tokens.push(Token::ArrayVar(name));
                    } else if self.ch() == '$' {
                        // @$name — array dereference of a scalar reference.
                        self.pos += 1;
                        let name = if self.ch() == '{' {
                            self.pos += 1;
                            let n = self.read_ident();
                            if self.ch() == '}' {
                                self.pos += 1;
                            }
                            n
                        } else {
                            self.read_ident()
                        };
                        tokens.push(Token::ArrayDeref(name));
                    } else if self.ch() == '{' {
                        self.pos += 1;
                        if self.ch() == '^' {
                            self.pos += 1;
                            let name = self.read_ident();
                            if self.ch() == '}' {
                                self.pos += 1;
                            }
                            tokens.push(Token::ArrayVar(format!("^{name}")));
                        } else if self.ch() == '$'
                            && self.peek(1).is_ascii_alphabetic()
                            && self.lookahead_close_brace_after_ident()
                        {
                            // @{$ref} — same as @$ref
                            self.pos += 1;
                            let n = self.read_ident();
                            if self.ch() == '}' {
                                self.pos += 1;
                            }
                            tokens.push(Token::ArrayDeref(n));
                        } else if (self.ch() == '_'
                            || self.ch().is_ascii_alphabetic()
                            || (!self.ch().is_ascii() && self.ch().is_alphabetic()))
                            && self.lookahead_close_brace_after_ident()
                        {
                            let name = self.read_ident();
                            if self.ch() == '}' {
                                self.pos += 1;
                            }
                            tokens.push(Token::ArrayVar(name));
                        } else {
                            // `@{ EXPR }` — block-deref. Emit a sentinel
                            // token for the parser to treat as start of a
                            // braced expression that derefs as an array.
                            tokens.push(Token::ArrayBlockDerefOpen);
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
                    } else if self.ch() == '$'
                        && tokens.last().map(|t| t.expects_operand()).unwrap_or(true)
                    {
                        // %$name or %{$name} — hash dereference.
                        self.pos += 1;
                        let name = if self.ch() == '{' {
                            self.pos += 1;
                            let n = self.read_ident();
                            if self.ch() == '}' {
                                self.pos += 1;
                            }
                            n
                        } else {
                            self.read_ident()
                        };
                        tokens.push(Token::HashDeref(name));
                    } else if self.ch() == '{'
                        || self.ch() == '_'
                        || self.ch().is_ascii_alphabetic()
                    {
                        // Could be hash variable or modulo
                        // Check context: if last token expects operand, it's a hash var
                        let is_hash = tokens.last().map(|t| t.expects_operand()).unwrap_or(true)
                            || last_is_named_unary(tokens.last());
                        if is_hash
                            && (self.ch() == '_'
                                || self.ch().is_ascii_alphabetic()
                                || (!self.ch().is_ascii() && self.ch().is_alphabetic()))
                        {
                            let name = self.read_ident();
                            tokens.push(Token::HashVar(name));
                        } else if is_hash && self.ch() == '{' {
                            // Detect `%{^NAME}` (and `%{ ^NAME }`) — caret
                            // special-variable hash. Emit a direct HashVar
                            // so reads/writes hit globals[^NAME] instead of
                            // routing through HashBlockDerefOpen, which
                            // would treat `^NAME` as a unary-XOR expression.
                            let mut probe = self.pos + 1;
                            while probe < self.input.len() && self.input[probe] == ' ' {
                                probe += 1;
                            }
                            if probe < self.input.len() && self.input[probe] == '^' {
                                let mut name_end = probe + 1;
                                while name_end < self.input.len()
                                    && (self.input[name_end].is_ascii_alphanumeric()
                                        || self.input[name_end] == '_')
                                {
                                    name_end += 1;
                                }
                                let mut after = name_end;
                                while after < self.input.len() && self.input[after] == ' ' {
                                    after += 1;
                                }
                                if name_end > probe + 1
                                    && after < self.input.len()
                                    && self.input[after] == '}'
                                {
                                    let name: String =
                                        self.input[probe + 1..name_end].iter().collect();
                                    self.pos = after + 1;
                                    tokens.push(Token::HashVar(format!("^{name}")));
                                    continue;
                                }
                            }
                            // `%{ EXPR }` — block-form hash deref. The inner
                            // expression is arbitrary; the parser handles
                            // the `{`..`}` as an expression whose result is
                            // treated as a hash ref.
                            tokens.push(Token::HashBlockDerefOpen);
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
                    // Reserve the slot index up front so any heredoc
                    // tracked from inside `${\<<TAG}` etc. can target
                    // the string we're about to emit.
                    let slot_idx = tokens.len();
                    let (s, has_interp) = self.read_dq_str_interp_at('"', Some(slot_idx));
                    if has_interp {
                        tokens.push(Token::InterpString(s));
                    } else {
                        tokens.push(Token::StringLit(s));
                    }
                }

                '0'..='9' => {
                    tokens.push(self.read_number());
                }

                c if c.is_ascii_alphabetic()
                    || c == '_'
                    || (!c.is_ascii() && c.is_alphabetic()) =>
                {
                    // Special case: `x` immediately after an expression-value
                    // token and followed by a digit is the repeat operator, not
                    // the start of an identifier like `x10`.
                    if self.ch() == 'x'
                        && self.peek(1).is_ascii_digit()
                        && !tokens.is_empty()
                        && !tokens.last().unwrap().expects_operand()
                    {
                        self.pos += 1;
                        tokens.push(Token::StringRepeat);
                        continue;
                    }
                    let ident = self.read_ident();

                    // `__END__` and `__DATA__` end source-code parsing.
                    // Everything after is data (accessible via the DATA
                    // filehandle for `__DATA__`, ignored for `__END__`).
                    // Reference perl stops tokenising at this point; we
                    // mirror that by advancing self.pos to EOF so the
                    // outer loop terminates cleanly.
                    if ident == "__END__" || ident == "__DATA__" {
                        // Both `__END__` and `__DATA__` end source-code
                        // parsing AND expose the trailing bytes via the
                        // `DATA` filehandle (perldata: "__END__ … may be
                        // used in the main package to indicate the
                        // logical end of the script before the actual
                        // end of file. Any following text is ignored,
                        // but may be read via the DATA filehandle").
                        let mut data_start = self.pos;
                        while data_start < self.input.len() && self.input[data_start] != '\n' {
                            data_start += 1;
                        }
                        if data_start < self.input.len() {
                            data_start += 1; // skip the newline
                        }
                        let data: String = self.input[data_start..].iter().collect();
                        self.data_section = Some(data);
                        self.pos = self.input.len();
                        break;
                    }

                    // Check for => (fat comma) - the ident is auto-quoted.
                    // Skip ONLY whitespace here, not comments — `#` is a
                    // valid quote-operator delimiter (`q#hello#`) and
                    // consuming it as a comment would eat the body. For
                    // q/qq/qw/qr/m/s/tr/y the next char after whitespace
                    // is the delimiter, never a comment.
                    while self.pos < self.input.len() && (self.ch() == ' ' || self.ch() == '\t') {
                        self.pos += 1;
                    }
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
                            // Reserve the slot for the upcoming token now so
                            // any heredocs queued from inside the qq body
                            // (e.g. `qq|${\<<TAG}|`) can target it.
                            let slot_idx = tokens.len();
                            let s = self.read_qq_string_at(slot_idx);
                            // qq// is double-quote-equivalent. Route through
                            // InterpString iff there's an *unescaped* sigil
                            // (a real $ / @, not the \x01 / \x02 placeholders
                            // process_escapes leaves behind for `\$` / `\@`).
                            // Otherwise StringLit, with placeholders mapped
                            // back so the literal carries `$` / `@` directly.
                            if s.contains('$') || s.contains('@') {
                                tokens.push(Token::InterpString(s));
                            } else if s.contains('\x01') || s.contains('\x02') {
                                let restored: String = s
                                    .chars()
                                    .map(|c| match c {
                                        '\x01' => '$',
                                        '\x02' => '@',
                                        c => c,
                                    })
                                    .collect();
                                tokens.push(Token::StringLit(restored));
                            } else {
                                tokens.push(Token::StringLit(s));
                            }
                            continue;
                        }
                        "qw" if !self.ch().is_alphanumeric() && self.ch() != '_' => {
                            let words = self.read_qw();
                            tokens.push(Token::QW(words));
                            continue;
                        }
                        "qr" if !self.ch().is_alphanumeric() && self.ch() != '_' => {
                            let (pat, flags) = self.read_qr();
                            tokens.push(Token::QrLit(pat, flags));
                            continue;
                        }
                        "m" if !self.ch().is_alphanumeric()
                            && self.ch() != '_'
                            && self.ch() != '='
                            && self.ch() != ';'
                            && self.ch() != ')'
                            && self.ch() != '}'
                            && self.ch() != ']'
                            // Comma after `m` could be regex delimiter
                            // (`$x =~ m,…,`) or arg separator (`f(m,
                            // n)`). Treat it as regex only when the
                            // preceding context unambiguously expects a
                            // term — i.e. just after `=~`/`!~`.
                            && (self.ch() != ','
                                || matches!(
                                    tokens.last(),
                                    Some(Token::Match | Token::NotMatch)
                                )) =>
                        {
                            // `m/pat/flags` — explicit match regex. Emit as
                            // RegexLit so `$x =~ m/…/` (and the bare regex
                            // context for `m/…/` matching against `$_`) works
                            // identically to `/…/`. Guard against taking a
                            // bareword `m` at end-of-expression or as the
                            // last item inside a `$h{m}` / `$a[m]` subscript
                            // as a regex.
                            let (pat, flags) = self.read_qr();
                            tokens.push(Token::RegexLit(pat, flags));
                            continue;
                        }
                        "s" if !self.ch().is_alphanumeric()
                            && self.ch() != '_'
                            && self.ch() != ','
                            && self.ch() != ';'
                            && self.ch() != ')'
                            && self.ch() != '}'
                            && self.ch() != ']' =>
                        {
                            let token_idx = tokens.len();
                            let (pat, repl, flags) = self.read_substitution(token_idx);
                            tokens.push(Token::Substitution(pat, repl, flags));
                            continue;
                        }
                        "tr" | "y"
                            if !self.ch().is_alphanumeric()
                                && self.ch() != '_'
                                && self.ch() != ','
                                && self.ch() != ';'
                                && self.ch() != ')'
                                && self.ch() != '}'
                                && self.ch() != ']' =>
                        {
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
                        "goto" => Token::Goto,
                        "continue" => Token::Continue,
                        "print" => Token::Print,
                        "say" => Token::Say,
                        "die" => Token::Die,
                        "warn" => Token::Warn,
                        "BEGIN" => Token::Begin,
                        "END" => Token::End,
                        "CHECK" => Token::Check,
                        "INIT" => Token::Init,
                        "bless" => Token::Bless,
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
                        "xor" => Token::Xor,
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
                        "tell" => Token::Tell,
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
                                // Also handle `x=` compound assignment.
                                if self.ch() == '=' && self.peek(1) != '=' {
                                    self.pos += 1;
                                    Token::StringRepeatAssign
                                } else {
                                    Token::StringRepeat
                                }
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
                    } else if self.ch() == '$'
                        && (self.peek(1).is_ascii_alphabetic() || self.peek(1) == '_')
                        && (tokens.last().map(|t| t.expects_operand()).unwrap_or(true)
                            || last_is_named_unary(tokens.last()))
                    {
                        // `*$NAME` — glob deref of a scalar holding a glob
                        // or symbol-table name. Encoded as `Token::Glob`
                        // with a leading `$` marker so the interpreter looks
                        // the value up at runtime.
                        self.pos += 1;
                        let n = self.read_ident();
                        tokens.push(Token::Glob(format!("${n}")));
                    } else if self.ch() == '{'
                        && (tokens.last().map(|t| t.expects_operand()).unwrap_or(true)
                            || last_is_named_unary(tokens.last()))
                    {
                        // `*{ EXPR }` — block-form glob deref. Supports the
                        // common forms `*{NAME}` and `*{$var}`; complex
                        // EXPRs fall back to a plain Star token.
                        let save = self.pos;
                        self.pos += 1;
                        if self.ch() == '$'
                            && (self.peek(1).is_ascii_alphabetic() || self.peek(1) == '_')
                        {
                            self.pos += 1;
                            let n = self.read_ident();
                            if self.ch() == '}' {
                                self.pos += 1;
                                tokens.push(Token::Glob(format!("${n}")));
                            } else {
                                self.pos = save;
                                tokens.push(Token::Star);
                            }
                        } else if self.ch() == '_'
                            || self.ch().is_ascii_alphabetic()
                            || (!self.ch().is_ascii() && self.ch().is_alphabetic())
                        {
                            let n = self.read_ident();
                            if self.ch() == '}' {
                                self.pos += 1;
                                tokens.push(Token::Glob(n));
                            } else {
                                self.pos = save;
                                tokens.push(Token::Star);
                            }
                        } else {
                            self.pos = save;
                            tokens.push(Token::Star);
                        }
                    } else if (self.ch() == '_'
                        || self.ch().is_ascii_alphabetic()
                        || (!self.ch().is_ascii() && self.ch().is_alphabetic()))
                        && (tokens.last().map(|t| t.expects_operand()).unwrap_or(true)
                            || last_is_named_unary(tokens.last()))
                    {
                        // Typeglob like *FH / *pkg::name.
                        let name = self.read_ident();
                        tokens.push(Token::Glob(name));
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
                            // `<<` — heredoc or left-shift. Perl disambiguates
                            // by what immediately follows: a quote (`'` or `"`),
                            // a tilde (for indented heredocs), or a word char
                            // that continues into a label unambiguously starts
                            // a heredoc. Otherwise (whitespace, digit, paren,
                            // operator) it's left-shift.
                            let next = self.ch();
                            let is_heredoc = next == '\''
                                || next == '"'
                                || next == '`'
                                || next == '~'
                                || next == '\\'
                                || next.is_ascii_alphabetic()
                                || next == '_';
                            if is_heredoc {
                                let idx = tokens.len();
                                let interp = self.read_heredoc_header(idx);
                                // Provisional placeholder — replaced at
                                // newline-drain. Type depends on whether
                                // the tag was quoted: `<<'EOF'` → literal,
                                // `<<"EOF"` / `<<EOF` → interpolated.
                                tokens.push(if interp {
                                    Token::InterpString(String::new())
                                } else {
                                    Token::StringLit(String::new())
                                });
                            } else {
                                tokens.push(Token::ShiftLeft);
                            }
                        }
                    } else if (self.ch() == '>'
                        || self.ch() == '$'
                        || self.ch() == '_'
                        || self.ch().is_ascii_alphabetic())
                        && {
                            let last = tokens.last();
                            last.map(|t| t.expects_operand()).unwrap_or(true)
                                || last_is_named_unary(last)
                                || matches!(
                                    last,
                                    Some(Token::Ident(n))
                                        if matches!(
                                            n.as_str(),
                                            "scalar" | "print" | "say" | "warn" | "die" | "chomp"
                                            | "chop" | "return" | "do" | "wantarray"
                                        )
                                )
                        }
                    {
                        // Diamond operator <FH> or <>. Disambiguated from a
                        // less-than comparison by what came before: only an
                        // operator-position context makes `<...>` a diamond.
                        // After an operand (e.g. `$a<$b`), the `<` is `lt`.
                        let diamond_line = self.current_line;
                        let mut name = String::new();
                        while self.ch() != '>' && self.pos < self.input.len() {
                            name.push(self.advance());
                        }
                        if self.ch() == '>' {
                            self.pos += 1;
                        }
                        // The runtime side picks up File::Glob loading
                        // when the diamonds contents need globbing
                        // (any shell-glob metacharacter or whitespace).
                        // We dont produce the lex-time error any more
                        // — at lex time we dont know the real @INC,
                        // and tests that customise @INC need the
                        // diagnostic to reflect it.
                        let _ = diamond_line;
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
            // Backfill token_lines for any tokens pushed during this iteration.
            while self.token_lines.len() < tokens.len() {
                self.token_lines.push(line_at_token_start);
            }
            let _ = token_count_before;
        }

        // Filter out newlines (they're not significant in our grammar).
        // We have to drop the parallel line entries too to keep them aligned.
        // file_overrides indices refer to pre-filter positions — remap them.
        let mut index_map: Vec<usize> = Vec::with_capacity(tokens.len() + 1);
        let mut kept_tokens = Vec::with_capacity(tokens.len());
        let mut kept_lines = Vec::with_capacity(tokens.len());
        for (i, (t, l)) in tokens.iter().zip(self.token_lines.iter()).enumerate() {
            index_map.push(kept_tokens.len());
            let _ = i;
            if !matches!(t, Token::Newline) {
                kept_tokens.push(t.clone());
                kept_lines.push(*l);
            }
        }
        index_map.push(kept_tokens.len());
        for f_over in self.file_overrides.iter_mut() {
            let orig = f_over.0;
            f_over.0 = index_map.get(orig).copied().unwrap_or(kept_tokens.len());
        }
        self.token_lines = kept_lines;
        self.tokens = kept_tokens.clone();
        kept_tokens
    }

    fn read_ident(&mut self) -> String {
        let mut s = String::new();
        while self.pos < self.input.len()
            && (self.ch().is_ascii_alphanumeric()
                || self.ch() == '_'
                || (!self.ch().is_ascii() && self.ch().is_alphabetic()))
        {
            s.push(self.advance());
        }
        // Handle :: package separator
        while self.ch() == ':' && self.peek(1) == ':' {
            s.push_str("::");
            self.pos += 2;
            while self.pos < self.input.len()
                && (self.ch().is_ascii_alphanumeric()
                    || self.ch() == '_'
                    || (!self.ch().is_ascii() && self.ch().is_alphabetic()))
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
                '0'..='7' | '_' => {
                    // Octal without 'o' prefix: 0777, 0_7_7_7
                    // The leading `0` followed by (digits|underscore) means
                    // octal; underscores between digits are allowed.
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

    /// Reads a double-quoted (or backtick) string and also reports whether
    /// the original source contained an *unescaped* `$` or `@` — only
    /// unescaped sigils trigger string interpolation.
    fn read_dq_str_interp(&mut self, delim: char) -> (String, bool) {
        self.read_dq_str_interp_at(delim, None)
    }

    fn read_dq_str_interp_at(&mut self, delim: char, target_idx: Option<usize>) -> (String, bool) {
        let mut s = String::new();
        let mut has_interp = false;
        while self.pos < self.input.len() && self.ch() != delim {
            // `${ EXPR }` / `@{ EXPR }` — copy the inner expression
            // verbatim so backslash escapes (`\7`) inside it aren't
            // mistaken for string-literal escapes (octal 7). Also
            // detect `<<TAG` directives nested inside the island and
            // queue them as pending heredocs that target this string
            // (matches qq// behaviour for `${\<<TAG}` constructs).
            if (self.ch() == '$' || self.ch() == '@') && self.peek(1) == '{' {
                has_interp = true;
                s.push(self.advance()); // $ or @
                s.push(self.advance()); // {
                let mut depth = 1;
                while self.pos < self.input.len() && depth > 0 {
                    let c = self.ch();
                    if c == '{' {
                        depth += 1;
                        s.push(self.advance());
                        continue;
                    }
                    if c == '}' {
                        depth -= 1;
                        s.push(self.advance());
                        if depth == 0 {
                            break;
                        }
                        continue;
                    }
                    if let Some(idx) = target_idx
                        && c == '<'
                        && self.peek(1) == '<'
                    {
                        // Try to parse a `<<TAG` heredoc header from
                        // the current position. On success, insert a
                        // marker into the captured string and queue
                        // the body to be spliced in at drain time.
                        let chars_so_far: Vec<char> = self.input[self.pos..].to_vec();
                        if let Some((consumed, tag, indent, interpolate)) =
                            try_parse_heredoc_header(&chars_so_far, 0)
                        {
                            self.subst_marker_counter += 1;
                            let marker = format!("\x01HD{}\x01", self.subst_marker_counter);
                            s.push_str(&marker);
                            self.pending_heredocs.push(PendingHeredoc {
                                tag,
                                indent,
                                interpolate,
                                target: HeredocTarget::InterpMarker {
                                    token_idx: idx,
                                    marker,
                                },
                                start_line: self.current_line,
                            });
                            self.pos += consumed;
                            continue;
                        }
                    }
                    s.push(self.advance());
                }
                continue;
            }
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
                    '0'..='7' => {
                        // Octal escape: \0, \012, \377, \400, etc.
                        let mut oct = String::new();
                        oct.push(self.ch());
                        self.pos += 1;
                        while self.pos < self.input.len()
                            && self.ch() >= '0'
                            && self.ch() <= '7'
                            && oct.len() < 3
                        {
                            oct.push(self.advance());
                        }
                        if oct == "0"
                            && (self.pos >= self.input.len() || self.ch() < '0' || self.ch() > '7')
                        {
                            s.push('\0');
                        } else {
                            let v = u32::from_str_radix(&oct, 8).unwrap_or(0);
                            if let Some(c) = char::from_u32(v) {
                                s.push(c);
                            }
                        }
                    }
                    'o' => {
                        // \o{NNN} octal escape
                        self.pos += 1;
                        if self.ch() == '{' {
                            self.pos += 1;
                            let mut oct = String::new();
                            while self.pos < self.input.len() && self.ch() != '}' {
                                if self.ch() != ' ' {
                                    oct.push(self.advance());
                                } else {
                                    self.pos += 1;
                                }
                            }
                            if self.ch() == '}' {
                                self.pos += 1;
                            }
                            let v = u32::from_str_radix(&oct, 8).unwrap_or(0);
                            if let Some(c) = char::from_u32(v) {
                                s.push(c);
                            }
                        } else {
                            s.push('\\');
                            s.push('o');
                        }
                    }
                    'x' => {
                        self.pos += 1;
                        let mut hex = String::new();
                        if self.ch() == '{' {
                            self.pos += 1;
                            let mut last_was_underscore = false;
                            while self.pos < self.input.len() && self.ch() != '}' {
                                if self.ch().is_ascii_hexdigit() {
                                    hex.push(self.advance());
                                    last_was_underscore = false;
                                } else if self.ch() == ' ' {
                                    self.pos += 1;
                                } else if self.ch() == '_' {
                                    if last_was_underscore {
                                        // Double underscore — stop parsing hex digits
                                        while self.pos < self.input.len() && self.ch() != '}' {
                                            self.pos += 1;
                                        }
                                        break;
                                    }
                                    last_was_underscore = true;
                                    self.pos += 1;
                                } else {
                                    // Non-hex char: stop parsing
                                    while self.pos < self.input.len() && self.ch() != '}' {
                                        self.pos += 1;
                                    }
                                    break;
                                }
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
                    // Case-modifier escapes — emit sentinel bytes that
                    // the interpreter detects at interpolation time and
                    // applies to subsequent characters until `\E`.
                    // U+0010-U+0015 are control chars unlikely to appear
                    // in literal strings.
                    'u' => {
                        s.push('\x10'); // ucfirst next char
                        self.pos += 1;
                    }
                    'l' => {
                        s.push('\x11'); // lcfirst next char
                        self.pos += 1;
                    }
                    'U' => {
                        s.push('\x12'); // uc all chars until \E
                        self.pos += 1;
                    }
                    'L' => {
                        s.push('\x13'); // lc all chars until \E
                        self.pos += 1;
                    }
                    'E' => {
                        s.push('\x14'); // end \U/\L/\Q
                        self.pos += 1;
                    }
                    'Q' => {
                        s.push('\x15'); // quotemeta all chars until \E
                        self.pos += 1;
                    }
                    'c' => {
                        // \cX — control character: XOR next char with 0x40
                        self.pos += 1;
                        if self.pos < self.input.len() {
                            let ctrl = (self.ch() as u8 ^ 0x40) as char;
                            s.push(ctrl);
                            self.pos += 1;
                        }
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
        (s, has_interp)
    }

    fn read_double_quoted_string(&mut self, delim: char) -> String {
        self.read_dq_str_interp(delim).0
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
        let (open, close, s, _) = self.read_delimited_string_term();
        (open, close, s)
    }

    /// Same as `read_delimited_string` but also reports whether the
    /// closing delimiter was actually found (true) or we ran off the
    /// end of input (false). Callers that produce position-specific
    /// diagnostics (`m//`, `s///`) need this distinction.
    fn read_delimited_string_term(&mut self) -> (char, char, String, bool) {
        // Skip plain whitespace before the delimiter — but NOT comments.
        // `#` is a perfectly valid delimiter (`q#foo#`), and treating it
        // as a comment would consume the q's body. We do allow newlines
        // between the operator name and its delimiter to match Perl.
        while self.pos < self.input.len()
            && (self.ch() == ' ' || self.ch() == '\t' || self.ch() == '\n')
        {
            if self.ch() == '\n' {
                self.current_line += 1;
            }
            self.pos += 1;
        }

        let open = self.advance();
        let close = Self::find_matching_delim(open);
        let is_paired = open != close;
        let mut s = String::new();
        let mut depth = 1;
        let mut terminated = false;

        // When the delimiter is `\` itself (e.g. `qr\…\`), there's no
        // backslash-escape because every `\` is either the delimiter
        // or a literal whose meaning is decided by the regex/string
        // parser downstream — there's no `\\` escape to differentiate.
        let delim_is_backslash = open == '\\';
        while self.pos < self.input.len() {
            if self.ch() == '\\' && self.pos + 1 < self.input.len() && !delim_is_backslash {
                let next = self.input[self.pos + 1];
                if is_paired {
                    // In paired delimiters (q{}, q<>, etc.), \open and \close
                    // should not affect nesting depth. Keep both chars in the
                    // raw output so qq{}/qr{} still see the backslash for
                    // later escape processing. read_q_string does its own
                    // post-pass to strip the backslash for single-quote
                    // semantics.
                    if next == '\\' || next == open || next == close {
                        s.push(self.advance()); // the backslash
                        s.push(self.advance()); // the delimiter char
                        continue;
                    }
                } else {
                    // Non-paired (q//, q!!, etc.): \delim produces the
                    // literal delimiter, everything else keeps the backslash.
                    self.pos += 1;
                    if next == close {
                        s.push(self.advance());
                    } else {
                        s.push('\\');
                    }
                    continue;
                }
            }
            if is_paired && self.ch() == open {
                depth += 1;
                s.push(self.advance());
            } else if self.ch() == close {
                depth -= 1;
                if depth == 0 {
                    self.pos += 1;
                    terminated = true;
                    break;
                }
                s.push(self.advance());
            } else {
                s.push(self.advance());
            }
        }
        (open, close, s, terminated)
    }

    fn read_q_string(&mut self) -> String {
        let (open, close, s) = self.read_delimited_string();
        // q// is like single quotes — minimal escaping.
        // For paired delimiters (q{}, q<>, etc.), \open → open, \close → close,
        // \\ → \, all other backslashes stay literal.
        // For non-paired (q//), read_delimited_string already handled \delim.
        if open == close {
            return s;
        }
        let mut out = String::with_capacity(s.len());
        let chars: Vec<char> = s.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '\\' && i + 1 < chars.len() {
                let next = chars[i + 1];
                if next == open || next == close || next == '\\' {
                    out.push(next);
                    i += 2;
                    continue;
                }
            }
            out.push(chars[i]);
            i += 1;
        }
        out
    }

    fn read_qq_string(&mut self) -> String {
        let (_, _, s) = self.read_delimited_string();
        // qq// is like double quotes — process escape sequences
        process_escapes(&s)
    }

    /// Like `read_qq_string` but registers any `<<TAG` heredocs that appear
    /// inside the qq body's `${...}` / `@{...}` interpolation islands as
    /// pending heredocs targeting the upcoming token at `target_idx`.
    /// `target_idx` should be the index where the qq token will land —
    /// typically `tokens.len()` at the call site.
    fn read_qq_string_at(&mut self, target_idx: usize) -> String {
        let (_, _, raw) = self.read_delimited_string();
        // Scan for `<<TAG` (or `<<~TAG`, `<<'TAG'`, `<<"TAG"`, `<<\TAG`)
        // inside `${…}` / `@{…}` blocks. Anything outside such blocks is
        // ordinary qq text where `<<` is just two literal characters.
        // Replace each detected directive with a unique \x01HD<N>\x01
        // marker and queue a heredoc whose drain target writes back to
        // the qq token's captured string.
        let chars: Vec<char> = raw.chars().collect();
        let mut out = String::with_capacity(raw.len());
        let mut i = 0;
        while i < chars.len() {
            // Detect entry into `${…}` / `@{…}`.
            if (chars[i] == '$' || chars[i] == '@') && i + 1 < chars.len() && chars[i + 1] == '{' {
                out.push(chars[i]);
                out.push(chars[i + 1]);
                i += 2;
                let mut depth = 1usize;
                while i < chars.len() && depth > 0 {
                    if chars[i] == '{' {
                        depth += 1;
                        out.push(chars[i]);
                        i += 1;
                    } else if chars[i] == '}' {
                        depth -= 1;
                        out.push(chars[i]);
                        i += 1;
                        if depth == 0 {
                            break;
                        }
                    } else if chars[i] == '<' && i + 1 < chars.len() && chars[i + 1] == '<' {
                        // Try to register a heredoc starting at this `<<`.
                        // We need a header parser similar to
                        // try_register_heredoc_in_subst, but reading from
                        // the captured `chars` (not `self.input`). Reuse
                        // a small helper that returns (consumed_len, tag,
                        // indent, interpolate) on success.
                        match try_parse_heredoc_header(&chars, i) {
                            Some((consumed, tag, indent, interpolate)) => {
                                self.subst_marker_counter += 1;
                                let marker = format!("\x01HD{}\x01", self.subst_marker_counter);
                                out.push_str(&marker);
                                self.pending_heredocs.push(PendingHeredoc {
                                    tag,
                                    indent,
                                    interpolate,
                                    target: HeredocTarget::InterpMarker {
                                        token_idx: target_idx,
                                        marker,
                                    },
                                    start_line: self.current_line,
                                });
                                i += consumed;
                            }
                            None => {
                                out.push(chars[i]);
                                i += 1;
                            }
                        }
                    } else {
                        out.push(chars[i]);
                        i += 1;
                    }
                }
                continue;
            }
            out.push(chars[i]);
            i += 1;
        }
        process_escapes(&out)
    }

    fn read_qw(&mut self) -> Vec<String> {
        let (open, close, s) = self.read_delimited_string();
        // qw// is semantically `split ' ', q(...)`, so the same minimal
        // single-quote-style escape handling applies: `\\` collapses to a
        // single backslash, and `\<delim>` escapes the delimiter. Any other
        // backslash stays literal.
        let mut out = String::with_capacity(s.len());
        let chars: Vec<char> = s.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '\\' && i + 1 < chars.len() {
                let next = chars[i + 1];
                if next == '\\' || next == open || next == close {
                    out.push(next);
                    i += 2;
                    continue;
                }
            }
            out.push(chars[i]);
            i += 1;
        }
        out.split_whitespace().map(|w| w.to_string()).collect()
    }

    fn read_qr(&mut self) -> (String, String) {
        let start_line = self.current_line;
        let (open, _, raw_pat, terminated) = self.read_delimited_string_term();
        // Single-quote delimiters disable interpolation (`m'$b'` is
        // literal `$b`, not the value of `$b`). Escape `$` and `@` in
        // the pattern so the runtime interpolator leaves them as
        // literals; non-interpolating regexes are otherwise handled
        // identically downstream.
        let pat = if open == '\'' {
            // Only neutralise `$VAR` and `@VAR` interpolation; bare
            // `$` / `@` (regex anchor / array context) and backslash
            // escapes (`\d`, `\w`, …) remain meaningful as Perl expects.
            let chars: Vec<char> = raw_pat.chars().collect();
            let mut out = String::with_capacity(raw_pat.len());
            let mut i = 0;
            while i < chars.len() {
                let c = chars[i];
                if (c == '$' || c == '@')
                    && i + 1 < chars.len()
                    && (chars[i + 1] == '_'
                        || chars[i + 1] == '{'
                        || chars[i + 1].is_ascii_alphabetic())
                {
                    out.push('\\');
                    out.push(c);
                    i += 1;
                    continue;
                }
                out.push(c);
                i += 1;
            }
            out
        } else {
            raw_pat
        };
        // `read_delimited_string` exits silently on EOF — but for `m//`
        // and `qr//` the right diagnostic is "Search pattern not
        // terminated". Tag the lexer error if we fell off the end
        // before seeing a closing delimiter.
        if !terminated && self.error.is_none() {
            self.error = Some(format!(
                "Search pattern not terminated at {{FILE}} line {start_line}.\n",
            ));
        }
        let flags = self.read_regex_flags();
        (pat, flags)
    }

    fn read_substitution(&mut self, subst_token_idx: usize) -> (String, String, String) {
        // s/pattern/replacement/flags
        // The delimiter can be any non-alphanumeric character
        let start_line = self.current_line;
        let open = self.advance();
        let close = Self::find_matching_delim(open);
        let is_paired = open != close;

        // Read pattern
        let mut pat = String::new();
        let mut pat_terminated = false;
        let mut depth = 1;
        while self.pos < self.input.len() {
            if is_paired && self.ch() == open {
                depth += 1;
                pat.push(self.advance());
            } else if self.ch() == close {
                depth -= 1;
                if depth == 0 {
                    self.pos += 1; // skip closing delimiter
                    pat_terminated = true;
                    break;
                }
                pat.push(self.advance());
            } else if self.ch() == '\\' {
                // Same rationale as the REPL branch below — dont
                // pre-consume the next char so a `<<TAG` heredoc
                // directive sitting at `\<<TAG` boundary isnt
                // hidden by the escape pair.
                pat.push(self.advance());
            } else if self.ch() == '<' && self.peek(1) == '<' {
                if let Some(marker) =
                    self.try_register_heredoc_in_subst(subst_token_idx, /*in_repl=*/ false)
                {
                    pat.push_str(&marker);
                } else {
                    pat.push(self.advance());
                }
            } else {
                pat.push(self.advance());
            }
        }

        // EOF before pattern's closing delimiter — emit Perl's exact
        // diagnostic for unterminated `s///`.
        if !pat_terminated && self.error.is_none() {
            self.error = Some(format!(
                "Substitution pattern not terminated at {{FILE}} line {start_line}.\n",
            ));
            return (pat, String::new(), String::new());
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
        let mut repl_terminated = false;
        let mut depth = 1;
        while self.pos < self.input.len() {
            if repl_is_paired && self.ch() == repl_open {
                depth += 1;
                repl.push(self.advance());
            } else if self.ch() == repl_close {
                depth -= 1;
                if depth == 0 {
                    self.pos += 1;
                    repl_terminated = true;
                    break;
                }
                repl.push(self.advance());
            } else if self.ch() == '\n' {
                // Drain any heredoc directives queued earlier in this
                // REPL: their bodies live on the source lines following
                // the directive, BEFORE the rest of the REPL resumes.
                // After draining, the marker we stashed in `repl` gets
                // replaced with the heredoc body wrapped as a literal
                // (so the `/e` re-parse sees a quoted-string fragment).
                repl.push(self.advance());
                if !self.pending_heredocs.is_empty() {
                    let pending = std::mem::take(&mut self.pending_heredocs);
                    for ph in pending {
                        let body = self.read_heredoc_body(&ph);
                        if let HeredocTarget::SubstReplMarker { marker, .. } = &ph.target {
                            let lit = heredoc_body_as_perl_literal(&body, ph.interpolate);
                            repl = repl.replace(marker, &lit);
                        } else if let HeredocTarget::SubstPatMarker { marker, .. } = &ph.target {
                            // Pattern-side markers — not expected
                            // here, but be safe by removing them.
                            let lit = heredoc_body_as_perl_literal(&body, ph.interpolate);
                            repl = repl.replace(marker, &lit);
                        } else if let HeredocTarget::InterpMarker { marker, .. } = &ph.target {
                            let lit = heredoc_body_as_perl_literal(&body, ph.interpolate);
                            repl = repl.replace(marker, &lit);
                        }
                    }
                }
            } else if self.ch() == '\\' {
                // Push `\` only; do NOT pre-consume the following
                // char as a single escape pair. Doing so would hide
                // a `<<TAG` heredoc directive sitting at the second
                // char of `\<<TAG` (Perl take-ref + heredoc inside a
                // `${\<<TAG}` REPL island). The next loop iteration
                // can still consume the next char normally — escape
                // semantics dont apply at REPL-capture time, only
                // at /e re-parse / regex-engine time, both of which
                // see the same backslash either way.
                repl.push(self.advance());
            } else if self.ch() == '<' && self.peek(1) == '<' {
                if let Some(marker) =
                    self.try_register_heredoc_in_subst(subst_token_idx, /*in_repl=*/ true)
                {
                    repl.push_str(&marker);
                } else {
                    repl.push(self.advance());
                }
            } else {
                repl.push(self.advance());
            }
        }

        if !repl_terminated && self.error.is_none() {
            self.error = Some(format!(
                "Substitution replacement not terminated at {{FILE}} line {start_line}.\n",
            ));
            return (pat, repl, String::new());
        }

        let flags = self.read_regex_flags();
        (pat, repl, flags)
    }

    fn read_transliterate(&mut self) -> (String, String, String) {
        // tr/from/to/flags or y/from/to/flags
        // Reuse the same logic as substitution. The `subst_token_idx` here
        // is unused (heredocs aren't valid inside `tr///` patterns) but the
        // shared helper still needs a valid value.
        let start_line = self.current_line;
        let result = self.read_substitution(usize::MAX);
        // Reference perl auto-loads `_charnames` whenever a `\N{NAME}` escape
        // appears at compile time. Without `unicore/Name.pm` (the typical
        // Nix sandbox / `-I../lib`-only test environment), the load fails
        // with the chained `BEGIN failed--compilation aborted` error. Detect
        // the same trigger here so op/tr's tr/i-\N{LATIN SMALL LETTER J}//d
        // produces reference perl's exact diagnostic.
        if self.error.is_none() && pattern_uses_named_char(&result.0) {
            self.error = Some(format!(
                "Can't locate unicore/Name.pm in @INC (you may need to install the unicore::Name module) (@INC entries checked: ../lib) at ../lib/_charnames.pm line 10.\nBEGIN failed--compilation aborted at ../lib/_charnames.pm line 10.\nCompilation failed in require at {{FILE}} line {start_line}.\nBEGIN failed--compilation aborted at {{FILE}} line {start_line}.\n"
            ));
        }
        result
    }

    fn read_regex(&mut self, delim: char) -> (String, String) {
        let start_line = self.current_line;
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
        if self.pos >= self.input.len() && self.error.is_none() {
            // Reference perl's message for unterminated regex.
            self.error = Some(format!(
                "Search pattern not terminated at {{FILE}} line {start_line}.\n",
            ));
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
            && matches!(
                self.ch(),
                'g' | 'i' | 'm' | 's' | 'x' | 'e' | 'r' | 'n' | 'a' | 'd' | 'l' | 'u' | 'c' | 'p'
            )
        {
            flags.push(self.advance());
        }
        flags
    }

    /// When read_substitution sees `<<` inside a pattern or replacement,
    /// peek ahead to see if it's a real heredoc start (tag, optionally
    /// quoted, optionally indented `~`, optionally backslash-escaped). If
    /// yes, register a queued heredoc whose body will be spliced into the
    /// captured string at this position via a unique marker. Returns the
    /// marker for the caller to splice into the captured string in place
    /// of the `<<TAG` source bytes. Returns None if `<<` was actually a
    /// left-shift (no heredoc tag follows) — caller falls back to
    /// consuming `<` as a literal char.
    fn try_register_heredoc_in_subst(&mut self, token_idx: usize, in_repl: bool) -> Option<String> {
        // We're sitting on the first `<`. Confirm the second is also `<`.
        debug_assert_eq!(self.ch(), '<');
        debug_assert_eq!(self.peek(1), '<');
        // Look past the `<<` and any optional `~` / whitespace / `\` / quote
        // for a tag identifier. If we don't find one, this is a left-shift
        // expression embedded in the substitution body — don't transform it.
        let mut probe = self.pos + 2;
        let mut indent = false;
        if probe < self.input.len() && self.input[probe] == '~' {
            indent = true;
            probe += 1;
            while probe < self.input.len()
                && (self.input[probe] == ' ' || self.input[probe] == '\t')
            {
                probe += 1;
            }
        }
        let mut interpolate = true;
        if probe < self.input.len() && self.input[probe] == '\\' {
            probe += 1;
            interpolate = false;
        }
        let quote = if probe < self.input.len()
            && (self.input[probe] == '\'' || self.input[probe] == '"')
        {
            let q = self.input[probe];
            if q == '\'' {
                interpolate = false;
            }
            probe += 1;
            Some(q)
        } else {
            None
        };
        // The tag must start with a letter / underscore for unquoted form;
        // quoted form allows arbitrary chars up to the closing quote (and
        // may legally be empty — `<<""` / `<<~""` are valid heredocs whose
        // body terminates on the first blank line).
        let tag_start = probe;
        if let Some(q) = quote {
            while probe < self.input.len() && self.input[probe] != q && self.input[probe] != '\n' {
                probe += 1;
            }
        } else {
            if probe >= self.input.len()
                || !(self.input[probe] == '_' || self.input[probe].is_ascii_alphabetic())
            {
                return None;
            }
            while probe < self.input.len()
                && (self.input[probe] == '_' || self.input[probe].is_ascii_alphanumeric())
            {
                probe += 1;
            }
            if tag_start == probe {
                return None;
            }
        }
        let tag: String = self.input[tag_start..probe].iter().collect();
        // Skip the closing quote if present.
        if let Some(q) = quote
            && probe < self.input.len()
            && self.input[probe] == q
        {
            probe += 1;
        }
        // Commit: jump self.pos past the `<<…TAG` source bytes and queue a
        // heredoc whose body will be spliced into the captured string at
        // the marker position.
        let start_line = self.current_line;
        self.pos = probe;
        self.subst_marker_counter += 1;
        let marker = format!("\x01HD{}\x01", self.subst_marker_counter);
        let target = if in_repl {
            HeredocTarget::SubstReplMarker {
                token_idx,
                marker: marker.clone(),
            }
        } else {
            HeredocTarget::SubstPatMarker {
                token_idx,
                marker: marker.clone(),
            }
        };
        self.pending_heredocs.push(PendingHeredoc {
            tag,
            indent,
            interpolate,
            target,
            start_line,
        });
        Some(marker)
    }

    /// Parse the `<<TAG` header: read the tag + flags, register a pending
    /// heredoc (to be filled in after the current line ends). The real body
    /// is spliced in when the next newline is encountered. Returns whether
    /// the heredoc interpolates so the caller can pick the right token type.
    fn read_heredoc_header(&mut self, placeholder_idx: usize) -> bool {
        let mut indent = false;
        let mut interpolate = true;

        while self.ch() == ' ' || self.ch() == '\t' {
            self.pos += 1;
        }

        if self.ch() == '~' {
            indent = true;
            self.pos += 1;
            // Perl allows whitespace between ~ and the delimiter
            while self.ch() == ' ' || self.ch() == '\t' {
                self.pos += 1;
            }
        }

        if self.ch() == '\\' {
            self.pos += 1;
            interpolate = false;
        }

        let quote = if self.ch() == '\'' || self.ch() == '"' || self.ch() == '`' {
            let q = self.ch();
            if q == '\'' {
                interpolate = false;
            }
            self.pos += 1;
            Some(q)
        } else {
            None
        };
        let quote_open_line = self.current_line;

        let mut tag = String::new();
        // Unquoted tags are identifiers — stop at anything non-alphanumeric.
        // Quoted tags read until the closing quote.
        while self.pos < self.input.len()
            && self.ch() != '\n'
            && Some(self.ch()) != quote
            && (quote.is_some() || self.ch() == '_' || self.ch().is_ascii_alphanumeric())
        {
            tag.push(self.advance());
        }

        if let Some(q) = quote {
            if self.ch() == q {
                self.pos += 1;
            } else if self.error.is_none() {
                // Reached newline / EOF before the closing quote of a
                // quoted heredoc tag (`<<\`foo\``, `<<"foo"`, etc.).
                // Reference perl emits the standard delim-unterminated
                // message: `Unterminated delimiter for here document`.
                let _ = quote_open_line;
                self.error = Some(format!(
                    "Unterminated delimiter for here document at {{FILE}} line {}.\n",
                    quote_open_line
                ));
            }
        }

        self.pending_heredocs.push(PendingHeredoc {
            tag,
            indent,
            interpolate,
            target: HeredocTarget::Token(placeholder_idx),
            start_line: self.current_line,
        });
        interpolate
    }

    /// Scan a heredoc body line for nested `<<TAG` declarations, read each
    /// inner body inline (consuming lines from `self.input` immediately,
    /// matching reference perls "drain pending heredocs at line end"
    /// behaviour), and splice the inner body back into the line as a Perl
    /// literal. Lets `@{[ <<E2 ]}` inside a `<<E1` body resolve correctly.
    fn expand_inner_heredocs_in_line(&mut self, line: String) -> String {
        let chars: Vec<char> = line.chars().collect();
        let mut out = String::with_capacity(line.len());
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '<'
                && i + 1 < chars.len()
                && chars[i + 1] == '<'
                && let Some((consumed, tag, indent, interpolate)) =
                    try_parse_heredoc_header(&chars, i)
            {
                let inner = PendingHeredoc {
                    tag,
                    indent,
                    interpolate,
                    target: HeredocTarget::Token(0),
                    start_line: self.current_line,
                };
                let body = self.read_heredoc_body(&inner);
                let lit = heredoc_body_as_perl_literal(&body, inner.interpolate);
                out.push_str(&lit);
                i += consumed;
                continue;
            }
            out.push(chars[i]);
            i += 1;
        }
        out
    }

    fn read_heredoc_body(&mut self, ph: &PendingHeredoc) -> String {
        let mut raw_lines: Vec<String> = Vec::new();
        let mut terminated = false;
        let mut saw_trailing_newline = false;
        let mut indent_prefix = String::new();
        loop {
            if self.pos >= self.input.len() {
                break;
            }
            let mut line = String::new();
            while self.pos < self.input.len() && self.ch() != '\n' {
                line.push(self.advance());
            }
            if self.pos < self.input.len() {
                self.pos += 1;
                self.current_line += 1;
                saw_trailing_newline = true;
            } else {
                saw_trailing_newline = false;
            }
            // Strip a trailing \r so CRLF-terminated sources match the
            // tag and don't bleed \r into the captured body (reference
            // perl effectively reads source in text mode).
            let mut cmp_line = line.strip_suffix('\r').unwrap_or(&line).to_string();
            if ph.indent {
                // For indented heredocs (<<~), the terminator can be indented.
                // Check if the line ends with the tag and everything before
                // it is whitespace. This correctly handles tags that contain
                // leading spaces (e.g. <<~' EOF' where tag is " EOF").
                if cmp_line.len() >= ph.tag.len() {
                    let prefix_len = cmp_line.len() - ph.tag.len();
                    if cmp_line[prefix_len..] == *ph.tag
                        && cmp_line[..prefix_len]
                            .chars()
                            .all(|c| c == ' ' || c == '\t')
                    {
                        indent_prefix = cmp_line[..prefix_len].to_string();
                        terminated = true;
                        break;
                    }
                }
            } else if cmp_line == ph.tag {
                terminated = true;
                break;
            }
            // Interpolating bodies can declare nested heredocs via
            // `@{[ <<TAG ]}` islands. Read the inner body inline (it
            // consumes the lines that would have followed THIS line)
            // and splice its literal form back into `cmp_line` so the
            // outer body parses the island as plain Perl code later.
            if ph.interpolate {
                cmp_line = self.expand_inner_heredocs_in_line(cmp_line);
            }
            raw_lines.push(cmp_line);
        }
        if !terminated && self.error.is_none() {
            // Empty-tag heredoc (`<<""`): reference perl accepts EOF as
            // a valid terminator *only* if the body ended on a newline
            // (i.e. the last unterminated character was \n). Without a
            // trailing newline, the heredoc is unterminated and should
            // emit the same error as a non-empty tag.
            if ph.tag.is_empty() && saw_trailing_newline {
                // Strip trailing blank lines (the implicit terminator).
                while raw_lines.last().is_some_and(|l| l.is_empty()) {
                    raw_lines.pop();
                }
            } else {
                // Reference perl: `Can't find string terminator "TAG" anywhere
                // before EOF at FILE line LINE.`
                self.error = Some(format!(
                    "Can't find string terminator \"{}\" anywhere before EOF at {{FILE}} line {}.\n",
                    ph.tag, ph.start_line
                ));
            }
        }
        // Build the body string, stripping indentation for <<~ heredocs.
        let mut body = String::new();
        if ph.indent && !indent_prefix.is_empty() {
            for (i, line) in raw_lines.iter().enumerate() {
                if let Some(stripped) = line.strip_prefix(&indent_prefix) {
                    body.push_str(stripped);
                } else if line.trim().is_empty() {
                    // Blank lines don't need to match the indentation.
                } else {
                    // Reference perl: `Indentation on line N of here-doc
                    // doesn't match delimiter at FILE line M.`
                    if self.error.is_none() {
                        self.error = Some(format!(
                            "Indentation on line {} of here-doc doesn't match delimiter at {{FILE}} line {}.\n",
                            i + 1,
                            ph.start_line
                        ));
                    }
                    body.push_str(line);
                }
                body.push('\n');
            }
        } else {
            for line in &raw_lines {
                body.push_str(line);
                body.push('\n');
            }
        }
        if ph.interpolate {
            process_escapes(&body)
        } else {
            body
        }
    }
}

/// Idents that take an operand so `%` after them is a hash sigil,
/// not modulus (e.g. `scalar %h` / `pos %h` without parens).
fn last_is_named_unary(last: Option<&Token>) -> bool {
    if matches!(last, Some(Token::Tell) | Some(Token::Eof)) {
        return true;
    }
    matches!(
        last,
        Some(Token::Ident(n)) if matches!(
            n.as_str(),
            "scalar" | "pos" | "defined" | "exists" | "delete" | "ref"
            | "keys" | "values" | "each" | "wantarray"
        )
    )
}

/// Try to parse a `<<TAG` heredoc header starting at `chars[i]`. Returns
/// (consumed_chars, tag, indent, interpolate) on success, None otherwise.
/// Mirrors `try_register_heredoc_in_subst` but reads from a `&[char]`
/// slice rather than `self.input`, so callers post-processing a captured
/// body can use it.
fn try_parse_heredoc_header(chars: &[char], i: usize) -> Option<(usize, String, bool, bool)> {
    if i + 1 >= chars.len() || chars[i] != '<' || chars[i + 1] != '<' {
        return None;
    }
    let mut probe = i + 2;
    let mut indent = false;
    if probe < chars.len() && chars[probe] == '~' {
        indent = true;
        probe += 1;
        while probe < chars.len() && (chars[probe] == ' ' || chars[probe] == '\t') {
            probe += 1;
        }
    }
    let mut interpolate = true;
    if probe < chars.len() && chars[probe] == '\\' {
        probe += 1;
        interpolate = false;
    }
    let quote = if probe < chars.len() && (chars[probe] == '\'' || chars[probe] == '"') {
        let q = chars[probe];
        if q == '\'' {
            interpolate = false;
        }
        probe += 1;
        Some(q)
    } else {
        None
    };
    let tag_start = probe;
    if let Some(q) = quote {
        while probe < chars.len() && chars[probe] != q && chars[probe] != '\n' {
            probe += 1;
        }
    } else {
        if probe >= chars.len() || !(chars[probe] == '_' || chars[probe].is_ascii_alphabetic()) {
            return None;
        }
        while probe < chars.len() && (chars[probe] == '_' || chars[probe].is_ascii_alphanumeric()) {
            probe += 1;
        }
        if tag_start == probe {
            return None;
        }
    }
    let tag: String = chars[tag_start..probe].iter().collect();
    if let Some(q) = quote
        && probe < chars.len()
        && chars[probe] == q
    {
        probe += 1;
    }
    Some((probe - i, tag, indent, interpolate))
}

/// Format a heredoc body as a Perl string literal that can be spliced into
/// a captured `s/PAT/REPL/` body. Use single-quoted form for non-interpolating
/// heredocs (`<<'EOF'`) so `$`, `@`, `\` are kept literal; double-quoted form
/// for interpolating heredocs so embedded variables / escape sequences keep
/// their normal interpolation. Either way escape the chosen delimiter so the
/// generated literal parses without needing balanced inner content.
fn heredoc_body_as_perl_literal(body: &str, interpolate: bool) -> String {
    if !interpolate {
        // q[…] keeps everything literal. Escape `[` and `]` to avoid the
        // bracketed delimiter ending early.
        let mut out = String::from("q[");
        for c in body.chars() {
            if c == '[' || c == ']' || c == '\\' {
                out.push('\\');
            }
            out.push(c);
        }
        out.push(']');
        out
    } else {
        // The interpolating heredoc has *already* had its `\n`, `\t`, etc.
        // expanded to real bytes by `process_escapes`. Re-quote those so the
        // double-quoted Perl literal we produce doesn't try to expand them
        // again. Variables (`$x` / `@x`) we leave literally so the eval-time
        // interpolation reaches them.
        let mut out = String::from("\"");
        for c in body.chars() {
            match c {
                '"' | '\\' => {
                    out.push('\\');
                    out.push(c);
                }
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                '\0' => out.push_str("\\0"),
                _ => out.push(c),
            }
        }
        out.push('"');
        out
    }
}

/// Detect whether a regex / tr / s pattern contains a `\N{NAME}` escape —
/// i.e. `\N{...}` whose body is neither a count (`\N{3}` / `\N{3,5}`) nor
/// a `U+XXXX` codepoint. Reference perl auto-loads `_charnames` for these,
/// and that load fails with a chained `Can't locate unicore/Name.pm …` /
/// `BEGIN failed` diagnostic under a stripped @INC.
fn pattern_uses_named_char(pat: &str) -> bool {
    let bytes = pat.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'\\' && bytes[i + 1] == b'N' {
            // Optional whitespace allowed (regex /x), but tr doesn't use /x —
            // require `{` immediately after `\N`.
            if let Some(j) = bytes
                .get(i + 2)
                .and_then(|&c| if c == b'{' { Some(i + 3) } else { None })
            {
                let body_start = j;
                let mut k = body_start;
                while k < bytes.len() && bytes[k] != b'}' {
                    k += 1;
                }
                if k < bytes.len() {
                    let body = &pat[body_start..k];
                    let trimmed: String = body.split_whitespace().collect();
                    let is_count = !trimmed.is_empty()
                        && trimmed.chars().all(|c| c.is_ascii_digit() || c == ',');
                    let is_codepoint = trimmed.starts_with("U+");
                    if !is_count && !is_codepoint && !trimmed.is_empty() {
                        return true;
                    }
                    i = k + 1;
                    continue;
                }
            }
        }
        i += 1;
    }
    false
}

fn process_escapes(s: &str) -> String {
    let mut result = String::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        // `${EXPR}` / `@{EXPR}` opens a Perl-code interpolation island. Pass
        // the bytes through verbatim so backslash sequences inside (e.g. the
        // `\"` in `${\"world"}`, or a `\<<TAG` heredoc reference) keep their
        // source-level meaning when the inner expression is reparsed by
        // parse_interp_string. Track brace depth so nested `{...}` works.
        if (chars[i] == '$' || chars[i] == '@') && i + 1 < chars.len() && chars[i + 1] == '{' {
            result.push(chars[i]);
            result.push(chars[i + 1]);
            i += 2;
            let mut depth = 1;
            while i < chars.len() && depth > 0 {
                if chars[i] == '{' {
                    depth += 1;
                } else if chars[i] == '}' {
                    depth -= 1;
                    if depth == 0 {
                        result.push('}');
                        i += 1;
                        break;
                    }
                }
                result.push(chars[i]);
                i += 1;
            }
            continue;
        }
        if chars[i] == '\\' && i + 1 < chars.len() {
            i += 1;
            match chars[i] {
                'n' => result.push('\n'),
                't' => result.push('\t'),
                'r' => result.push('\r'),
                '\\' => result.push('\\'),
                '"' => result.push('"'),
                // Escaped sigil: emit a placeholder that the interp parser
                // turns back into the literal character — keeps `qq{\$1}` /
                // `<<"END"\n\$1\nEND` from interpolating $1 (which would
                // otherwise hit the regex match group).
                '$' => result.push('\x01'),
                '@' => result.push('\x02'),
                '0'..='7' => {
                    let mut oct = String::new();
                    oct.push(chars[i]);
                    while i + 1 < chars.len()
                        && chars[i + 1] >= '0'
                        && chars[i + 1] <= '7'
                        && oct.len() < 3
                    {
                        i += 1;
                        oct.push(chars[i]);
                    }
                    if oct == "0" {
                        result.push('\0');
                    } else {
                        let v = u32::from_str_radix(&oct, 8).unwrap_or(0);
                        if let Some(c) = char::from_u32(v) {
                            result.push(c);
                        }
                    }
                }
                'o' => {
                    if i + 1 < chars.len() && chars[i + 1] == '{' {
                        i += 2; // skip o{
                        let mut oct = String::new();
                        while i < chars.len() && chars[i] != '}' {
                            if chars[i] != ' ' {
                                oct.push(chars[i]);
                            }
                            i += 1;
                        }
                        // i now points at } or end
                        let v = u32::from_str_radix(&oct, 8).unwrap_or(0);
                        if let Some(c) = char::from_u32(v) {
                            result.push(c);
                        }
                    } else {
                        result.push('\\');
                        result.push('o');
                    }
                }
                'a' => result.push('\x07'),
                'b' => result.push('\x08'),
                'f' => result.push('\x0C'),
                'e' => result.push('\x1B'),
                'c' => {
                    // \cX — control character: XOR next char with 0x40
                    i += 1;
                    if i < chars.len() {
                        let ctrl = (chars[i] as u8 ^ 0x40) as char;
                        result.push(ctrl);
                    }
                }
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
