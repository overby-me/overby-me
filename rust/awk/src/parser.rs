use crate::ast::*;
use crate::lexer::Token;

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    function_names: std::collections::HashSet<String>,
    in_print_context: bool,
    token_lines: Vec<usize>,
    token_cols: Vec<usize>,
    source_lines: Vec<String>,
}

const BUILTIN_FUNCTIONS: &[&str] = &[
    "length", "substr", "index", "split", "sub", "gsub", "gensub", "match", "sprintf", "tolower",
    "toupper", "sin", "cos", "atan2", "exp", "log", "sqrt", "int", "rand", "srand", "system",
    "close", "mktime", "systime", "strftime", "typeof", "asort", "asorti", "patsplit",
];

impl Parser {
    pub fn new_with_source(
        tokens: Vec<Token>,
        token_lines: Vec<usize>,
        token_cols: Vec<usize>,
        source: &str,
    ) -> Self {
        let source_lines: Vec<String> = source.lines().map(|s| s.to_string()).collect();
        let mut p = Self::new(tokens);
        p.token_lines = token_lines;
        p.token_cols = token_cols;
        p.source_lines = source_lines;
        p
    }

    pub fn new(tokens: Vec<Token>) -> Self {
        // Pre-scan for function definitions to enable forward references
        let mut function_names: std::collections::HashSet<String> =
            BUILTIN_FUNCTIONS.iter().map(|s| s.to_string()).collect();
        let mut i = 0;
        while i < tokens.len() {
            if matches!(tokens[i], Token::Function)
                && let Some(Token::Ident(name)) = tokens.get(i + 1)
            {
                function_names.insert(name.clone());
            }
            i += 1;
        }
        Parser {
            tokens,
            pos: 0,
            function_names,
            in_print_context: false,
            token_lines: Vec::new(),
            token_cols: Vec::new(),
            source_lines: Vec::new(),
        }
    }

    fn peek(&self) -> &Token {
        self.tokens.get(self.pos).unwrap_or(&Token::Eof)
    }

    fn advance(&mut self) -> Token {
        let tok = self.tokens.get(self.pos).cloned().unwrap_or(Token::Eof);
        self.pos += 1;
        tok
    }

    fn syntax_error_at_current(&self) -> ! {
        let pos = self.pos.min(self.token_lines.len().saturating_sub(1));
        let line = self.token_lines.get(pos).copied().unwrap_or(1);
        let col = self.token_cols.get(pos).copied().unwrap_or(1);
        if let Some(src) = self.source_lines.get(line.saturating_sub(1)) {
            eprintln!("awk: {src}");
            eprintln!("awk: {:>width$} syntax error", "^", width = col);
        } else {
            eprintln!("awk: syntax error");
        }
        std::process::exit(1);
    }

    fn syntax_error_at(&self, pos: usize, msg: &str) -> ! {
        let line = self.token_lines.get(pos).copied().unwrap_or(1);
        let col = self.token_cols.get(pos).copied().unwrap_or(1);
        if let Some(src) = self.source_lines.get(line.saturating_sub(1)) {
            eprintln!("awk: {src}");
            eprintln!("awk: {:>width$} {msg}", "^", width = col);
        } else {
            eprintln!("awk: {msg}");
        }
        std::process::exit(1);
    }

    fn expect(&mut self, expected: &Token) {
        let tok = self.advance();
        if std::mem::discriminant(&tok) != std::mem::discriminant(expected) {
            // Point at the unexpected token we just consumed
            self.syntax_error_at(self.pos.saturating_sub(1), "syntax error");
        }
    }

    fn skip_terminators(&mut self) {
        while matches!(self.peek(), Token::Newline | Token::Semicolon) {
            self.advance();
        }
    }

