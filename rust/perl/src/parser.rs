use crate::ast::*;
use crate::lexer::Token;
use std::collections::HashSet;

pub struct Parser {
    tokens: Vec<Token>,
    /// Parallel to `tokens` — 1-based line number of each token in source.
    /// Empty when line tracking isn't wired up (treat as line 0).
    token_lines: Vec<usize>,
    pos: usize,
    /// Names that appear as `sub NAME` in this token stream — populated
    /// in a first pass so that bareword references later in the file can
    /// be recognised as no-arg sub calls (`done_testing;` after the sub
    /// is declared elsewhere).
    known_subs: HashSet<String>,
    /// First parse error encountered, with `{FILE}` as a placeholder for
    /// the filename. Main/eval_string reads this and surfaces it like the
    /// Lexer::error path (into `$@` inside eval, to stderr at top level).
    pub error: Option<String>,
    /// Pending file overrides from `# line N "FILE"` directives.
    /// Sorted by token_idx; drained as we emit LineMarks.
    file_overrides: std::collections::VecDeque<(usize, String)>,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        let known_subs = scan_sub_names(&tokens);
        Parser {
            tokens,
            token_lines: Vec::new(),
            pos: 0,
            known_subs,
            error: None,
            file_overrides: std::collections::VecDeque::new(),
        }
    }

    pub fn new_with_lines(tokens: Vec<Token>, token_lines: Vec<usize>) -> Self {
        Self::new_with_lines_and_files(tokens, token_lines, Vec::new())
    }

    pub fn new_with_lines_and_files(
        tokens: Vec<Token>,
        token_lines: Vec<usize>,
        file_overrides: Vec<(usize, String)>,
    ) -> Self {
        let known_subs = scan_sub_names(&tokens);
        Parser {
            tokens,
            token_lines,
            pos: 0,
            known_subs,
            error: None,
            file_overrides: file_overrides.into_iter().collect(),
        }
    }

    /// Line number of the current token in the source, or 0 if unknown.
    pub fn current_line(&self) -> usize {
        self.token_lines.get(self.pos).copied().unwrap_or(0)
    }

    fn tok(&self) -> &Token {
        if self.pos < self.tokens.len() {
            &self.tokens[self.pos]
        } else {
            &Token::EOF
        }
    }

    fn peek(&self, offset: usize) -> &Token {
        let p = self.pos + offset;
        if p < self.tokens.len() {
            &self.tokens[p]
        } else {
            &Token::EOF
        }
    }

    fn advance(&mut self) -> Token {
        let t = self.tok().clone();
        self.pos += 1;
        t
    }

    fn expect(&mut self, expected: &Token) {
        if self.tok() == expected {
            self.pos += 1;
        }
        // Silently skip if not found — permissive parser
    }

    fn at(&self, tok: &Token) -> bool {
        self.tok() == tok
    }

    fn eat(&mut self, tok: &Token) -> bool {
        if self.tok() == tok {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    pub fn parse_program(&mut self) -> Vec<Stmt> {
        let mut stmts = Vec::new();
        let mut last_line = 0;
        while !self.at(&Token::EOF) {
            // Skip stray semicolons/newlines
            if self.eat(&Token::Semi) || self.eat(&Token::Newline) {
                continue;
            }
            // Unmatched right brace at top level — reference perl
            // emits "Unmatched right curly bracket at FILE line N, at
            // end of line\nsyntax error at FILE line N, near "…}"" for
            // `eval '5; }'` etc. Record once, then keep scanning so a
            // subsequent eval-string check (or compile-time-use-check)
            // can still report the surrounding context.
            if self.at(&Token::RBrace) && self.error.is_none() {
                let line = self.current_line();
                self.error = Some(format!(
                    "Unmatched right curly bracket at {{FILE}} line {line}, at end of line\nsyntax error at {{FILE}} line {line}, near \"}}\"\nExecution of {{FILE}} aborted due to compilation errors.\n"
                ));
                self.pos += 1;
                continue;
            }
            let line = self.current_line();
            if line != 0 && line != last_line {
                while let Some((idx, _)) = self.file_overrides.front() {
                    if *idx <= self.pos {
                        let (_, f) = self.file_overrides.pop_front().unwrap();
                        stmts.push(Stmt::FileMark(f));
                    } else {
                        break;
                    }
                }
                stmts.push(Stmt::LineMark(line));
                last_line = line;
            }
            if let Some(stmt) = self.parse_stmt() {
                // Flatten Stmt::BareBlock that's just a hoist wrapper
                // (`[Stmt::Begin(...), inner]`) so the BEGIN ends up at
                // the program's top level where `run()` /
                // `eval_string_inner` will pick it up for compile-time
                // execution. Used by `package NAME VERSION { … }` to
                // assign `$NAME::VERSION` before any later read of it
                // inside the same eval.
                if let Stmt::BareBlock(inner) = &stmt
                    && inner.len() == 2
                    && matches!(inner[0], Stmt::Begin(_, _))
                {
                    let Stmt::BareBlock(inner) = stmt else {
                        unreachable!()
                    };
                    let mut iter = inner.into_iter();
                    stmts.push(iter.next().unwrap());
                    stmts.push(iter.next().unwrap());
                } else {
                    stmts.push(stmt);
                }
            }
        }
        stmts
    }

    fn parse_stmt(&mut self) -> Option<Stmt> {
        // Check for label
        let label = self.try_parse_label();

        // If the label is followed by a non-loop/block statement (e.g.
        // `loop: my $test = undef;`), emit a standalone `Stmt::Label` so a
        // subsequent `goto LABEL` can target it. Loops/blocks/if/foreach
        // below still consume the label themselves via the `label` local.
        if let Some(ref lbl) = label
            && !matches!(
                self.tok(),
                Token::LBrace
                    | Token::While
                    | Token::Until
                    | Token::For
                    | Token::Foreach
                    | Token::If
                    | Token::Unless
                    | Token::Do
            )
        {
            return Some(Stmt::Label(lbl.clone()));
        }

        match self.tok() {
            Token::EOF => return None,
            Token::Semi => {
                self.pos += 1;
                return Some(Stmt::Nop);
            }
            // `defer { … }` — Perl 5.36+ experimental: defer body to
            // lexical scope exit, in LIFO order. op/defer.
            Token::Ident(name)
                if name.as_str() == "defer" && matches!(self.peek(1), Token::LBrace) =>
            {
                self.pos += 1;
                let body = self.parse_brace_block();
                return Some(Stmt::Defer(body));
            }
            // `...` (yada-yada) — Perl 5.12+ placeholder operator that
            // dies "Unimplemented" when reached. Only valid as a bare
            // statement (`...;` or `... EOF` or `... }`); any other
            // following token is a syntax error in reference perl,
            // including postfix `if`/`unless`/etc. op/yadayada 10-22.
            // base/lex tests 107, 108.
            Token::Ident(name) if name.as_str() == "..." => {
                self.pos += 1;
                if !matches!(self.tok(), Token::Semi | Token::EOF | Token::RBrace) {
                    let line = self.current_line();
                    let near = format!("... {}", token_display(self.tok()));
                    self.error = Some(format!(
                        "syntax error at {{FILE}} line {line}, near \"{near}\"\n"
                    ));
                    return Some(Stmt::Nop);
                }
                self.eat(&Token::Semi);
                return Some(Stmt::Die(vec![Expr::StringLit(
                    "Unimplemented".to_string(),
                )]));
            }
            Token::LBrace => {
                // Bare block
                self.pos += 1;
                let body = self.parse_block_body();
                self.eat(&Token::RBrace);
                // A trailing `continue { ... }` turns the bare block into a
                // one-shot loop with a continue block (matches Perl).
                if let Some(continue_body) = self.try_parse_continue() {
                    return Some(Stmt::BlockWithContinue {
                        body,
                        continue_body,
                        label,
                    });
                }
                if let Some(label) = label {
                    return Some(Stmt::NamedBlock(label, body));
                }
                return Some(Stmt::BareBlock(body));
            }
            Token::If => {
                self.pos += 1;
                return Some(self.parse_if());
            }
            Token::Unless => {
                self.pos += 1;
                return Some(self.parse_unless());
            }
            Token::While => {
                self.pos += 1;
                let mut stmt = self.parse_while();
                if let Some(l) = label {
                    if let Stmt::While {
                        label: ref mut lbl, ..
                    } = stmt
                    {
                        *lbl = Some(l);
                    }
                }
                return Some(stmt);
            }
            Token::Until => {
                self.pos += 1;
                let cond = self.parse_paren_expr();
                let body = self.parse_brace_block();
                let continue_body = self.try_parse_continue();
                return Some(Stmt::Until {
                    cond,
                    body,
                    continue_body,
                    label,
                });
            }
            Token::For | Token::Foreach => {
                self.pos += 1;
                return Some(self.parse_for(label));
            }
            Token::Sub => {
                // Distinguish `sub NAME { ... }` (declaration) from
                // `sub { ... }->(args)` (anonymous sub, immediately invoked,
                // or otherwise used as an expression). If the next token is
                // `{` (no name, no proto, no attribs) we may have an anon
                // sub expression — peek past the matching `}` to see if the
                // next token after the body is one that continues an
                // expression (`->`, `(`, `->(`, operator, etc.).
                // Also catch `sub :ATTR { ... }` (anon sub with attributes
                // like `sub :lvalue { … }`) when the next non-attribute
                // token is `{` and the body is followed by an expr-
                // continuation token — base/lex test 110
                // (`map { sub :lvalue { "a" } } 1`).
                let mut p = self.pos + 1;
                while matches!(self.tokens.get(p), Some(Token::Colon)) {
                    p += 1;
                    if matches!(self.tokens.get(p), Some(Token::Ident(_))) {
                        p += 1;
                    }
                }
                if matches!(self.tokens.get(p), Some(Token::LBrace)) && self.anon_sub_starts_expr(p)
                {
                    let expr = self.parse_expr();
                    let stmt = Stmt::Expr(expr);
                    let stmt = self.maybe_postfix(stmt);
                    self.eat(&Token::Semi);
                    return Some(stmt);
                }
                self.pos += 1;
                return Some(self.parse_sub_decl());
            }
            Token::My => {
                self.pos += 1;
                let stmt = self.parse_my_decl();
                let stmt = self.maybe_postfix(stmt);
                self.eat(&Token::Semi);
                return Some(stmt);
            }
            Token::Our => {
                self.pos += 1;
                let stmt = self.parse_our_decl();
                let stmt = self.maybe_postfix(stmt);
                self.eat(&Token::Semi);
                return Some(stmt);
            }
            Token::Local => {
                self.pos += 1;
                let stmt = self.parse_local_decl();
                let stmt = self.maybe_postfix(stmt);
                self.eat(&Token::Semi);
                return Some(stmt);
            }
            Token::State => {
                self.pos += 1;
                // Reuse parse_var_list (same shape as `my`) and emit
                // Stmt::State so the interpreter can install
                // per-sub persistent storage. op/state.
                let (mut vars, list_ctx) = self.parse_var_list();
                // `state ($t) //= EXPR` — list-context init using
                // defined-or-assign. Treat the RHS as the init
                // expression for the only var; the interpreter
                // already runs init exactly once. op/state 6-14.
                if list_ctx
                    && vars.len() == 1
                    && vars[0].1.is_none()
                    && matches!(self.tok(), Token::DefOrAssign)
                {
                    self.pos += 1;
                    let init = self.parse_expr();
                    vars[0].1 = Some(init);
                }
                let stmt = self.maybe_postfix(Stmt::State(vars, list_ctx));
                self.eat(&Token::Semi);
                return Some(stmt);
            }
            Token::Package => {
                self.pos += 1;
                // Accept tokens that are normally keywords as package
                // names — `package next` / `package maybe` etc. are
                // legal in reference perl (mro.pm itself uses
                // `package next;` to hide the `next::method` /
                // `next::can` namespace).
                let name = match self.tok() {
                    Token::Ident(name) => {
                        let n = name.clone();
                        self.pos += 1;
                        n
                    }
                    Token::Next => {
                        self.pos += 1;
                        "next".to_string()
                    }
                    Token::Last => {
                        self.pos += 1;
                        "last".to_string()
                    }
                    Token::Redo => {
                        self.pos += 1;
                        "redo".to_string()
                    }
                    _ => "main".to_string(),
                };
                // `package NAME VERSION` — assign $NAME::VERSION = VERSION
                // before the block (or as a side-effect of the bare form).
                let version: Option<Expr> =
                    if matches!(self.tok(), Token::Float(_) | Token::Integer(_)) {
                        Some(self.parse_unary())
                    } else {
                        None
                    };
                // `package NAME { BLOCK }` — scoped package: switch to
                // NAME inside the block, revert on exit. Implement by
                // wrapping the block's statements with a Package(NAME) at
                // the front and a Package(<previous>) at the end. The
                // interpreter's `Stmt::Block` already reverts the package
                // on exit (see `Stmt::Block` handling).
                // `package NAME VERSION;` and `package NAME VERSION { … }`
                // — Perl evaluates the version at compile time. We mirror
                // that by wrapping the assignment in an implicit BEGIN
                // block so it runs before the surrounding statements (and
                // before any later `$NAME::VERSION` reads in the same
                // eval).
                let version_begin = version.map(|ver| {
                    Stmt::Begin(
                        vec![Stmt::Expr(Expr::Assign(
                            Box::new(Expr::ScalarVar(format!("{name}::VERSION"))),
                            Box::new(ver),
                        ))],
                        0,
                    )
                });
                if self.at(&Token::LBrace) {
                    let body = self.parse_brace_block();
                    let mut block_stmts = Vec::with_capacity(body.len() + 1);
                    block_stmts.push(Stmt::Package(name.clone()));
                    block_stmts.extend(body);
                    let block = Stmt::Block(block_stmts);
                    // Hoist the implicit `BEGIN { $NAME::VERSION = … }`
                    // OUT of the surrounding Block so it gets recognised
                    // by `run()` / `eval_string_inner`'s BEGIN pass and
                    // runs at compile-time. Inside-the-block placement
                    // would defer it to runtime, which is too late for
                    // earlier `$Foo::VERSION` reads in the same eval.
                    if let Some(begin) = version_begin {
                        return Some(Stmt::BareBlock(vec![begin, block]));
                    }
                    return Some(block);
                }
                self.eat(&Token::Semi);
                if let Some(begin) = version_begin {
                    return Some(Stmt::BareBlock(vec![begin, Stmt::Package(name)]));
                }
                return Some(Stmt::Package(name));
            }
            Token::Use => {
                self.pos += 1;
                return Some(self.parse_use());
            }
            Token::Require => {
                self.pos += 1;
                let expr = self.parse_expr();
                self.eat(&Token::Semi);
                return Some(Stmt::Require(expr));
            }
            Token::Begin => {
                self.pos += 1;
                let body = self.parse_brace_block();
                // `self.pos` points just past the closing `}`; the token
                // whose line we want is the one at `pos - 1`.
                let end_line = self
                    .token_lines
                    .get(self.pos.saturating_sub(1))
                    .copied()
                    .unwrap_or(0);
                return Some(Stmt::Begin(body, end_line));
            }
            Token::End => {
                // `END { ... }` is a phaser. Bare `END` (no following
                // `{`) is just the bareword string "END" — common in
                // heredoc bodies (`<<END\n...\nEND`) where the
                // trailing tag terminates the heredoc and any later
                // expression evaluates `END` as a bareword. Letting
                // the phaser parser swallow the rest of the file as
                // its body would absorb subsequent statements and
                // run them in END phase (after lexicals are torn down).
                if matches!(self.tokens.get(self.pos + 1), Some(Token::LBrace)) {
                    self.pos += 1;
                    let body = self.parse_brace_block();
                    return Some(Stmt::End(body));
                }
                // Fall through: parse as expression. The bareword
                // resolves to the string "END".
                self.pos += 1;
                let stmt = Stmt::Expr(Expr::StringLit("END".to_string()));
                let stmt = self.maybe_postfix(stmt);
                self.eat(&Token::Semi);
                return Some(stmt);
            }
            Token::Check => {
                if matches!(self.tokens.get(self.pos + 1), Some(Token::LBrace)) {
                    self.pos += 1;
                    let body = self.parse_brace_block();
                    return Some(Stmt::Check(body));
                }
                self.pos += 1;
                let stmt = Stmt::Expr(Expr::StringLit("CHECK".to_string()));
                let stmt = self.maybe_postfix(stmt);
                self.eat(&Token::Semi);
                return Some(stmt);
            }
            Token::Init => {
                if matches!(self.tokens.get(self.pos + 1), Some(Token::LBrace)) {
                    self.pos += 1;
                    let body = self.parse_brace_block();
                    return Some(Stmt::Init(body));
                }
                self.pos += 1;
                let stmt = Stmt::Expr(Expr::StringLit("INIT".to_string()));
                let stmt = self.maybe_postfix(stmt);
                self.eat(&Token::Semi);
                return Some(stmt);
            }
            Token::Last => {
                self.pos += 1;
                let label = if let Token::Ident(name) = self.tok() {
                    let n = name.clone();
                    self.pos += 1;
                    Some(n)
                } else {
                    None
                };
                let stmt = Stmt::Last(label);
                return Some(self.maybe_postfix(stmt));
            }
            Token::Goto => {
                self.pos += 1;
                // `goto &sub` — tail call to the named sub, passing the
                // current @_. Encoded into Stmt::Goto with the `&` prefix
                // so the interpreter recognises the sub-form.
                // `goto \&sub` — same semantics; the `\&` produces a
                // coderef, and we normalise it here to `&NAME` form.
                // `goto &$var` — dynamic; encode as `&$NAME` so the
                // interpreter can resolve at runtime.
                let label = if matches!(self.tok(), Token::BitAnd) {
                    self.pos += 1;
                    if let Token::Ident(name) = self.tok() {
                        let n = name.clone();
                        self.pos += 1;
                        format!("&{n}")
                    } else if let Token::ScalarVar(name) = self.tok() {
                        let n = name.clone();
                        self.pos += 1;
                        format!("&${n}")
                    } else {
                        "&".to_string()
                    }
                } else if matches!(self.tok(), Token::Backslash)
                    && matches!(self.peek(1), Token::BitAnd)
                    && matches!(self.peek(2), Token::Ident(_))
                {
                    self.pos += 2; // skip \ and &
                    if let Token::Ident(name) = self.tok() {
                        let n = name.clone();
                        self.pos += 1;
                        format!("&{n}")
                    } else {
                        "&".to_string()
                    }
                } else if let Token::Ident(name) = self.tok() {
                    let n = name.clone();
                    self.pos += 1;
                    n
                } else {
                    String::new()
                };
                let stmt = Stmt::Goto(label);
                return Some(self.maybe_postfix(stmt));
            }
            Token::Next => {
                self.pos += 1;
                let label = if let Token::Ident(name) = self.tok() {
                    let n = name.clone();
                    self.pos += 1;
                    Some(n)
                } else {
                    None
                };
                let stmt = Stmt::Next(label);
                return Some(self.maybe_postfix(stmt));
            }
            Token::Redo => {
                self.pos += 1;
                let label = if let Token::Ident(name) = self.tok() {
                    let n = name.clone();
                    self.pos += 1;
                    Some(n)
                } else {
                    None
                };
                let stmt = Stmt::Redo(label);
                return Some(self.maybe_postfix(stmt));
            }
            Token::Return => {
                self.pos += 1;
                let expr = if self.at(&Token::Semi)
                    || self.at(&Token::RBrace)
                    || self.at(&Token::If)
                    || self.at(&Token::Unless)
                {
                    None
                } else {
                    Some(self.parse_expr())
                };
                let stmt = Stmt::Return(expr);
                return Some(self.maybe_postfix(stmt));
            }
            Token::Print | Token::Say => {
                let is_say = matches!(self.tok(), Token::Say);
                self.pos += 1;
                let stmt = self.parse_print_stmt(is_say);
                return Some(self.maybe_postfix(stmt));
            }
            Token::Printf => {
                self.pos += 1;
                let args = self.parse_list_expr();
                let stmt = Stmt::Printf(None, args);
                return Some(self.maybe_postfix(stmt));
            }
            Token::Die => {
                self.pos += 1;
                let args = self.parse_list_expr();
                let stmt = Stmt::Die(args);
                return Some(self.maybe_postfix(stmt));
            }
            Token::Warn => {
                self.pos += 1;
                let args = self.parse_list_expr();
                let stmt = Stmt::Warn(args);
                return Some(self.maybe_postfix(stmt));
            }
            Token::Eval
                if matches!(self.peek(1), Token::LBrace) && {
                    // Peek past the matching `}` to see whether this eval
                    // is followed by a binary operator that needs to bind
                    // the whole expression — `eval { … } // "fallback"`,
                    // `eval { … } or die …`, etc. In that case the guard
                    // returns false and we fall through to the parse_expr
                    // path so the binop's right-hand side attaches
                    // correctly. Without this, the dedicated Stmt::Eval
                    // would consume just the block and leave `// "fallback"`
                    // orphaned. op/catch test 10.
                    let mut depth = 0i32;
                    let mut probe = self.pos + 1;
                    let mut found_close = None;
                    while probe < self.tokens.len() {
                        match &self.tokens[probe] {
                            Token::LBrace => depth += 1,
                            Token::RBrace => {
                                depth -= 1;
                                if depth == 0 {
                                    found_close = Some(probe);
                                    break;
                                }
                            }
                            _ => {}
                        }
                        probe += 1;
                    }
                    let next_is_binop = matches!(
                        found_close.and_then(|p| self.tokens.get(p + 1)),
                        Some(
                            Token::DefOr
                                | Token::LogOr
                                | Token::LogAnd
                                | Token::Or
                                | Token::And
                                | Token::Plus
                                | Token::Minus
                                | Token::Star
                                | Token::Slash
                                | Token::Dot
                                | Token::DotDot
                                | Token::Question,
                        )
                    );
                    !next_is_binop
                } =>
            {
                self.pos += 1;
                let body = self.parse_brace_block();
                let stmt = Stmt::Eval(Box::new(EvalArg::Block(body)));
                return Some(self.maybe_postfix(stmt));
            }
            // `eval EXPR or … ` and similar — fall through to parse_expr
            // so the `eval` is parsed as a primary-level expression and
            // the `or`/`and` on its right gets the correct precedence.
            // `eval { BLOCK }` keeps the dedicated Stmt::Eval path above
            // so its die-trapping / `$@` semantics aren't routed through
            // an extra Stmt::Expr layer.
            // `DESTROY { ... }` / `AUTOLOAD { ... }` at statement position
            // parse as `sub NAME { ... }` (Perl's parser hardcodes these two
            // special subs so the `sub` keyword is optional). Other barewords
            // followed by `{...}` are still call-with-hashref (matches B::Deparse
            // output on reference perl).
            Token::Ident(name)
                if (name == "DESTROY" || name == "AUTOLOAD")
                    && matches!(self.tokens.get(self.pos + 1), Some(Token::LBrace)) =>
            {
                return Some(self.parse_sub_decl());
            }
            _ => {
                // Expression statement. A top-level comma operator — e.g.
                // `f(), last unless COND` — groups multiple expressions so a
                // trailing postfix modifier applies to the whole group. When
                // the group includes a bare control keyword (last / next /
                // return / redo), parse the flow stmt *without* letting it
                // consume any postfix modifier, so the outer `unless` gates
                // the whole group (Perl-consistent behaviour).
                let expr = self.parse_expr();
                if self.at(&Token::Comma) || self.at(&Token::FatComma) {
                    let mut exprs = vec![expr];
                    let mut flow_stmt: Option<Stmt> = None;
                    while self.eat(&Token::Comma) || self.eat(&Token::FatComma) {
                        if self.at(&Token::Semi) || self.at(&Token::EOF) || self.at(&Token::RBrace)
                        {
                            break;
                        }
                        match self.tok() {
                            Token::Last => {
                                self.pos += 1;
                                let lbl = if let Token::Ident(name) = self.tok() {
                                    let n = name.clone();
                                    self.pos += 1;
                                    Some(n)
                                } else {
                                    None
                                };
                                flow_stmt = Some(Stmt::Last(lbl));
                                break;
                            }
                            Token::Next => {
                                self.pos += 1;
                                let lbl = if let Token::Ident(name) = self.tok() {
                                    let n = name.clone();
                                    self.pos += 1;
                                    Some(n)
                                } else {
                                    None
                                };
                                flow_stmt = Some(Stmt::Next(lbl));
                                break;
                            }
                            Token::Redo => {
                                self.pos += 1;
                                flow_stmt = Some(Stmt::Redo(None));
                                break;
                            }
                            Token::Return => {
                                self.pos += 1;
                                let ret_expr = if self.at(&Token::Semi)
                                    || self.at(&Token::If)
                                    || self.at(&Token::Unless)
                                {
                                    None
                                } else {
                                    Some(self.parse_expr())
                                };
                                flow_stmt = Some(Stmt::Return(ret_expr));
                                break;
                            }
                            _ => {
                                exprs.push(self.parse_expr());
                            }
                        }
                    }
                    let expr = if exprs.len() == 1 {
                        exprs.into_iter().next().unwrap()
                    } else {
                        Expr::ArrayLit(exprs)
                    };
                    let stmt = if let Some(flow) = flow_stmt {
                        Stmt::Block(vec![Stmt::Expr(expr), flow])
                    } else {
                        Stmt::Expr(expr)
                    };
                    return Some(self.maybe_postfix(stmt));
                }
                let stmt = Stmt::Expr(expr);
                return Some(self.maybe_postfix(stmt));
            }
        }
    }

    fn maybe_postfix(&mut self, stmt: Stmt) -> Stmt {
        match self.tok() {
            Token::If => {
                self.pos += 1;
                let cond = self.parse_expr();
                self.eat(&Token::Semi);
                Stmt::PostfixIf(Box::new(stmt), cond)
            }
            Token::Unless => {
                self.pos += 1;
                let cond = self.parse_expr();
                self.eat(&Token::Semi);
                Stmt::PostfixUnless(Box::new(stmt), cond)
            }
            Token::While => {
                self.pos += 1;
                let cond = self.parse_expr();
                self.eat(&Token::Semi);
                Stmt::PostfixWhile(Box::new(stmt), cond)
            }
            Token::Until => {
                self.pos += 1;
                let cond = self.parse_expr();
                self.eat(&Token::Semi);
                Stmt::PostfixUntil(Box::new(stmt), cond)
            }
            Token::For | Token::Foreach => {
                self.pos += 1;
                // Postfix `for LIST` accepts comma-separated lists:
                // `stmt for 'a', 'b'` iterates over both elements.
                // parse_expr alone stops at the first comma.
                let items = self.parse_list_expr();
                let list = if items.len() == 1 {
                    items.into_iter().next().unwrap()
                } else {
                    Expr::ArrayLit(items)
                };
                self.eat(&Token::Semi);
                Stmt::PostfixFor(Box::new(stmt), list)
            }
            _ => {
                self.eat(&Token::Semi);
                stmt
            }
        }
    }

    fn try_parse_label(&mut self) -> Option<String> {
        if let Token::Ident(name) = self.tok() {
            if self.peek(1) == &Token::Colon && self.peek(2) != &Token::Colon {
                let label = name.clone();
                self.pos += 2; // skip ident and colon
                return Some(label);
            }
        }
        None
    }

    fn parse_if(&mut self) -> Stmt {
        let cond = self.parse_paren_expr();
        let then = self.parse_brace_block();
        let mut elsifs = Vec::new();
        let mut else_block = None;

        loop {
            if self.eat(&Token::Elsif) {
                let cond = self.parse_paren_expr();
                let body = self.parse_brace_block();
                elsifs.push((cond, body));
            } else if self.eat(&Token::Else) {
                else_block = Some(self.parse_brace_block());
                break;
            } else {
                break;
            }
        }

        Stmt::If {
            cond,
            then,
            elsifs,
            else_block,
        }
    }

    fn parse_unless(&mut self) -> Stmt {
        let cond = self.parse_paren_expr();
        let then = self.parse_brace_block();
        let else_block = if self.eat(&Token::Else) {
            Some(self.parse_brace_block())
        } else {
            None
        };
        Stmt::Unless {
            cond,
            then,
            else_block,
        }
    }

    fn parse_while(&mut self) -> Stmt {
        let cond = self.parse_paren_expr();
        let cond = wrap_iter_cond_with_defined(cond);
        let body = self.parse_brace_block();
        let continue_body = self.try_parse_continue();
        Stmt::While {
            cond,
            body,
            continue_body,
            label: None,
        }
    }

    /// `continue { ... }` follows a loop body. Returns `Some(body)` if
    /// present, `None` otherwise.
    fn try_parse_continue(&mut self) -> Option<Vec<Stmt>> {
        if self.eat(&Token::Continue) {
            Some(self.parse_brace_block())
        } else {
            None
        }
    }

    fn parse_for(&mut self, label: Option<String>) -> Stmt {
        // Check if it's foreach-style: for my $var (list) { }
        // or C-style: for (init; cond; step) { }

        if self.at(&Token::My) || self.at(&Token::Our) || matches!(self.tok(), Token::ScalarVar(_))
        {
            // Foreach style. `for my $i (…)` declares a lexical; `for
            // our $i (…)` declares a package var. We model both as
            // `is_my` since the interpreter localises the loop var.
            let is_my = self.eat(&Token::My) || self.eat(&Token::Our);
            let var = if let Token::ScalarVar(name) = self.tok() {
                let n = name.clone();
                self.pos += 1;
                n
            } else {
                "_".to_string()
            };

            self.expect(&Token::LParen);
            let items = self.parse_list_expr();
            let list = if items.len() == 1 {
                items.into_iter().next().unwrap()
            } else {
                Expr::ArrayLit(items)
            };
            self.expect(&Token::RParen);
            let body = self.parse_brace_block();
            let continue_body = self.try_parse_continue();
            return Stmt::Foreach {
                var,
                is_my,
                list,
                body,
                continue_body,
                label,
            };
        }

        if self.at(&Token::LParen) {
            self.pos += 1;

            // Check if it's C-style for or foreach
            // C-style: for (init; cond; step) { }
            // Foreach: for (list) { }

            // Look ahead to determine which kind
            let saved = self.pos;

            // Try to detect C-style by looking for semicolons
            let mut depth = 1;
            let mut has_semi = false;
            let mut scan = self.pos;
            while scan < self.tokens.len() && depth > 0 {
                match &self.tokens[scan] {
                    Token::LParen => depth += 1,
                    Token::RParen => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    Token::Semi if depth == 1 => {
                        has_semi = true;
                        break;
                    }
                    _ => {}
                }
                scan += 1;
            }

            if has_semi {
                // C-style for
                let init = if self.at(&Token::Semi) {
                    None
                } else if self.at(&Token::My) {
                    Some(Box::new(self.parse_my_decl_no_semi()))
                } else {
                    Some(Box::new(Stmt::Expr(self.parse_expr())))
                };
                self.expect(&Token::Semi);

                let cond = if self.at(&Token::Semi) {
                    None
                } else {
                    // Same defined() auto-wrap as `while` for `each`,
                    // `readline`, and `<FH>` in the cond slot of
                    // C-style for. Without this, `for(; $k=each(@a) ;)`
                    // would exit on the first iteration because the
                    // first key (0) is falsy. op/each_array 55-57.
                    let raw = self.parse_expr();
                    Some(wrap_iter_cond_with_defined(raw))
                };
                self.expect(&Token::Semi);

                let step = if self.at(&Token::RParen) {
                    None
                } else {
                    Some(self.parse_expr())
                };
                self.expect(&Token::RParen);

                let body = self.parse_brace_block();
                return Stmt::For {
                    init,
                    cond,
                    step,
                    body,
                    label,
                };
            } else {
                // Foreach style: for (list) { }
                let items = self.parse_list_expr();
                let list = if items.len() == 1 {
                    items.into_iter().next().unwrap()
                } else {
                    Expr::ArrayLit(items)
                };
                self.expect(&Token::RParen);
                let body = self.parse_brace_block();
                let continue_body = self.try_parse_continue();
                return Stmt::Foreach {
                    var: "_".to_string(),
                    is_my: false,
                    list,
                    body,
                    continue_body,
                    label,
                };
            }
        }

        // Foreach with $_ implicit
        if self.at(&Token::LParen) {
            // Already handled above
        }

        // Default: treat as foreach
        let list = self.parse_expr();
        let body = self.parse_brace_block();
        let continue_body = self.try_parse_continue();
        Stmt::Foreach {
            var: "_".to_string(),
            is_my: false,
            list,
            body,
            continue_body,
            label,
        }
    }

    fn parse_paren_expr(&mut self) -> Expr {
        self.expect(&Token::LParen);
        let expr = self.parse_expr();
        self.expect(&Token::RParen);
        expr
    }

    fn parse_brace_block(&mut self) -> Vec<Stmt> {
        self.expect(&Token::LBrace);
        let body = self.parse_block_body();
        self.expect(&Token::RBrace);
        body
    }

    fn parse_block_body(&mut self) -> Vec<Stmt> {
        let mut stmts = Vec::new();
        let mut last_line = 0;
        while !self.at(&Token::RBrace) && !self.at(&Token::EOF) {
            if self.eat(&Token::Semi) {
                continue;
            }
            let line = self.current_line();
            if line != 0 && line != last_line {
                while let Some((idx, _)) = self.file_overrides.front() {
                    if *idx <= self.pos {
                        let (_, f) = self.file_overrides.pop_front().unwrap();
                        stmts.push(Stmt::FileMark(f));
                    } else {
                        break;
                    }
                }
                stmts.push(Stmt::LineMark(line));
                last_line = line;
            }
            if let Some(stmt) = self.parse_stmt() {
                stmts.push(stmt);
            }
        }
        stmts
    }

    fn parse_sub_decl(&mut self) -> Stmt {
        self.parse_sub_decl_inner(false)
    }

    fn parse_sub_decl_inner(&mut self, is_my_sub: bool) -> Stmt {
        let name = if let Token::Ident(name) = self.tok() {
            let n = name.clone();
            self.pos += 1;
            n
        } else {
            String::new()
        };
        // Capture prototype into `params` as a single-element string like `"$$@"`
        // so the interpreter can enforce per-arg context for classic sigil
        // prototypes (needed so `is(reverse("abc"),...)` sees scalar reverse).
        let mut proto: Vec<String> = Vec::new();
        if self.at(&Token::LParen) {
            self.pos += 1;
            let mut s = String::new();
            while !self.at(&Token::RParen) && !self.at(&Token::EOF) {
                match self.tok() {
                    // `$@` / `$%` inside a prototype lex as Token::ScalarVar("@")
                    // / ScalarVar("%") (the special vars). In proto context the
                    // second char is really a sigil, not part of the var name —
                    // `sub ok ($@)` means "scalar + rest-as-array", not "scalar
                    // var `$@`". Push both chars.
                    Token::ScalarVar(name) if name == "@" || name == "%" || name == "$" => {
                        s.push('$');
                        s.push_str(name);
                    }
                    Token::ScalarVar(_) => s.push('$'),
                    Token::ArrayVar(_) => s.push('@'),
                    Token::HashVar(_) => s.push('%'),
                    // `$$@` lexes as ScalarDeref("") + ArrayVar("") — treat
                    // each double-sigil as two `$`s for prototype purposes.
                    Token::ScalarDeref(_) => s.push_str("$$"),
                    Token::ArrayDeref(_) => s.push_str("$@"),
                    Token::HashDeref(_) => s.push_str("$%"),
                    Token::Backslash => s.push('\\'),
                    Token::Semi => s.push(';'),
                    Token::StringRepeat => s.push('*'),
                    Token::Ident(x) if x == "_" => s.push('_'),
                    _ => {}
                }
                self.pos += 1;
            }
            self.eat(&Token::RParen);
            // Push even an empty prototype string — `sub f ()` has a
            // declared (empty) prototype, distinct from `sub f` (no
            // prototype). prototype(\&f) returns "" vs undef
            // respectively. comp/proto. Encode with leading `(` so the
            // interpreter can distinguish prototype-params from regular
            // signature params.
            proto.push(format!("({s}"));
        }

        // Parse attributes — record `:lvalue` so the sub can be called
        // as an assignment target. Skip the rest.
        let mut is_lvalue = false;
        while self.eat(&Token::Colon) {
            if let Token::Ident(name) = self.tok() {
                if name == "lvalue" {
                    is_lvalue = true;
                }
                self.pos += 1;
            }
        }

        // `sub NAME;` or `sub NAME (PROTO);` — forward declaration with no
        // body. Accept that by consuming the trailing semicolon and emitting
        // an empty-body Sub. We still install the name so later calls parse
        // as sub calls. `sub;` / `sub ($) ;` / `sub` followed by anything
        // that isn't a block when the sub is anonymous → "Illegal
        // declaration of anonymous subroutine". op/anonsub 1-4.
        let body = if self.at(&Token::Semi) {
            if name.is_empty() {
                let line = self.current_line();
                self.error = Some(format!(
                    "Illegal declaration of anonymous subroutine at {{FILE}} line {line}.\n"
                ));
            }
            self.pos += 1;
            Vec::new()
        } else if self.at(&Token::LBrace) {
            self.parse_brace_block()
        } else {
            if name.is_empty() {
                let line = self.current_line();
                self.error = Some(format!(
                    "Illegal declaration of anonymous subroutine at {{FILE}} line {line}.\n"
                ));
            }
            Vec::new()
        };
        Stmt::Sub {
            name,
            params: proto,
            body,
            is_lvalue,
            is_my_sub,
        }
    }

    fn parse_my_decl(&mut self) -> Stmt {
        // `my sub NAME { ... }` — the `lexical_subs` feature. We don't
        // model true lexical scoping (the sub stays globally callable
        // for simplicity), but we DO record the my-sub flag so DB-magic
        // eval skips the my-sub caller's lexical chain. That replays
        // reference perl's known bug where `my sub f { DB::do_eval(...) }`
        // can't reach f's captured lexicals from inside DB::do_eval's
        // eval STRING — exercised by op/eval TODO tests 98–101.
        if matches!(self.tok(), Token::Sub) {
            self.pos += 1;
            return self.parse_sub_decl_inner(true);
        }
        // Reject `my $$x`, `my @$x`, `my %$x`, `my $$$x`, etc.
        // Reference perl errors: `Can't declare scalar dereference in
        // "my" at FILE line N`. Detect a deref token (ScalarDeref /
        // ArrayDeref / HashDeref) right after `my` (or `my (` ).
        let probe_pos = if matches!(self.tok(), Token::LParen) {
            self.pos + 1
        } else {
            self.pos
        };
        let bad = match self.tokens.get(probe_pos) {
            Some(Token::ScalarDeref(_)) => Some("scalar"),
            Some(Token::ArrayDeref(_)) => Some("array"),
            Some(Token::HashDeref(_)) => Some("hash"),
            _ => None,
        };
        if let Some(kind) = bad {
            // Consume the offending tokens up to a sane point so the
            // surrounding parser can keep going (the eval body using us
            // catches the die into $@).
            // Skip the deref token (and any following ident).
            self.pos = probe_pos + 1;
            // Eat balanced parens if we opened any.
            if matches!(self.tok(), Token::RParen) {
                self.pos += 1;
            }
            let line = self.token_lines.get(probe_pos).copied().unwrap_or(0);
            let file = "{FILE}".to_string();
            let msg =
                format!("Can't declare {kind} dereference in \"my\" at {file} line {line}.\n");
            // Surface as a parse error — the eval string boundary turns
            // this into $@.
            self.error.get_or_insert(msg);
            return Stmt::Nop;
        }
        // Reject `my $^X`, `my ${^XYZ}`, `my $_`, etc. Reference perl
        // errors with `Can't use global $X in "my" at FILE line N`
        // for any of the punctuation/caret special variables.
        if let Some(Token::ScalarVar(name)) = self.tokens.get(probe_pos)
            && (name.starts_with('^') || name == "_")
        {
            let line = self.token_lines.get(probe_pos).copied().unwrap_or(0);
            let file = "{FILE}".to_string();
            let msg = format!("Can't use global ${name} in \"my\" at {file} line {line}.\n");
            // Skip past the var token; if we opened a paren, eat the close.
            self.pos = probe_pos + 1;
            if matches!(self.tok(), Token::RParen) {
                self.pos += 1;
            }
            self.error.get_or_insert(msg);
            return Stmt::Nop;
        }
        let (vars, list_ctx) = self.parse_var_list();
        Stmt::My(vars, list_ctx)
    }

    fn parse_my_decl_no_semi(&mut self) -> Stmt {
        self.eat(&Token::My);
        let (vars, list_ctx) = self.parse_var_list();
        Stmt::My(vars, list_ctx)
    }

    fn parse_our_decl(&mut self) -> Stmt {
        let (mut vars, list_ctx) = self.parse_var_list();
        // `our $X++;` / `our $X--;` — Perl treats `our` as returning
        // the var as an lvalue, so the `++`/`--` follows the
        // declaration. Encode the post-op by stashing it as the var's
        // init expression; the Stmt::Our handler detects this self-
        // referential PostfixOp pattern and increments AFTER the
        // lexical alias is installed.
        if vars.len() == 1
            && vars[0].1.is_none()
            && !list_ctx
            && (matches!(self.tok(), Token::PlusPlus) || matches!(self.tok(), Token::MinusMinus))
        {
            let is_inc = matches!(self.tok(), Token::PlusPlus);
            self.pos += 1;
            let raw = vars[0].0.trim_start_matches('$').to_string();
            let var_expr = Expr::ScalarVar(raw);
            vars[0].1 = Some(Expr::PostfixOp(
                if is_inc {
                    crate::ast::PostfixOp::Inc
                } else {
                    crate::ast::PostfixOp::Dec
                },
                Box::new(var_expr),
            ));
        }
        Stmt::Our(vars, list_ctx)
    }

    fn parse_local_decl(&mut self) -> Stmt {
        // `local($$ref)`, `local(@$ref)`, `local(%$ref)` — perl raises
        // "Can't localize through a reference" at compile time. Detect
        // this at parse time by looking for a deref token right after
        // `local` or `local(`.
        let tok_after_paren = if matches!(self.tok(), Token::LParen) {
            self.peek(1)
        } else {
            self.tok()
        };
        if matches!(
            tok_after_paren,
            Token::ScalarDeref(_) | Token::ArrayDeref(_) | Token::HashDeref(_)
        ) {
            // Consume the whole local(...) / local ... form so the rest
            // of the program still parses; then emit a Die at runtime.
            if self.eat(&Token::LParen) {
                let mut depth = 1;
                while depth > 0 && !matches!(self.tok(), Token::Eof) {
                    match self.tok() {
                        Token::LParen => depth += 1,
                        Token::RParen => depth -= 1,
                        _ => {}
                    }
                    if depth > 0 {
                        self.pos += 1;
                    }
                }
                self.eat(&Token::RParen);
                // Optional `= EXPR` after the paren group.
                if self.eat(&Token::Assign) {
                    let _ = self.parse_expr();
                }
            } else {
                let _ = self.parse_unary();
                if self.eat(&Token::Assign) {
                    let _ = self.parse_expr();
                }
            }
            return Stmt::Die(vec![Expr::StringLit(
                "Can't localize through a reference".to_string(),
            )]);
        }
        // `local(@NAME[i,j,...])` / `local(%NAME{a,b,...})` — slice
        // localisation. Parse the key list and emit Stmt::LocalSlice.
        if matches!(self.tok(), Token::LParen)
            && (matches!(self.peek(1), Token::ArrayVar(_))
                || matches!(self.peek(1), Token::HashVar(_)))
            && matches!(self.peek(2), Token::LBracket | Token::LBrace)
        {
            let (var_name, is_hash) = match self.peek(1) {
                Token::ArrayVar(n) => (format!("@{n}"), false),
                Token::HashVar(n) => (format!("%{n}"), true),
                _ => unreachable!(),
            };
            self.pos += 3; // ( @arr [   or   ( %h {
            let mut keys = Vec::new();
            loop {
                let k = self.parse_expr();
                keys.push(k);
                if !self.eat(&Token::Comma) {
                    break;
                }
            }
            let close = if matches!(self.tok(), Token::RBracket) {
                Token::RBracket
            } else {
                Token::RBrace
            };
            self.expect(&close);
            self.expect(&Token::RParen);
            let val = if self.eat(&Token::Assign) {
                Some(self.parse_expr())
            } else {
                None
            };
            return Stmt::LocalSlice(var_name, keys, val);
        }
        // `local($NAME{KEY})` / `local($NAME[IDX])` — paren-wrapped
        // single-element form. Same shape as the bare form below, just
        // with surrounding parens that we strip.
        if matches!(self.tok(), Token::LParen)
            && matches!(self.peek(1), Token::ScalarVar(_))
            && matches!(self.peek(2), Token::LBrace | Token::LBracket)
        {
            let var_name = match self.peek(1) {
                Token::ScalarVar(n) => n.clone(),
                _ => unreachable!(),
            };
            let is_hash = matches!(self.peek(2), Token::LBrace);
            self.pos += 3; // skip `(`, ScalarVar, `{`/`[`
            let key = self.parse_expr();
            let close = if is_hash {
                &Token::RBrace
            } else {
                &Token::RBracket
            };
            self.expect(close);
            self.expect(&Token::RParen);
            let val = if self.eat(&Token::Assign) {
                Some(self.parse_expr())
            } else {
                None
            };
            if is_hash {
                return Stmt::LocalHashElem(var_name, key, val);
            }
            return Stmt::LocalHashElem(format!("@{var_name}"), key, val);
        }
        // `local $NAME{KEY}` / `local $NAME[IDX]` — hash/array element
        // localisation. Peek before falling through to parse_var_list,
        // which only understands bare `$`/`@`/`%` vars.
        if let Token::ScalarVar(name) = self.tok()
            && matches!(self.peek(1), Token::LBrace | Token::LBracket)
        {
            let var_name = name.clone();
            let is_hash = matches!(self.peek(1), Token::LBrace);
            self.pos += 2; // skip ScalarVar and `{` / `[`
            let key = self.parse_expr();
            let close = if is_hash {
                &Token::RBrace
            } else {
                &Token::RBracket
            };
            self.expect(close);
            let val = if self.eat(&Token::Assign) {
                Some(self.parse_expr())
            } else {
                None
            };
            if is_hash {
                return Stmt::LocalHashElem(var_name, key, val);
            }
            // Array element `local $a[0]` is rare — synthesise the equivalent
            // hash-elem form with an Integer-keyed array-slot; interpreter
            // handles both via `LocalHashElem` for now.
            return Stmt::LocalHashElem(format!("@{var_name}"), key, val);
        }
        let (vars, list_ctx) = self.parse_var_list();
        Stmt::Local(vars, list_ctx)
    }

    fn parse_var_list(&mut self) -> (Vec<(String, Option<Expr>)>, bool) {
        let mut vars = Vec::new();
        let mut list_ctx = false;

        if self.eat(&Token::LParen) {
            list_ctx = true;
            // my ($a, $b, @c, %d) = expr;
            let mut names = Vec::new();
            loop {
                match self.tok() {
                    Token::ScalarVar(name) => {
                        let n = name.clone();
                        self.pos += 1;
                        // `local (..., $NAME{KEY}, ...)` — hash element
                        // form. Encode the literal key into the name string
                        // with a NUL separator so Stmt::Local can recognise
                        // it and route to a single-element localisation.
                        if matches!(self.tok(), Token::LBrace) {
                            let save = self.pos;
                            self.pos += 1;
                            let key_str = match self.tok() {
                                Token::Ident(k) | Token::StringLit(k) => {
                                    let k = k.clone();
                                    self.pos += 1;
                                    if matches!(self.tok(), Token::RBrace) {
                                        self.pos += 1;
                                        Some(k)
                                    } else {
                                        self.pos = save;
                                        None
                                    }
                                }
                                _ => {
                                    self.pos = save;
                                    None
                                }
                            };
                            if let Some(k) = key_str {
                                names.push(format!("${n}\u{0}{k}"));
                            } else {
                                names.push(format!("${n}"));
                            }
                        } else {
                            names.push(format!("${n}"));
                        }
                    }
                    Token::ArrayVar(name) => {
                        names.push(format!("@{}", name));
                        self.pos += 1;
                    }
                    Token::HashVar(name) => {
                        names.push(format!("%{}", name));
                        self.pos += 1;
                    }
                    Token::UndefKw => {
                        // undef as placeholder in list destructuring
                        names.push("$_undef_placeholder".to_string());
                        self.pos += 1;
                    }
                    Token::Glob(name) => {
                        // `local(*FH) = ...` — prefix `*` distinguishes it
                        // from scalar/array/hash slots in later handling.
                        names.push(format!("*{}", name));
                        self.pos += 1;
                    }
                    _ => break,
                }
                if !self.eat(&Token::Comma) {
                    break;
                }
            }
            self.expect(&Token::RParen);

            if self.eat(&Token::Assign) {
                let expr = self.parse_expr();
                // First var gets the assignment
                for (i, name) in names.into_iter().enumerate() {
                    if i == 0 {
                        vars.push((name, Some(expr.clone())));
                    } else {
                        vars.push((name, None));
                    }
                }
            } else {
                for name in names {
                    vars.push((name, None));
                }
            }
        } else {
            // Single variable: my $x = expr; or local *FH;
            let name = match self.tok() {
                Token::ScalarVar(name) => {
                    let n = format!("${}", name);
                    self.pos += 1;
                    n
                }
                Token::ArrayVar(name) => {
                    let n = format!("@{}", name);
                    self.pos += 1;
                    n
                }
                Token::HashVar(name) => {
                    let n = format!("%{}", name);
                    self.pos += 1;
                    n
                }
                Token::Glob(name) => {
                    // `local *FH;` — typeglob localisation. Prefix `*`
                    // distinguishes it from scalar/array/hash slots so
                    // Stmt::Local recognises the typeglob form. op/yadayada
                    // tests 32-34 (`local *STDOUT`).
                    let n = format!("*{}", name);
                    self.pos += 1;
                    n
                }
                _ => return (vars, list_ctx),
            };

            // Variable attributes (`:NAME`, `:NAME(arg)` etc.). vanilla
            // perl rejects `:shared` as a syntax error unless
            // `use threads::shared` is in scope. We don't model
            // threads::shared, so emit the syntax error so eval q{...}
            // fails and any sub containing it stays undefined. op/state
            // tests 41-58 (stateful_attr never registers, the call to
            // it then dies via failed_eval_subs).
            while self.eat(&Token::Colon) {
                if let Token::Ident(attr_name) = self.tok() {
                    let attr = attr_name.clone();
                    if attr == "shared" {
                        let line = self.current_line();
                        let near = format!("{} :", name);
                        self.error = Some(format!(
                            "syntax error at {{FILE}} line {line}, near \"{near}\"\n"
                        ));
                        while !matches!(self.tok(), Token::Semi | Token::EOF | Token::RBrace) {
                            self.pos += 1;
                        }
                        return (vars, list_ctx);
                    }
                    self.pos += 1;
                    if self.eat(&Token::LParen) {
                        let mut depth = 1;
                        while depth > 0 {
                            match self.tok() {
                                Token::LParen => {
                                    depth += 1;
                                    self.pos += 1;
                                }
                                Token::RParen => {
                                    depth -= 1;
                                    self.pos += 1;
                                }
                                Token::EOF => break,
                                _ => self.pos += 1,
                            }
                        }
                    }
                } else {
                    break;
                }
            }

            let init = if self.eat(&Token::Assign) {
                Some(self.parse_expr())
            } else {
                None
            };
            vars.push((name, init));
        }

        (vars, list_ctx)
    }

    fn parse_print_stmt(&mut self, is_say: bool) -> Stmt {
        // print [FILEHANDLE] LIST
        // Can also be: print +(...) to force list context
        let has_plus = self.eat(&Token::Plus);

        // `print(LIST)` — explicit function-call form. The parens
        // delimit args; anything after `)` (e.g. `, exit`) belongs to
        // the enclosing comma expression, not print's arg list.
        if !has_plus && matches!(self.tok(), Token::LParen) {
            self.pos += 1;
            let args = self.parse_list_expr();
            self.eat(&Token::RParen);
            // `, exit` (or any other comma-chained expression) after
            // print's `)` must still execute. Park it as a tail
            // expression on this statement so exec_print returning
            // signals propagation.
            let mut tail: Vec<Expr> = Vec::new();
            while self.eat(&Token::Comma) || self.eat(&Token::FatComma) {
                if self.at(&Token::Semi) || self.at(&Token::EOF) {
                    break;
                }
                tail.push(self.parse_expr());
            }
            let stmt = if is_say {
                Stmt::Say(None, args)
            } else {
                Stmt::Print(None, args)
            };
            if tail.is_empty() {
                return self.maybe_postfix(stmt);
            }
            // Wrap into a block: print first, then evaluate tail
            // expressions in order so a trailing `exit` etc. fires.
            let mut block: Vec<Stmt> = vec![stmt];
            for t in tail {
                block.push(Stmt::Expr(t));
            }
            return self.maybe_postfix(Stmt::Block(block));
        }

        let filehandle = if !has_plus {
            // Check if first token is a bareword (filehandle)
            if let Token::Ident(name) = self.tok() {
                // If followed by a comma or expression, it's a filehandle
                let saved = self.pos;
                let fh_name = name.clone();
                self.pos += 1;

                // Check if it's actually a filehandle. A trailing `;`/EOF
                // after an all-caps identifier still counts as a filehandle —
                // `print STDOUT;` should print `$_` *to* STDOUT, not print
                // the literal "STDOUT".
                let is_fh_name = matches!(fh_name.as_str(), "STDOUT" | "STDERR" | "STDIN")
                    || fh_name.chars().all(|c| c.is_ascii_uppercase() || c == '_');
                // After consuming the bareword, if the next token is one
                // that cleanly starts an expression *with no comma between*,
                // treat the bareword as a filehandle. Perl's actual rule is
                // "no comma between FH and first list element" — so anything
                // that begins a valid expression (not a comma/operator)
                // counts.
                // After consuming the bareword, treat it as a filehandle
                // only when the following token unambiguously starts a new
                // expression that isn't a function-call paren. Specifically:
                // `print fname(args)` should be `print(fname(args))`, not
                // `print fname (args)`.
                let next_starts_expr = matches!(
                    self.tok(),
                    Token::StringLit(_)
                        | Token::InterpString(_)
                        | Token::ScalarVar(_)
                        | Token::ArrayVar(_)
                        | Token::HashVar(_)
                );
                // `print BAREWORD ... REST` — `...` between barewords is
                // the range operator, not a filehandle terminator. Treat
                // BAREWORD as part of the print list. op/yadayada test 33.
                let next_is_range_op =
                    self.at(&Token::DotDot) || matches!(self.tok(), Token::Ident(n) if n == "...");
                if next_is_range_op {
                    self.pos = saved;
                    None
                } else if self.at(&Token::Arrow) {
                    // `print A->foo` — `A` is a class name (method-call
                    // receiver), not a filehandle. Reference perl
                    // resolves the bareword to a class when followed by
                    // `->`. Without this, `print A->foo` parses as
                    // `print A (->foo)` with A as the filehandle.
                    self.pos = saved;
                    None
                } else if self.at(&Token::FatComma) {
                    // `print FOO => …` — not a filehandle call, `FOO` is a
                    // bareword hash key.
                    self.pos = saved;
                    None
                } else if self.at(&Token::Semi) || self.at(&Token::EOF) {
                    if is_fh_name {
                        Some(Expr::StringLit(fh_name))
                    } else {
                        self.pos = saved;
                        None
                    }
                } else if is_fh_name {
                    // Uppercase bareword followed by an expression — FH.
                    Some(Expr::StringLit(fh_name))
                } else if next_starts_expr
                    && !matches!(
                        fh_name.as_str(),
                        // Lowercase barewords that are list-ops / named-unaries
                        // shouldn't be treated as filehandles (`print scalar
                        // @x, ...`, `print sort @x`, etc.).
                        "scalar"
                            | "sort"
                            | "reverse"
                            | "map"
                            | "grep"
                            | "join"
                            | "split"
                            | "keys"
                            | "values"
                            | "each"
                            | "length"
                            | "ref"
                            | "defined"
                            | "exists"
                            | "delete"
                            | "chr"
                            | "ord"
                            | "int"
                            | "abs"
                            | "sqrt"
                            | "sprintf"
                            | "uc"
                            | "lc"
                            | "ucfirst"
                            | "lcfirst"
                            | "hex"
                            | "oct"
                            | "chop"
                            | "chomp"
                            | "wantarray"
                            | "caller"
                            | "die"
                            | "warn"
                            | "pop"
                            | "shift"
                            | "push"
                            | "unshift"
                            | "splice"
                    )
                    && !self.known_subs.contains(&fh_name)
                {
                    Some(Expr::StringLit(fh_name))
                } else {
                    self.pos = saved;
                    None
                }
            } else if let Token::ScalarVar(name) = self.tok() {
                // print $fh EXPR — scalar var as filehandle if followed by an expression
                // (not by an operator like comma, semicolon, etc.)
                let saved = self.pos;
                let var_name = name.clone();
                self.pos += 1;
                let next_is_expr = matches!(
                    self.tok(),
                    Token::StringLit(_)
                        | Token::InterpString(_)
                        | Token::ScalarVar(_)
                        | Token::ArrayVar(_)
                        | Token::Integer(_)
                        | Token::Float(_)
                        | Token::LParen
                );
                if next_is_expr {
                    Some(Expr::ScalarVar(var_name))
                } else {
                    self.pos = saved;
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        let args = self.parse_list_expr();
        if is_say {
            Stmt::Say(filehandle, args)
        } else {
            Stmt::Print(filehandle, args)
        }
    }

    fn parse_list_expr(&mut self) -> Vec<Expr> {
        let mut exprs = Vec::new();
        if self.at_list_end() {
            return exprs;
        }
        exprs.push(self.parse_expr());
        while self.eat(&Token::Comma) || self.eat(&Token::FatComma) {
            // Skip extra commas — `f(1, , 'a')` is Perl-legal and the
            // empty slot is silently dropped, not coerced to undef.
            // op/exists_sub test 10 (`ok( defined &t5, , 't5 defined' )`).
            while self.eat(&Token::Comma) || self.eat(&Token::FatComma) {}
            if self.at_list_end() {
                break;
            }
            exprs.push(self.parse_expr());
        }
        exprs
    }

    /// A token that terminates a list-context argument list without being
    /// consumed. Includes the usual closers plus Perl's postfix statement
    /// modifiers (`if`, `unless`, `while`, `until`, `for`, `foreach`) so
    /// `die if $@;` parses as `die` + postfix-if, not `die(if ...)`.
    fn at_list_end(&self) -> bool {
        matches!(
            self.tok(),
            Token::Semi
                | Token::EOF
                | Token::RBrace
                | Token::RParen
                | Token::RBracket
                | Token::If
                | Token::Unless
                | Token::While
                | Token::Until
                | Token::For
                | Token::Foreach
        )
    }

    fn parse_use(&mut self) -> Stmt {
        // use Module; or use Module qw(...); or use Module LIST;
        let module = if let Token::Ident(name) = self.tok() {
            let n = name.clone();
            self.pos += 1;
            n
        } else if let Token::Float(_) | Token::Integer(_) = self.tok() {
            // use 5.010; — version requirement, skip
            self.pos += 1;
            // Skip trailing `.` + numbers (for `use v5.27.0` already handled
            // as two tokens) and fall through to a no-op.
            while matches!(self.tok(), Token::Dot | Token::Integer(_) | Token::Float(_)) {
                self.pos += 1;
            }
            self.eat(&Token::Semi);
            return Stmt::Nop;
        } else {
            self.eat(&Token::Semi);
            return Stmt::Nop;
        };

        // `use v5.27;` — v-string version requirement. The lexer splits it
        // into `Ident("v5")`, `Dot`, `Integer(27)` — swallow the remaining
        // version components and skip the stmt entirely.
        if module.starts_with('v')
            && module.len() >= 2
            && module[1..].chars().all(|c| c.is_ascii_digit())
        {
            while matches!(self.tok(), Token::Dot | Token::Integer(_) | Token::Float(_)) {
                self.pos += 1;
            }
            self.eat(&Token::Semi);
            return Stmt::Nop;
        }

        let args = if self.at(&Token::Semi) || self.at(&Token::EOF) {
            Vec::new()
        } else {
            self.parse_list_expr()
        };
        // `current_line()` here refers to the line of the next token —
        // for a typical `use Module qw(...);` that's the `;` line, which
        // is what reference perl blames on a missing-module abort.
        let end_line = self.current_line();
        self.eat(&Token::Semi);
        Stmt::Use(module, args, end_line)
    }

    // --- Expression parsing with precedence climbing ---

    pub fn parse_expr(&mut self) -> Expr {
        // Perl precedence: `not`, `and`, `or`/`xor` are lower-precedence
        // than `=` (so `$r = expr or die` parses as `($r = expr) or die`).
        // Walk down low → high: or/xor → and → not → assignment → ternary
        // → … → primary.
        self.parse_or()
    }

    fn parse_assign(&mut self) -> Expr {
        let left = self.parse_ternary();

        match self.tok() {
            Token::Assign => {
                self.pos += 1;
                let right = self.parse_assign();
                Expr::Assign(Box::new(left), Box::new(right))
            }
            Token::PlusAssign => {
                self.pos += 1;
                let right = self.parse_assign();
                Expr::OpAssign(BinOp::Add, Box::new(left), Box::new(right))
            }
            Token::MinusAssign => {
                self.pos += 1;
                let right = self.parse_assign();
                Expr::OpAssign(BinOp::Sub, Box::new(left), Box::new(right))
            }
            Token::StarAssign => {
                self.pos += 1;
                let right = self.parse_assign();
                Expr::OpAssign(BinOp::Mul, Box::new(left), Box::new(right))
            }
            Token::SlashAssign => {
                self.pos += 1;
                let right = self.parse_assign();
                Expr::OpAssign(BinOp::Div, Box::new(left), Box::new(right))
            }
            Token::PercentAssign => {
                self.pos += 1;
                let right = self.parse_assign();
                Expr::OpAssign(BinOp::Mod, Box::new(left), Box::new(right))
            }
            Token::DotAssign => {
                self.pos += 1;
                let right = self.parse_assign();
                Expr::OpAssign(BinOp::Concat, Box::new(left), Box::new(right))
            }
            Token::PowerAssign => {
                self.pos += 1;
                let right = self.parse_assign();
                Expr::OpAssign(BinOp::Pow, Box::new(left), Box::new(right))
            }
            Token::StringRepeatAssign => {
                self.pos += 1;
                let right = self.parse_assign();
                Expr::OpAssign(BinOp::Repeat, Box::new(left), Box::new(right))
            }
            Token::LogOrAssign => {
                self.pos += 1;
                let right = self.parse_assign();
                Expr::OpAssign(BinOp::LogOr, Box::new(left), Box::new(right))
            }
            Token::LogAndAssign => {
                self.pos += 1;
                let right = self.parse_assign();
                Expr::OpAssign(BinOp::LogAnd, Box::new(left), Box::new(right))
            }
            Token::DefOrAssign => {
                self.pos += 1;
                let right = self.parse_assign();
                Expr::OpAssign(BinOp::DefOr, Box::new(left), Box::new(right))
            }
            Token::LogXorAssign => {
                self.pos += 1;
                let right = self.parse_assign();
                Expr::OpAssign(BinOp::Xor, Box::new(left), Box::new(right))
            }
            Token::BitOrAssign => {
                self.pos += 1;
                let right = self.parse_assign();
                Expr::OpAssign(BinOp::BitOr, Box::new(left), Box::new(right))
            }
            Token::BitAndAssign => {
                self.pos += 1;
                let right = self.parse_assign();
                Expr::OpAssign(BinOp::BitAnd, Box::new(left), Box::new(right))
            }
            Token::BitXorAssign => {
                self.pos += 1;
                let right = self.parse_assign();
                Expr::OpAssign(BinOp::BitXor, Box::new(left), Box::new(right))
            }
            Token::ShiftLeftAssign => {
                self.pos += 1;
                let right = self.parse_assign();
                Expr::OpAssign(BinOp::ShiftLeft, Box::new(left), Box::new(right))
            }
            Token::ShiftRightAssign => {
                self.pos += 1;
                let right = self.parse_assign();
                Expr::OpAssign(BinOp::ShiftRight, Box::new(left), Box::new(right))
            }
            _ => left,
        }
    }

    fn parse_ternary(&mut self) -> Expr {
        let cond = self.parse_range();
        if self.eat(&Token::Question) {
            let then = self.parse_assign();
            self.expect(&Token::Colon);
            let else_ = self.parse_assign();
            Expr::Ternary(Box::new(cond), Box::new(then), Box::new(else_))
        } else {
            cond
        }
    }

    fn parse_range(&mut self) -> Expr {
        // `or`/`and`/`not` are *lower* precedence than `..` — drop down
        // into the logical-or tier directly.
        let left = self.parse_log_or();
        // `..` and `...` are both range operators in expression
        // position. `...` differs only in flip-flop semantics, which
        // we don't model separately. op/yadayada 31+ ('A' ... 'D').
        let is_triple_dot = matches!(self.tok(), Token::Ident(n) if n == "...");
        if self.eat(&Token::DotDot) || is_triple_dot {
            if is_triple_dot {
                self.pos += 1;
            }
            let right = self.parse_log_or();
            Expr::Range(Box::new(left), Box::new(right))
        } else {
            left
        }
    }

    fn parse_or(&mut self) -> Expr {
        let mut left = self.parse_and();
        loop {
            if self.eat(&Token::Or) {
                let right = self.parse_and();
                left = Expr::BinOp(BinOp::Or, Box::new(left), Box::new(right));
            } else if self.eat(&Token::Xor) {
                let right = self.parse_and();
                left = Expr::BinOp(BinOp::Xor, Box::new(left), Box::new(right));
            } else {
                break;
            }
        }
        left
    }

    fn parse_and(&mut self) -> Expr {
        let mut left = self.parse_not();
        loop {
            if self.eat(&Token::And) {
                let right = self.parse_not();
                left = Expr::BinOp(BinOp::And, Box::new(left), Box::new(right));
            } else {
                break;
            }
        }
        left
    }

    fn parse_not(&mut self) -> Expr {
        if self.eat(&Token::Not) {
            let expr = self.parse_not();
            Expr::UnaryOp(UnaryOp::Not, Box::new(expr))
        } else {
            // `not`/`and`/`or` are lower precedence than `=` — drop down
            // into the assignment-and-ternary tier so `$r = … or die`
            // parses as `($r = …) or die` (matching Perl precedence).
            self.parse_assign()
        }
    }

    fn parse_log_or(&mut self) -> Expr {
        let mut left = self.parse_log_and();
        loop {
            if self.eat(&Token::LogOr) {
                let right = self.parse_log_and();
                left = Expr::BinOp(BinOp::LogOr, Box::new(left), Box::new(right));
            } else if self.eat(&Token::DefOr) {
                let right = self.parse_log_and();
                left = Expr::BinOp(BinOp::DefOr, Box::new(left), Box::new(right));
            } else if self.eat(&Token::LogXor) {
                // `^^` — logical XOR (Perl 5.40+). Same precedence as
                // `||` / `//`. Returns 1 if exactly one operand is
                // true, else "" (empty string). op/lop.
                let right = self.parse_log_and();
                left = Expr::BinOp(BinOp::Xor, Box::new(left), Box::new(right));
            } else {
                break;
            }
        }
        left
    }

    fn parse_log_and(&mut self) -> Expr {
        let mut left = self.parse_bit_or();
        loop {
            if self.eat(&Token::LogAnd) {
                let right = self.parse_bit_or();
                left = Expr::BinOp(BinOp::LogAnd, Box::new(left), Box::new(right));
            } else {
                break;
            }
        }
        left
    }

    fn parse_bit_or(&mut self) -> Expr {
        let mut left = self.parse_bit_xor();
        loop {
            if self.eat(&Token::BitOr) {
                let right = self.parse_bit_xor();
                left = Expr::BinOp(BinOp::BitOr, Box::new(left), Box::new(right));
            } else {
                break;
            }
        }
        left
    }

    fn parse_bit_xor(&mut self) -> Expr {
        let mut left = self.parse_bit_and();
        loop {
            if self.eat(&Token::BitXor) {
                let right = self.parse_bit_and();
                left = Expr::BinOp(BinOp::BitXor, Box::new(left), Box::new(right));
            } else {
                break;
            }
        }
        left
    }

    fn parse_bit_and(&mut self) -> Expr {
        let mut left = self.parse_comparison();
        loop {
            if self.eat(&Token::BitAnd) {
                let right = self.parse_comparison();
                left = Expr::BinOp(BinOp::BitAnd, Box::new(left), Box::new(right));
            } else {
                break;
            }
        }
        left
    }

    fn parse_comparison(&mut self) -> Expr {
        let mut left = self.parse_relational();
        // Loop to handle chained equality operators (==, !=, eq, ne)
        // which are mutually chainable in Perl 5.32+. Each iteration
        // also validates against the non-associative group
        // (<=>, cmp, ~~, isa) which can't be chained with anything.
        let nceq_tok = |t: &Token| {
            matches!(
                t,
                Token::Spaceship | Token::Cmp | Token::Smartmatch | Token::Isa
            )
        };
        let eqop_tok = |t: &Token| matches!(t, Token::NumEq | Token::NumNe | Token::Eq | Token::Ne);
        loop {
            let (op, op_name) = match self.tok() {
                Token::NumEq => (Some(BinOp::NumEq), "=="),
                Token::NumNe => (Some(BinOp::NumNe), "!="),
                Token::Spaceship => (Some(BinOp::Spaceship), "<=>"),
                Token::Eq => (Some(BinOp::StrEq), "eq"),
                Token::Ne => (Some(BinOp::StrNe), "ne"),
                Token::Cmp => (Some(BinOp::StrCmp), "cmp"),
                Token::Smartmatch => (Some(BinOp::Smartmatch), "~~"),
                Token::Isa => (Some(BinOp::Isa), "isa"),
                _ => (None, ""),
            };
            let Some(op) = op else { return left };
            self.pos += 1;
            let right = self.parse_relational();
            // Helpers (closures captured each iteration).
            let is_nceq = |o: &BinOp| {
                matches!(
                    o,
                    BinOp::Spaceship | BinOp::StrCmp | BinOp::Smartmatch | BinOp::Isa
                )
            };
            let is_eqop =
                |o: &BinOp| matches!(o, BinOp::NumEq | BinOp::NumNe | BinOp::StrEq | BinOp::StrNe);
            let is_relational = |o: &BinOp| {
                matches!(
                    o,
                    BinOp::NumLt
                        | BinOp::NumGt
                        | BinOp::NumLe
                        | BinOp::NumGe
                        | BinOp::StrLt
                        | BinOp::StrGt
                        | BinOp::StrLe
                        | BinOp::StrGe,
                )
            };
            // Recursively detect a chained-relational shape: either a
            // direct relational BinOp, or a LogAnd whose either side
            // resolves to one (produced by our chained-comparison rewrite).
            fn contains_relational(e: &Expr) -> bool {
                match e {
                    Expr::BinOp(
                        BinOp::NumLt
                        | BinOp::NumGt
                        | BinOp::NumLe
                        | BinOp::NumGe
                        | BinOp::StrLt
                        | BinOp::StrGt
                        | BinOp::StrLe
                        | BinOp::StrGe,
                        _,
                        _,
                    ) => true,
                    Expr::BinOp(BinOp::LogAnd, l, r) => {
                        contains_relational(l) || contains_relational(r)
                    }
                    _ => false,
                }
            }
            // Likewise for eq-class chains rewritten to LogAnd.
            fn contains_eqop(e: &Expr) -> bool {
                match e {
                    Expr::BinOp(
                        BinOp::NumEq | BinOp::NumNe | BinOp::StrEq | BinOp::StrNe,
                        _,
                        _,
                    ) => true,
                    Expr::BinOp(BinOp::LogAnd, l, r) => contains_eqop(l) || contains_eqop(r),
                    _ => false,
                }
            }
            // Non-assoc op cannot chain with anything that follows.
            if is_nceq(&op) && (nceq_tok(self.tok()) || eqop_tok(self.tok())) {
                let line = self.current_line();
                self.error = Some(format!(
                    "syntax error at {{FILE}} line {line}, near \"{op_name}\"\n"
                ));
            }
            // Eq-class op followed by a non-assoc op is illegal.
            if is_eqop(&op) && nceq_tok(self.tok()) {
                let line = self.current_line();
                self.error = Some(format!(
                    "syntax error at {{FILE}} line {line}, near \"{op_name}\"\n"
                ));
            }
            // Mixing with a relational chain on either side is illegal
            // when the current op is non-assoc or eq-class.
            let left_is_rel = contains_relational(&left);
            let right_is_rel = contains_relational(&right);
            let _ = is_relational; // kept for clarity above
            if (left_is_rel || right_is_rel) && (is_nceq(&op) || is_eqop(&op)) {
                let line = self.current_line();
                self.error = Some(format!(
                    "syntax error at {{FILE}} line {line}, near \"{op_name}\"\n"
                ));
            }
            // Non-assoc op mixed with an eq-class chain on either side.
            let left_is_eq = contains_eqop(&left);
            let right_is_eq = contains_eqop(&right);
            if (left_is_eq || right_is_eq) && is_nceq(&op) {
                let line = self.current_line();
                self.error = Some(format!(
                    "syntax error at {{FILE}} line {line}, near \"{op_name}\"\n"
                ));
            }
            // For chained eq-class ops, rewrite as && so each step
            // short-circuits and the result is the conjunction of
            // pairwise comparisons. We reuse the intermediate operand
            // expr by reference — naive (doesn't preserve single-
            // evaluation across side-effects), but matches the common
            // chained-comparison usage.
            let left_is_eqop_top = matches!(&left, Expr::BinOp(o, _, _) if is_eqop(o));
            if is_eqop(&op) && left_is_eqop_top {
                if let Expr::BinOp(_, _, b_box) = &left {
                    let b = (**b_box).clone();
                    let new_pair = Expr::BinOp(op.clone(), Box::new(b), Box::new(right));
                    left = Expr::BinOp(BinOp::LogAnd, Box::new(left), Box::new(new_pair));
                    continue;
                }
            }
            left = Expr::BinOp(op.clone(), Box::new(left), Box::new(right));
            // Only eq-class operators chain — for everything else stop
            // the loop so the caller (parse_bit_and / higher) handles
            // the remaining tokens.
            if !is_eqop(&op) {
                return left;
            }
        }
    }

    fn parse_relational(&mut self) -> Expr {
        let mut left = self.parse_shift();
        // Loop to handle chained comparisons like 32 <= $x <= 126.
        // For chained relational comparisons Perl evaluates them as
        // pairwise short-circuit AND: `$a < $b < $c` ⇒
        // `($a < $b) && ($b < $c)`. We rewrite to && at parse time.
        // op/cmpchain. (Note: doesn't preserve single-evaluation of
        // the shared operand across side effects.)
        let is_rel = |o: &BinOp| {
            matches!(
                o,
                BinOp::NumLt
                    | BinOp::NumGt
                    | BinOp::NumLe
                    | BinOp::NumGe
                    | BinOp::StrLt
                    | BinOp::StrGt
                    | BinOp::StrLe
                    | BinOp::StrGe,
            )
        };
        loop {
            let op = match self.tok() {
                Token::NumLt => Some(BinOp::NumLt),
                Token::NumGt => Some(BinOp::NumGt),
                Token::NumLe => Some(BinOp::NumLe),
                Token::NumGe => Some(BinOp::NumGe),
                Token::Lt => Some(BinOp::StrLt),
                Token::Gt => Some(BinOp::StrGt),
                Token::Le => Some(BinOp::StrLe),
                Token::Ge => Some(BinOp::StrGe),
                _ => None,
            };
            let Some(op) = op else { break };
            self.pos += 1;
            let right = self.parse_shift();
            let left_is_rel_top = matches!(&left, Expr::BinOp(o, _, _) if is_rel(o));
            if left_is_rel_top {
                if let Expr::BinOp(_, _, b_box) = &left {
                    let b = (**b_box).clone();
                    let new_pair = Expr::BinOp(op, Box::new(b), Box::new(right));
                    left = Expr::BinOp(BinOp::LogAnd, Box::new(left), Box::new(new_pair));
                    continue;
                }
            }
            left = Expr::BinOp(op, Box::new(left), Box::new(right));
        }
        left
    }

    fn parse_shift(&mut self) -> Expr {
        let mut left = self.parse_additive();
        loop {
            match self.tok() {
                Token::ShiftLeft => {
                    self.pos += 1;
                    left = Expr::BinOp(
                        BinOp::ShiftLeft,
                        Box::new(left),
                        Box::new(self.parse_additive()),
                    );
                }
                Token::ShiftRight => {
                    self.pos += 1;
                    left = Expr::BinOp(
                        BinOp::ShiftRight,
                        Box::new(left),
                        Box::new(self.parse_additive()),
                    );
                }
                _ => break,
            }
        }
        left
    }

    fn parse_additive(&mut self) -> Expr {
        let mut left = self.parse_multiplicative();
        loop {
            match self.tok() {
                Token::Plus => {
                    self.pos += 1;
                    left = Expr::BinOp(
                        BinOp::Add,
                        Box::new(left),
                        Box::new(self.parse_multiplicative()),
                    );
                }
                Token::Minus => {
                    self.pos += 1;
                    left = Expr::BinOp(
                        BinOp::Sub,
                        Box::new(left),
                        Box::new(self.parse_multiplicative()),
                    );
                }
                Token::Dot => {
                    self.pos += 1;
                    left = Expr::BinOp(
                        BinOp::Concat,
                        Box::new(left),
                        Box::new(self.parse_multiplicative()),
                    );
                }
                _ => break,
            }
        }
        left
    }

    fn parse_multiplicative(&mut self) -> Expr {
        let mut left = self.parse_regex_ops();
        loop {
            match self.tok() {
                Token::Star => {
                    self.pos += 1;
                    left =
                        Expr::BinOp(BinOp::Mul, Box::new(left), Box::new(self.parse_regex_ops()));
                }
                Token::Slash => {
                    self.pos += 1;
                    left =
                        Expr::BinOp(BinOp::Div, Box::new(left), Box::new(self.parse_regex_ops()));
                }
                Token::Percent => {
                    self.pos += 1;
                    left =
                        Expr::BinOp(BinOp::Mod, Box::new(left), Box::new(self.parse_regex_ops()));
                }
                Token::StringRepeat => {
                    self.pos += 1;
                    left = Expr::BinOp(
                        BinOp::Repeat,
                        Box::new(left),
                        Box::new(self.parse_regex_ops()),
                    );
                }
                _ => break,
            }
        }
        left
    }

    fn parse_regex_ops(&mut self) -> Expr {
        let left = self.parse_unary();
        match self.tok() {
            Token::Match => {
                self.pos += 1;
                // =~ /regex/ or =~ s/pat/repl/flags
                if let Token::Substitution(pat, repl, flags) = self.tok() {
                    let p = pat.clone();
                    let r = repl.clone();
                    let f = flags.clone();
                    self.pos += 1;
                    Expr::Substitution(Box::new(left), p, r, f)
                } else if let Token::RegexLit(pat, flags) | Token::QrLit(pat, flags) = self.tok() {
                    let p = pat.clone();
                    let f = flags.clone();
                    self.pos += 1;
                    Expr::RegexMatch(Box::new(left), p, f)
                } else if let Token::Transliterate(from, to, flags) = self.tok() {
                    let f = from.clone();
                    let t = to.clone();
                    let fl = flags.clone();
                    self.pos += 1;
                    Expr::Call(
                        "_tr_apply".to_string(),
                        vec![
                            left,
                            Expr::StringLit(f),
                            Expr::StringLit(t),
                            Expr::StringLit(fl),
                        ],
                    )
                } else if matches!(self.tok(), Token::ScalarVar(_) | Token::LParen) {
                    // `$str =~ $pat` or `$str =~ (expr)` — dynamic pattern.
                    let pat_expr = self.parse_unary();
                    Expr::Call("_regex_match_dyn".to_string(), vec![left, pat_expr])
                } else {
                    left
                }
            }
            Token::NotMatch => {
                self.pos += 1;
                if let Token::RegexLit(pat, flags) | Token::QrLit(pat, flags) = self.tok() {
                    let p = pat.clone();
                    let f = flags.clone();
                    self.pos += 1;
                    Expr::RegexNotMatch(Box::new(left), p, f)
                } else if let Token::Transliterate(from, to, flags) = self.tok() {
                    // !~ tr/from/to/ — apply transliteration, negate count
                    let f = from.clone();
                    let t = to.clone();
                    let fl = flags.clone();
                    self.pos += 1;
                    // For now, treat as the tr count expression
                    Expr::Call(
                        "_tr_count".to_string(),
                        vec![
                            left,
                            Expr::StringLit(f),
                            Expr::StringLit(t),
                            Expr::StringLit(fl),
                        ],
                    )
                } else if matches!(self.tok(), Token::ScalarVar(_) | Token::LParen) {
                    // `$str !~ $pat` or `$str !~ (expr)` — dynamic negated pattern
                    let pat_expr = self.parse_unary();
                    Expr::Call("_regex_not_match_dyn".to_string(), vec![left, pat_expr])
                } else {
                    left
                }
            }
            _ => left,
        }
    }

    fn parse_unary(&mut self) -> Expr {
        match self.tok() {
            Token::Minus => {
                self.pos += 1;
                // Recurse through parse_unary so consecutive unary
                // signs chain: `- -10` → `-(-10)` → 10, `-+5` → -5.
                // Precedence over `**` is preserved because parse_unary
                // falls through to parse_power for the operand.
                // op/negate 2-9.
                let expr = self.parse_unary();
                Expr::UnaryOp(UnaryOp::Neg, Box::new(expr))
            }
            Token::Plus => {
                self.pos += 1;
                let expr = self.parse_unary();
                Expr::UnaryOp(UnaryOp::Pos, Box::new(expr))
            }
            Token::LogNot => {
                self.pos += 1;
                let expr = self.parse_unary();
                Expr::UnaryOp(UnaryOp::LogNot, Box::new(expr))
            }
            Token::BitNot => {
                self.pos += 1;
                let expr = self.parse_unary();
                Expr::UnaryOp(UnaryOp::BitNot, Box::new(expr))
            }
            Token::PlusPlus => {
                self.pos += 1;
                // Recurse into parse_unary so chained prefix operators
                // (including `++my $x` / `++state $x`) reach the My/State
                // handlers. op/state test 19+.
                let expr = self.parse_unary();
                // `++` at end-of-input (or against a non-lvalue) is a
                // syntax error. Perl emits "syntax error" with details;
                // match the "syntax error" substring so eval captures it.
                if matches!(expr, Expr::Undef) && self.error.is_none() {
                    let line = self.current_line();
                    self.error = Some(format!("syntax error at {{FILE}} line {line}, at EOF\n"));
                }
                Expr::UnaryOp(UnaryOp::PreInc, Box::new(expr))
            }
            Token::MinusMinus => {
                self.pos += 1;
                let expr = self.parse_unary();
                if matches!(expr, Expr::Undef) && self.error.is_none() {
                    let line = self.current_line();
                    self.error = Some(format!("syntax error at {{FILE}} line {line}, at EOF\n"));
                }
                Expr::UnaryOp(UnaryOp::PreDec, Box::new(expr))
            }
            Token::Backslash => {
                self.pos += 1;
                // `\(LIST)` distributes the ref over each element — `\(1, 2,
                // @a, %h)` returns a list of refs (per-element), not a single
                // ref to the list. The parens themselves are the marker for
                // list-context backslash; bare `\@a` keeps array-ref form.
                // op/array tests 175-178 (`\(@q)` returns SCALAR refs to each
                // element, comparable to `\$q[$i]`) require this distribution.
                if self.at(&Token::LParen) {
                    self.pos += 1;
                    let mut items: Vec<Expr> = Vec::new();
                    if !self.at(&Token::RParen) {
                        loop {
                            let e = self.parse_expr();
                            items.push(e);
                            if !self.eat(&Token::Comma) && !self.eat(&Token::FatComma) {
                                break;
                            }
                            if self.at(&Token::RParen) {
                                break;
                            }
                        }
                    }
                    self.expect(&Token::RParen);
                    return Expr::Call("_distribute_backslash".to_string(), items);
                }
                let expr = self.parse_unary();
                Expr::Ref(Box::new(expr))
            }
            Token::BitAnd => {
                // &func() call syntax
                self.pos += 1;
                if let Token::Ident(name) = self.tok() {
                    let name = name.clone();
                    self.pos += 1;
                    let (had_parens, args) = if self.eat(&Token::LParen) {
                        let a = self.parse_list_expr();
                        self.expect(&Token::RParen);
                        (true, a)
                    } else {
                        (false, Vec::new())
                    };
                    // Distinguish `&NAME` from `&NAME()` (empty parens
                    // is still a call). Tag empty-parens calls with a
                    // sentinel `_amp_call_parens` first-arg so `exists`
                    // can reject `exists &NAME()` while still allowing
                    // `exists &NAME`. op/exists_sub.
                    //
                    // `&NAME` (no parens at all) inherits the caller's
                    // @_ as its args — Perl's pass-through call form.
                    // Tag with `_amp_call_inherit_args` so the
                    // interpreter forwards @_ only when called from
                    // inside a sub. op/args `&methimpl` tests.
                    if had_parens && args.is_empty() {
                        Expr::Call(
                            name,
                            vec![Expr::Call("_amp_call_parens".to_string(), Vec::new())],
                        )
                    } else if !had_parens {
                        Expr::Call(
                            name,
                            vec![Expr::Call("_amp_call_inherit_args".to_string(), Vec::new())],
                        )
                    } else {
                        Expr::Call(name, args)
                    }
                } else if let Token::ScalarVar(name) = self.tok() {
                    // `&$subref(args)` / `&$subref` — invoke the code ref
                    // in `$subref`. Maps to `$subref->(args)`; with no args
                    // the current `@_` is forwarded in Perl, but for most
                    // tests an empty arg list works.
                    let name = name.clone();
                    self.pos += 1;
                    let args = if self.eat(&Token::LParen) {
                        let a = self.parse_list_expr();
                        self.expect(&Token::RParen);
                        a
                    } else {
                        Vec::new()
                    };
                    Expr::CodeCall(Box::new(Expr::ScalarVar(name)), args)
                } else if self.at(&Token::LBrace) {
                    // `&{EXPR}(args)` / `&{EXPR}` — invoke the code ref
                    // produced by EXPR.
                    self.pos += 1;
                    let inner = self.parse_expr();
                    self.expect(&Token::RBrace);
                    let args = if self.eat(&Token::LParen) {
                        let a = self.parse_list_expr();
                        self.expect(&Token::RParen);
                        a
                    } else {
                        Vec::new()
                    };
                    Expr::CodeCall(Box::new(inner), args)
                } else {
                    // Regular bitwise-and as unary (take address)
                    let expr = self.parse_unary();
                    Expr::Ref(Box::new(expr))
                }
            }
            Token::Defined => {
                self.pos += 1;
                // `defined` has prototype `$`. With parens, accept any
                // expression (so `defined($false ? $x : @arr)` works). Without
                // parens, parse a single unary so it binds like other
                // prototype-$ builtins. Bare `defined` with no arg = `defined($_)`.
                let expr = if self.eat(&Token::LParen) {
                    if self.eat(&Token::RParen) {
                        Expr::ScalarVar("_".to_string())
                    } else {
                        let e = self.parse_expr();
                        self.eat(&Token::RParen);
                        e
                    }
                } else if matches!(
                    self.tok(),
                    Token::Semi
                        | Token::Comma
                        | Token::RParen
                        | Token::RBrace
                        | Token::RBracket
                        | Token::Question
                        | Token::Colon
                        | Token::EOF
                        | Token::Newline
                        | Token::FatComma
                ) {
                    Expr::ScalarVar("_".to_string())
                } else {
                    self.parse_unary()
                };
                Expr::Defined(Box::new(expr))
            }
            Token::Our => {
                // `our $T` in expression position — e.g.
                // `open our $T, "...";`. Treat as a global var reference
                // so open() / readline() route through the global slot.
                self.pos += 1;
                if let Token::ScalarVar(name) = self.tok() {
                    let n = name.clone();
                    self.pos += 1;
                    Expr::ScalarVar(n)
                } else if let Token::ArrayVar(name) = self.tok() {
                    let n = name.clone();
                    self.pos += 1;
                    Expr::ArrayVar(n)
                } else if let Token::HashVar(name) = self.tok() {
                    let n = name.clone();
                    self.pos += 1;
                    Expr::HashVar(n)
                } else if self.at(&Token::LParen) {
                    // `our (...)` — collect the names, return an array
                    // literal of bare var refs so a following `= LIST`
                    // does proper list-context destructure.
                    self.pos += 1;
                    let mut names = Vec::new();
                    loop {
                        match self.tok() {
                            Token::ScalarVar(name) => {
                                names.push(format!("${}", name));
                                self.pos += 1;
                            }
                            Token::ArrayVar(name) => {
                                names.push(format!("@{}", name));
                                self.pos += 1;
                            }
                            Token::HashVar(name) => {
                                names.push(format!("%{}", name));
                                self.pos += 1;
                            }
                            _ => break,
                        }
                        if !self.eat(&Token::Comma) {
                            break;
                        }
                    }
                    self.expect(&Token::RParen);
                    if names.is_empty() {
                        Expr::Undef
                    } else if names.len() == 1 {
                        let n = names.into_iter().next().unwrap();
                        if let Some(rest) = n.strip_prefix('@') {
                            Expr::ArrayVar(rest.to_string())
                        } else if let Some(rest) = n.strip_prefix('%') {
                            Expr::HashVar(rest.to_string())
                        } else {
                            let rest = n.strip_prefix('$').unwrap_or(&n).to_string();
                            Expr::ScalarVar(rest)
                        }
                    } else {
                        let exprs = names
                            .into_iter()
                            .map(|n| {
                                if let Some(rest) = n.strip_prefix('@') {
                                    Expr::ArrayVar(rest.to_string())
                                } else if let Some(rest) = n.strip_prefix('%') {
                                    Expr::HashVar(rest.to_string())
                                } else {
                                    let rest = n.strip_prefix('$').unwrap_or(&n).to_string();
                                    Expr::ScalarVar(rest)
                                }
                            })
                            .collect();
                        Expr::ArrayLit(exprs)
                    }
                } else {
                    Expr::Undef
                }
            }
            Token::My => {
                self.pos += 1;
                if let Token::ScalarVar(name) = self.tok() {
                    let n = name.clone();
                    self.pos += 1;
                    Expr::MyVar(n)
                } else if let Token::ArrayVar(name) = self.tok() {
                    // `my @name` in expression position — internal call
                    // that declares the lexical and returns Expr::ArrayVar
                    // so a following `= LIST` does list-assignment that
                    // returns the assigned list.
                    let n = name.clone();
                    self.pos += 1;
                    // Synthesize a Stmt::My so the var is declared as
                    // lexical, then return ArrayVar so `= LIST` works.
                    Expr::DoBlock(vec![
                        Stmt::My(vec![(format!("@{n}"), None)], false),
                        Stmt::Expr(Expr::ArrayVar(n)),
                    ])
                } else if let Token::HashVar(name) = self.tok() {
                    let n = name.clone();
                    self.pos += 1;
                    Expr::DoBlock(vec![
                        Stmt::My(vec![(format!("%{n}"), None)], false),
                        Stmt::Expr(Expr::HashVar(n)),
                    ])
                } else if self.at(&Token::LParen) {
                    // my (...) in expression context
                    // For now, just parse the first var
                    self.pos += 1;
                    let mut names = Vec::new();
                    loop {
                        match self.tok() {
                            Token::ScalarVar(name) => {
                                names.push(name.clone());
                                self.pos += 1;
                            }
                            Token::ArrayVar(name) => {
                                names.push(format!("@{}", name));
                                self.pos += 1;
                            }
                            Token::HashVar(name) => {
                                names.push(format!("%{}", name));
                                self.pos += 1;
                            }
                            _ => break,
                        }
                        if !self.eat(&Token::Comma) {
                            break;
                        }
                    }
                    self.expect(&Token::RParen);
                    if names.is_empty() {
                        Expr::Undef
                    } else if names.len() == 1 {
                        let n = names.into_iter().next().unwrap();
                        // Single `my (@arr)` / `my (%h)` should still be
                        // declared as the right kind of slot — same trick
                        // as the bare `my @arr` form below.
                        if let Some(rest) = n.strip_prefix('@') {
                            Expr::DoBlock(vec![
                                Stmt::My(vec![(format!("@{rest}"), None)], true),
                                Stmt::Expr(Expr::ArrayVar(rest.to_string())),
                            ])
                        } else if let Some(rest) = n.strip_prefix('%') {
                            Expr::DoBlock(vec![
                                Stmt::My(vec![(format!("%{rest}"), None)], true),
                                Stmt::Expr(Expr::HashVar(rest.to_string())),
                            ])
                        } else {
                            Expr::MyVar(n)
                        }
                    } else {
                        // Mixed list — convert each name to the right
                        // expression kind so the outer `= LIST` does a
                        // proper list-context destructure.
                        let exprs = names
                            .into_iter()
                            .map(|n| {
                                if let Some(rest) = n.strip_prefix('@') {
                                    Expr::DoBlock(vec![
                                        Stmt::My(vec![(format!("@{rest}"), None)], true),
                                        Stmt::Expr(Expr::ArrayVar(rest.to_string())),
                                    ])
                                } else if let Some(rest) = n.strip_prefix('%') {
                                    Expr::DoBlock(vec![
                                        Stmt::My(vec![(format!("%{rest}"), None)], true),
                                        Stmt::Expr(Expr::HashVar(rest.to_string())),
                                    ])
                                } else {
                                    Expr::MyVar(n)
                                }
                            })
                            .collect();
                        Expr::ArrayLit(exprs)
                    }
                } else {
                    Expr::Undef
                }
            }
            Token::State => {
                // `state $y = EXPR` in expression position — emit a
                // special _state_expr call so the interpreter can install
                // the state binding in the CURRENT (sub) scope rather than
                // a DoBlock-pushed child scope. Without this, the alias to
                // the persistent Rc is lost when the DoBlock scope pops,
                // so subsequent reads see undef. op/state test 48
                // (`my $x = state $y = 42`); opbasic/concat test 34
                // (`ref(state $y = "a $o b")`).
                self.pos += 1;
                if let Token::ScalarVar(name) = self.tok() {
                    let n = name.clone();
                    self.pos += 1;
                    let mut args = vec![Expr::StringLit(format!("${n}"))];
                    if self.eat(&Token::Assign) {
                        args.push(self.parse_assign());
                    }
                    Expr::Call("_state_expr".to_string(), args)
                } else if let Token::ArrayVar(name) = self.tok() {
                    let n = name.clone();
                    self.pos += 1;
                    Expr::DoBlock(vec![
                        Stmt::State(vec![(format!("@{n}"), None)], false),
                        Stmt::Expr(Expr::ArrayVar(n)),
                    ])
                } else if let Token::HashVar(name) = self.tok() {
                    let n = name.clone();
                    self.pos += 1;
                    Expr::DoBlock(vec![
                        Stmt::State(vec![(format!("%{n}"), None)], false),
                        Stmt::Expr(Expr::HashVar(n)),
                    ])
                } else {
                    Expr::Undef
                }
            }
            Token::Ident(name) if name.starts_with('-') && name.len() == 2 => {
                // File test operators: -e, -f, -d, etc.
                let op = name.clone();
                self.pos += 1;
                // Default operand: `$_`. Applies when the next token
                // can't start a primary expression. op/dor `-f // 0`.
                let needs_default = matches!(
                    self.tok(),
                    Token::Semi
                        | Token::Comma
                        | Token::RParen
                        | Token::RBrace
                        | Token::RBracket
                        | Token::Question
                        | Token::Colon
                        | Token::LogAnd
                        | Token::LogOr
                        | Token::DefOr
                        | Token::And
                        | Token::Or
                        | Token::NumEq
                        | Token::NumNe
                        | Token::NumLt
                        | Token::NumGt
                        | Token::NumLe
                        | Token::NumGe
                        | Token::Eq
                        | Token::Ne
                );
                let expr = if needs_default {
                    Expr::ScalarVar("_".to_string())
                } else if matches!(self.tok(), Token::Ident(n) if n.starts_with('-') && n.len() == 2)
                {
                    // Stacked file tests: `-t -e $null` parses as
                    // `-t (-e $null)`. Recurse through parse_unary so
                    // the inner `-e` is recognized as a file test and
                    // not treated as a bareword sub call.
                    // op/filetest_t (`!-t -e $null` chains).
                    self.parse_unary()
                } else {
                    self.parse_primary()
                };
                Expr::FileTest(op, Box::new(expr))
            }
            _ => self.parse_power(),
        }
    }

    fn parse_power(&mut self) -> Expr {
        let base = self.parse_postfix();
        if self.eat(&Token::Power) {
            let exp = self.parse_unary(); // right-associative
            Expr::BinOp(BinOp::Pow, Box::new(base), Box::new(exp))
        } else {
            base
        }
    }

    fn parse_postfix(&mut self) -> Expr {
        let mut expr = self.parse_primary();

        loop {
            match self.tok() {
                Token::PlusPlus => {
                    self.pos += 1;
                    expr = Expr::PostfixOp(PostfixOp::Inc, Box::new(expr));
                }
                Token::MinusMinus => {
                    self.pos += 1;
                    expr = Expr::PostfixOp(PostfixOp::Dec, Box::new(expr));
                }
                Token::LBracket => {
                    // Array subscript
                    self.pos += 1;
                    let first = self.parse_expr();
                    let mut indices = vec![first];
                    while self.eat(&Token::Comma) {
                        if matches!(self.tok(), Token::RBracket) {
                            break;
                        }
                        indices.push(self.parse_expr());
                    }
                    self.expect(&Token::RBracket);
                    let single_index = indices.len() == 1;
                    match &expr {
                        Expr::ScalarVar(name) | Expr::ArrayVar(name) | Expr::MyVar(name) => {
                            let name = name.clone();
                            if single_index {
                                expr = Expr::ArrayElement(
                                    name,
                                    Box::new(indices.into_iter().next().unwrap()),
                                );
                            } else {
                                expr = Expr::ArraySlice(name, indices);
                            }
                        }
                        // `%arr[i,j]` — array key/value slice. Returns
                        // (i, $arr[i], j, $arr[j]) in list context. Parsed
                        // as HashVar + `[…]` because the lexer already
                        // stripped the `%` sigil; promote to an AK-V slice
                        // helper call.
                        Expr::HashVar(name) => {
                            let name = name.clone();
                            let mut call_args = vec![Expr::StringLit(name)];
                            call_args.extend(indices);
                            expr = Expr::Call("_array_kvslice".to_string(), call_args);
                        }
                        // `${EXPR}[i]` — Perl shorthand for `(EXPR)->[i]`.
                        // The `${…}` block already evaluates to the scalar
                        // ref target; the `[i]` subscripts it as an array
                        // ref (same as `$$r[i]` / `$r->[i]`).
                        Expr::Call(n, call_args)
                            if n == "_scalar_block_deref" && call_args.len() == 1 =>
                        {
                            let inner = call_args[0].clone();
                            if single_index {
                                expr = Expr::ArrowElement(
                                    Box::new(inner),
                                    Box::new(indices.into_iter().next().unwrap()),
                                    ArrowKind::Array,
                                );
                            } else {
                                let mut args = vec![expr];
                                args.extend(indices);
                                expr = Expr::Call("_list_slice".to_string(), args);
                            }
                        }
                        // `$$ref[i]` — subscript of a scalar-deref is the
                        // shorthand for `$ref->[i]`. Without this, the
                        // fallback `_list_slice` treats `$$ref` as a single
                        // value and picks its [i]th "element" (usually
                        // undef).
                        Expr::ScalarDerefVar(name) => {
                            let name = name.clone();
                            if single_index {
                                expr = Expr::ArrowElement(
                                    Box::new(Expr::ScalarVar(name)),
                                    Box::new(indices.into_iter().next().unwrap()),
                                    ArrowKind::Array,
                                );
                            } else {
                                let mut args = vec![expr];
                                args.extend(indices);
                                expr = Expr::Call("_list_slice".to_string(), args);
                            }
                        }
                        // Chained subscript on an already-evaluated element
                        // (e.g. `$arr[0][1]`, `$h{k}[0]`, `$ref->[0][0]`):
                        // implicit arrow-deref the inner value as an array ref.
                        Expr::ArrayElement(_, _)
                        | Expr::HashElement(_, _)
                        | Expr::ArrowElement(_, _, _) => {
                            if single_index {
                                expr = Expr::ArrowElement(
                                    Box::new(expr),
                                    Box::new(indices.into_iter().next().unwrap()),
                                    ArrowKind::Array,
                                );
                            } else {
                                let mut args = vec![expr];
                                args.extend(indices);
                                expr = Expr::Call("_list_slice".to_string(), args);
                            }
                        }
                        _ => {
                            // `(LIST)[idx1, idx2, ...]` — list slice. Use a
                            // helper Call so the interpreter evaluates LIST
                            // in list context and picks out each index.
                            let mut args = vec![expr];
                            args.extend(indices);
                            expr = Expr::Call("_list_slice".to_string(), args);
                        }
                    }
                }
                Token::LBrace => {
                    // Hash subscript — but only if it looks like a subscript, not a block
                    let saved = self.pos;
                    self.pos += 1;

                    // Improved heuristic: scan ahead for the matching } to check if this
                    // is a hash subscript expression or a block with statements.
                    // A block contains statement keywords (if/unless/while/for) or
                    // semicolons before the closing brace. A subscript does not.
                    let first_is_value = matches!(
                        self.tok(),
                        Token::StringLit(_)
                            | Token::InterpString(_)
                            | Token::ScalarVar(_)
                            | Token::ArrayVar(_)
                            | Token::HashVar(_)
                            | Token::Integer(_)
                            | Token::Float(_)
                            | Token::Ident(_)
                            // Named-unary / list-op builtins as the
                            // first token of a hash key. Without these,
                            // `$h{ord("a")} = …` was treated as a block
                            // (since the heuristic only recognised
                            // value-shaped first tokens), so the hash
                            // assignment silently no-op'd.
                            | Token::Ord
                            | Token::Chr
                            | Token::Length
                            | Token::Lc
                            | Token::Uc
                            | Token::Lcfirst
                            | Token::Ucfirst
                            | Token::Hex
                            | Token::Oct
                            | Token::Int
                            | Token::Abs
                            | Token::Ref
                            | Token::Defined
                            | Token::Exists
                            | Token::Sprintf
                            | Token::Substr
                            | Token::Index
                            | Token::Rindex
                            | Token::Join
                            | Token::Eval
                            | Token::Caller
                            | Token::Wantarray
                            // Unary `-EXPR`, `+EXPR`, `!EXPR`, `\EXPR`
                            // as the first token of a hash key.
                            | Token::Minus
                            | Token::Plus
                            | Token::LogNot
                            | Token::Not
                            | Token::Backslash
                    );
                    let is_subscript = if first_is_value {
                        // Scan forward to find the matching } and check for block indicators
                        let mut scan = self.pos;
                        let mut depth = 1;
                        let mut has_block_indicator = false;
                        while scan < self.tokens.len() && depth > 0 {
                            match &self.tokens[scan] {
                                Token::LBrace => depth += 1,
                                Token::RBrace => {
                                    depth -= 1;
                                    if depth == 0 {
                                        break;
                                    }
                                }
                                Token::Semi
                                | Token::If
                                | Token::Unless
                                | Token::While
                                | Token::Until
                                | Token::For
                                | Token::Foreach
                                    if depth == 1 =>
                                {
                                    has_block_indicator = true;
                                    break;
                                }
                                _ => {}
                            }
                            scan += 1;
                        }
                        !has_block_indicator
                    } else {
                        false
                    };

                    if is_subscript {
                        // Perl auto-quotes a sole bareword inside `{...}`
                        // when used as a hash key — `$h{foo}` means
                        // `$h{"foo"}` regardless of whether `foo` names
                        // a sub. Detect the simple bareword case. Also
                        // accept named-operator tokens (e.g. `length`,
                        // `pos`) that lex as their own token so
                        // `$h{length}` etc. work. re/regexp op/pos
                        // tests 29-32.
                        let bareword_name: Option<String> = match self.tok() {
                            Token::Ident(n) => Some(n.clone()),
                            tok => named_op_token_name(tok),
                        };
                        let key = if let Some(n) = bareword_name
                            && matches!(self.tokens.get(self.pos + 1), Some(Token::RBrace))
                        {
                            self.pos += 1;
                            Expr::StringLit(n)
                        } else {
                            let first = self.parse_expr();
                            // `$h{a, b}` joins keys with $; (Perl semantics).
                            if matches!(self.tok(), Token::Comma) {
                                let mut items = vec![first];
                                while self.eat(&Token::Comma) {
                                    if matches!(self.tok(), Token::RBrace) {
                                        break;
                                    }
                                    items.push(self.parse_expr());
                                }
                                Expr::Call("_subscript_join".to_string(), items)
                            } else {
                                first
                            }
                        };
                        self.expect(&Token::RBrace);
                        match expr {
                            Expr::ScalarVar(name) | Expr::HashVar(name) | Expr::MyVar(name) => {
                                expr = Expr::HashElement(name, Box::new(key));
                            }
                            // `${EXPR}{k}` — `(EXPR)->{k}` — hash-ref
                            // subscript through a scalar-block deref.
                            Expr::Call(ref n, ref call_args)
                                if n == "_scalar_block_deref" && call_args.len() == 1 =>
                            {
                                let inner = call_args[0].clone();
                                expr = Expr::ArrowElement(
                                    Box::new(inner),
                                    Box::new(key),
                                    ArrowKind::Hash,
                                );
                            }
                            // `$$ref{k}` — hash subscript of a scalar
                            // deref is shorthand for `$ref->{k}`.
                            Expr::ScalarDerefVar(name) => {
                                expr = Expr::ArrowElement(
                                    Box::new(Expr::ScalarVar(name)),
                                    Box::new(key),
                                    ArrowKind::Hash,
                                );
                            }
                            // Chained subscript on an already-evaluated
                            // element (`$arr[0]{k}`, `$h{a}{b}`,
                            // `$ref->[0]{k}`): implicit arrow-deref the
                            // inner value as a hash ref.
                            Expr::ArrayElement(_, _)
                            | Expr::HashElement(_, _)
                            | Expr::ArrowElement(_, _, _) => {
                                expr = Expr::ArrowElement(
                                    Box::new(expr),
                                    Box::new(key),
                                    ArrowKind::Hash,
                                );
                            }
                            _ => {
                                expr = Expr::HashElement("_deref_".to_string(), Box::new(key));
                            }
                        }
                    } else {
                        self.pos = saved;
                        break;
                    }
                }
                Token::Arrow => {
                    self.pos += 1;
                    // Postfix deref forms: `->$*` (scalar), `->@*` (array
                    // flat), `->%*` (hash flat). These are equivalent to
                    // `${EXPR}` / `@{EXPR}` / `%{EXPR}`.
                    if let Token::ScalarVar(n) = self.tok()
                        && n == "*"
                    {
                        self.pos += 1;
                        // Wrap as a scalar block deref so a Value::ScalarRef
                        // is followed through to its inner value.
                        let inner = expr;
                        expr = Expr::Call("_scalar_block_deref".to_string(), vec![inner]);
                        continue;
                    }
                    // `->@*` — postfix array-flat deref. Lexer emits
                    // `ArrayVar("")` (no following name char) + `Star`.
                    if let Token::ArrayVar(n) = self.tok()
                        && n.is_empty()
                        && matches!(self.peek(1), Token::Star)
                    {
                        self.pos += 2;
                        let inner = expr;
                        expr = Expr::Call("_array_block_deref".to_string(), vec![inner]);
                        continue;
                    }
                    // `->@[INDEX_LIST]` — postfix array slice through ref.
                    // `->@{KEY_LIST}` — postfix hash slice through ref.
                    // The lexer turns `@{` into Token::ArrayBlockDerefOpen
                    // (and may consume the `{`); the `@[` form leaves
                    // `ArrayVar("") + LBracket`. Handle both.
                    if let Token::ArrayVar(n) = self.tok()
                        && n.is_empty()
                        && matches!(self.peek(1), Token::LBracket)
                    {
                        self.pos += 2;
                        let mut indices = vec![self.parse_expr()];
                        while self.eat(&Token::Comma) || self.eat(&Token::FatComma) {
                            if self.at(&Token::RBracket) {
                                break;
                            }
                            indices.push(self.parse_expr());
                        }
                        self.expect(&Token::RBracket);
                        let mut args = vec![expr];
                        args.extend(indices);
                        expr = Expr::Call("_postfix_array_slice".to_string(), args);
                        continue;
                    }
                    if matches!(self.tok(), Token::ArrayBlockDerefOpen) {
                        self.pos += 1;
                        self.eat(&Token::LBrace);
                        let mut keys = vec![self.parse_expr()];
                        while self.eat(&Token::Comma) || self.eat(&Token::FatComma) {
                            if self.at(&Token::RBrace) {
                                break;
                            }
                            keys.push(self.parse_expr());
                        }
                        self.expect(&Token::RBrace);
                        let mut args = vec![expr];
                        args.extend(keys);
                        expr = Expr::Call("_postfix_hash_slice".to_string(), args);
                        continue;
                    }
                    if let Token::ArrayVar(n) = self.tok()
                        && n.is_empty()
                        && matches!(self.peek(1), Token::LBrace)
                    {
                        self.pos += 2;
                        let mut keys = vec![self.parse_expr()];
                        while self.eat(&Token::Comma) || self.eat(&Token::FatComma) {
                            if self.at(&Token::RBrace) {
                                break;
                            }
                            keys.push(self.parse_expr());
                        }
                        self.expect(&Token::RBrace);
                        let mut args = vec![expr];
                        args.extend(keys);
                        expr = Expr::Call("_postfix_hash_slice".to_string(), args);
                        continue;
                    }
                    // `->%*` — postfix hash-flat deref. The lexer's `%`
                    // handler falls through to `Token::Percent` (no name
                    // char before `*`), so we recognise `Percent + Star`
                    // after the arrow.
                    if matches!(self.tok(), Token::Percent) && matches!(self.peek(1), Token::Star) {
                        self.pos += 2;
                        let inner = expr;
                        expr = Expr::Call("_hash_block_deref".to_string(), vec![inner]);
                        continue;
                    }
                    // `->&*` — postfix code deref / invoke. Same as
                    // `&$coderef` (call coderef with empty args). For
                    // `$cref->&*(args)` we'd need explicit parens, which
                    // becomes `->&*(args)` — fold subsequent parens into
                    // a CodeCall on the inner.
                    if matches!(self.tok(), Token::BitAnd) && matches!(self.peek(1), Token::Star) {
                        self.pos += 2;
                        let inner = expr;
                        let call_args = if self.eat(&Token::LParen) {
                            let a = self.parse_list_expr();
                            self.expect(&Token::RParen);
                            a
                        } else {
                            Vec::new()
                        };
                        expr = Expr::CodeCall(Box::new(inner), call_args);
                        continue;
                    }
                    // `->%[i1,i2]` — array key/value slice through ref.
                    // `->%{k1,k2}` — hash key/value slice through ref.
                    if matches!(self.tok(), Token::Percent)
                        && matches!(self.peek(1), Token::LBracket)
                    {
                        self.pos += 2;
                        let mut indices = vec![self.parse_expr()];
                        while self.eat(&Token::Comma) || self.eat(&Token::FatComma) {
                            if self.at(&Token::RBracket) {
                                break;
                            }
                            indices.push(self.parse_expr());
                        }
                        self.expect(&Token::RBracket);
                        let mut args = vec![expr];
                        args.extend(indices);
                        expr = Expr::Call("_postfix_array_kvslice".to_string(), args);
                        continue;
                    }
                    if matches!(self.tok(), Token::HashBlockDerefOpen) {
                        self.pos += 1;
                        self.eat(&Token::LBrace);
                        let mut keys = vec![self.parse_expr()];
                        while self.eat(&Token::Comma) || self.eat(&Token::FatComma) {
                            if self.at(&Token::RBrace) {
                                break;
                            }
                            keys.push(self.parse_expr());
                        }
                        self.expect(&Token::RBrace);
                        let mut args = vec![expr];
                        args.extend(keys);
                        expr = Expr::Call("_postfix_hash_kvslice".to_string(), args);
                        continue;
                    }
                    if matches!(self.tok(), Token::Percent) && matches!(self.peek(1), Token::LBrace)
                    {
                        self.pos += 2;
                        let mut keys = vec![self.parse_expr()];
                        while self.eat(&Token::Comma) || self.eat(&Token::FatComma) {
                            if self.at(&Token::RBrace) {
                                break;
                            }
                            keys.push(self.parse_expr());
                        }
                        self.expect(&Token::RBrace);
                        let mut args = vec![expr];
                        args.extend(keys);
                        expr = Expr::Call("_postfix_hash_kvslice".to_string(), args);
                        continue;
                    }
                    // `->$#*` — postfix array-length deref. Same as
                    // `$#{EXPR}`. Lexer produces `ArrayLen("")` followed
                    // by `Times` (`*`) since `read_ident` after `$#`
                    // hits `*` and returns empty.
                    if let Token::ArrayLen(n) = self.tok()
                        && n.is_empty()
                        && matches!(self.peek(1), Token::Star)
                    {
                        self.pos += 2; // consume ArrayLen("") and Star
                        let inner = expr;
                        expr = Expr::Call("_arylen_block_deref".to_string(), vec![inner]);
                        continue;
                    }
                    match self.tok() {
                        Token::LBracket => {
                            self.pos += 1;
                            let index = self.parse_expr();
                            self.expect(&Token::RBracket);
                            expr = Expr::ArrowElement(
                                Box::new(expr),
                                Box::new(index),
                                ArrowKind::Array,
                            );
                        }
                        Token::LBrace => {
                            self.pos += 1;
                            // Auto-quote a sole bareword inside `->{...}`
                            // so `$r->{length}` etc. work even when
                            // `length` lexes as a named-operator token.
                            let bareword_name: Option<String> = match self.tok() {
                                Token::Ident(n) => Some(n.clone()),
                                tok => named_op_token_name(tok),
                            };
                            let key = if let Some(n) = bareword_name
                                && matches!(self.tokens.get(self.pos + 1), Some(Token::RBrace))
                            {
                                self.pos += 1;
                                Expr::StringLit(n)
                            } else {
                                self.parse_expr()
                            };
                            self.expect(&Token::RBrace);
                            expr =
                                Expr::ArrowElement(Box::new(expr), Box::new(key), ArrowKind::Hash);
                        }
                        Token::LParen => {
                            // `$coderef->(args)` — invoke coderef.
                            self.pos += 1;
                            let args = if self.at(&Token::RParen) {
                                Vec::new()
                            } else {
                                self.parse_list_expr()
                            };
                            self.expect(&Token::RParen);
                            expr = Expr::CodeCall(Box::new(expr), args);
                        }
                        Token::Ident(name) => {
                            let method = name.clone();
                            self.pos += 1;
                            let args = if self.eat(&Token::LParen) {
                                let a = self.parse_list_expr();
                                self.expect(&Token::RParen);
                                a
                            } else {
                                Vec::new()
                            };
                            expr = Expr::MethodCall(Box::new(expr), method, args);
                        }
                        // `$obj->isa(...)` — `isa` lexes as its own
                        // token now that it's an operator. Allow it as
                        // a method name here.
                        Token::Isa => {
                            self.pos += 1;
                            let args = if self.eat(&Token::LParen) {
                                let a = self.parse_list_expr();
                                self.expect(&Token::RParen);
                                a
                            } else {
                                Vec::new()
                            };
                            expr = Expr::MethodCall(Box::new(expr), "isa".to_string(), args);
                        }
                        // `$obj->$method` — method name from scalar var.
                        // Stringify at runtime via a marker method name
                        // and pass the var as the first arg. op/method.
                        Token::ScalarVar(name) => {
                            let var_name = name.clone();
                            self.pos += 1;
                            let mut args = vec![Expr::ScalarVar(var_name)];
                            if self.eat(&Token::LParen) {
                                let extra = self.parse_list_expr();
                                self.expect(&Token::RParen);
                                args.extend(extra);
                            }
                            expr = Expr::MethodCall(
                                Box::new(expr),
                                "_dynamic_method".to_string(),
                                args,
                            );
                        }
                        // `$obj->${EXPR}` / `$obj->${\&sub}` — coderef
                        // method dispatch. EXPR evaluates to a coderef
                        // (or scalar holding a method name); we invoke
                        // it with $obj as first arg. op/sub_lval 53.
                        Token::ScalarBlockDerefOpen => {
                            self.pos += 1;
                            self.eat(&Token::LBrace);
                            let inner = self.parse_expr();
                            self.expect(&Token::RBrace);
                            let mut args =
                                vec![Expr::Call("_scalar_block_deref".to_string(), vec![inner])];
                            if self.eat(&Token::LParen) {
                                let extra = self.parse_list_expr();
                                self.expect(&Token::RParen);
                                args.extend(extra);
                            }
                            expr = Expr::MethodCall(
                                Box::new(expr),
                                "_dynamic_method".to_string(),
                                args,
                            );
                        }
                        _ => break,
                    }
                }
                _ => break,
            }
        }

        expr
    }

    fn parse_primary(&mut self) -> Expr {
        // `...` (yada-yada) in expression position is a syntax error
        // in reference perl. The valid form `... ;` (as a statement)
        // is caught in parse_stmt before reaching here. op/yadayada
        // 10-22.
        if let Token::Ident(name) = self.tok()
            && name == "..."
        {
            self.pos += 1;
            let line = self.current_line();
            let near = format!("... {}", token_display(self.tok()));
            self.error = Some(format!(
                "syntax error at {{FILE}} line {line}, near \"{near}\"\n"
            ));
            return Expr::Undef;
        }
        match self.tok().clone() {
            Token::Integer(n) => {
                self.pos += 1;
                Expr::IntLit(n)
            }
            Token::Float(n) => {
                self.pos += 1;
                Expr::FloatLit(n)
            }
            Token::StringLit(s) => {
                self.pos += 1;
                Expr::StringLit(s)
            }
            Token::InterpString(s) => {
                self.pos += 1;
                // Parse the interpolated string into parts
                parse_interp_string(&s)
            }
            Token::RegexLit(pat, flags) => {
                self.pos += 1;
                // Bare /regex/ matches against $_
                Expr::RegexMatch(Box::new(Expr::ScalarVar("_".to_string())), pat, flags)
            }
            Token::QrLit(pat, flags) => {
                self.pos += 1;
                // qr// yields a regex *value*, not a match.
                Expr::RegexLit(pat, flags)
            }
            Token::Substitution(pat, repl, flags) => {
                self.pos += 1;
                // Bare s/// applies to $_
                Expr::Substitution(Box::new(Expr::ScalarVar("_".to_string())), pat, repl, flags)
            }
            Token::Transliterate(from, to, flags) => {
                self.pos += 1;
                // Bare tr/// (and y///) applies to $_.
                Expr::Call(
                    "_tr_apply".to_string(),
                    vec![
                        Expr::ScalarVar("_".to_string()),
                        Expr::StringLit(from),
                        Expr::StringLit(to),
                        Expr::StringLit(flags),
                    ],
                )
            }
            Token::QW(words) => {
                self.pos += 1;
                Expr::QW(words)
            }

            Token::ScalarVar(name) => {
                self.pos += 1;
                Expr::ScalarVar(name)
            }
            Token::ArrayVar(name) => {
                self.pos += 1;
                // `@foo[1,2]` — slice of @foo; `@foo{k1,k2}` — slice of %foo.
                if matches!(self.tok(), Token::LBracket) {
                    self.pos += 1;
                    let keys = self.parse_list_expr();
                    self.expect(&Token::RBracket);
                    Expr::ArraySlice(name, keys)
                } else if matches!(self.tok(), Token::LBrace) {
                    self.pos += 1;
                    let keys = self.parse_list_expr();
                    self.expect(&Token::RBrace);
                    Expr::HashSlice(name, keys)
                } else {
                    Expr::ArrayVar(name)
                }
            }
            Token::HashVar(name) => {
                self.pos += 1;
                // `%h{k1,k2}` — key-value slice (Perl 5.20+): returns
                // (k1, $h{k1}, k2, $h{k2}, ...).
                if matches!(self.tok(), Token::LBrace) {
                    self.pos += 1;
                    let keys = self.parse_list_expr();
                    self.expect(&Token::RBrace);
                    Expr::HashKVSlice(name, keys)
                } else {
                    Expr::HashVar(name)
                }
            }
            Token::ArrayDeref(name) => {
                self.pos += 1;
                // `@$h[1,2]` — array slice through arrayref $h.
                // `@$h{k1,k2}` — hash slice through hashref $h.
                if matches!(self.tok(), Token::LBracket) {
                    self.pos += 1;
                    let keys = self.parse_list_expr();
                    self.expect(&Token::RBracket);
                    let mut args = vec![Expr::ScalarVar(name)];
                    args.extend(keys);
                    Expr::Call("_postfix_array_slice".to_string(), args)
                } else if matches!(self.tok(), Token::LBrace) {
                    self.pos += 1;
                    let keys = self.parse_list_expr();
                    self.expect(&Token::RBrace);
                    let mut args = vec![Expr::ScalarVar(name)];
                    args.extend(keys);
                    Expr::Call("_postfix_hash_slice".to_string(), args)
                } else {
                    Expr::ArrayDerefVar(name)
                }
            }
            Token::ArrayBlockDerefOpen => {
                // `@{ EXPR }` — evaluate EXPR, treat its result as an
                // array ref and return its elements. Lexer emits this for
                // `@` only; eat the trailing `{` so parse_expr sees the
                // inner expression, not a block primary.
                self.pos += 1;
                self.eat(&Token::LBrace);
                let inner = self.parse_expr();
                self.expect(&Token::RBrace);
                Expr::Call("_array_block_deref".to_string(), vec![inner])
            }
            Token::HashBlockDerefOpen => {
                // `%{ EXPR }` — evaluate EXPR, treat its result as a hash
                // ref and return the key/value list. The lexer emits
                // HashBlockDerefOpen for the `%` but leaves the `{` as a
                // separate Token::LBrace — consume it here so the inner
                // parse_expr sees the actual expression, not a primary
                // `{ … }` block.
                self.pos += 1;
                self.eat(&Token::LBrace);
                let inner = self.parse_expr();
                self.expect(&Token::RBrace);
                Expr::Call("_hash_block_deref".to_string(), vec![inner])
            }
            Token::ScalarBlockDerefOpen => {
                // `${ EXPR }` — evaluate EXPR, dereference as scalar. If
                // EXPR is a comma list (`${f(), \$x}`), each arg is
                // evaluated in order (for side effects) and the LAST
                // value is used as the scalar deref target. `${...}` is
                // the special yada-yada-in-block-deref form: reference
                // perl evaluates `...` as a statement-level yada-yada
                // that dies "Unimplemented". base/lex test 107.
                self.pos += 1;
                self.eat(&Token::LBrace);
                if let Token::Ident(n) = self.tok()
                    && n == "..."
                    && matches!(self.peek(1), Token::RBrace)
                {
                    self.pos += 1;
                    self.expect(&Token::RBrace);
                    return Expr::Call("_yada_yada".to_string(), vec![]);
                }
                let inner = self.parse_list_expr();
                self.expect(&Token::RBrace);
                Expr::Call("_scalar_block_deref".to_string(), inner)
            }
            Token::HashDeref(name) => {
                self.pos += 1;
                Expr::HashDerefVar(name)
            }
            Token::ScalarDeref(name) => {
                self.pos += 1;
                Expr::ScalarDerefVar(name)
            }
            Token::Glob(name) => {
                self.pos += 1;
                Expr::GlobVar(name)
            }
            Token::ArrayLen(name) => {
                self.pos += 1;
                if name == "{" {
                    // `$#{ EXPR }` — block-form last-index. Consume
                    // the `{`, parse the inner expression, consume
                    // `}`. Emit as a special call so the interpreter
                    // handles symbolic-ref / array-ref dispatch.
                    self.expect(&Token::LBrace);
                    let inner = self.parse_expr();
                    self.expect(&Token::RBrace);
                    Expr::Call("_arylen_block_deref".to_string(), vec![inner])
                } else {
                    Expr::ArrayLen(name)
                }
            }

            Token::Diamond(name) => {
                self.pos += 1;
                Expr::Diamond(name)
            }

            Token::UndefKw => {
                self.pos += 1;
                // undef EXPR — clears the lvalue and returns undef
                // undef()       — also returns undef
                // undef alone (no argument) — returns undef
                if self.eat(&Token::LParen) {
                    if self.eat(&Token::RParen) {
                        Expr::Undef
                    } else {
                        let arg = self.parse_expr();
                        self.expect(&Token::RParen);
                        Expr::Call("undef".to_string(), vec![arg])
                    }
                } else if matches!(
                    self.tok(),
                    Token::ScalarVar(_) | Token::ArrayVar(_) | Token::HashVar(_) | Token::BitAnd
                ) {
                    let arg = self.parse_unary();
                    Expr::Call("undef".to_string(), vec![arg])
                } else if matches!(
                    self.tok(),
                    Token::Ident(_) | Token::Integer(_) | Token::Float(_) | Token::StringLit(_)
                ) {
                    // `undef BAREWORD` / `undef NUMLIT` — constant-item
                    // target that should die with Perl's "Can't modify
                    // constant item in undef operator". Capture the arg
                    // as a StringLit (for barewords) / NumLit so the
                    // builtin's constant-check fires.
                    let arg = self.parse_primary();
                    Expr::Call("undef".to_string(), vec![arg])
                } else {
                    Expr::Undef
                }
            }

            Token::LParen => {
                self.pos += 1;
                if self.at(&Token::RParen) {
                    self.pos += 1;
                    return Expr::ArrayLit(Vec::new());
                }
                let expr = self.parse_expr();
                // Check if there are more items (it's a list) — fat-comma also
                // acts as a list separator.
                if self.eat(&Token::Comma) || self.eat(&Token::FatComma) {
                    let mut items = vec![expr];
                    loop {
                        if self.at(&Token::RParen) {
                            break;
                        }
                        items.push(self.parse_expr());
                        if !self.eat(&Token::Comma) && !self.eat(&Token::FatComma) {
                            break;
                        }
                    }
                    self.expect(&Token::RParen);
                    Expr::ArrayLit(items)
                } else {
                    self.expect(&Token::RParen);
                    // `(EXPR) x N` — list-context x-repeat.
                    // `(EXPR) = …`  — list-context assignment with one target.
                    // `(EXPR)[N]`  — list slice on the one-element list.
                    if matches!(
                        self.tok(),
                        Token::StringRepeat | Token::Assign | Token::LBracket
                    ) {
                        return Expr::ArrayLit(vec![expr]);
                    }
                    expr
                }
            }

            Token::LBracket => {
                // Anonymous array ref [...]
                self.pos += 1;
                let mut items = Vec::new();
                while !self.at(&Token::RBracket) && !self.at(&Token::EOF) {
                    items.push(self.parse_expr());
                    if !self.eat(&Token::Comma) && !self.eat(&Token::FatComma) {
                        break;
                    }
                }
                self.expect(&Token::RBracket);
                Expr::ArrayRef(items)
            }

            Token::LBrace => {
                // Anonymous hash ref {...} or block
                // Heuristic: { ident => ... } is a hash ref; `{}` (empty)
                // is an empty hashref (the common `my $h = {};` idiom);
                // otherwise it's a block. Also: `{LIST}` with no `;` inside
                // the outer braces is treated as a hashref — e.g. `{1..3}`.
                let saved = self.pos;
                self.pos += 1;

                // Check if it looks like a hash ref
                let mut is_hash = self.at(&Token::RBrace)
                    || matches!(
                        (self.tok(), self.peek(1)),
                        (Token::StringLit(_), Token::FatComma)
                            | (Token::Ident(_), Token::FatComma)
                            | (Token::Integer(_), Token::FatComma)
                    );
                // If not already detected, scan for shape clues at depth 0:
                // - no `;` and contains a `..` (Range) or `,`/`=>` — looks
                //   like a list-producing expression. Treat as hashref.
                // - presence of `;` is a strong block marker.
                // - a bare `{ SINGLE_EXPR }` is a hashref only when the
                //   `{` appears in a value-expecting context (after `=`,
                //   `,`, `(`, `return`, `=>`, etc.). Otherwise we'd
                //   misparse forms like `*{"name"}` (typeglob deref) as
                //   anonymous hashref construction.
                if !is_hash {
                    let mut depth: i32 = 1;
                    let mut p = self.pos;
                    let mut has_semi = false;
                    let mut has_list_marker = false;
                    while p < self.tokens.len() {
                        match &self.tokens[p] {
                            Token::LBrace | Token::LParen | Token::LBracket => depth += 1,
                            Token::RBrace | Token::RParen | Token::RBracket => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            Token::Semi if depth == 1 => {
                                has_semi = true;
                                break;
                            }
                            Token::DotDot | Token::Comma | Token::FatComma if depth == 1 => {
                                has_list_marker = true;
                            }
                            _ => {}
                        }
                        p += 1;
                    }
                    let in_value_ctx = saved == 0
                        || matches!(
                            self.tokens.get(saved - 1),
                            Some(Token::Assign)
                                | Some(Token::FatComma)
                                | Some(Token::Comma)
                                | Some(Token::LParen)
                                | Some(Token::Return)
                                | Some(Token::Print)
                                | Some(Token::Say)
                        );
                    if !has_semi && (has_list_marker || in_value_ctx) {
                        is_hash = true;
                    }
                }

                if is_hash {
                    // Collect inner content as a flat list of expressions.
                    // Each element is evaluated in list context at runtime
                    // and the resulting flat value list is paired key/value,
                    // emitting the standard anonymous-hash odd-elements
                    // warning when the count is odd.
                    let mut flat: Vec<Expr> = Vec::new();
                    while !self.at(&Token::RBrace) && !self.at(&Token::EOF) {
                        flat.push(self.parse_expr());
                        if !self.eat(&Token::Comma) && !self.eat(&Token::FatComma) {
                            break;
                        }
                    }
                    self.expect(&Token::RBrace);
                    Expr::HashRef(flat)
                } else {
                    // It's a block
                    self.pos = saved;
                    self.pos += 1;
                    let body = self.parse_block_body();
                    self.expect(&Token::RBrace);
                    Expr::DoBlock(body)
                }
            }

            Token::Sub => {
                // Anonymous sub
                self.pos += 1;
                // Skip prototype if present
                if self.at(&Token::LParen) {
                    self.pos += 1;
                    while !self.at(&Token::RParen) && !self.at(&Token::EOF) {
                        self.pos += 1;
                    }
                    self.eat(&Token::RParen);
                }
                // Skip attributes
                while self.eat(&Token::Colon) {
                    if let Token::Ident(_) = self.tok() {
                        self.pos += 1;
                    }
                }
                // Anonymous `sub` requires a body block — `{ $x = sub }`
                // is a syntax error in reference perl (op/anonsub 3).
                if !self.at(&Token::LBrace) {
                    let line = self.current_line();
                    self.error = Some(format!(
                        "Illegal declaration of anonymous subroutine at {{FILE}} line {line}.\n"
                    ));
                    return Expr::Undef;
                }
                let body = self.parse_brace_block();
                Expr::AnonSub(Vec::new(), body)
            }

            // Named unary builtins
            Token::Exists
            | Token::Abs
            | Token::Int
            | Token::Length
            | Token::Chr
            | Token::Ord
            | Token::Lc
            | Token::Uc
            | Token::Lcfirst
            | Token::Ucfirst
            | Token::Hex
            | Token::Oct
            | Token::Ref
            | Token::Chomp
            | Token::Chop
            | Token::Pop
            | Token::Shift
            | Token::Caller
            | Token::Eof
            | Token::Tell
            | Token::Wantarray => {
                let func = match self.tok() {
                    Token::Exists => "exists",
                    Token::Abs => "abs",
                    Token::Int => "int",
                    Token::Length => "length",
                    Token::Chr => "chr",
                    Token::Ord => "ord",
                    Token::Lc => "lc",
                    Token::Uc => "uc",
                    Token::Lcfirst => "lcfirst",
                    Token::Ucfirst => "ucfirst",
                    Token::Hex => "hex",
                    Token::Oct => "oct",
                    Token::Ref => "ref",
                    Token::Chomp => "chomp",
                    Token::Chop => "chop",
                    Token::Pop => "pop",
                    Token::Shift => "shift",
                    Token::Caller => "caller",
                    Token::Eof => "eof",
                    Token::Tell => "tell",
                    Token::Wantarray => {
                        return {
                            self.pos += 1;
                            Expr::Wantarray
                        };
                    }
                    _ => unreachable!(),
                }
                .to_string();
                self.pos += 1;
                let args = if self.eat(&Token::LParen) {
                    let a = self.parse_list_expr();
                    self.expect(&Token::RParen);
                    a
                } else if !self.at(&Token::Semi)
                    && !self.at(&Token::Comma)
                    && !self.at(&Token::RParen)
                    && !self.at(&Token::RBrace)
                    && !self.at(&Token::RBracket)
                    && !matches!(
                        self.tok(),
                        Token::Question
                            | Token::Colon
                            | Token::LogAnd
                            | Token::LogOr
                            | Token::And
                            | Token::Or
                            | Token::DefOr
                            | Token::NumEq
                            | Token::NumNe
                            | Token::NumLt
                            | Token::NumGt
                            | Token::NumLe
                            | Token::NumGe
                            | Token::Eq
                            | Token::Ne
                            | Token::If
                            | Token::Unless
                            | Token::While
                            | Token::Until
                            | Token::For
                            | Token::Foreach
                            // `->` is a postfix on the call result, not
                            // the start of an argument: `shift->[0]` is
                            // `(shift())->[0]`.
                            | Token::Arrow
                    )
                {
                    vec![self.parse_unary()]
                } else {
                    Vec::new()
                };
                Expr::Call(func, args)
            }

            // map/grep with { BLOCK } LIST syntax
            Token::Grep | Token::Map => {
                let func = if matches!(self.tok(), Token::Map) {
                    "map"
                } else {
                    "grep"
                }
                .to_string();
                self.pos += 1;

                // Disambiguate `{...}` after map/grep: a hashref (an EXPR
                // arg) if the first key looks like `WORD =>`, `"x" =>` or
                // `N =>`; otherwise a code BLOCK.
                let brace_is_hashref = |this: &Self| {
                    if !this.at(&Token::LBrace) {
                        return false;
                    }
                    matches!(
                        (this.peek(1), this.peek(2)),
                        (Token::StringLit(_), Token::FatComma)
                            | (Token::Ident(_), Token::FatComma)
                            | (Token::Integer(_), Token::FatComma)
                    )
                };

                if self.at(&Token::LBrace) && !brace_is_hashref(self) {
                    // map { BLOCK } LIST
                    let block = self.parse_brace_block();
                    // Skip comma if present
                    self.eat(&Token::Comma);
                    let list_args = self.parse_list_expr();
                    // For now, return a call with block as first arg
                    let block_expr = Expr::DoBlock(block);
                    let mut args = vec![block_expr];
                    args.extend(list_args);
                    Expr::Call(func, args)
                } else if self.eat(&Token::LParen) {
                    // `map(BLOCK LIST)` or `map(EXPR, LIST)` — both forms
                    // appear in the wild. If we see `{` first AND it
                    // doesn't look like a hashref, it's the block form.
                    let mut args = Vec::new();
                    if self.at(&Token::LBrace) && !brace_is_hashref(self) {
                        let block = self.parse_brace_block();
                        args.push(Expr::DoBlock(block));
                        self.eat(&Token::Comma);
                    }
                    let list_args = self.parse_list_expr();
                    args.extend(list_args);
                    self.expect(&Token::RParen);
                    Expr::Call(func, args)
                } else {
                    // `grep $var (list)` / `map $var (list)` — Perl rejects
                    // this at parse time (the parentheses-around-list form
                    // without a comma after the first argument). Detect
                    // `$ident (` and emit Perl's exact error message via
                    // the error_diagnostic flag so `eval` catches it.
                    if matches!(self.tok(), Token::ScalarVar(_))
                        && matches!(self.peek(1), Token::LParen)
                    {
                        return Expr::Call(
                            "_parse_error".to_string(),
                            vec![Expr::StringLit(format!(
                                "Missing comma after first argument to {func} function"
                            ))],
                        );
                    }
                    let args = self.parse_list_expr();
                    Expr::Call(func, args)
                }
            }

            // sort with optional { BLOCK } or sub name
            Token::Sort => {
                self.pos += 1;
                if self.at(&Token::LBrace) {
                    let block = self.parse_brace_block();
                    let list_args = self.parse_list_expr();
                    let block_expr = Expr::DoBlock(block);
                    let mut args = vec![block_expr];
                    args.extend(list_args);
                    Expr::Call("sort".to_string(), args)
                } else if self.eat(&Token::LParen) {
                    let args = self.parse_list_expr();
                    self.expect(&Token::RParen);
                    Expr::Call("sort".to_string(), args)
                } else {
                    let args = self.parse_list_expr();
                    Expr::Call("sort".to_string(), args)
                }
            }

            // Unary-on-hash/array builtins: take a single expression so the
            // ternary and list comma at the call-site bind correctly.
            // `print keys %h ? a : b, "rest"` must parse as
            // `print((keys %h) ? a : b, "rest")`, not as `keys(...rest)`.
            Token::Keys | Token::Values | Token::Each => {
                let func = format!("{:?}", self.tok()).to_lowercase();
                self.pos += 1;
                let arg = if self.eat(&Token::LParen) {
                    let a = self.parse_expr();
                    self.expect(&Token::RParen);
                    a
                } else {
                    self.parse_unary()
                };
                Expr::Call(func, vec![arg])
            }

            // List builtins: push, unshift, splice, delete, exists,
            // reverse, join, split, substr, index, rindex, sprintf,
            // open, close, read, binmode, unlink, rename, mkdir, rmdir, chdir, stat
            Token::Push
            | Token::Unshift
            | Token::Splice
            | Token::Reverse
            | Token::Join
            | Token::Split
            | Token::Substr
            | Token::Index
            | Token::Rindex
            | Token::Sprintf
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
            | Token::Bless => {
                let func = format!("{:?}", self.tok()).to_lowercase();
                self.pos += 1;
                let args = if self.eat(&Token::LParen) {
                    let a = self.parse_list_expr();
                    self.expect(&Token::RParen);
                    a
                } else if matches!(
                    self.tok(),
                    Token::Comma
                        | Token::Semi
                        | Token::EOF
                        | Token::RBrace
                        | Token::RParen
                        | Token::RBracket
                ) {
                    // `unlink, …` / `unlink;` / `unlink}` — bare form
                    // with no args; the builtin's docs say it defaults
                    // to `$_`. Don't consume the comma; let the outer
                    // arg-list collect the rest. op/unlink 5-6.
                    Vec::new()
                } else {
                    self.parse_list_expr()
                };
                Expr::Call(func, args)
            }

            Token::Delete => {
                self.pos += 1;
                // `delete local $h{k}` / `delete local $a[i]` /
                // `delete local @arr[i,j]` / `delete local %h{a,b}` —
                // route to a synthetic `_delete_local` call carrying
                // the variable shape. Marker arg #0 distinguishes the
                // single (`elem`) vs slice (`aslice`/`hslice`) forms.
                let inner_paren = self.eat(&Token::LParen);
                if matches!(self.tok(), Token::Local) {
                    self.pos += 1; // `local`
                    if let Token::ScalarVar(name) = self.tok()
                        && matches!(self.peek(1), Token::LBrace | Token::LBracket)
                    {
                        let var_name = name.clone();
                        let is_hash = matches!(self.peek(1), Token::LBrace);
                        self.pos += 2;
                        let key = self.parse_expr();
                        let close = if is_hash {
                            &Token::RBrace
                        } else {
                            &Token::RBracket
                        };
                        self.expect(close);
                        if inner_paren {
                            self.expect(&Token::RParen);
                        }
                        let bucket = if is_hash {
                            var_name
                        } else {
                            format!("@{var_name}")
                        };
                        return Expr::Call(
                            "_delete_local".to_string(),
                            vec![
                                Expr::StringLit("elem".to_string()),
                                Expr::StringLit(bucket),
                                key,
                            ],
                        );
                    }
                    if (matches!(self.tok(), Token::ArrayVar(_))
                        || matches!(self.tok(), Token::HashVar(_)))
                        && matches!(self.peek(1), Token::LBracket | Token::LBrace)
                    {
                        let (var_name, is_hash) = match self.tok() {
                            Token::ArrayVar(n) => (format!("@{n}"), false),
                            Token::HashVar(n) => (format!("%{n}"), true),
                            _ => unreachable!(),
                        };
                        self.pos += 2; // sigil + open bracket/brace
                        let mut keys = Vec::new();
                        loop {
                            keys.push(self.parse_expr());
                            if !self.eat(&Token::Comma) {
                                break;
                            }
                        }
                        let close = if matches!(self.tok(), Token::RBracket) {
                            Token::RBracket
                        } else {
                            Token::RBrace
                        };
                        self.expect(&close);
                        if inner_paren {
                            self.expect(&Token::RParen);
                        }
                        let mut out = vec![
                            Expr::StringLit(if is_hash { "hslice" } else { "aslice" }.to_string()),
                            Expr::StringLit(var_name),
                        ];
                        out.extend(keys);
                        return Expr::Call("_delete_local".to_string(), out);
                    }
                    // Unrecognised shape after `delete local`; fall through
                    // and let the inner parse handle it.
                }
                let arg = if inner_paren {
                    let e = self.parse_expr();
                    self.expect(&Token::RParen);
                    e
                } else {
                    // Bare `delete EXPR` takes one term, not a comma-list:
                    // `is delete $h{$k}, undef, "name"` must parse as
                    // `is( delete($h{$k}), undef, "name" )`.
                    self.parse_unary()
                };
                Expr::Call("delete".to_string(), vec![arg])
            }

            Token::Eval => {
                self.pos += 1;
                if self.at(&Token::LBrace) {
                    let body = self.parse_brace_block();
                    // `eval { BLOCK }` is an expression — evaluates the block
                    // with errors trapped into $@. Wrap in a Call so the
                    // interpreter can handle the trap semantics.
                    Expr::Call("eval".to_string(), vec![Expr::DoBlock(body)])
                } else if self.at(&Token::LParen) {
                    // `eval(EXPR)` — parenthesized form; the arg list
                    // ends at the matching `)`. Avoid letting an outer
                    // operator (`.`, etc.) bleed into the arg, which
                    // would happen if we used `parse_additive` here:
                    // `eval("x") . "y"` must NOT be `eval("x" . "y")`.
                    // comp/package_block depends on this.
                    self.pos += 1;
                    let args = self.parse_list_expr();
                    self.expect(&Token::RParen);
                    Expr::Call("eval".to_string(), args)
                } else {
                    // `eval EXPR` is a named-unary op: its arg includes
                    // `.` (concat) and arithmetic operators but NOT
                    // relational comparisons or `or`/`and`/`= …`. Use
                    // parse_additive so `eval "my " . $code` parses as
                    // `eval(("my ") . $code)` and `eval $arr[$i]` reads
                    // the element.
                    let expr = self.parse_additive();
                    Expr::Call("eval".to_string(), vec![expr])
                }
            }

            Token::Do => {
                self.pos += 1;
                if self.at(&Token::LBrace) {
                    let body = self.parse_brace_block();
                    Expr::DoBlock(body)
                } else {
                    let expr = self.parse_primary();
                    Expr::DoFile(Box::new(expr))
                }
            }

            // `last`/`next`/`redo` in expression position — e.g. inside
            // a C-style for's step (`for(;;last LABEL)`). Wrap as a
            // DoBlock containing the flow-control stmt so the existing
            // Stmt handler runs (sets pending_flow). Without this, the
            // step would be a no-op and the for loop would spin
            // forever. op/loopctl 16+.
            Token::Last | Token::Next | Token::Redo => {
                let which = self.tok().clone();
                self.pos += 1;
                let label = if let Token::Ident(name) = self.tok() {
                    let n = name.clone();
                    self.pos += 1;
                    Some(n)
                } else {
                    None
                };
                let stmt = match which {
                    Token::Last => Stmt::Last(label),
                    Token::Next => Stmt::Next(label),
                    Token::Redo => Stmt::Redo(label),
                    _ => unreachable!(),
                };
                Expr::DoBlock(vec![stmt])
            }

            Token::Local => {
                // `local $var` / `local @arr` / `local %h` / `local (...)`
                // in expression position. Wrap each in a `do { local …;
                // EXPR }` so the local's save+restore happens, and the
                // expression evaluates to the just-localised slot — so a
                // following `= LIST` does the assignment under list context.
                self.pos += 1;
                if let Token::ScalarVar(name) = self.tok() {
                    let n = name.clone();
                    self.pos += 1;
                    Expr::DoBlock(vec![
                        Stmt::Local(vec![(format!("${n}"), None)], false),
                        Stmt::Expr(Expr::ScalarVar(n)),
                    ])
                } else if let Token::ArrayVar(name) = self.tok() {
                    let n = name.clone();
                    self.pos += 1;
                    Expr::DoBlock(vec![
                        Stmt::Local(vec![(format!("@{n}"), None)], false),
                        Stmt::Expr(Expr::ArrayVar(n)),
                    ])
                } else if let Token::HashVar(name) = self.tok() {
                    let n = name.clone();
                    self.pos += 1;
                    Expr::DoBlock(vec![
                        Stmt::Local(vec![(format!("%{n}"), None)], false),
                        Stmt::Expr(Expr::HashVar(n)),
                    ])
                } else if self.at(&Token::LParen) {
                    // `local (...)` in expression position. Collect names,
                    // declare each as local (snapshot+save), then return an
                    // ArrayLit referencing the bare slots so a following
                    // `= LIST` does proper list-context destructure.
                    self.pos += 1;
                    let mut names = Vec::new();
                    loop {
                        match self.tok() {
                            Token::ScalarVar(name) => {
                                names.push(format!("${}", name));
                                self.pos += 1;
                            }
                            Token::ArrayVar(name) => {
                                names.push(format!("@{}", name));
                                self.pos += 1;
                            }
                            Token::HashVar(name) => {
                                names.push(format!("%{}", name));
                                self.pos += 1;
                            }
                            Token::UndefKw => {
                                names.push("$_undef_placeholder".to_string());
                                self.pos += 1;
                            }
                            _ => break,
                        }
                        if !self.eat(&Token::Comma) {
                            break;
                        }
                    }
                    self.expect(&Token::RParen);
                    if names.is_empty() {
                        return Expr::Undef;
                    }
                    // Build a do-block: declare all locals (no init), then
                    // emit an expression list of the bare vars.
                    let local_decls: Vec<(String, Option<Expr>)> =
                        names.iter().map(|n| (n.clone(), None)).collect();
                    let mut stmts: Vec<Stmt> = vec![Stmt::Local(local_decls, true)];
                    let exprs: Vec<Expr> = names
                        .iter()
                        .map(|n| {
                            if n == "$_undef_placeholder" {
                                Expr::Undef
                            } else if let Some(rest) = n.strip_prefix('@') {
                                Expr::ArrayVar(rest.to_string())
                            } else if let Some(rest) = n.strip_prefix('%') {
                                Expr::HashVar(rest.to_string())
                            } else {
                                let rest = n.strip_prefix('$').unwrap_or(n).to_string();
                                Expr::ScalarVar(rest)
                            }
                        })
                        .collect();
                    if exprs.len() == 1 {
                        stmts.push(Stmt::Expr(exprs.into_iter().next().unwrap()));
                    } else {
                        stmts.push(Stmt::Expr(Expr::ArrayLit(exprs)));
                    }
                    Expr::DoBlock(stmts)
                } else {
                    Expr::Undef
                }
            }

            // print/say/die/warn/printf in expression context
            Token::Print | Token::Say | Token::Die | Token::Warn | Token::Printf => {
                let func = match self.tok() {
                    Token::Print => "print",
                    Token::Say => "say",
                    Token::Die => "die",
                    Token::Warn => "warn",
                    Token::Printf => "printf",
                    _ => unreachable!(),
                }
                .to_string();
                self.pos += 1;
                // `:` is the ternary separator; without bailing out
                // here, `parse_list_expr → parse_expr` would consume
                // the `:` (silent-skip on unknown primary), corrupting
                // the surrounding ternary parse. op/catch test 10.
                let args = if matches!(self.tok(), Token::Colon) {
                    Vec::new()
                } else {
                    self.parse_list_expr()
                };
                Expr::Call(func, args)
            }

            Token::Ident(name) => {
                let mut name = name.clone();
                self.pos += 1;

                // Indirect method-call syntax: `method Class(args)` →
                // `Class->method(args)`. Only fire for the
                // explicit-parens form to avoid breaking the common
                // `func arg1, arg2` list-op pattern. The receiver must
                // be a class-shaped bareword (alpha first char), and
                // the method name must not be a list-op/builtin
                // (excluded by is_method_name_excluded). op/method
                // indirect tests.
                if !is_method_name_excluded(&name)
                    && name != "::"
                    && !name.starts_with('-')
                    && name
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_ascii_lowercase() || c == '_')
                {
                    let is_known_sub = self.known_subs.contains(&name);
                    // Indirect method-call: lowercase bareword followed by
                    //   * Uppercase class-shaped bareword: `method Class …`
                    //   * Scalar variable:                 `method $obj …`
                    // Both then accept args with or without parens.
                    // Known subs are eligible only when the scalar receiver
                    // is followed by `(` (method call args). Otherwise (e.g.
                    // `tryeq $T++, …`) we treat it as a function call.
                    // op/method indirect-syntax tests 2-22, 25-26.
                    let recv: Option<Expr> = match self.tok().clone() {
                        // Capitalised bareword receiver: ALWAYS indirect
                        // even if `name` names a known sub (`method
                        // Pack(...)` inside op/method.t where `sub
                        // method` is defined for the test).
                        Token::Ident(class)
                            if !class.starts_with('-')
                                && class != "::"
                                && class.chars().next().is_some_and(|c| c.is_ascii_uppercase()) =>
                        {
                            self.pos += 1;
                            Some(Expr::StringLit(class))
                        }
                        // Scalar receiver: when name is NOT a known sub,
                        // always indirect. When name IS a known sub, only
                        // indirect when the scalar is followed by `(args)`
                        // OR by an EXPRESSION STARTER without a comma —
                        // those signal indirect-method `$obj->name args`.
                        // Followed by `,`/`;`/etc. it's a function call
                        // (`foo $msg;` → `foo($msg)`). op/method test 26
                        // (`method $obj "a","b","c"`) needs the
                        // expression-starter case.
                        Token::ScalarVar(v) if !is_known_sub => {
                            self.pos += 1;
                            Some(Expr::ScalarVar(v))
                        }
                        Token::ScalarVar(v)
                            if matches!(
                                self.peek(1),
                                Token::LParen | Token::StringLit(_) | Token::InterpString(_)
                            ) =>
                        {
                            self.pos += 1;
                            Some(Expr::ScalarVar(v))
                        }
                        _ => None,
                    };
                    if let Some(recv) = recv {
                        let args = if self.eat(&Token::LParen) {
                            let a = self.parse_list_expr();
                            self.expect(&Token::RParen);
                            a
                        } else if matches!(
                            self.tok(),
                            Token::Semi
                                | Token::EOF
                                | Token::RParen
                                | Token::RBrace
                                | Token::RBracket
                                | Token::Comma
                        ) {
                            Vec::new()
                        } else {
                            self.parse_list_expr()
                        };
                        return Expr::MethodCall(Box::new(recv), name, args);
                    }
                }

                // `::name` — explicit top-level / `main::name` reference.
                // The lexer emitted `Ident("::")` + `Ident("name")`; stitch
                // them into the full qualified name here so call resolution
                // finds the sub. Further `::segment` suffixes collapse too.
                if name == "::" {
                    while let Token::Ident(n) = self.tok() {
                        if n == "::" {
                            name.push_str("::");
                            self.pos += 1;
                        } else {
                            name.push_str(n);
                            self.pos += 1;
                            // Allow `A::B::C`.
                            while let Token::Ident(m) = self.tok() {
                                if m == "::" {
                                    name.push_str("::");
                                    self.pos += 1;
                                } else {
                                    name.push_str(m);
                                    self.pos += 1;
                                }
                            }
                            break;
                        }
                    }
                    // Drop the leading `::` — callers index the sub table
                    // without it (main::is is registered as either "is" or
                    // "main::is", and we canonicalize to the short form).
                    if let Some(rest) = name.strip_prefix("::") {
                        name = rest.to_string();
                    }
                }

                // Compile-time constants the interpreter resolves at runtime
                // from `current_file` / `current_line`. Treat like zero-arg
                // function calls so they don't get parsed as barewords.
                if name == "__FILE__" || name == "__LINE__" || name == "__PACKAGE__" {
                    return Expr::Call(name, Vec::new());
                }
                if name == "__SUB__" || name == "CORE::__SUB__" {
                    return Expr::Call(name, Vec::new());
                }
                // Perl nullary builtins. Recognise them before the
                // "function call without parens" branch so `time - $t`
                // parses as `time() - $t` rather than `time(-$t)`.
                // `time()` (with parens) is handled by the LParen branch
                // above.
                if matches!(
                    name.as_str(),
                    "time" | "times" | "wait" | "fork" | "getppid" | "getpid"
                ) && !self.at(&Token::LParen)
                {
                    return Expr::Call(name, Vec::new());
                }
                // `pos` with no parens / no arg-starter defaults to $_
                // (named unary). Without this, bare `pos` (e.g. inside
                // `sub position : lvalue { pos }`) is parsed as the
                // string literal "pos". op/sub_lval test 74.
                if name == "pos" && !self.at(&Token::LParen) {
                    let starts_arg = matches!(
                        self.tok(),
                        Token::ScalarVar(_)
                            | Token::ArrayVar(_)
                            | Token::HashVar(_)
                            | Token::Glob(_)
                            | Token::Star
                    );
                    if starts_arg {
                        return Expr::Call(name, vec![self.parse_unary()]);
                    }
                    return Expr::Call(name, vec![Expr::ScalarVar("_".to_string())]);
                }
                // `exit` (bare, no parens, no arg-starter) — without this
                // `exit;` parsed as the bareword string. Same parens-or-arg
                // shape as rand/srand below.
                if name == "exit" && !self.at(&Token::LParen) {
                    let starts_arg = !self.at(&Token::Semi)
                        && !self.at(&Token::Comma)
                        && !self.at(&Token::RParen)
                        && !self.at(&Token::RBrace)
                        && !self.at(&Token::RBracket)
                        && !matches!(
                            self.tok(),
                            Token::Question
                                | Token::Colon
                                | Token::LogAnd
                                | Token::LogOr
                                | Token::And
                                | Token::Or
                                | Token::Xor
                                | Token::If
                                | Token::Unless
                                | Token::While
                                | Token::Until
                                | Token::For
                                | Token::Foreach
                        );
                    let args = if starts_arg {
                        vec![self.parse_unary()]
                    } else {
                        Vec::new()
                    };
                    return Expr::Call(name, args);
                }
                // `rand` / `srand` — nullary if next token isn't an
                // operand-starter, otherwise consume one optional arg
                // (parses `rand 10` and `srand $s` as `rand(10)`).
                if matches!(name.as_str(), "rand" | "srand") && !self.at(&Token::LParen) {
                    let starts_arg = !self.at(&Token::Semi)
                        && !self.at(&Token::Comma)
                        && !self.at(&Token::RParen)
                        && !self.at(&Token::RBrace)
                        && !self.at(&Token::RBracket)
                        && !matches!(
                            self.tok(),
                            Token::Question
                                | Token::Colon
                                | Token::LogAnd
                                | Token::LogOr
                                | Token::And
                                | Token::Or
                                | Token::NumEq
                                | Token::NumNe
                                | Token::NumLt
                                | Token::NumGt
                                | Token::NumLe
                                | Token::NumGe
                                | Token::Eq
                                | Token::Ne
                                | Token::If
                                | Token::Unless
                                | Token::While
                                | Token::Until
                                | Token::For
                                | Token::Foreach
                                | Token::Plus
                                | Token::Minus
                                | Token::Star
                                | Token::Slash
                                | Token::Percent
                                | Token::Dot
                        );
                    let args = if starts_arg {
                        vec![self.parse_unary()]
                    } else {
                        Vec::new()
                    };
                    return Expr::Call(name, args);
                }

                // Check for backtick execution: Ident("backtick") followed by StringLit
                if name == "backtick" {
                    if let Token::StringLit(cmd) = self.tok() {
                        let cmd = cmd.clone();
                        self.pos += 1;
                        return Expr::Backtick(cmd);
                    } else if let Token::InterpString(cmd) = self.tok() {
                        let cmd = cmd.clone();
                        self.pos += 1;
                        // Parse the interpolated string and wrap in Backtick-like handling
                        // We'll store it as a special call
                        return Expr::BacktickInterp(Box::new(parse_interp_string(&cmd)));
                    }
                }

                // Check for function call
                if self.at(&Token::LParen) {
                    self.pos += 1;
                    let args = self.parse_list_expr();
                    self.expect(&Token::RParen);
                    Expr::Call(name, args)
                } else if name == "1" {
                    // "1 while ..." pattern
                    Expr::IntLit(1)
                } else if matches!(
                    self.tok(),
                    Token::StringLit(_)
                        | Token::InterpString(_)
                        | Token::Integer(_)
                        | Token::Float(_)
                        | Token::ScalarVar(_)
                        | Token::ArrayVar(_)
                        | Token::HashVar(_)
                        | Token::ArrayLen(_)
                        | Token::Plus
                        | Token::Minus
                        | Token::LogNot
                        | Token::Backslash
                        | Token::Defined
                        | Token::UndefKw
                        | Token::Not
                        | Token::Eval
                        // `tie my $t, …` / `untie my $t` etc. — `my` in
                        // expression position is a valid first-arg start
                        // (yields the new lexical slot).
                        | Token::My
                        | Token::Our
                        | Token::Local
                        | Token::ArrayDeref(_)
                        | Token::HashDeref(_)
                        | Token::ScalarDeref(_)
                        | Token::ArrayBlockDerefOpen
                        | Token::HashBlockDerefOpen
                        | Token::ScalarBlockDerefOpen
                        | Token::QrLit(_, _)
                        | Token::Ident(_)
                        // Builtins that themselves take args also count as
                        // valid first-arg starters when calling another sub
                        // without parens (e.g. `myis sprintf "...", ...`).
                        | Token::Sprintf
                        | Token::Printf
                        | Token::Print
                        | Token::Push
                        | Token::Pop
                        | Token::Shift
                        | Token::Unshift
                        | Token::Splice
                        | Token::Reverse
                        | Token::Sort
                        | Token::Join
                        | Token::Split
                        | Token::Grep
                        | Token::Map
                        | Token::Keys
                        | Token::Values
                        | Token::Each
                        | Token::Substr
                        | Token::Length
                        | Token::Index
                        | Token::Rindex
                        | Token::Chr
                        | Token::Ord
                        | Token::Lc
                        | Token::Uc
                        | Token::Hex
                        | Token::Oct
                        | Token::Abs
                        | Token::Int
                        | Token::Ref
                        | Token::Caller
                        | Token::Open
                        | Token::Close
                        | Token::Read
                        | Token::Eof
                        | Token::Tell
                        | Token::Delete
                        | Token::Exists
                        | Token::Glob(_)
                        | Token::Diamond(_)
                        | Token::Unlink
                        | Token::Rename
                        | Token::Mkdir
                        | Token::Rmdir
                        | Token::Chdir
                        | Token::Stat
                        | Token::Bless
                        | Token::Binmode
                ) && !matches!(self.tok(), Token::Ident(n) if n == "...")
                {
                    // Function call without parentheses: func arg, ...
                    // Perl prototype-`$` builtins only take a single scalar arg.
                    // Without this, `is(scalar @b, 1, $name)` would be parsed as
                    // `is(scalar(@b, 1, $name))` — the scalar() swallowing the
                    // remaining arguments to the outer call.
                    let args = if is_unary_builtin(&name) {
                        vec![self.parse_unary()]
                    } else {
                        self.parse_list_expr()
                    };
                    Expr::Call(name, args)
                } else if self.known_subs.contains(&name) {
                    // Bareword that names a known sub — emit a no-arg
                    // call so `done_testing;` (after `sub done_testing`)
                    // dispatches to the sub instead of returning the
                    // string "done_testing".
                    Expr::Call(name, Vec::new())
                } else {
                    // Bareword — treat as string in most contexts. Perl's
                    // `Foo::` bareword is the string "Foo" (not "Foo::"),
                    // used as the class name in e.g. `bless {}, Foo::`.
                    let s = name.strip_suffix("::").unwrap_or(&name).to_string();
                    Expr::StringLit(s)
                }
            }

            _ => {
                // Unknown token in primary position. If we hit a
                // statement/expression terminator (`;`, `}`, EOF) without
                // having parsed anything yet, this is a syntax error
                // (e.g. `$foo =;` or `($x,)`). Record it so `eval STRING`
                // can surface it via `$@`.
                let here = self.tok().clone();
                // Only `;` and EOF are unambiguous: hitting them in
                // primary position means an expression was abandoned
                // mid-stream (e.g. `$foo =;`). Other "terminators"
                // (`}`, `)`, `]`, `,`) appear validly in expressions
                // we don't yet model (e.g. UTF-8 identifiers, attributes),
                // so we keep the silent-skip fallback for those.
                let is_terminator = matches!(here, Token::Semi | Token::EOF);
                // A binary operator at primary position means the LHS is
                // missing — `eval '&& 5'` etc. Reference perl reports
                // `syntax error at … near "&&"`. Treat these as terminators
                // for error-reporting purposes.
                let is_binop_no_lhs = matches!(
                    here,
                    Token::LogAnd
                        | Token::LogOr
                        | Token::DefOr
                        | Token::And
                        | Token::Or
                        | Token::Spaceship
                        | Token::NumEq
                        | Token::NumNe
                        | Token::NumLe
                        | Token::NumGe
                );
                if (is_terminator || is_binop_no_lhs) && self.error.is_none() {
                    let line = self.current_line();
                    let at_eof = matches!(here, Token::EOF);
                    let where_ = if at_eof {
                        ", at EOF".to_string()
                    } else {
                        format!(", near \"{}\"", token_display(&here))
                    };
                    self.error = Some(format!("syntax error at {{FILE}} line {line}{where_}\n"));
                    Expr::Undef
                } else {
                    // Skip the bad token to make progress.
                    self.pos += 1;
                    Expr::Undef
                }
            }
        }
    }

    // Helper for matching Token::ScalarVar in match arms
    fn at_scalar_var(&self) -> bool {
        matches!(self.tok(), Token::ScalarVar(_))
    }

    /// Scan past `{ ... }` starting at token index `brace_pos` (which must
    /// point at the `{`) and return whether the next token continues an
    /// expression — i.e., the `sub { ... }` is being used as a value, not
    /// declared. True for `->`, operators, or end-of-expression punctuation
    /// that implies the whole `sub {…}` is a statement expression.
    fn anon_sub_starts_expr(&self, brace_pos: usize) -> bool {
        // Scan past the matching close brace.
        let mut depth = 0i32;
        let mut i = brace_pos;
        while i < self.tokens.len() {
            match &self.tokens[i] {
                Token::LBrace => depth += 1,
                Token::RBrace => {
                    depth -= 1;
                    if depth == 0 {
                        i += 1;
                        break;
                    }
                }
                _ => {}
            }
            i += 1;
        }
        // Look at the next non-LineMark-ish token.
        matches!(
            self.tokens.get(i),
            Some(Token::Arrow)
                | Some(Token::LParen)
                | Some(Token::Plus)
                | Some(Token::Minus)
                | Some(Token::StringRepeat)
                | Some(Token::Slash)
                | Some(Token::Star)
                | Some(Token::Percent)
                | Some(Token::Dot)
                | Some(Token::LogAnd)
                | Some(Token::LogOr)
                | Some(Token::DefOr)
                | Some(Token::Question)
                | Some(Token::Comma)
                // `sub { … }` at end of an enclosing block is the anon
                // sub returned as the block's value (Perl's last-expr
                // implicit return). Without this, a tail `sub { … }` was
                // parsed as a nameless `sub NAME { … }` declaration,
                // dropping the CodeRef. Same for trailing `;` and EOF
                // (e.g. `eval 'sub { … }'`, where the whole eval body
                // is just the anon sub).
                | Some(Token::RBrace)
                | Some(Token::Semi)
                | Some(Token::EOF)
                | None
        )
    }
}

/// Perl builtins with prototype `$` — they take exactly one scalar-context
/// argument. Without this, something like `is(scalar @b, 1, "msg")` would
/// Perl's `while/for/until` condition auto-wraps iterator-style
/// expressions in `defined()` so the loop body runs even when each()
/// returns the falsy key 0 or readline() returns "0". Returns the
/// (possibly wrapped) condition.
///
/// Wraps when COND is:
///   * `<FH>` — auto-translate to `defined($_ = <FH>)`
///   * `each(X)` / `readline(X)` — bare iterator call
///   * `$var = <iter>` — assignment whose RHS is an iterator
///
/// Other conditions are returned unchanged.
/// Parse `EXPR` inside an interpolated subscript like `"$a[EXPR]"`.
/// Falls back to a string literal if parsing fails.
fn parse_index_expr(s: &str) -> Expr {
    use crate::lexer::Lexer;
    let mut lex = Lexer::new(s);
    let tokens = lex.tokenize();
    if !tokens.is_empty() && lex.error.is_none() {
        let mut parser = Parser::new(tokens);
        let e = parser.parse_expr();
        if parser.error.is_none() {
            return e;
        }
    }
    // Fall back to legacy behaviour for very simple cases so we
    // don't regress when the inner contains chars the standalone
    // lexer/parser would choke on.
    if let Ok(n) = s.trim().parse::<i64>() {
        return Expr::IntLit(n);
    }
    if let Some(v) = s.trim().strip_prefix('$') {
        return Expr::ScalarVar(v.to_string());
    }
    Expr::StringLit(s.to_string())
}

/// Names that should NOT trigger indirect method-call parsing even
/// when followed by another bareword. These are commonly-used
/// builtins/list operators where `name BAREWORD ...` is the regular
/// list-op syntax, not an indirect method call.
fn is_method_name_excluded(name: &str) -> bool {
    matches!(
        name,
        "print"
            | "printf"
            | "say"
            | "die"
            | "warn"
            | "return"
            | "do"
            | "eval"
            | "require"
            | "use"
            | "no"
            | "my"
            | "our"
            | "local"
            | "state"
            | "if"
            | "unless"
            | "while"
            | "until"
            | "for"
            | "foreach"
            | "sub"
            | "package"
            | "BEGIN"
            | "END"
            | "CHECK"
            | "INIT"
            | "and"
            | "or"
            | "not"
            | "xor"
            | "defined"
            | "exists"
            | "delete"
            | "ref"
            | "bless"
            | "scalar"
            | "wantarray"
            | "caller"
            | "open"
            | "close"
            | "read"
            | "binmode"
            | "split"
            | "join"
            | "map"
            | "grep"
            | "sort"
            | "reverse"
            | "push"
            | "pop"
            | "shift"
            | "unshift"
            | "splice"
            | "keys"
            | "values"
            | "each"
            | "substr"
            | "index"
            | "rindex"
            | "length"
            | "lc"
            | "uc"
            | "lcfirst"
            | "ucfirst"
            | "chr"
            | "ord"
            | "hex"
            | "oct"
            | "abs"
            | "int"
            | "sqrt"
            | "rand"
            | "srand"
            | "time"
            | "times"
            | "sprintf"
            | "chomp"
            | "chop"
            | "chown"
            | "chmod"
            | "exit"
            | "fork"
            | "wait"
            | "waitpid"
            | "system"
            | "exec"
            | "kill"
            | "sleep"
            | "tie"
            | "untie"
            | "tied"
            | "pos"
            | "study"
            | "select"
            | "fileno"
            | "tell"
            | "seek"
            | "eof"
            | "unlink"
            | "rename"
            | "mkdir"
            | "rmdir"
            | "chdir"
            | "stat"
            | "lstat"
            | "glob"
            | "readdir"
            | "opendir"
            | "closedir"
            | "qw"
            | "qr"
            | "q"
            | "qq"
            | "qx"
            | "pack"
            | "unpack"
            | "vec"
            | "format"
            | "write"
            | "lock"
            // Common test.pl helpers — `is $a, $b` must NOT parse as
            // `$a->is($b)`.
            | "is"
            | "isnt"
            | "like"
            | "unlike"
            | "cmp_ok"
            | "isa_ok"
            | "object_ok"
            | "can_ok"
            | "class_ok"
            | "ok"
            | "pass"
            | "fail"
            | "diag"
            | "note"
            | "plan"
            | "skip"
            | "todo"
            | "todo_skip"
            | "skip_all"
            | "eq_array"
            | "eq_hash"
            | "eq_set"
            | "warning_is"
            | "warning_like"
            | "warnings_like"
            | "fresh_perl"
            | "fresh_perl_is"
            | "fresh_perl_like"
            | "runperl"
            | "which_perl"
    )
}

fn wrap_iter_cond_with_defined(cond: Expr) -> Expr {
    match cond {
        Expr::Diamond(name) => Expr::Defined(Box::new(Expr::Assign(
            Box::new(Expr::ScalarVar("_".to_string())),
            Box::new(Expr::Diamond(name)),
        ))),
        Expr::Call(ref name, _) if name == "each" || name == "readline" => {
            Expr::Defined(Box::new(cond))
        }
        Expr::Assign(ref lhs, ref rhs) => {
            let lhs_is_scalar = matches!(
                lhs.as_ref(),
                Expr::ScalarVar(_) | Expr::MyVar(_) | Expr::LocalVar(_)
            );
            let rhs_is_iter = match rhs.as_ref() {
                Expr::Diamond(_) => true,
                Expr::Call(n, _) => n == "each" || n == "readline",
                _ => false,
            };
            if lhs_is_scalar && rhs_is_iter {
                Expr::Defined(Box::new(cond))
            } else {
                cond
            }
        }
        other => other,
    }
}

/// parse as `is(scalar(@b, 1, "msg"))`, swallowing the outer call's args.
/// Map a named-operator token back to its identifier so it can be
/// used as a bareword hash key inside `{...}`. Returns None for
/// tokens that aren't named operators (i.e. that have no bareword
/// equivalent). op/pos test #29-32 with `{length}`.
fn named_op_token_name(tok: &Token) -> Option<String> {
    let s: &str = match tok {
        Token::Length => "length",
        Token::Print => "print",
        Token::Say => "say",
        Token::Printf => "printf",
        Token::Return => "return",
        Token::My => "my",
        Token::Our => "our",
        Token::Local => "local",
        Token::Sub => "sub",
        Token::If => "if",
        Token::Unless => "unless",
        Token::Else => "else",
        Token::Elsif => "elsif",
        Token::While => "while",
        Token::Until => "until",
        Token::For => "for",
        Token::Foreach => "foreach",
        Token::Do => "do",
        Token::Last => "last",
        Token::Next => "next",
        Token::Redo => "redo",
        Token::Use => "use",
        Token::Package => "package",
        Token::Require => "require",
        Token::Die => "die",
        Token::Warn => "warn",
        Token::Eval => "eval",
        Token::Goto => "goto",
        Token::Not => "not",
        Token::And => "and",
        Token::Or => "or",
        Token::Xor => "xor",
        _ => return None,
    };
    Some(s.to_string())
}

fn is_unary_builtin(name: &str) -> bool {
    matches!(
        name,
        "scalar"
            | "defined"
            | "ref"
            | "chr"
            | "ord"
            | "lc"
            | "uc"
            | "lcfirst"
            | "ucfirst"
            | "hex"
            | "oct"
            | "int"
            | "abs"
            | "sqrt"
            | "exp"
            | "log"
            | "sin"
            | "cos"
            | "quotemeta"
            | "chop"
            | "chomp"
            | "pos"
            // `keys`/`values`/`each` take a single hash (or array) argument.
            // Without this, `print keys %h ? a : b, "rest"` parses as
            // `print keys(%h ? a : b, "rest")` — the ternary and the comma
            // get swallowed into keys()'s argument list.
            | "keys"
            | "values"
            | "each"
    )
}

/// Parse a double-quoted string with variable interpolation into an Interp expression.
fn parse_interp_string(s: &str) -> Expr {
    let mut parts = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut lit = String::new();

    while i < chars.len() {
        if chars[i] == '$' && i + 1 < chars.len() {
            // Variable interpolation
            let starts_pkg_prefix =
                chars[i + 1] == ':' && i + 2 < chars.len() && chars[i + 2] == ':';
            // Perl's punctuation globals: $@ (eval error), $! (errno), $/ (RS),
            // $\ (ORS), $, (OFS), $" (list separator), $; (subscript separator),
            // $| (autoflush), $& $` $' (regex matches).
            let is_punct_special = matches!(
                chars[i + 1],
                '@' | '!'
                    | '/'
                    | '\\'
                    | ','
                    | '"'
                    | ';'
                    | '|'
                    | '&'
                    | '`'
                    | '\''
                    | '?'
                    | '-'
                    | '+'
                    | '~'
                    | '%'
                    | '='
                    | ']'
                    | '['
            ) || (chars[i + 1] == '.'
                && (i + 2 >= chars.len() || !chars[i + 2].is_ascii_digit()))
                || (chars[i + 1] == ':' && (i + 2 >= chars.len() || chars[i + 2] != ':'));
            // `$$name` — scalar deref interpolation. Detect `$$` followed
            // by an ident (otherwise `$$` is the pid var or literal).
            let is_scalar_deref = chars[i + 1] == '$'
                && i + 2 < chars.len()
                && (chars[i + 2] == '_' || chars[i + 2].is_ascii_alphabetic());
            // `$$` followed by anything other than an ident (or end of
            // string) is the process-id special var.
            let is_pid = chars[i + 1] == '$' && !is_scalar_deref;
            // `$#name` — last-index of @name, interpolated.
            let is_array_len = chars[i + 1] == '#'
                && i + 2 < chars.len()
                && (chars[i + 2] == '_' || chars[i + 2].is_ascii_alphabetic());
            if chars[i + 1] == '_'
                || chars[i + 1].is_ascii_alphabetic()
                || chars[i + 1].is_ascii_digit()
                || chars[i + 1] == '{'
                || chars[i + 1] == '^'
                || starts_pkg_prefix
                || is_punct_special
                || is_scalar_deref
                || is_pid
                || is_array_len
            {
                // Flush literal
                if !lit.is_empty() {
                    parts.push(InterpPart::Lit(std::mem::take(&mut lit)));
                }

                i += 1; // skip $

                if chars[i] == '#' {
                    // `$#name` — last index of @name.
                    i += 1;
                    let mut name = String::new();
                    while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                        name.push(chars[i]);
                        i += 1;
                    }
                    parts.push(InterpPart::Expr(Box::new(Expr::ArrayLen(name))));
                } else if chars[i] == '$' {
                    // `$$name` — scalar deref of $name; `$$` alone is PID.
                    i += 1;
                    let mut name = String::new();
                    while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                        name.push(chars[i]);
                        i += 1;
                    }
                    if name.is_empty() {
                        parts.push(InterpPart::ScalarVar("$".to_string()));
                    } else {
                        parts.push(InterpPart::Expr(Box::new(Expr::ScalarDerefVar(name))));
                    }
                } else if chars[i] == '{' {
                    // ${var}, ${^VAR}, or ${ EXPR } block deref.
                    i += 1;
                    // Read the brace contents (matched braces).
                    let mut depth = 1;
                    let mut inner = String::new();
                    while i < chars.len() && depth > 0 {
                        match chars[i] {
                            '{' => {
                                depth += 1;
                                inner.push('{');
                            }
                            '}' => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                                inner.push('}');
                            }
                            c => inner.push(c),
                        }
                        i += 1;
                    }
                    if i < chars.len() {
                        i += 1; // skip closing }
                    } else if depth > 0 {
                        // Reference perl rejects unbalanced `${...}` (e.g.
                        // op/heredoc test 138's `${sub{b{]]]{}` shape) at
                        // parse time. Emit a `_parse_error` deferred to
                        // runtime so an enclosing `eval` can catch it via
                        // `$@`.
                        parts.push(InterpPart::Expr(Box::new(Expr::Call(
                            "_parse_error".to_string(),
                            vec![Expr::StringLit("syntax error".to_string())],
                        ))));
                        if !lit.is_empty() {
                            parts
                                .insert(parts.len() - 1, InterpPart::Lit(std::mem::take(&mut lit)));
                        }
                        return Expr::Interp(parts);
                    }
                    // Simple ident → scalar var. Otherwise, parse as expr
                    // and emit a block-deref Call.
                    let trimmed = inner.trim();
                    let is_simple_ident = !trimmed.is_empty()
                        && trimmed.chars().enumerate().all(|(idx, c)| {
                            if idx == 0 {
                                c == '_' || c == '^' || c.is_ascii_alphabetic()
                            } else {
                                c == '_' || c.is_ascii_alphanumeric() || c == ':'
                            }
                        });
                    if is_simple_ident {
                        parts.push(InterpPart::ScalarVar(trimmed.to_string()));
                    } else {
                        // Detect `${NAME[EXPR]}` / `${NAME{EXPR}}` —
                        // these are equivalent to `$NAME[EXPR]` / `$NAME{EXPR}`
                        // (array/hash element). Perl's brace-around-name
                        // disambiguator, not a scalar deref.
                        let bytes = trimmed.as_bytes();
                        let mut name_end = 0usize;
                        while name_end < bytes.len() {
                            let c = bytes[name_end] as char;
                            let ok = if name_end == 0 {
                                c == '_' || c == '^' || c.is_ascii_alphabetic()
                            } else {
                                c == '_' || c.is_ascii_alphanumeric() || c == ':'
                            };
                            if !ok {
                                break;
                            }
                            name_end += 1;
                        }
                        let after = trimmed[name_end..].trim();
                        let name_part = &trimmed[..name_end];
                        if !name_part.is_empty()
                            && (after.starts_with('[') || after.starts_with('{'))
                            && (after.ends_with(']') || after.ends_with('}'))
                        {
                            use crate::lexer::Lexer;
                            // Re-lex/parse the inner subscript expression.
                            let bracket = &after[..1];
                            let close = &after[after.len() - 1..];
                            let inside = after[1..after.len() - 1].trim();
                            let mut lex = Lexer::new(inside);
                            let toks = lex.tokenize();
                            let tl = std::mem::take(&mut lex.token_lines);
                            let f_over = std::mem::take(&mut lex.file_overrides);
                            let mut p = Parser::new_with_lines_and_files(toks, tl, f_over);
                            let idx_expr = p.parse_expr();
                            if bracket == "[" && close == "]" {
                                parts.push(InterpPart::ArrayElement(
                                    name_part.to_string(),
                                    Box::new(idx_expr),
                                ));
                            } else if bracket == "{" && close == "}" {
                                parts.push(InterpPart::HashElement(
                                    name_part.to_string(),
                                    Box::new(idx_expr),
                                ));
                            } else {
                                use crate::lexer::Lexer;
                                let mut lex = Lexer::new(&inner);
                                let toks = lex.tokenize();
                                let tl = std::mem::take(&mut lex.token_lines);
                                let f_over = std::mem::take(&mut lex.file_overrides);
                                let mut p = Parser::new_with_lines_and_files(toks, tl, f_over);
                                let inner_expr = p.parse_expr();
                                parts.push(InterpPart::Expr(Box::new(Expr::Call(
                                    "_scalar_block_deref".to_string(),
                                    vec![inner_expr],
                                ))));
                            }
                        } else {
                            use crate::lexer::Lexer;
                            let mut lex = Lexer::new(&inner);
                            let toks = lex.tokenize();
                            let tl = std::mem::take(&mut lex.token_lines);
                            let f_over = std::mem::take(&mut lex.file_overrides);
                            let mut p = Parser::new_with_lines_and_files(toks, tl, f_over);
                            let inner_expr = p.parse_expr();
                            // After `${EXPR}`, peek for `{KEY}` or `[IDX]`
                            // — these are arrow-deref subscripts treating
                            // EXPR as a hash/array ref. `"${\%x}{3}"`
                            // means `$x{3}`. base/lex test 78.
                            if i < chars.len() && (chars[i] == '{' || chars[i] == '[') {
                                let open = chars[i];
                                let close = if open == '{' { '}' } else { ']' };
                                i += 1;
                                let mut sub = String::new();
                                let mut depth = 1;
                                while i < chars.len() && depth > 0 {
                                    if chars[i] == open {
                                        depth += 1;
                                    } else if chars[i] == close {
                                        depth -= 1;
                                        if depth == 0 {
                                            break;
                                        }
                                    }
                                    sub.push(chars[i]);
                                    i += 1;
                                }
                                if i < chars.len() {
                                    i += 1;
                                }
                                let mut idx_lex = Lexer::new(&sub);
                                let idx_toks = idx_lex.tokenize();
                                let idx_tl = std::mem::take(&mut idx_lex.token_lines);
                                let idx_files = std::mem::take(&mut idx_lex.file_overrides);
                                let mut idx_p =
                                    Parser::new_with_lines_and_files(idx_toks, idx_tl, idx_files);
                                let idx_expr = idx_p.parse_expr();
                                let kind = if open == '[' {
                                    ArrowKind::Array
                                } else {
                                    ArrowKind::Hash
                                };
                                parts.push(InterpPart::Expr(Box::new(Expr::ArrowElement(
                                    Box::new(inner_expr),
                                    Box::new(idx_expr),
                                    kind,
                                ))));
                            } else {
                                parts.push(InterpPart::Expr(Box::new(Expr::Call(
                                    "_scalar_block_deref".to_string(),
                                    vec![inner_expr],
                                ))));
                            }
                        }
                    }
                } else if chars[i] == '^' && i + 1 < chars.len() {
                    i += 1;
                    let c = chars[i];
                    i += 1;
                    parts.push(InterpPart::ScalarVar(format!("^{c}")));
                } else if matches!(
                    chars[i],
                    '@' | '!'
                        | '/'
                        | '\\'
                        | ','
                        | '"'
                        | ';'
                        | '|'
                        | '&'
                        | '`'
                        | '\''
                        | '?'
                        | '-'
                        | '+'
                        | '~'
                        | '%'
                        | '='
                        | ']'
                        | '['
                ) || (chars[i] == '.'
                    && (i + 1 >= chars.len() || !chars[i + 1].is_ascii_digit()))
                    || (chars[i] == ':' && (i + 1 >= chars.len() || chars[i + 1] != ':'))
                {
                    // Single-char punctuation special variable.
                    // May be followed by an arrow-chain: `$@->{k}`, `$!->[0]`.
                    let c = chars[i];
                    i += 1;
                    if (c == '-' || c == '+') && i < chars.len() && chars[i] == '[' {
                        // `$-[N]` / `$+[N]` — match offset arrays, indexed by [N].
                        i += 1;
                        let mut idx_str = String::new();
                        let mut depth = 1;
                        while i < chars.len() && depth > 0 {
                            if chars[i] == '[' {
                                depth += 1;
                            } else if chars[i] == ']' {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            idx_str.push(chars[i]);
                            i += 1;
                        }
                        if i < chars.len() && chars[i] == ']' {
                            i += 1;
                        }
                        let idx_expr = if let Ok(n) = idx_str.parse::<i64>() {
                            Box::new(Expr::IntLit(n))
                        } else {
                            let v = idx_str.strip_prefix('$').unwrap_or(&idx_str);
                            Box::new(Expr::ScalarVar(v.to_string()))
                        };
                        parts.push(InterpPart::ArrayElement(c.to_string(), idx_expr));
                    } else if (c == '-' || c == '+') && i < chars.len() && chars[i] == '{' {
                        // `$-{name}` / `$+{name}` — named-capture hashes
                        // %- (multi-value) and %+. Single subscript only;
                        // arrow chains land in the next branch.
                        i += 1;
                        let mut key_str = String::new();
                        while i < chars.len() && chars[i] != '}' {
                            key_str.push(chars[i]);
                            i += 1;
                        }
                        if i < chars.len() && chars[i] == '}' {
                            i += 1;
                        }
                        let key_expr = if let Some(varname) = key_str.strip_prefix('$') {
                            Expr::ScalarVar(varname.to_string())
                        } else {
                            Expr::StringLit(key_str)
                        };
                        parts.push(InterpPart::HashElement(c.to_string(), Box::new(key_expr)));
                    } else if i + 1 < chars.len()
                        && chars[i] == '-'
                        && chars[i + 1] == '>'
                        && i + 2 < chars.len()
                        && (chars[i + 2] == '{' || chars[i + 2] == '[')
                    {
                        let mut accum: Expr = Expr::ScalarVar(c.to_string());
                        loop {
                            if i + 1 < chars.len() && chars[i] == '-' && chars[i + 1] == '>' {
                                i += 2;
                            }
                            if i >= chars.len() {
                                break;
                            }
                            match chars[i] {
                                '[' => {
                                    i += 1;
                                    let mut inner = String::new();
                                    let mut depth = 1;
                                    while i < chars.len() && depth > 0 {
                                        if chars[i] == '[' {
                                            depth += 1;
                                        } else if chars[i] == ']' {
                                            depth -= 1;
                                            if depth == 0 {
                                                break;
                                            }
                                        }
                                        inner.push(chars[i]);
                                        i += 1;
                                    }
                                    if i < chars.len() {
                                        i += 1;
                                    }
                                    let idx_expr = if let Ok(n) = inner.parse::<i64>() {
                                        Expr::IntLit(n)
                                    } else if let Some(v) = inner.strip_prefix('$') {
                                        Expr::ScalarVar(v.to_string())
                                    } else {
                                        Expr::StringLit(inner)
                                    };
                                    accum = Expr::ArrowElement(
                                        Box::new(accum),
                                        Box::new(idx_expr),
                                        crate::ast::ArrowKind::Array,
                                    );
                                }
                                '{' => {
                                    i += 1;
                                    let mut inner = String::new();
                                    while i < chars.len() && chars[i] != '}' {
                                        inner.push(chars[i]);
                                        i += 1;
                                    }
                                    if i < chars.len() {
                                        i += 1;
                                    }
                                    let key_expr = if let Some(v) = inner.strip_prefix('$') {
                                        Expr::ScalarVar(v.to_string())
                                    } else {
                                        Expr::StringLit(inner)
                                    };
                                    accum = Expr::ArrowElement(
                                        Box::new(accum),
                                        Box::new(key_expr),
                                        crate::ast::ArrowKind::Hash,
                                    );
                                }
                                _ => break,
                            }
                            if !(i + 1 < chars.len()
                                && chars[i] == '-'
                                && chars[i + 1] == '>'
                                && i + 2 < chars.len()
                                && (chars[i + 2] == '{' || chars[i + 2] == '['))
                            {
                                break;
                            }
                        }
                        parts.push(InterpPart::Expr(Box::new(accum)));
                    } else {
                        parts.push(InterpPart::ScalarVar(c.to_string()));
                    }
                } else {
                    let mut name = String::new();
                    // `$::foo` — leading `::` with no package name.
                    if i + 1 < chars.len() && chars[i] == ':' && chars[i + 1] == ':' {
                        name.push_str("::");
                        i += 2;
                    }
                    while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                        name.push(chars[i]);
                        i += 1;
                    }
                    // Check for :: package separator
                    while i + 1 < chars.len() && chars[i] == ':' && chars[i + 1] == ':' {
                        name.push_str("::");
                        i += 2;
                        while i < chars.len()
                            && (chars[i].is_ascii_alphanumeric() || chars[i] == '_')
                        {
                            name.push(chars[i]);
                            i += 1;
                        }
                    }
                    // Check for array subscript $name[idx]
                    if i < chars.len() && chars[i] == '[' {
                        i += 1;
                        let mut idx_str = String::new();
                        let mut depth = 1;
                        while i < chars.len() && depth > 0 {
                            if chars[i] == '[' {
                                depth += 1;
                            } else if chars[i] == ']' {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            idx_str.push(chars[i]);
                            i += 1;
                        }
                        if i < chars.len() && chars[i] == ']' {
                            i += 1;
                        }
                        // Parse the index as a full expression so
                        // `$a[$i+1]`, `$a[$x*2]` etc. work in
                        // interpolation (op/lop ^^ truth table uses
                        // `$a[$i+1]` in print).
                        let idx_expr = Box::new(parse_index_expr(&idx_str));
                        // `$name[i][j]` / `$name[i]{k}` — chained
                        // subscripts in interpolation become arrow
                        // chains (Perl auto-inserts `->` between them).
                        // Without this, `$a[0][0]` interpolates as
                        // `$a[0]` followed by literal `[0]`.
                        let mut chain_started = false;
                        let mut accum: Expr = Expr::Undef;
                        loop {
                            // Optional `->` separator (Perl allows it
                            // but doesn't require it between chained
                            // subscripts).
                            if i + 1 < chars.len() && chars[i] == '-' && chars[i + 1] == '>' {
                                i += 2;
                            }
                            if i >= chars.len() || (chars[i] != '[' && chars[i] != '{') {
                                break;
                            }
                            if !chain_started {
                                accum = Expr::ArrayElement(name.clone(), idx_expr.clone());
                                chain_started = true;
                            }
                            let opener = chars[i];
                            let (closer, kind) = if opener == '[' {
                                (']', crate::ast::ArrowKind::Array)
                            } else {
                                ('}', crate::ast::ArrowKind::Hash)
                            };
                            i += 1;
                            let mut inner = String::new();
                            let mut depth = 1;
                            while i < chars.len() && depth > 0 {
                                if chars[i] == opener {
                                    depth += 1;
                                } else if chars[i] == closer {
                                    depth -= 1;
                                    if depth == 0 {
                                        break;
                                    }
                                }
                                inner.push(chars[i]);
                                i += 1;
                            }
                            if i < chars.len() && chars[i] == closer {
                                i += 1;
                            }
                            let key_expr = if opener == '[' {
                                if let Ok(n) = inner.trim().parse::<i64>() {
                                    Expr::IntLit(n)
                                } else {
                                    let v = inner.strip_prefix('$').unwrap_or(&inner);
                                    Expr::ScalarVar(v.to_string())
                                }
                            } else if let Some(v) = inner.strip_prefix('$') {
                                Expr::ScalarVar(v.to_string())
                            } else {
                                Expr::StringLit(inner)
                            };
                            accum = Expr::ArrowElement(Box::new(accum), Box::new(key_expr), kind);
                        }
                        if chain_started {
                            parts.push(InterpPart::Expr(Box::new(accum)));
                        } else {
                            parts.push(InterpPart::ArrayElement(name, idx_expr));
                        }
                    } else if i < chars.len() && chars[i] == '{' {
                        // Hash subscript $name{key}
                        i += 1;
                        let mut key_str = String::new();
                        while i < chars.len() && chars[i] != '}' {
                            key_str.push(chars[i]);
                            i += 1;
                        }
                        if i < chars.len() && chars[i] == '}' {
                            i += 1;
                        }
                        // `$h{$k}` — key is a scalar variable reference.
                        // `$h{foo}` — auto-quoted bareword key (Perl semantics).
                        // `$h{\1}` / `$h{anything-non-bareword}` — parse the
                        // key as a Perl expression (op/hashwarn test 20).
                        let key_expr = if let Some(varname) = key_str.strip_prefix('$') {
                            Expr::ScalarVar(varname.to_string())
                        } else {
                            Expr::StringLit(key_str)
                        };
                        // Chained subscripts after the hash key
                        // (`$h{a}[0]`, `$h{a}{b}`).
                        let mut chain_started = false;
                        let mut accum: Expr = Expr::Undef;
                        loop {
                            if i + 1 < chars.len() && chars[i] == '-' && chars[i + 1] == '>' {
                                i += 2;
                            }
                            if i >= chars.len() || (chars[i] != '[' && chars[i] != '{') {
                                break;
                            }
                            if !chain_started {
                                accum = Expr::HashElement(name.clone(), Box::new(key_expr.clone()));
                                chain_started = true;
                            }
                            let opener = chars[i];
                            let (closer, kind) = if opener == '[' {
                                (']', crate::ast::ArrowKind::Array)
                            } else {
                                ('}', crate::ast::ArrowKind::Hash)
                            };
                            i += 1;
                            let mut inner = String::new();
                            let mut depth = 1;
                            while i < chars.len() && depth > 0 {
                                if chars[i] == opener {
                                    depth += 1;
                                } else if chars[i] == closer {
                                    depth -= 1;
                                    if depth == 0 {
                                        break;
                                    }
                                }
                                inner.push(chars[i]);
                                i += 1;
                            }
                            if i < chars.len() && chars[i] == closer {
                                i += 1;
                            }
                            let kex = if opener == '[' {
                                if let Ok(n) = inner.trim().parse::<i64>() {
                                    Expr::IntLit(n)
                                } else {
                                    let v = inner.strip_prefix('$').unwrap_or(&inner);
                                    Expr::ScalarVar(v.to_string())
                                }
                            } else if let Some(v) = inner.strip_prefix('$') {
                                Expr::ScalarVar(v.to_string())
                            } else {
                                Expr::StringLit(inner)
                            };
                            accum = Expr::ArrowElement(Box::new(accum), Box::new(kex), kind);
                        }
                        if chain_started {
                            parts.push(InterpPart::Expr(Box::new(accum)));
                        } else {
                            parts.push(InterpPart::HashElement(name, Box::new(key_expr)));
                        }
                    } else if i + 1 < chars.len()
                        && chars[i] == '-'
                        && chars[i + 1] == '>'
                        && i + 2 < chars.len()
                        && (chars[i + 2] == '{' || chars[i + 2] == '[')
                    {
                        // `$ref->[i]` / `$ref->{k}` — arrow deref. Walk a chain
                        // of `->[...]` and `->{...}` so `$h->{a}->[0]->{b}`
                        // interpolates as the chained arrow expression.
                        let mut accum: Expr = Expr::ScalarVar(name);
                        loop {
                            // Optional `->`. The first iteration's `->` was
                            // already detected; subsequent ones also need it
                            // (Perl allows omitting `->` between successive
                            // subscripts, so we accept either).
                            if i + 1 < chars.len() && chars[i] == '-' && chars[i + 1] == '>' {
                                i += 2;
                            }
                            if i >= chars.len() {
                                break;
                            }
                            match chars[i] {
                                '[' => {
                                    i += 1;
                                    let mut inner = String::new();
                                    let mut depth = 1;
                                    while i < chars.len() && depth > 0 {
                                        if chars[i] == '[' {
                                            depth += 1;
                                        } else if chars[i] == ']' {
                                            depth -= 1;
                                            if depth == 0 {
                                                break;
                                            }
                                        }
                                        inner.push(chars[i]);
                                        i += 1;
                                    }
                                    if i < chars.len() {
                                        i += 1; // skip ]
                                    }
                                    let idx_expr = if let Ok(n) = inner.parse::<i64>() {
                                        Expr::IntLit(n)
                                    } else if let Some(v) = inner.strip_prefix('$') {
                                        Expr::ScalarVar(v.to_string())
                                    } else {
                                        Expr::StringLit(inner)
                                    };
                                    accum = Expr::ArrowElement(
                                        Box::new(accum),
                                        Box::new(idx_expr),
                                        crate::ast::ArrowKind::Array,
                                    );
                                }
                                '{' => {
                                    i += 1;
                                    let mut inner = String::new();
                                    while i < chars.len() && chars[i] != '}' {
                                        inner.push(chars[i]);
                                        i += 1;
                                    }
                                    if i < chars.len() {
                                        i += 1; // skip }
                                    }
                                    let key_expr = if let Some(v) = inner.strip_prefix('$') {
                                        Expr::ScalarVar(v.to_string())
                                    } else {
                                        Expr::StringLit(inner)
                                    };
                                    accum = Expr::ArrowElement(
                                        Box::new(accum),
                                        Box::new(key_expr),
                                        crate::ast::ArrowKind::Hash,
                                    );
                                }
                                _ => break,
                            }
                            // Continue if another arrow + subscript follows.
                            if !(i + 1 < chars.len()
                                && chars[i] == '-'
                                && chars[i + 1] == '>'
                                && i + 2 < chars.len()
                                && (chars[i + 2] == '{' || chars[i + 2] == '['))
                            {
                                break;
                            }
                        }
                        parts.push(InterpPart::Expr(Box::new(accum)));
                    } else {
                        parts.push(InterpPart::ScalarVar(name));
                    }
                }
            } else {
                lit.push(chars[i]);
                i += 1;
            }
        } else if chars[i] == '@' && i + 1 < chars.len() && chars[i + 1] == '\'' {
            // `@'NAME` uses old Perl's apostrophe-as-namespace-separator
            // (`'` ~= `::`). Bare `@'` is variable `@::` (empty).
            // Consume `@'` plus any further ident-or-' chars; emit empty.
            if !lit.is_empty() {
                parts.push(InterpPart::Lit(std::mem::take(&mut lit)));
            }
            i += 2; // skip @'
            while i < chars.len()
                && (chars[i].is_ascii_alphanumeric() || chars[i] == '_' || chars[i] == '\'')
            {
                i += 1;
            }
        } else if chars[i] == '@'
            && i + 1 < chars.len()
            && (chars[i + 1].is_ascii_alphanumeric()
                || chars[i + 1] == '_'
                || chars[i + 1] == '{'
                || chars[i + 1] == '$'
                || chars[i + 1] == '-'
                || chars[i + 1] == '+')
        {
            // Array interpolation
            if !lit.is_empty() {
                parts.push(InterpPart::Lit(std::mem::take(&mut lit)));
            }
            i += 1; // skip @
            if chars[i] == '{' {
                // `@{ EXPR }` — parse the inner expression and treat its
                // value as an array (block deref).
                i += 1;
                let mut depth = 1;
                let mut inner = String::new();
                while i < chars.len() && depth > 0 {
                    match chars[i] {
                        '{' => {
                            depth += 1;
                            inner.push('{');
                        }
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                            inner.push('}');
                        }
                        c => inner.push(c),
                    }
                    i += 1;
                }
                if i < chars.len() {
                    i += 1; // skip closing }
                }
                // Parse the inner string via the lexer/parser pair.
                use crate::lexer::Lexer;
                let mut lex = Lexer::new(&inner);
                let toks = lex.tokenize();
                let tl = std::mem::take(&mut lex.token_lines);
                let f_over = std::mem::take(&mut lex.file_overrides);
                let mut p = Parser::new_with_lines_and_files(toks, tl, f_over);
                let inner_expr = p.parse_expr();
                parts.push(InterpPart::Expr(Box::new(Expr::Call(
                    "_array_block_deref".to_string(),
                    vec![inner_expr],
                ))));
            } else if chars[i] == '$' {
                // @$name — dereference array ref
                i += 1;
                let mut name = String::new();
                while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                    name.push(chars[i]);
                    i += 1;
                }
                parts.push(InterpPart::Expr(Box::new(Expr::ArrayDerefVar(name))));
            } else if chars[i] == '-' || chars[i] == '+' {
                // `@-` / `@+` — match-position special arrays. Single-
                // character name; no further name chars consumed.
                let name = chars[i].to_string();
                i += 1;
                parts.push(InterpPart::ArrayVar(name));
            } else {
                let mut name = String::new();
                while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                    name.push(chars[i]);
                    i += 1;
                }
                // Allow `@Pkg::name` (greedy `::` segments after the head).
                while i + 1 < chars.len() && chars[i] == ':' && chars[i + 1] == ':' {
                    name.push_str("::");
                    i += 2;
                    while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                        name.push(chars[i]);
                        i += 1;
                    }
                }
                parts.push(InterpPart::ArrayVar(name));
            }
        } else if chars[i] == '\u{F0001}' {
            // Escaped $ placeholder (private-use plane to avoid colliding
            // with literal `\x01` the user wrote).
            lit.push('$');
            i += 1;
        } else if chars[i] == '\u{F0002}' {
            // Escaped @ placeholder.
            lit.push('@');
            i += 1;
        } else if chars[i] == '\u{F0003}' {
            // Zero-width var-boundary marker emitted by process_escapes
            // for unknown `\X` escapes. Strip it from the literal — its
            // only role is to terminate any preceding sigil's var name.
            i += 1;
        } else {
            lit.push(chars[i]);
            i += 1;
        }
    }

    if !lit.is_empty() {
        parts.push(InterpPart::Lit(lit));
    }

    if parts.len() == 1 {
        match parts.into_iter().next().unwrap() {
            InterpPart::Lit(s) => Expr::StringLit(s),
            other => Expr::Interp(vec![other]),
        }
    } else {
        Expr::Interp(parts)
    }
}

