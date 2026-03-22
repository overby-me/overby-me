/// Parser for the Meson build language.
/// Transforms a token stream into an AST.
use crate::ast::*;
use crate::lexer::{Token, TokenKind};

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    pub fn parse(&mut self) -> Result<Program, String> {
        self.skip_newlines();
        let stmts = self.parse_block(&[])?;
        if !self.is_at_end() {
            let tok = &self.tokens[self.pos];
            return Err(format!(
                "{}:{}: Unexpected token {:?}",
                tok.line, tok.col, tok.kind
            ));
        }
        Ok(Program { statements: stmts })
    }

    fn parse_block(&mut self, terminators: &[&str]) -> Result<Vec<Statement>, String> {
        let mut stmts = Vec::new();
        loop {
            self.skip_newlines();
            if self.is_at_end() {
                break;
            }
            if self.is_terminator(terminators) {
                break;
            }
            stmts.push(self.parse_statement()?);
        }
        Ok(stmts)
    }

    fn is_terminator(&self, terminators: &[&str]) -> bool {
        if terminators.is_empty() {
            return false;
        }
        match &self.tokens[self.pos].kind {
            TokenKind::Endif => terminators.contains(&"endif"),
            TokenKind::Elif => terminators.contains(&"elif"),
            TokenKind::Else => terminators.contains(&"else"),
            TokenKind::Endforeach => terminators.contains(&"endforeach"),
            _ => false,
        }
    }

    fn parse_statement(&mut self) -> Result<Statement, String> {
        match &self.tokens[self.pos].kind {
            TokenKind::If => self.parse_if(),
            TokenKind::Foreach => self.parse_foreach(),
            TokenKind::Break => {
                let tok = &self.tokens[self.pos];
                let loc = SourceLocation {
                    line: tok.line,
                    col: tok.col,
                };
                self.advance();
                self.expect_newline_or_eof()?;
                Ok(Statement::Break(loc))
            }
            TokenKind::Continue => {
                let tok = &self.tokens[self.pos];
                let loc = SourceLocation {
                    line: tok.line,
                    col: tok.col,
                };
                self.advance();
                self.expect_newline_or_eof()?;
                Ok(Statement::Continue(loc))
            }
            _ => {
                let expr = self.parse_expression()?;
                // Check for assignment
                if let Expression::Identifier(name, loc) = &expr {
                    match &self.tokens[self.pos].kind {
                        TokenKind::Assign => {
                            self.advance();
                            let value = self.parse_expression()?;
                            self.expect_newline_or_eof()?;
                            return Ok(Statement::Assignment(Assignment {
                                name: name.clone(),
                                value,
                                loc: loc.clone(),
                            }));
                        }
                        TokenKind::PlusAssign => {
                            self.advance();
                            let value = self.parse_expression()?;
                            self.expect_newline_or_eof()?;
                            return Ok(Statement::PlusAssignment(Assignment {
                                name: name.clone(),
                                value,
                                loc: loc.clone(),
                            }));
                        }
                        _ => {}
                    }
                }
                self.expect_newline_or_eof()?;
                Ok(Statement::Expression(expr))
            }
        }
    }

    fn parse_if(&mut self) -> Result<Statement, String> {
        let tok = &self.tokens[self.pos];
        let loc = SourceLocation {
            line: tok.line,
            col: tok.col,
        };
        self.advance(); // 'if'
        let condition = self.parse_expression()?;
        self.expect_newline()?;
        let body = self.parse_block(&["elif", "else", "endif"])?;

        let mut elif_clauses = Vec::new();
        while self.check(&TokenKind::Elif) {
            self.advance();
            let elif_cond = self.parse_expression()?;
            self.expect_newline()?;
            let elif_body = self.parse_block(&["elif", "else", "endif"])?;
            elif_clauses.push((elif_cond, elif_body));
        }

        let else_body = if self.check(&TokenKind::Else) {
            self.advance();
            self.expect_newline()?;
            Some(self.parse_block(&["endif"])?)
        } else {
            None
        };

        self.expect_keyword(TokenKind::Endif)?;
        self.expect_newline_or_eof()?;

        Ok(Statement::If(IfStatement {
            condition,
            body,
            elif_clauses,
            else_body,
            loc,
        }))
    }

    fn parse_foreach(&mut self) -> Result<Statement, String> {
        let tok = &self.tokens[self.pos];
        let loc = SourceLocation {
            line: tok.line,
            col: tok.col,
        };
        self.advance(); // 'foreach'

        let mut varnames = Vec::new();
        varnames.push(self.expect_identifier()?);
        while self.check(&TokenKind::Comma) {
            self.advance();
            varnames.push(self.expect_identifier()?);
        }
        self.expect_keyword(TokenKind::Colon)?;
        let iterable = self.parse_expression()?;
        self.expect_newline()?;
        let body = self.parse_block(&["endforeach"])?;
        self.expect_keyword(TokenKind::Endforeach)?;
        self.expect_newline_or_eof()?;

        Ok(Statement::Foreach(ForeachStatement {
            varnames,
            iterable,
            body,
            loc,
        }))
    }

    fn parse_expression(&mut self) -> Result<Expression, String> {
        self.parse_ternary()
    }

    fn parse_ternary(&mut self) -> Result<Expression, String> {
        let expr = self.parse_or()?;
        if self.check(&TokenKind::Question) {
            let loc = SourceLocation {
                line: self.tokens[self.pos].line,
                col: self.tokens[self.pos].col,
            };
            self.advance();
            let true_val = self.parse_or()?;
            self.expect_keyword(TokenKind::Colon)?;
            let false_val = self.parse_or()?;
            Ok(Expression::Ternary(
                Box::new(expr),
                Box::new(true_val),
                Box::new(false_val),
                loc,
            ))
        } else {
            Ok(expr)
        }
    }

    fn parse_or(&mut self) -> Result<Expression, String> {
        let mut left = self.parse_and()?;
        while self.check(&TokenKind::Or) {
            let loc = SourceLocation {
                line: self.tokens[self.pos].line,
                col: self.tokens[self.pos].col,
            };
            self.advance();
            let right = self.parse_and()?;
            left = Expression::BinaryOp(BinaryOp::Or, Box::new(left), Box::new(right), loc);
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expression, String> {
        let mut left = self.parse_comparison()?;
        while self.check(&TokenKind::And) {
            let loc = SourceLocation {
                line: self.tokens[self.pos].line,
                col: self.tokens[self.pos].col,
            };
            self.advance();
            let right = self.parse_comparison()?;
            left = Expression::BinaryOp(BinaryOp::And, Box::new(left), Box::new(right), loc);
        }
        Ok(left)
    }

    fn parse_comparison(&mut self) -> Result<Expression, String> {
        let mut left = self.parse_addition()?;
        loop {
            let op = match &self.tokens[self.pos].kind {
                TokenKind::Eq => BinaryOp::Eq,
                TokenKind::Neq => BinaryOp::Neq,
                TokenKind::Lt => BinaryOp::Lt,
                TokenKind::Gt => BinaryOp::Gt,
                TokenKind::Le => BinaryOp::Le,
                TokenKind::Ge => BinaryOp::Ge,
                TokenKind::In => BinaryOp::In,
                TokenKind::Not if self.peek_ahead_kind(1) == Some(&TokenKind::In) => {
                    self.advance(); // 'not'
                    BinaryOp::NotIn
                }
                _ => break,
            };
            let loc = SourceLocation {
                line: self.tokens[self.pos].line,
                col: self.tokens[self.pos].col,
            };
            self.advance();
            let right = self.parse_addition()?;
            left = Expression::BinaryOp(op, Box::new(left), Box::new(right), loc);
        }
        Ok(left)
    }

    fn parse_addition(&mut self) -> Result<Expression, String> {
        let mut left = self.parse_multiplication()?;
        loop {
            let op = match &self.tokens[self.pos].kind {
                TokenKind::Plus => BinaryOp::Add,
                TokenKind::Minus => BinaryOp::Sub,
                _ => break,
            };
            let loc = SourceLocation {
                line: self.tokens[self.pos].line,
                col: self.tokens[self.pos].col,
            };
            self.advance();
            let right = self.parse_multiplication()?;
            left = Expression::BinaryOp(op, Box::new(left), Box::new(right), loc);
        }
        Ok(left)
    }

    fn parse_multiplication(&mut self) -> Result<Expression, String> {
        let mut left = self.parse_unary()?;
        loop {
            let op = match &self.tokens[self.pos].kind {
                TokenKind::Star => BinaryOp::Mul,
                TokenKind::Slash => BinaryOp::Div,
                TokenKind::Percent => BinaryOp::Mod,
                _ => break,
            };
            let loc = SourceLocation {
                line: self.tokens[self.pos].line,
                col: self.tokens[self.pos].col,
            };
            self.advance();
            let right = self.parse_unary()?;
            left = Expression::BinaryOp(op, Box::new(left), Box::new(right), loc);
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expression, String> {
        match &self.tokens[self.pos].kind {
            TokenKind::Not => {
                let loc = SourceLocation {
                    line: self.tokens[self.pos].line,
                    col: self.tokens[self.pos].col,
                };
                self.advance();
                let expr = self.parse_unary()?;
                Ok(Expression::UnaryOp(UnaryOp::Not, Box::new(expr), loc))
            }
            TokenKind::Minus => {
                let loc = SourceLocation {
                    line: self.tokens[self.pos].line,
                    col: self.tokens[self.pos].col,
                };
                self.advance();
                let expr = self.parse_unary()?;
                Ok(Expression::UnaryOp(UnaryOp::Negate, Box::new(expr), loc))
            }
            _ => self.parse_postfix(),
        }
    }

    fn parse_postfix(&mut self) -> Result<Expression, String> {
        let mut expr = self.parse_primary()?;
        loop {
            match &self.tokens[self.pos].kind {
                TokenKind::Dot => {
                    self.advance();
                    let name = self.expect_identifier()?;
                    if self.check(&TokenKind::LParen) {
                        let args = self.parse_argument_list()?;
                        let loc = expr.loc().clone();
                        expr = Expression::MethodCall(Box::new(expr), name, args, loc);
                    } else {
                        // Property access — treat as method call with no args for compatibility
                        let loc = expr.loc().clone();
                        expr = Expression::MethodCall(Box::new(expr), name, vec![], loc);
                    }
                }
                TokenKind::LBracket => {
                    let loc = expr.loc().clone();
                    self.advance();
                    let index = self.parse_expression()?;
                    self.expect_kind(&TokenKind::RBracket)?;
                    expr = Expression::Index(Box::new(expr), Box::new(index), loc);
                }
                TokenKind::LParen => {
                    let args = self.parse_argument_list()?;
                    let loc = expr.loc().clone();
                    expr = Expression::FunctionCall(Box::new(expr), args, loc);
                }
                _ => break,
            }
        }
        Ok(expr)
    }

    fn parse_primary(&mut self) -> Result<Expression, String> {
        let tok = &self.tokens[self.pos];
        let loc = SourceLocation {
            line: tok.line,
            col: tok.col,
        };
        match &tok.kind {
            TokenKind::StringLiteral(s) => {
                let s = s.clone();
                self.advance();
                Ok(Expression::StringLiteral(s, loc))
            }
            TokenKind::MultilineStringLiteral(s) => {
                let s = s.clone();
                self.advance();
                Ok(Expression::MultilineStringLiteral(s, loc))
            }
            TokenKind::FStringLiteral(s) => {
                let s = s.clone();
                self.advance();
                Ok(Expression::FStringLiteral(s, loc))
            }
            TokenKind::IntLiteral(n) => {
                let n = *n;
                self.advance();
                Ok(Expression::IntLiteral(n, loc))
            }
            TokenKind::True => {
                self.advance();
                Ok(Expression::BoolLiteral(true, loc))
            }
            TokenKind::False => {
                self.advance();
                Ok(Expression::BoolLiteral(false, loc))
            }
            TokenKind::Identifier(name) => {
                let name = name.clone();
                self.advance();
                Ok(Expression::Identifier(name, loc))
            }
            TokenKind::LBracket => {
                self.advance();
                let mut elements = Vec::new();
                self.skip_newlines();
                if !self.check(&TokenKind::RBracket) {
                    elements.push(self.parse_expression()?);
                    self.skip_newlines();
                    while self.check(&TokenKind::Comma) {
                        self.advance();
                        self.skip_newlines();
                        if self.check(&TokenKind::RBracket) {
                            break; // trailing comma
                        }
                        elements.push(self.parse_expression()?);
                        self.skip_newlines();
                    }
                }
                self.skip_newlines();
                self.expect_kind(&TokenKind::RBracket)?;
                Ok(Expression::Array(elements, loc))
            }
            TokenKind::LBrace => {
                self.advance();
                let mut entries = Vec::new();
                self.skip_newlines();
                if !self.check(&TokenKind::RBrace) {
                    let key = self.parse_expression()?;
                    self.expect_kind(&TokenKind::Colon)?;
                    let value = self.parse_expression()?;
                    entries.push((key, value));
                    self.skip_newlines();
                    while self.check(&TokenKind::Comma) {
                        self.advance();
                        self.skip_newlines();
                        if self.check(&TokenKind::RBrace) {
                            break;
                        }
                        let key = self.parse_expression()?;
                        self.expect_kind(&TokenKind::Colon)?;
                        let value = self.parse_expression()?;
                        entries.push((key, value));
                        self.skip_newlines();
                    }
                }
                self.skip_newlines();
                self.expect_kind(&TokenKind::RBrace)?;
                Ok(Expression::Dict(entries, loc))
            }
            TokenKind::LParen => {
                self.advance();
                let expr = self.parse_expression()?;
                self.expect_kind(&TokenKind::RParen)?;
                Ok(expr)
            }
            _ => Err(format!(
                "{}:{}: Expected expression, got {:?}",
                tok.line, tok.col, tok.kind
            )),
        }
    }

    fn parse_argument_list(&mut self) -> Result<Vec<Argument>, String> {
        self.expect_kind(&TokenKind::LParen)?;
        let mut args = Vec::new();
        self.skip_newlines();
        if !self.check(&TokenKind::RParen) {
            args.push(self.parse_argument()?);
            self.skip_newlines();
            while self.check(&TokenKind::Comma) {
                self.advance();
                self.skip_newlines();
                if self.check(&TokenKind::RParen) {
                    break;
                }
                args.push(self.parse_argument()?);
                self.skip_newlines();
            }
        }
        self.skip_newlines();
        self.expect_kind(&TokenKind::RParen)?;
        Ok(args)
    }

    fn parse_argument(&mut self) -> Result<Argument, String> {
        // Try to parse as keyword argument: name : value
        if let TokenKind::Identifier(name) = &self.tokens[self.pos].kind {
            let name = name.clone();
            if self.peek_ahead_kind(1) == Some(&TokenKind::Colon) {
                self.advance(); // identifier
                self.advance(); // colon
                self.skip_newlines();
                let value = self.parse_expression()?;
                return Ok(Argument {
                    name: Some(name),
                    value,
                });
            }
        }
        let value = self.parse_expression()?;
        Ok(Argument { name: None, value })
    }

    // Helper methods

    fn advance(&mut self) {
        if self.pos < self.tokens.len() - 1 {
            self.pos += 1;
        }
    }

    fn check(&self, kind: &TokenKind) -> bool {
        std::mem::discriminant(&self.tokens[self.pos].kind) == std::mem::discriminant(kind)
    }

    fn is_at_end(&self) -> bool {
        matches!(self.tokens[self.pos].kind, TokenKind::Eof)
    }

    fn skip_newlines(&mut self) {
        while self.check(&TokenKind::Newline) {
            self.advance();
        }
    }

    fn expect_newline(&mut self) -> Result<(), String> {
        if self.check(&TokenKind::Newline) {
            self.advance();
            Ok(())
        } else if self.is_at_end() {
            Ok(())
        } else {
            let tok = &self.tokens[self.pos];
            Err(format!(
                "{}:{}: Expected newline, got {:?}",
                tok.line, tok.col, tok.kind
            ))
        }
    }

    fn expect_newline_or_eof(&mut self) -> Result<(), String> {
        if self.check(&TokenKind::Newline) || self.is_at_end() {
            if self.check(&TokenKind::Newline) {
                self.advance();
            }
            Ok(())
        } else {
            let tok = &self.tokens[self.pos];
            Err(format!(
                "{}:{}: Expected newline or end of file, got {:?}",
                tok.line, tok.col, tok.kind
            ))
        }
    }

    fn expect_identifier(&mut self) -> Result<String, String> {
        if let TokenKind::Identifier(name) = &self.tokens[self.pos].kind {
            let name = name.clone();
            self.advance();
            Ok(name)
        } else {
            let tok = &self.tokens[self.pos];
            Err(format!(
                "{}:{}: Expected identifier, got {:?}",
                tok.line, tok.col, tok.kind
            ))
        }
    }

    fn expect_keyword(&mut self, kind: TokenKind) -> Result<(), String> {
        if std::mem::discriminant(&self.tokens[self.pos].kind) == std::mem::discriminant(&kind) {
            self.advance();
            Ok(())
        } else {
            let tok = &self.tokens[self.pos];
            Err(format!(
                "{}:{}: Expected {:?}, got {:?}",
                tok.line, tok.col, kind, tok.kind
            ))
        }
    }

    fn expect_kind(&mut self, kind: &TokenKind) -> Result<(), String> {
        if std::mem::discriminant(&self.tokens[self.pos].kind) == std::mem::discriminant(kind) {
            self.advance();
            Ok(())
        } else {
            let tok = &self.tokens[self.pos];
            Err(format!(
                "{}:{}: Expected {:?}, got {:?}",
                tok.line, tok.col, kind, tok.kind
            ))
        }
    }

    fn peek_ahead_kind(&self, n: usize) -> Option<&TokenKind> {
        self.tokens.get(self.pos + n).map(|t| &t.kind)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;

    fn parse_str(s: &str) -> Program {
        let mut lexer = Lexer::new(s);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);
        parser.parse().unwrap()
    }

    #[test]
    fn test_simple_call() {
        let prog = parse_str("project('hello', 'c')\n");
        assert_eq!(prog.statements.len(), 1);
    }

    #[test]
    fn test_assignment() {
        let prog = parse_str("x = 42\n");
        assert!(matches!(prog.statements[0], Statement::Assignment(_)));
    }

    #[test]
    fn test_if_statement() {
        let prog = parse_str("if true\n  x = 1\nendif\n");
        assert!(matches!(prog.statements[0], Statement::If(_)));
    }

    #[test]
    fn test_foreach() {
        let prog = parse_str("foreach x : [1, 2, 3]\n  y = x\nendforeach\n");
        assert!(matches!(prog.statements[0], Statement::Foreach(_)));
    }
}