    pub fn parse(&mut self) -> Program {
        let mut rules = Vec::new();
        let mut functions = Vec::new();

        self.skip_terminators();

        while !matches!(self.peek(), Token::Eof) {
            if matches!(self.peek(), Token::Function) {
                functions.push(self.parse_function_def());
            } else {
                rules.push(self.parse_rule());
            }
            self.skip_terminators();
        }

        Program { rules, functions }
    }

    fn parse_function_def(&mut self) -> FuncDef {
        self.advance(); // 'function'
        let name = match self.advance() {
            Token::Ident(s) => s,
            _ => String::new(),
        };
        // Check for redefining builtin functions
        if BUILTIN_FUNCTIONS.contains(&name.as_str()) {
            let line = self
                .token_lines
                .get(self.pos.saturating_sub(1))
                .copied()
                .unwrap_or(1);
            if let Some(src) = self.source_lines.get(line.saturating_sub(1)) {
                eprintln!("awk: {src}");
            }
            eprintln!("awk:          ^ `{name}' is a built-in function, it cannot be redefined");
            std::process::exit(1);
        }
        self.expect(&Token::LParen);
        let mut params = Vec::new();
        let mut seen_params: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        let mut has_errors = false;
        let mut expect_param = false;
        while !matches!(self.peek(), Token::RParen | Token::Eof) {
            if matches!(self.peek(), Token::Comma) {
                if expect_param {
                    // Two commas in a row - syntax error
                    self.syntax_error_at_current();
                }
                self.advance();
                expect_param = true;
                continue;
            }
            expect_param = false;
            if let Token::Ident(s) = self.advance() {
                // Check for duplicate parameter names
                if let Some(&first_idx) = seen_params.get(&s) {
                    eprintln!(
                        "awk: error: function `{name}': parameter #{}, `{s}', duplicates parameter #{first_idx}",
                        params.len() + 1
                    );
                    has_errors = true;
                }
                // Check for function name used as parameter
                if s == name {
                    eprintln!(
                        "awk: error: function `{name}': cannot use function name as parameter name"
                    );
                    std::process::exit(1);
                }
                // Check for special variable used as parameter
                const SPECIAL_VARS: &[&str] = &[
                    "FS", "RS", "OFS", "ORS", "NR", "NF", "FNR", "FILENAME", "SUBSEP", "RSTART",
                    "RLENGTH", "OFMT", "CONVFMT", "ARGC", "ARGV", "ENVIRON", "ERRNO", "RT",
                ];
                if SPECIAL_VARS.contains(&s.as_str()) {
                    eprintln!(
                        "awk: error: function `{name}': parameter `{s}': POSIX disallows using a special variable as a function parameter"
                    );
                    has_errors = true;
                }
                seen_params.entry(s.clone()).or_insert(params.len() + 1);
                params.push(s);
            }
            if matches!(self.peek(), Token::Comma) {
                self.advance();
                expect_param = true;
            }
        }
        self.expect(&Token::RParen);
        if has_errors {
            std::process::exit(1);
        }
        self.skip_terminators();
        let body = self.parse_action();
        FuncDef { name, params, body }
    }

    fn parse_rule(&mut self) -> Rule {
        match self.peek().clone() {
            Token::Begin => {
                self.advance();
                self.skip_terminators();
                let action = self.parse_action();
                Rule {
                    pattern: Some(Pattern::Begin),
                    action,
                }
            }
            Token::End => {
                self.advance();
                self.skip_terminators();
                let action = self.parse_action();
                Rule {
                    pattern: Some(Pattern::End),
                    action,
                }
            }
            Token::LBrace => {
                let action = self.parse_action();
                Rule {
                    pattern: None,
                    action,
                }
            }
            _ => {
                let expr = self.parse_expr();
                if matches!(self.peek(), Token::Comma) {
                    self.advance();
                    self.skip_terminators();
                    let expr2 = self.parse_expr();
                    self.skip_terminators();
                    if matches!(self.peek(), Token::LBrace) {
                        let action = self.parse_action();
                        Rule {
                            pattern: Some(Pattern::Range(expr, expr2)),
                            action,
                        }
                    } else {
                        Rule {
                            pattern: Some(Pattern::Range(expr, expr2)),
                            action: vec![Stmt::Print(vec![], None)],
                        }
                    }
                } else {
                    self.skip_terminators();
                    if matches!(self.peek(), Token::LBrace) {
                        let action = self.parse_action();
                        Rule {
                            pattern: Some(Pattern::Expression(expr)),
                            action,
                        }
                    } else {
                        Rule {
                            pattern: Some(Pattern::Expression(expr)),
                            action: vec![Stmt::Print(vec![], None)],
                        }
                    }
                }
            }
        }
    }

