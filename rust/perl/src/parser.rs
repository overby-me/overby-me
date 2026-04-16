use crate::ast::*;
use crate::lexer::Token;

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Parser { tokens, pos: 0 }
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
        while !self.at(&Token::EOF) {
            // Skip stray semicolons/newlines
            if self.eat(&Token::Semi) || self.eat(&Token::Newline) {
                continue;
            }
            if let Some(stmt) = self.parse_stmt() {
                stmts.push(stmt);
            }
        }
        stmts
    }

    fn parse_stmt(&mut self) -> Option<Stmt> {
        // Check for label
        let label = self.try_parse_label();

        match self.tok() {
            Token::EOF => return None,
            Token::Semi => {
                self.pos += 1;
                return Some(Stmt::Nop);
            }
            Token::LBrace => {
                // Bare block
                self.pos += 1;
                let body = self.parse_block_body();
                self.eat(&Token::RBrace);
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
                return Some(Stmt::Until { cond, body, label });
            }
            Token::For | Token::Foreach => {
                self.pos += 1;
                return Some(self.parse_for(label));
            }
            Token::Sub => {
                self.pos += 1;
                return Some(self.parse_sub_decl());
            }
            Token::My => {
                self.pos += 1;
                let stmt = self.parse_my_decl();
                self.eat(&Token::Semi);
                return Some(stmt);
            }
            Token::Our => {
                self.pos += 1;
                let stmt = self.parse_our_decl();
                self.eat(&Token::Semi);
                return Some(stmt);
            }
            Token::Local => {
                self.pos += 1;
                let stmt = self.parse_local_decl();
                self.eat(&Token::Semi);
                return Some(stmt);
            }
            Token::Package => {
                self.pos += 1;
                let name = if let Token::Ident(name) = self.tok() {
                    let n = name.clone();
                    self.pos += 1;
                    n
                } else {
                    "main".to_string()
                };
                self.eat(&Token::Semi);
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
                return Some(Stmt::Begin(body));
            }
            Token::End => {
                self.pos += 1;
                let body = self.parse_brace_block();
                return Some(Stmt::End(body));
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
            Token::Eval => {
                self.pos += 1;
                let stmt = if self.at(&Token::LBrace) {
                    let body = self.parse_brace_block();
                    Stmt::Eval(Box::new(EvalArg::Block(body)))
                } else {
                    let expr = self.parse_expr();
                    Stmt::Eval(Box::new(EvalArg::Expr(expr)))
                };
                self.eat(&Token::Semi);
                return Some(stmt);
            }
            _ => {
                // Expression statement
                let expr = self.parse_expr();
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
                let list = self.parse_expr();
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
        let body = self.parse_brace_block();
        Stmt::While {
            cond,
            body,
            label: None,
        }
    }

    fn parse_for(&mut self, label: Option<String>) -> Stmt {
        // Check if it's foreach-style: for my $var (list) { }
        // or C-style: for (init; cond; step) { }

        if self.at(&Token::My) || matches!(self.tok(), Token::ScalarVar(_)) {
            // Foreach style
            let is_my = self.eat(&Token::My);
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
            return Stmt::Foreach {
                var,
                is_my,
                list,
                body,
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
                    Some(self.parse_expr())
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
                return Stmt::Foreach {
                    var: "_".to_string(),
                    is_my: false,
                    list,
                    body,
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
        Stmt::Foreach {
            var: "_".to_string(),
            is_my: false,
            list,
            body,
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
        while !self.at(&Token::RBrace) && !self.at(&Token::EOF) {
            if self.eat(&Token::Semi) {
                continue;
            }
            if let Some(stmt) = self.parse_stmt() {
                stmts.push(stmt);
            }
        }
        stmts
    }

    fn parse_sub_decl(&mut self) -> Stmt {
        let name = if let Token::Ident(name) = self.tok() {
            let n = name.clone();
            self.pos += 1;
            n
        } else {
            String::new()
        };
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

        let body = self.parse_brace_block();
        Stmt::Sub {
            name,
            params: Vec::new(),
            body,
        }
    }

    fn parse_my_decl(&mut self) -> Stmt {
        let vars = self.parse_var_list();
        Stmt::My(vars)
    }

    fn parse_my_decl_no_semi(&mut self) -> Stmt {
        self.eat(&Token::My);
        let vars = self.parse_var_list();
        Stmt::My(vars)
    }

    fn parse_our_decl(&mut self) -> Stmt {
        let vars = self.parse_var_list();
        Stmt::Our(vars)
    }

    fn parse_local_decl(&mut self) -> Stmt {
        let vars = self.parse_var_list();
        Stmt::Local(vars)
    }

    fn parse_var_list(&mut self) -> Vec<(String, Option<Expr>)> {
        let mut vars = Vec::new();

        if self.eat(&Token::LParen) {
            // my ($a, $b, @c, %d) = expr;
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
                        // undef as placeholder in list destructuring
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
            // Single variable: my $x = expr;
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
                _ => return vars,
            };

            let init = if self.eat(&Token::Assign) {
                Some(self.parse_expr())
            } else {
                None
            };
            vars.push((name, init));
        }

        vars
    }

    fn parse_print_stmt(&mut self, is_say: bool) -> Stmt {
        // print [FILEHANDLE] LIST
        // Can also be: print +(...) to force list context
        let has_plus = self.eat(&Token::Plus);

        let filehandle = if !has_plus {
            // Check if first token is a bareword (filehandle)
            if let Token::Ident(name) = self.tok() {
                // If followed by a comma or expression, it's a filehandle
                let saved = self.pos;
                let fh_name = name.clone();
                self.pos += 1;

                // Check if it's actually a filehandle
                if !self.at(&Token::Semi) && !self.at(&Token::EOF) && !self.at(&Token::FatComma) {
                    if matches!(fh_name.as_str(), "STDOUT" | "STDERR" | "STDIN")
                        || fh_name.chars().all(|c| c.is_ascii_uppercase() || c == '_')
                    {
                        Some(Expr::StringLit(fh_name))
                    } else {
                        self.pos = saved;
                        None
                    }
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
        if self.at(&Token::Semi)
            || self.at(&Token::EOF)
            || self.at(&Token::RBrace)
            || self.at(&Token::RParen)
        {
            return exprs;
        }
        exprs.push(self.parse_expr());
        while self.eat(&Token::Comma) || self.eat(&Token::FatComma) {
            if self.at(&Token::Semi)
                || self.at(&Token::EOF)
                || self.at(&Token::RBrace)
                || self.at(&Token::RParen)
            {
                break;
            }
            exprs.push(self.parse_expr());
        }
        exprs
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
            self.eat(&Token::Semi);
            return Stmt::Nop;
        } else {
            self.eat(&Token::Semi);
            return Stmt::Nop;
        };

        let args = if self.at(&Token::Semi) || self.at(&Token::EOF) {
            Vec::new()
        } else {
            self.parse_list_expr()
        };
        self.eat(&Token::Semi);
        Stmt::Use(module, args)
    }

    // --- Expression parsing with precedence climbing ---

    pub fn parse_expr(&mut self) -> Expr {
        self.parse_assign()
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
        let left = self.parse_or();
        if self.eat(&Token::DotDot) {
            let right = self.parse_or();
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
            self.parse_log_or()
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
        let left = self.parse_relational();
        match self.tok() {
            Token::NumEq => {
                self.pos += 1;
                Expr::BinOp(
                    BinOp::NumEq,
                    Box::new(left),
                    Box::new(self.parse_relational()),
                )
            }
            Token::NumNe => {
                self.pos += 1;
                Expr::BinOp(
                    BinOp::NumNe,
                    Box::new(left),
                    Box::new(self.parse_relational()),
                )
            }
            Token::Spaceship => {
                self.pos += 1;
                Expr::BinOp(
                    BinOp::Spaceship,
                    Box::new(left),
                    Box::new(self.parse_relational()),
                )
            }
            Token::Eq => {
                self.pos += 1;
                Expr::BinOp(
                    BinOp::StrEq,
                    Box::new(left),
                    Box::new(self.parse_relational()),
                )
            }
            Token::Ne => {
                self.pos += 1;
                Expr::BinOp(
                    BinOp::StrNe,
                    Box::new(left),
                    Box::new(self.parse_relational()),
                )
            }
            Token::Cmp => {
                self.pos += 1;
                Expr::BinOp(
                    BinOp::StrCmp,
                    Box::new(left),
                    Box::new(self.parse_relational()),
                )
            }
            _ => left,
        }
    }

    fn parse_relational(&mut self) -> Expr {
        let mut left = self.parse_shift();
        // Loop to handle chained comparisons like 32 <= $x <= 126
        loop {
            match self.tok() {
                Token::NumLt => {
                    self.pos += 1;
                    left = Expr::BinOp(BinOp::NumLt, Box::new(left), Box::new(self.parse_shift()));
                }
                Token::NumGt => {
                    self.pos += 1;
                    left = Expr::BinOp(BinOp::NumGt, Box::new(left), Box::new(self.parse_shift()));
                }
                Token::NumLe => {
                    self.pos += 1;
                    left = Expr::BinOp(BinOp::NumLe, Box::new(left), Box::new(self.parse_shift()));
                }
                Token::NumGe => {
                    self.pos += 1;
                    left = Expr::BinOp(BinOp::NumGe, Box::new(left), Box::new(self.parse_shift()));
                }
                Token::Lt => {
                    self.pos += 1;
                    left = Expr::BinOp(BinOp::StrLt, Box::new(left), Box::new(self.parse_shift()));
                }
                Token::Gt => {
                    self.pos += 1;
                    left = Expr::BinOp(BinOp::StrGt, Box::new(left), Box::new(self.parse_shift()));
                }
                Token::Le => {
                    self.pos += 1;
                    left = Expr::BinOp(BinOp::StrLe, Box::new(left), Box::new(self.parse_shift()));
                }
                Token::Ge => {
                    self.pos += 1;
                    left = Expr::BinOp(BinOp::StrGe, Box::new(left), Box::new(self.parse_shift()));
                }
                _ => break,
            }
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
                } else if let Token::RegexLit(pat, flags) = self.tok() {
                    let p = pat.clone();
                    let f = flags.clone();
                    self.pos += 1;
                    Expr::RegexMatch(Box::new(left), p, f)
                } else {
                    left
                }
            }
            Token::NotMatch => {
                self.pos += 1;
                if let Token::RegexLit(pat, flags) = self.tok() {
                    let p = pat.clone();
                    let f = flags.clone();
                    self.pos += 1;
                    Expr::RegexNotMatch(Box::new(left), p, f)
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
                let expr = self.parse_power();
                Expr::UnaryOp(UnaryOp::Neg, Box::new(expr))
            }
            Token::Plus => {
                self.pos += 1;
                let expr = self.parse_power();
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
                let expr = self.parse_postfix();
                Expr::UnaryOp(UnaryOp::PreInc, Box::new(expr))
            }
            Token::MinusMinus => {
                self.pos += 1;
                let expr = self.parse_postfix();
                Expr::UnaryOp(UnaryOp::PreDec, Box::new(expr))
            }
            Token::Backslash => {
                self.pos += 1;
                let expr = self.parse_unary();
                Expr::Ref(Box::new(expr))
            }
            Token::BitAnd => {
                // &func() call syntax
                self.pos += 1;
                if let Token::Ident(name) = self.tok() {
                    let name = name.clone();
                    self.pos += 1;
                    let args = if self.eat(&Token::LParen) {
                        let a = self.parse_list_expr();
                        self.expect(&Token::RParen);
                        a
                    } else {
                        Vec::new()
                    };
                    Expr::Call(name, args)
                } else {
                    // Regular bitwise-and as unary (take address)
                    let expr = self.parse_unary();
                    Expr::Ref(Box::new(expr))
                }
            }
            Token::Defined => {
                self.pos += 1;
                let has_paren = self.eat(&Token::LParen);
                let expr = self.parse_primary();
                if has_paren {
                    self.eat(&Token::RParen);
                }
                Expr::Defined(Box::new(expr))
            }
            Token::My => {
                self.pos += 1;
                if let Token::ScalarVar(name) = self.tok() {
                    let n = name.clone();
                    self.pos += 1;
                    Expr::MyVar(n)
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
                            _ => break,
                        }
                        if !self.eat(&Token::Comma) {
                            break;
                        }
                    }
                    self.expect(&Token::RParen);
                    if names.len() == 1 {
                        Expr::MyVar(names.into_iter().next().unwrap())
                    } else {
                        // Return the list as an array literal of MyVars
                        Expr::ArrayLit(names.into_iter().map(Expr::MyVar).collect())
                    }
                } else {
                    Expr::Undef
                }
            }
            Token::Ident(name) if name.starts_with('-') && name.len() == 2 => {
                // File test operators: -e, -f, -d, etc.
                let op = name.clone();
                self.pos += 1;
                let expr = self.parse_primary();
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
                    let index = self.parse_expr();
                    self.expect(&Token::RBracket);
                    match &expr {
                        Expr::ScalarVar(name) | Expr::ArrayVar(name) | Expr::MyVar(name) => {
                            let name = name.clone();
                            expr = Expr::ArrayElement(name, Box::new(index));
                        }
                        _ => {
                            // (expr)[idx] — index into result of expression as list
                            expr = Expr::Call("_list_index".to_string(), vec![expr, index]);
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
                            | Token::ScalarVar(_)
                            | Token::Integer(_)
                            | Token::Float(_)
                            | Token::Ident(_)
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
                        let key = self.parse_expr();
                        self.expect(&Token::RBrace);
                        match expr {
                            Expr::ScalarVar(name) | Expr::HashVar(name) | Expr::MyVar(name) => {
                                expr = Expr::HashElement(name, Box::new(key));
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
                    match self.tok() {
                        Token::LBracket => {
                            self.pos += 1;
                            let index = self.parse_expr();
                            self.expect(&Token::RBracket);
                            expr = Expr::ArrayElement("_deref_".to_string(), Box::new(index));
                        }
                        Token::LBrace => {
                            self.pos += 1;
                            let key = self.parse_expr();
                            self.expect(&Token::RBrace);
                            expr = Expr::HashElement("_deref_".to_string(), Box::new(key));
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
                        _ => break,
                    }
                }
                _ => break,
            }
        }

        expr
    }

    fn parse_primary(&mut self) -> Expr {
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
            Token::Substitution(pat, repl, flags) => {
                self.pos += 1;
                // Bare s/// applies to $_
                Expr::Substitution(Box::new(Expr::ScalarVar("_".to_string())), pat, repl, flags)
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
                Expr::ArrayVar(name)
            }
            Token::HashVar(name) => {
                self.pos += 1;
                Expr::HashVar(name)
            }
            Token::ArrayLen(name) => {
                self.pos += 1;
                Expr::ArrayLen(name)
            }

            Token::Diamond(name) => {
                self.pos += 1;
                Expr::Diamond(name)
            }

            Token::UndefKw => {
                self.pos += 1;
                Expr::Undef
            }

            Token::LParen => {
                self.pos += 1;
                if self.at(&Token::RParen) {
                    self.pos += 1;
                    return Expr::ArrayLit(Vec::new());
                }
                let expr = self.parse_expr();
                // Check if there are more items (it's a list)
                if self.eat(&Token::Comma) {
                    let mut items = vec![expr];
                    loop {
                        if self.at(&Token::RParen) {
                            break;
                        }
                        items.push(self.parse_expr());
                        if !self.eat(&Token::Comma) {
                            break;
                        }
                    }
                    self.expect(&Token::RParen);
                    Expr::ArrayLit(items)
                } else {
                    self.expect(&Token::RParen);
                    expr
                }
            }

            Token::LBracket => {
                // Anonymous array ref [...]
                self.pos += 1;
                let mut items = Vec::new();
                while !self.at(&Token::RBracket) && !self.at(&Token::EOF) {
                    items.push(self.parse_expr());
                    self.eat(&Token::Comma);
                }
                self.expect(&Token::RBracket);
                Expr::ArrayRef(items)
            }

            Token::LBrace => {
                // Anonymous hash ref {...} or block
                // Heuristic: { ident => ... } is a hash ref
                let saved = self.pos;
                self.pos += 1;

                // Check if it looks like a hash ref
                let is_hash = matches!(
                    (self.tok(), self.peek(1)),
                    (Token::StringLit(_), Token::FatComma)
                        | (Token::Ident(_), Token::FatComma)
                        | (Token::Integer(_), Token::FatComma)
                );

                if is_hash {
                    let mut pairs = Vec::new();
                    loop {
                        if self.at(&Token::RBrace) || self.at(&Token::EOF) {
                            break;
                        }
                        let key = self.parse_expr();
                        self.eat(&Token::FatComma);
                        self.eat(&Token::Comma);
                        let val = self.parse_expr();
                        pairs.push((key, val));
                        self.eat(&Token::Comma);
                    }
                    self.expect(&Token::RBrace);
                    Expr::HashRef(pairs)
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
                let body = self.parse_brace_block();
                // Return as a callable reference
                Expr::StringLit("CODE_REF".to_string()) // placeholder
            }

            // Named unary builtins
            Token::Abs
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
            | Token::Wantarray => {
                let func = match self.tok() {
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

                if self.at(&Token::LBrace) {
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
                    let args = self.parse_list_expr();
                    self.expect(&Token::RParen);
                    Expr::Call(func, args)
                } else {
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

            // List builtins: push, unshift, splice, delete, exists, keys, values, each,
            // reverse, join, split, substr, index, rindex, sprintf,
            // open, close, read, binmode, unlink, rename, mkdir, rmdir, chdir, stat
            Token::Push
            | Token::Unshift
            | Token::Splice
            | Token::Delete
            | Token::Exists
            | Token::Keys
            | Token::Values
            | Token::Each
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
            | Token::Stat => {
                let func = format!("{:?}", self.tok()).to_lowercase();
                self.pos += 1;
                let args = if self.eat(&Token::LParen) {
                    let a = self.parse_list_expr();
                    self.expect(&Token::RParen);
                    a
                } else {
                    self.parse_list_expr()
                };
                Expr::Call(func, args)
            }

            Token::Eval => {
                self.pos += 1;
                if self.at(&Token::LBrace) {
                    let body = self.parse_brace_block();
                    Expr::DoBlock(body)
                } else {
                    let expr = self.parse_primary();
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

            Token::Local => {
                self.pos += 1;
                if let Token::ScalarVar(name) = self.tok() {
                    let n = name.clone();
                    self.pos += 1;
                    Expr::LocalVar(n)
                } else if let Token::ArrayVar(name) = self.tok() {
                    let n = name.clone();
                    self.pos += 1;
                    Expr::LocalVar(format!("@{n}"))
                } else {
                    Expr::Undef
                }
            }

            // print/say/die/warn in expression context
            Token::Print | Token::Say | Token::Die | Token::Warn => {
                let func = match self.tok() {
                    Token::Print => "print",
                    Token::Say => "say",
                    Token::Die => "die",
                    Token::Warn => "warn",
                    _ => unreachable!(),
                }
                .to_string();
                self.pos += 1;
                let args = self.parse_list_expr();
                Expr::Call(func, args)
            }

            Token::Ident(name) => {
                let name = name.clone();
                self.pos += 1;

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
                        | Token::Minus
                        | Token::LogNot
                        | Token::Backslash
                ) {
                    // Function call without parentheses: func arg, ...
                    let args = self.parse_list_expr();
                    Expr::Call(name, args)
                } else {
                    // Bareword — treat as string in most contexts
                    Expr::StringLit(name)
                }
            }

            _ => {
                // Unknown token, skip and return undef
                self.pos += 1;
                Expr::Undef
            }
        }
    }

    // Helper for matching Token::ScalarVar in match arms
    fn at_scalar_var(&self) -> bool {
        matches!(self.tok(), Token::ScalarVar(_))
    }
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
            if chars[i + 1] == '_'
                || chars[i + 1].is_ascii_alphabetic()
                || chars[i + 1].is_ascii_digit()
                || chars[i + 1] == '{'
                || chars[i + 1] == '^'
            {
                // Flush literal
                if !lit.is_empty() {
                    parts.push(InterpPart::Lit(std::mem::take(&mut lit)));
                }

                i += 1; // skip $

                if chars[i] == '{' {
                    // ${var} or ${^VAR}
                    i += 1;
                    let mut name = String::new();
                    if i < chars.len() && chars[i] == '^' {
                        name.push('^');
                        i += 1;
                    }
                    while i < chars.len() && chars[i] != '}' {
                        name.push(chars[i]);
                        i += 1;
                    }
                    if i < chars.len() && chars[i] == '}' {
                        i += 1;
                    }
                    parts.push(InterpPart::ScalarVar(name));
                } else if chars[i] == '^' && i + 1 < chars.len() {
                    i += 1;
                    let c = chars[i];
                    i += 1;
                    parts.push(InterpPart::ScalarVar(format!("^{c}")));
                } else {
                    let mut name = String::new();
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
                        // Parse the index expression
                        let idx_expr = if let Ok(n) = idx_str.parse::<i64>() {
                            Box::new(Expr::IntLit(n))
                        } else {
                            // Strip $ sigil if present
                            let var_name = idx_str.strip_prefix('$').unwrap_or(&idx_str);
                            Box::new(Expr::ScalarVar(var_name.to_string()))
                        };
                        parts.push(InterpPart::ArrayElement(name, idx_expr));
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
                        parts.push(InterpPart::HashElement(
                            name,
                            Box::new(Expr::StringLit(key_str)),
                        ));
                    } else {
                        parts.push(InterpPart::ScalarVar(name));
                    }
                }
            } else {
                lit.push(chars[i]);
                i += 1;
            }
        } else if chars[i] == '@'
            && i + 1 < chars.len()
            && (chars[i + 1].is_ascii_alphabetic() || chars[i + 1] == '_' || chars[i + 1] == '{')
        {
            // Array interpolation
            if !lit.is_empty() {
                parts.push(InterpPart::Lit(std::mem::take(&mut lit)));
            }
            i += 1; // skip @
            let mut name = String::new();
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                name.push(chars[i]);
                i += 1;
            }
            parts.push(InterpPart::ArrayVar(name));
        } else if chars[i] == '\x01' {
            // Escaped $ placeholder
            lit.push('$');
            i += 1;
        } else if chars[i] == '\x02' {
            // Escaped @ placeholder
            lit.push('@');
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