impl PartialEq for Token {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Token::Ident(a), Token::Ident(b)) => a == b,
            (Token::StringLit(a), Token::StringLit(b)) => a == b,
            (Token::Integer(a), Token::Integer(b)) => a == b,
            (Token::Float(a), Token::Float(b)) => a == b,
            _ => std::mem::discriminant(self) == std::mem::discriminant(other),
        }
    }
}

/// Scan the token stream for `sub IDENT` declarations and return the set
/// of sub names. Used so the parser can convert `name;` (bareword statement)
/// into a Call when `name` is a known sub. Pre-seeded with the common
/// helpers from t/test.pl since test files reference them via bareword
/// before any local `sub NAME` declaration would put them in scope.
fn scan_sub_names(tokens: &[Token]) -> HashSet<String> {
    let mut subs: HashSet<String> = [
        // Subs declared by t/test.pl (loaded via `require './test.pl'`
        // in BEGIN blocks). These need to be known up-front so that
        // bareword statement forms like `pass;` / `note 'm';` parse as
        // calls instead of bareword strings.
        "BAIL_OUT",
        "can_ok",
        "capture_warnings",
        "class_ok",
        "cmp_ok",
        "curr_test",
        "diag",
        "DIE",
        "display",
        "display_rx",
        "done_testing",
        "eq_array",
        "eq_hash",
        "fail",
        "find_git_or_skip",
        "fresh_perl",
        "fresh_perl_is",
        "fresh_perl_like",
        "is",
        "isa_ok",
        "is_linux_container",
        "is_miniperl",
        "isnt",
        "like",
        "like_yn",
        "new_ok",
        "next_test",
        "note",
        "object_ok",
        "ok",
        "pass",
        "plan",
        "refcount_is",
        "register_tempfile",
        "require_ok",
        "run_multiple_progs",
        "runperl",
        "runperl_and_capture",
        "set_up_inc",
        "setup_multiple_progs",
        "skip",
        "skip_all",
        "skip_all_if_miniperl",
        "skip_all_without_config",
        "skip_all_without_dynamic_extension",
        "skip_all_without_perlio",
        "skip_all_without_unicode_tables",
        "skip_if_miniperl",
        "skip_without_dynamic_extension",
        "tempfile",
        "todo_skip",
        "unlike",
        "unlink_all",
        "unlink_tempfiles",
        "untaint_path",
        "use_ok",
        "warning_is",
        "warning_like",
        "warnings_like",
        "watchdog",
        "which_perl",
        "within",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    for w in tokens.windows(2) {
        if matches!(w[0], Token::Sub) {
            if let Token::Ident(n) = &w[1] {
                subs.insert(n.clone());
                // Also accept the unqualified tail of `Pkg::name`.
                if let Some(idx) = n.rfind("::") {
                    subs.insert(n[idx + 2..].to_string());
                }
            }
        }
    }
    subs
}

/// Best-effort short rendering of a token for error messages.
fn token_display(t: &Token) -> String {
    match t {
        Token::Semi => ";".to_string(),
        Token::RBrace => "}".to_string(),
        Token::RParen => ")".to_string(),
        Token::RBracket => "]".to_string(),
        Token::Comma => ",".to_string(),
        Token::EOF => "EOF".to_string(),
        Token::LogAnd => "&&".to_string(),
        Token::LogOr => "||".to_string(),
        Token::DefOr => "//".to_string(),
        Token::And => "and".to_string(),
        Token::Or => "or".to_string(),
        Token::Spaceship => "<=>".to_string(),
        Token::NumEq => "==".to_string(),
        Token::NumNe => "!=".to_string(),
        Token::NumLe => "<=".to_string(),
        Token::NumGe => ">=".to_string(),
        Token::Ident(s) => s.clone(),
        Token::ScalarVar(s) => format!("${s}"),
        Token::ArrayVar(s) => format!("@{s}"),
        Token::HashVar(s) => format!("%{s}"),
        Token::StringLit(s) => format!("\"{s}\""),
        Token::Integer(n) => n.to_string(),
        Token::Float(n) => n.to_string(),
        _ => format!("{t:?}"),
    }
}