    fn parse_action(&mut self) -> Vec<Stmt> {
        self.expect(&Token::LBrace);
        self.skip_terminators();
        let mut stmts = Vec::new();
        while !matches!(self.peek(), Token::RBrace | Token::Eof) {
            stmts.push(self.parse_stmt());
            self.skip_terminators();
        }
        if matches!(self.peek(), Token::Eof) {
            // Unterminated block — show the last non-empty source line
            let src = self
                .source_lines
                .iter()
                .rev()
                .find(|s| !s.trim().is_empty())
                .cloned()
                .unwrap_or_default();
            eprintln!("awk: {src}");
            eprintln!("awk: ^ unexpected newline or end of string");
            std::process::exit(1);
        }
        self.expect(&Token::RBrace);
        stmts
    }

    fn parse_stmt(&mut self) -> Stmt {
        match self.peek().clone() {
            Token::If => self.parse_if(),
            Token::While => self.parse_while(),
            Token::For => self.parse_for(),
            Token::Do => self.parse_do_while(),
            Token::LBrace => {
                let stmts = self.parse_action();
                Stmt::Block(stmts)
            }
            Token::Print => self.parse_print(),
            Token::Printf => self.parse_printf(),
            Token::Next => {
                self.advance();
                Stmt::Next
            }
            Token::Exit => {
                self.advance();
                if matches!(
                    self.peek(),
                    Token::Newline | Token::Semicolon | Token::RBrace | Token::Eof
                ) {
                    Stmt::Exit(None)
                } else {
                    Stmt::Exit(Some(self.parse_expr()))
                }
            }
            Token::Delete => {
                self.advance();
                if let Token::Ident(name) = self.advance() {
                    if matches!(self.peek(), Token::LBracket) {
                        self.advance();
                        let mut indices = vec![self.parse_expr()];
                        while matches!(self.peek(), Token::Comma) {
                            self.advance();
                            indices.push(self.parse_expr());
                        }
                        self.expect(&Token::RBracket);
                        Stmt::Delete(name, indices)
                    } else {
                        Stmt::Delete(name, vec![])
                    }
                } else {
                    Stmt::Expr(Expr::Number(0.0))
                }
            }
            Token::Break => {
                self.advance();
                Stmt::Break
            }
            Token::Continue => {
                self.advance();
                Stmt::Continue
            }
            Token::Return => {
                self.advance();
                if matches!(
                    self.peek(),
                    Token::Newline | Token::Semicolon | Token::RBrace | Token::Eof
                ) {
                    Stmt::Return(None)
                } else {
                    Stmt::Return(Some(self.parse_expr()))
                }
            }
            _ => {
                let expr = self.parse_expr();
                Stmt::Expr(expr)
            }
        }
    }

    fn parse_if(&mut self) -> Stmt {
        self.advance(); // if
        self.expect(&Token::LParen);
        let cond = self.parse_expr();
        self.expect(&Token::RParen);
        self.skip_terminators();
        let then_branch = self.parse_stmt();
        self.skip_terminators();
        let else_branch = if matches!(self.peek(), Token::Else) {
            self.advance();
            self.skip_terminators();
            Some(Box::new(self.parse_stmt()))
        } else {
            None
        };
        Stmt::If(cond, Box::new(then_branch), else_branch)
    }

    fn parse_while(&mut self) -> Stmt {
        self.advance(); // while
        self.expect(&Token::LParen);
        let cond = self.parse_expr();
        self.expect(&Token::RParen);
        self.skip_terminators();
        let body = self.parse_stmt();
        Stmt::While(cond, Box::new(body))
    }

    fn parse_for(&mut self) -> Stmt {
        self.advance(); // for
        self.expect(&Token::LParen);

        // Check for for-in: for (var in array)
        let saved_pos = self.pos;
        if let Token::Ident(var_name) = self.peek().clone() {
            self.advance();
            if matches!(self.peek(), Token::In) {
                self.advance();
                if let Token::Ident(arr_name) = self.advance() {
                    self.expect(&Token::RParen);
                    self.skip_terminators();
                    let body = self.parse_stmt();
                    return Stmt::ForIn(var_name, arr_name, Box::new(body));
                }
            }
        }
        self.pos = saved_pos;

        // Regular for loop
        let init = if matches!(self.peek(), Token::Semicolon) {
            None
        } else {
            Some(Box::new(self.parse_stmt()))
        };
        self.expect(&Token::Semicolon);
        let cond = if matches!(self.peek(), Token::Semicolon) {
            None
        } else {
            Some(self.parse_expr())
        };
        self.expect(&Token::Semicolon);
        let update = if matches!(self.peek(), Token::RParen) {
            None
        } else {
            Some(Box::new(self.parse_stmt()))
        };
        self.expect(&Token::RParen);
        self.skip_terminators();
        let body = self.parse_stmt();
        Stmt::For(init, cond, update, Box::new(body))
    }

    fn parse_do_while(&mut self) -> Stmt {
        self.advance(); // do
        self.skip_terminators();
        let body = self.parse_stmt();
        self.skip_terminators();
        self.expect(&Token::While);
        self.expect(&Token::LParen);
        let cond = self.parse_expr();
        self.expect(&Token::RParen);
        Stmt::DoWhile(Box::new(body), cond)
    }

    fn parse_output_dest(&mut self) -> Option<OutputDest> {
        match self.peek() {
            Token::Gt => {
                self.advance();
                Some(OutputDest::File(self.parse_primary()))
            }
            Token::Append => {
                self.advance();
                Some(OutputDest::Append(self.parse_primary()))
            }
            Token::Pipe => {
                self.advance();
                Some(OutputDest::Pipe(self.parse_primary()))
            }
            _ => None,
        }
    }

    fn parse_print(&mut self) -> Stmt {
        self.advance(); // print
        let mut args = Vec::new();

        if !matches!(
            self.peek(),
            Token::Newline
                | Token::Semicolon
                | Token::RBrace
                | Token::Eof
                | Token::Gt
                | Token::Append
                | Token::Pipe
        ) {
            args.push(self.parse_non_assign_expr());
            while matches!(self.peek(), Token::Comma) {
                self.advance();
                self.skip_terminators(); // skip newlines between args
                args.push(self.parse_non_assign_expr());
            }
        }

        let dest = self.parse_output_dest();
        Stmt::Print(args, dest)
    }

    fn parse_printf(&mut self) -> Stmt {
        self.advance(); // printf
        let mut args = Vec::new();

        // Handle printf(args) — parenthesized form
        if matches!(self.peek(), Token::LParen) {
            let saved = self.pos;
            self.advance(); // skip (
            let first = self.parse_expr();
            if matches!(self.peek(), Token::RParen) {
                // printf(expr) — single arg in parens
                self.advance();
                args.push(first);
                let dest = self.parse_output_dest();
                return Stmt::Printf(args, dest);
            } else if matches!(self.peek(), Token::Comma) {
                // printf(fmt, args...) — multiple args in parens
                args.push(first);
                while matches!(self.peek(), Token::Comma) {
                    self.advance();
                    self.skip_terminators(); // skip newlines between args
                    args.push(self.parse_expr());
                }
                self.expect(&Token::RParen);
                let dest = self.parse_output_dest();
                return Stmt::Printf(args, dest);
            }
            // Not a parenthesized call, backtrack
            self.pos = saved;
        }

        if !matches!(
            self.peek(),
            Token::Newline | Token::Semicolon | Token::RBrace | Token::Eof
        ) {
            args.push(self.parse_non_assign_expr());
            while matches!(self.peek(), Token::Comma) {
                self.advance();
                self.skip_terminators(); // skip newlines between args
                args.push(self.parse_non_assign_expr());
            }
        }

        let dest = self.parse_output_dest();
        Stmt::Printf(args, dest)
    }

    // Expression parsing with precedence climbing

    fn parse_expr(&mut self) -> Expr {
        self.parse_assignment()
    }

    fn parse_non_assign_expr(&mut self) -> Expr {
        // In print context, parse without consuming > as comparison
        // (it might be a redirect) but allow assignments like print b += 1
        self.in_print_context = true;
        let result = self.parse_assignment();
        self.in_print_context = false;
        result
    }

    fn check_lvalue(&self, expr: &Expr) {
        let is_post_inc = matches!(expr, Expr::PostIncrement(_) | Expr::PostDecrement(_));
        let is_field_post_inc = matches!(
            expr,
            Expr::FieldRef(inner) if matches!(inner.as_ref(), Expr::PostIncrement(_) | Expr::PostDecrement(_))
        );
        if is_post_inc || is_field_post_inc {
            self.syntax_error_at(
                self.pos,
                "cannot assign a value to the result of a field post-increment expression",
            );
        }
    }

    fn parse_assignment(&mut self) -> Expr {
        let expr = self.parse_ternary();
        match self.peek() {
            Token::Assign => {
                self.check_lvalue(&expr);
                self.advance();
                let rhs = self.parse_assignment();
                Expr::Assign(Box::new(expr), Box::new(rhs))
            }
            Token::PlusAssign => {
                self.advance();
                let rhs = self.parse_assignment();
                Expr::OpAssign(Box::new(expr), BinOp::Add, Box::new(rhs))
            }
            Token::MinusAssign => {
                self.advance();
                let rhs = self.parse_assignment();
                Expr::OpAssign(Box::new(expr), BinOp::Sub, Box::new(rhs))
            }
            Token::StarAssign => {
                self.advance();
                let rhs = self.parse_assignment();
                Expr::OpAssign(Box::new(expr), BinOp::Mul, Box::new(rhs))
            }
            Token::SlashAssign => {
                self.advance();
                let rhs = self.parse_assignment();
                Expr::OpAssign(Box::new(expr), BinOp::Div, Box::new(rhs))
            }
            Token::PercentAssign => {
                self.advance();
                let rhs = self.parse_assignment();
                Expr::OpAssign(Box::new(expr), BinOp::Mod, Box::new(rhs))
            }
            Token::CaretAssign => {
                self.advance();
                let rhs = self.parse_assignment();
                Expr::OpAssign(Box::new(expr), BinOp::Pow, Box::new(rhs))
            }
            _ => expr,
        }
    }

    fn parse_ternary(&mut self) -> Expr {
        let cond = self.parse_or();
        if matches!(self.peek(), Token::Question) {
            self.advance();
            let then_expr = self.parse_assignment();
            self.expect(&Token::Colon);
            let else_expr = self.parse_assignment();
            Expr::Ternary(Box::new(cond), Box::new(then_expr), Box::new(else_expr))
        } else {
            cond
        }
    }

    fn parse_or(&mut self) -> Expr {
        let mut left = self.parse_and();
        while matches!(self.peek(), Token::Or) {
            self.advance();
            // Right operand of || allows assignment (e.g., expr || $0 = $1)
            let right = self.parse_assignment();
            left = Expr::Binop(Box::new(left), BinOp::Or, Box::new(right));
        }
        left
    }

    fn parse_and(&mut self) -> Expr {
        let mut left = self.parse_in_expr();
        while matches!(self.peek(), Token::And) {
            self.advance();
            // Right operand of && allows assignment (e.g., gsub() && $0 = $1)
            let right = self.parse_assignment();
            left = Expr::Binop(Box::new(left), BinOp::And, Box::new(right));
        }
        left
    }

    fn parse_in_expr(&mut self) -> Expr {
        let left = self.parse_match();
        if matches!(self.peek(), Token::In) {
            self.advance();
            if let Token::Ident(arr) = self.advance() {
                return Expr::In(Box::new(left), arr);
            }
        }
        left
    }

    fn parse_match(&mut self) -> Expr {
        let left = self.parse_comparison();
        match self.peek() {
            Token::Match => {
                self.advance();
                let right = self.parse_comparison();
                Expr::Match(Box::new(left), Box::new(right))
            }
            Token::NotMatch => {
                self.advance();
                let right = self.parse_comparison();
                Expr::NotMatch(Box::new(left), Box::new(right))
            }
            _ => left,
        }
    }

    fn parse_comparison(&mut self) -> Expr {
        let left = self.parse_concatenation();
        let op = match self.peek() {
            Token::Eq => BinOp::Eq,
            Token::Ne => BinOp::Ne,
            Token::Lt => BinOp::Lt,
            // In print context, > and >= could be redirects, skip them
            Token::Gt if !self.in_print_context => BinOp::Gt,
            Token::Ge if !self.in_print_context => BinOp::Ge,
            Token::Gt | Token::Ge if self.in_print_context => return left,
            Token::Le => BinOp::Le,
            _ => return left,
        };
        self.advance();
        let right = self.parse_concatenation();
        Expr::Binop(Box::new(left), op, Box::new(right))
    }

    fn parse_concatenation(&mut self) -> Expr {
        let mut left = self.parse_addition();

        // Concatenation by juxtaposition: if the next token can start an expression
        // but is not an operator, it's concatenation
        while let Token::Number(_)
        | Token::StringLit(_)
        | Token::Ident(_)
        | Token::Dollar
        | Token::LParen
        | Token::Not
        | Token::Increment
        | Token::Decrement = self.peek()
        {
            let right = self.parse_addition();
            left = Expr::Concat(Box::new(left), Box::new(right));
        }

        // Check for pipe-getline AFTER concatenation:
        // "echo " "date" | getline → ("echo " "date") | getline
        if matches!(self.peek(), Token::Pipe) {
            let saved = self.pos;
            self.advance();
            if matches!(self.peek(), Token::Getline) {
                self.advance();
                let var = self.parse_getline_var();
                let getline_expr = Expr::Getline(var, None, GetlineSource::Pipe);
                left = Expr::Pipe(Box::new(left), Box::new(getline_expr));

                // Handle addition/subtraction after pipe-getline:
                // cmd | getline x + 1 → (cmd | getline x) + 1
                loop {
                    match self.peek() {
                        Token::Plus => {
                            self.advance();
                            let right = self.parse_multiplication();
                            left = Expr::Binop(Box::new(left), BinOp::Add, Box::new(right));
                        }
                        Token::Minus => {
                            self.advance();
                            let right = self.parse_multiplication();
                            left = Expr::Binop(Box::new(left), BinOp::Sub, Box::new(right));
                        }
                        _ => break,
                    }
                }

                // Continue concatenation after pipe-getline:
                // cmd | getline x y → (cmd | getline x) concat y
                while let Token::Number(_)
                | Token::StringLit(_)
                | Token::Ident(_)
                | Token::Dollar
                | Token::LParen
                | Token::Not
                | Token::Increment
                | Token::Decrement = self.peek()
                {
                    let right = self.parse_addition();
                    left = Expr::Concat(Box::new(left), Box::new(right));
                }
            } else {
                self.pos = saved;
            }
        }

        left
    }

    fn parse_getline_var(&mut self) -> Option<Box<Expr>> {
        if matches!(self.peek(), Token::Dollar) {
            // $expr as getline target
            self.advance(); // $
            let expr = self.parse_primary();
            return Some(Box::new(Expr::FieldRef(Box::new(expr))));
        }
        if matches!(self.peek(), Token::Ident(_)) {
            let saved = self.pos;
            if let Token::Ident(v) = self.advance() {
                if matches!(self.peek(), Token::LBracket) {
                    // Array reference: a[expr]
                    self.advance(); // [
                    let mut indices = vec![self.parse_expr()];
                    while matches!(self.peek(), Token::Comma) {
                        self.advance();
                        indices.push(self.parse_expr());
                    }
                    self.expect(&Token::RBracket);
                    return Some(Box::new(Expr::ArrayRef(v, indices)));
                }
                // Simple variable (not function call)
                if !matches!(self.peek(), Token::LParen) {
                    return Some(Box::new(Expr::Var(v)));
                }
            }
            self.pos = saved;
        }
        None
    }

    fn parse_addition(&mut self) -> Expr {
        let mut left = self.parse_multiplication();
        loop {
            match self.peek() {
                Token::Plus => {
                    self.advance();
                    let right = self.parse_multiplication();
                    left = Expr::Binop(Box::new(left), BinOp::Add, Box::new(right));
                }
                Token::Minus => {
                    self.advance();
                    let right = self.parse_multiplication();
                    left = Expr::Binop(Box::new(left), BinOp::Sub, Box::new(right));
                }
                _ => break,
            }
        }
        left
    }

    fn parse_multiplication(&mut self) -> Expr {
        let mut left = self.parse_power();
        loop {
            match self.peek() {
                Token::Star => {
                    self.advance();
                    let right = self.parse_power();
                    left = Expr::Binop(Box::new(left), BinOp::Mul, Box::new(right));
                }
                Token::Slash => {
                    self.advance();
                    let right = self.parse_power();
                    left = Expr::Binop(Box::new(left), BinOp::Div, Box::new(right));
                }
                Token::Percent => {
                    self.advance();
                    let right = self.parse_power();
                    left = Expr::Binop(Box::new(left), BinOp::Mod, Box::new(right));
                }
                _ => break,
            }
        }
        left
    }

    fn parse_power(&mut self) -> Expr {
        let base = self.parse_unary();
        if matches!(self.peek(), Token::Caret) {
            self.advance();
            let exp = self.parse_power(); // right associative
            Expr::Binop(Box::new(base), BinOp::Pow, Box::new(exp))
        } else {
            base
        }
    }

    fn parse_unary(&mut self) -> Expr {
        match self.peek() {
            Token::Plus => {
                self.advance();
                let expr = self.parse_unary();
                // Unary plus: coerce to number by adding 0
                Expr::Binop(Box::new(expr), BinOp::Add, Box::new(Expr::Number(0.0)))
            }
            Token::Minus => {
                self.advance();
                let expr = self.parse_unary();
                Expr::Unop(UnOp::Neg, Box::new(expr))
            }
            Token::Not => {
                self.advance();
                let expr = self.parse_unary();
                Expr::Unop(UnOp::Not, Box::new(expr))
            }
            Token::Increment => {
                self.advance();
                let expr = self.parse_unary();
                Expr::Unop(UnOp::PreIncrement, Box::new(expr))
            }
            Token::Decrement => {
                self.advance();
                let expr = self.parse_unary();
                Expr::Unop(UnOp::PreDecrement, Box::new(expr))
            }
            _ => self.parse_postfix(),
        }
    }

    fn parse_postfix(&mut self) -> Expr {
        let mut expr = self.parse_primary();
        loop {
            match self.peek() {
                Token::Increment => {
                    self.advance();
                    expr = Expr::PostIncrement(Box::new(expr));
                }
                Token::Decrement => {
                    self.advance();
                    expr = Expr::PostDecrement(Box::new(expr));
                }
                Token::LBracket => {
                    if let Expr::Var(name) = expr {
                        self.advance();
                        let mut indices = vec![self.parse_expr()];
                        while matches!(self.peek(), Token::Comma) {
                            self.advance();
                            indices.push(self.parse_expr());
                        }
                        self.expect(&Token::RBracket);
                        expr = Expr::ArrayRef(name, indices);
                    } else {
                        break;
                    }
                }
                _ => break,
            }
        }
        expr
    }

    fn parse_primary(&mut self) -> Expr {
        match self.peek().clone() {
            Token::Number(n) => {
                self.advance();
                Expr::Number(n)
            }
            Token::StringLit(s) => {
                self.advance();
                Expr::StringLit(s)
            }
            Token::Regex(r) => {
                self.advance();
                // Bare regex matches against $0
                Expr::Match(
                    Box::new(Expr::FieldRef(Box::new(Expr::Number(0.0)))),
                    Box::new(Expr::Regex(r)),
                )
            }
            Token::Dollar => {
                self.advance();
                let expr = self.parse_unary();
                Expr::FieldRef(Box::new(expr))
            }
            Token::LParen => {
                self.advance();
                // Check for (expr) in array
                let expr = self.parse_expr();
                if matches!(self.peek(), Token::RParen) {
                    self.advance();
                    // Check for (expr) in array at higher level
                    if matches!(self.peek(), Token::In) {
                        self.advance();
                        if let Token::Ident(arr) = self.advance() {
                            return Expr::In(Box::new(expr), arr);
                        }
                    }
                    expr
                } else {
                    // This shouldn't normally happen
                    self.expect(&Token::RParen);
                    expr
                }
            }
            Token::Getline => {
                self.advance();
                // getline [var] [< file]
                let var = self.parse_getline_var();

                if matches!(self.peek(), Token::Lt) {
                    self.advance();
                    let file = self.parse_primary();
                    Expr::Getline(var, Some(Box::new(file)), GetlineSource::File)
                } else {
                    Expr::Getline(var, None, GetlineSource::Stdin)
                }
            }
            Token::Ident(name) => {
                self.advance();
                if matches!(self.peek(), Token::LParen)
                    && (self.function_names.contains(&name) || {
                        // Check if ( is adjacent (no whitespace) - then it's a function call
                        // even if the name isn't a known function
                        let cur_col = self.token_cols.get(self.pos).copied().unwrap_or(0);
                        let prev_end =
                            self.token_cols.get(self.pos - 1).copied().unwrap_or(0) + name.len();
                        cur_col == prev_end
                    })
                {
                    // Function call
                    self.advance();
                    let mut args = Vec::new();
                    self.skip_terminators(); // skip newlines before first arg
                    while !matches!(self.peek(), Token::RParen | Token::Eof) {
                        args.push(self.parse_expr());
                        if matches!(self.peek(), Token::Comma) {
                            self.advance();
                            self.skip_terminators(); // skip newlines between args
                        }
                    }
                    self.expect(&Token::RParen);
                    Expr::FuncCall(name, args)
                } else if matches!(self.peek(), Token::LBracket) {
                    self.advance();
                    let mut indices = vec![self.parse_expr()];
                    while matches!(self.peek(), Token::Comma) {
                        self.advance();
                        indices.push(self.parse_expr());
                    }
                    self.expect(&Token::RBracket);
                    Expr::ArrayRef(name, indices)
                } else if BUILTIN_FUNCTIONS.contains(&name.as_str()) {
                    // Builtin function used without () is a syntax error
                    self.syntax_error_at_current();
                } else {
                    Expr::Var(name)
                }
            }
            _ => {
                self.syntax_error_at_current();
            }
        }
    }
}
