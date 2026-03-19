use crate::ast::*;
use crate::lexer::{Lexer, Token};

pub struct Parser {
    lexer: Lexer,
    current: Token,
}

impl Parser {
    pub fn new(input: &str) -> Self {
        let mut lexer = Lexer::new(input);
        let current = lexer.next_token();
        Self { lexer, current }
    }

    fn advance(&mut self) -> Token {
        std::mem::replace(&mut self.current, self.lexer.next_token())
    }

    fn eat(&mut self, expected: &Token) -> bool {
        if std::mem::discriminant(&self.current) == std::mem::discriminant(expected) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn skip_newlines(&mut self) {
        while self.current == Token::Newline {
            self.advance();
        }
    }

    fn is_keyword(&self, kw: &str) -> bool {
        if let Token::Word(parts) = &self.current
            && parts.len() == 1
            && let WordPart::Literal(s) = &parts[0]
        {
            return s == kw;
        }
        false
    }

    fn eat_keyword(&mut self, kw: &str) -> bool {
        if self.is_keyword(kw) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect_keyword(&mut self, kw: &str) -> Result<(), String> {
        if self.eat_keyword(kw) {
            Ok(())
        } else {
            Err(format!("expected keyword '{}', got {:?}", kw, self.current))
        }
    }

    fn word_text(&self) -> Option<String> {
        if let Token::Word(parts) = &self.current {
            Some(word_to_string(parts))
        } else {
            None
        }
    }

    fn take_word(&mut self) -> Option<Word> {
        if let Token::Word(_) = &self.current {
            if let Token::Word(w) = self.advance() {
                Some(w)
            } else {
                None
            }
        } else {
            None
        }
    }

    pub fn parse_program(&mut self) -> Result<Program, String> {
        let mut commands = Vec::new();
        self.skip_newlines();

        while self.current != Token::Eof {
            if self.is_keyword("}")
                || self.is_keyword("fi")
                || self.is_keyword("done")
                || self.is_keyword("esac")
                || self.is_keyword("then")
                || self.is_keyword("else")
                || self.is_keyword("elif")
                || self.is_keyword("do")
                || self.current == Token::RParen
                || self.current == Token::DSemi
            {
                break;
            }

            let cmd = self.parse_complete_command()?;
            commands.push(cmd);
            self.skip_newlines();
        }

        Ok(commands)
    }

    fn parse_complete_command(&mut self) -> Result<CompleteCommand, String> {
        let list = self.parse_and_or_list()?;

        let background = match self.current {
            Token::Amp => {
                self.advance();
                true
            }
            Token::Semi | Token::Newline => {
                self.advance();
                false
            }
            _ => false,
        };

        Ok(CompleteCommand { list, background })
    }

    fn parse_and_or_list(&mut self) -> Result<AndOrList, String> {
        let first = self.parse_pipeline()?;
        let mut rest = Vec::new();

        loop {
            let op = match self.current {
                Token::AndIf => {
                    self.advance();
                    AndOr::And
                }
                Token::OrIf => {
                    self.advance();
                    AndOr::Or
                }
                _ => break,
            };
            self.skip_newlines();
            let pipeline = self.parse_pipeline()?;
            rest.push((op, pipeline));
        }

        Ok(AndOrList { first, rest })
    }

    fn parse_pipeline(&mut self) -> Result<Pipeline, String> {
        let negated = self.eat_keyword("!");

        let first = self.parse_command()?;
        let mut commands = vec![first];

        while self.current == Token::Pipe {
            self.advance();
            self.skip_newlines();
            commands.push(self.parse_command()?);
        }

        Ok(Pipeline { negated, commands })
    }

    fn parse_command(&mut self) -> Result<Command, String> {
        // Check for function definition: name () compound_command
        if let Some(name) = self.word_text()
            && !is_reserved_word(&name)
        {
            // Look ahead for () - save full state for backtrack
            let saved_lexer_pos = self.lexer.save_position();
            let saved_current = self.current.clone();

            self.advance();
            if self.current == Token::LParen {
                let saved2_lexer_pos = self.lexer.save_position();
                let saved2_current = self.current.clone();
                self.advance();
                if self.current == Token::RParen {
                    self.advance();
                    self.skip_newlines();
                    let body = self.parse_compound_command()?;
                    return Ok(Command::FunctionDef(name, Box::new(body)));
                }
                // Backtrack from inner lookahead
                self.lexer.restore_position(saved2_lexer_pos);
                self.current = saved2_current;
            }

            // Backtrack
            self.lexer.restore_position(saved_lexer_pos);
            self.current = saved_current;
        }

        // Check for 'function' keyword
        if self.is_keyword("function") {
            self.advance();
            let name = self
                .word_text()
                .ok_or_else(|| "expected function name".to_string())?;
            self.advance();
            // Optional ()
            if self.current == Token::LParen {
                self.advance();
                if self.current == Token::RParen {
                    self.advance();
                }
            }
            self.skip_newlines();
            let body = self.parse_compound_command()?;
            return Ok(Command::FunctionDef(name, Box::new(body)));
        }

        // Check for compound command
        if self.is_keyword("{")
            || self.is_keyword("if")
            || self.is_keyword("for")
            || self.is_keyword("while")
            || self.is_keyword("until")
            || self.is_keyword("case")
            || self.current == Token::LParen
        {
            let compound = self.parse_compound_command()?;
            let redirections = self.parse_redirections()?;
            return Ok(Command::Compound(compound, redirections));
        }

        // Simple command
        let cmd = self.parse_simple_command()?;
        Ok(Command::Simple(cmd))
    }

    fn parse_compound_command(&mut self) -> Result<CompoundCommand, String> {
        if self.is_keyword("{") {
            self.parse_brace_group()
        } else if self.current == Token::LParen {
            self.parse_subshell()
        } else if self.is_keyword("if") {
            self.parse_if()
        } else if self.is_keyword("for") {
            self.parse_for()
        } else if self.is_keyword("while") {
            self.parse_while()
        } else if self.is_keyword("until") {
            self.parse_until()
        } else if self.is_keyword("case") {
            self.parse_case()
        } else {
            Err(format!("expected compound command, got {:?}", self.current))
        }
    }

    fn parse_brace_group(&mut self) -> Result<CompoundCommand, String> {
        self.expect_keyword("{")?;
        self.skip_newlines();
        let body = self.parse_program()?;
        self.expect_keyword("}")?;
        Ok(CompoundCommand::BraceGroup(body))
    }

    fn parse_subshell(&mut self) -> Result<CompoundCommand, String> {
        assert!(self.eat(&Token::LParen));
        self.skip_newlines();
        let body = self.parse_program()?;
        if !self.eat(&Token::RParen) {
            return Err("expected ')'".to_string());
        }
        Ok(CompoundCommand::Subshell(body))
    }

    fn parse_if(&mut self) -> Result<CompoundCommand, String> {
        self.expect_keyword("if")?;
        self.skip_newlines();
        let condition = self.parse_compound_list()?;
        self.expect_keyword("then")?;
        self.skip_newlines();
        let then_body = self.parse_program()?;

        let mut elif_parts = Vec::new();
        while self.is_keyword("elif") {
            self.advance();
            self.skip_newlines();
            let elif_cond = self.parse_compound_list()?;
            self.expect_keyword("then")?;
            self.skip_newlines();
            let elif_body = self.parse_program()?;
            elif_parts.push((elif_cond, elif_body));
        }

        let else_body = if self.is_keyword("else") {
            self.advance();
            self.skip_newlines();
            Some(self.parse_program()?)
        } else {
            None
        };

        self.expect_keyword("fi")?;
        Ok(CompoundCommand::If(IfClause {
            condition,
            then_body,
            elif_parts,
            else_body,
        }))
    }

    fn parse_for(&mut self) -> Result<CompoundCommand, String> {
        self.expect_keyword("for")?;
        let var = self
            .word_text()
            .ok_or_else(|| "expected variable name after 'for'".to_string())?;
        self.advance();

        self.skip_newlines();

        let words = if self.is_keyword("in") {
            self.advance();
            let mut words = Vec::new();
            while !matches!(self.current, Token::Semi | Token::Newline | Token::Eof) {
                if self.is_keyword("do") {
                    break;
                }
                if let Some(w) = self.take_word() {
                    words.push(w);
                } else {
                    break;
                }
            }
            if self.current == Token::Semi || self.current == Token::Newline {
                self.advance();
            }
            Some(words)
        } else {
            if self.current == Token::Semi || self.current == Token::Newline {
                self.advance();
            }
            None
        };

        self.skip_newlines();
        self.expect_keyword("do")?;
        self.skip_newlines();
        let body = self.parse_program()?;
        self.expect_keyword("done")?;

        Ok(CompoundCommand::For(ForClause { var, words, body }))
    }

    fn parse_while(&mut self) -> Result<CompoundCommand, String> {
        self.expect_keyword("while")?;
        self.skip_newlines();
        let condition = self.parse_compound_list()?;
        self.expect_keyword("do")?;
        self.skip_newlines();
        let body = self.parse_program()?;
        self.expect_keyword("done")?;
        Ok(CompoundCommand::While(WhileClause { condition, body }))
    }

    fn parse_until(&mut self) -> Result<CompoundCommand, String> {
        self.expect_keyword("until")?;
        self.skip_newlines();
        let condition = self.parse_compound_list()?;
        self.expect_keyword("do")?;
        self.skip_newlines();
        let body = self.parse_program()?;
        self.expect_keyword("done")?;
        Ok(CompoundCommand::Until(WhileClause { condition, body }))
    }

    fn parse_case(&mut self) -> Result<CompoundCommand, String> {
        self.expect_keyword("case")?;
        let word = self
            .take_word()
            .ok_or_else(|| "expected word after 'case'".to_string())?;
        self.skip_newlines();
        self.expect_keyword("in")?;
        self.skip_newlines();

        let mut items = Vec::new();
        while !self.is_keyword("esac") && self.current != Token::Eof {
            // Optional leading (
            if self.current == Token::LParen {
                self.advance();
            }

            let mut patterns = Vec::new();
            while let Some(w) = self.take_word() {
                patterns.push(w);
                if self.current == Token::Pipe {
                    self.advance();
                } else {
                    break;
                }
            }

            if !self.eat(&Token::RParen) {
                // Try to recover
                if patterns.is_empty() {
                    break;
                }
            }

            self.skip_newlines();
            let body = self.parse_program()?;

            if self.current == Token::DSemi {
                self.advance();
            }
            self.skip_newlines();

            items.push(CaseItem { patterns, body });
        }

        self.expect_keyword("esac")?;
        Ok(CompoundCommand::Case(CaseClause { word, items }))
    }

    fn parse_compound_list(&mut self) -> Result<Program, String> {
        self.skip_newlines();
        let mut commands = Vec::new();

        loop {
            if self.current == Token::Eof
                || self.is_keyword("then")
                || self.is_keyword("do")
                || self.is_keyword("done")
                || self.is_keyword("fi")
                || self.is_keyword("else")
                || self.is_keyword("elif")
                || self.is_keyword("esac")
            {
                break;
            }

            let cmd = self.parse_complete_command()?;
            commands.push(cmd);
            self.skip_newlines();
        }

        Ok(commands)
    }

    fn parse_simple_command(&mut self) -> Result<SimpleCommand, String> {
        let mut assignments = Vec::new();
        let mut words = Vec::new();
        let mut redirections = Vec::new();

        // Parse leading assignments
        while words.is_empty() {
            if let Some(assign) = self.try_parse_assignment() {
                assignments.push(assign);
            } else {
                break;
            }
        }

        // Parse words and redirections
        loop {
            // Check for redirections
            if let Some(redir) = self.try_parse_redirection()? {
                redirections.push(redir);
                continue;
            }

            // Check for word
            if let Token::Word(_) = &self.current {
                // Don't consume keywords that end compound commands
                if let Some(text) = self.word_text()
                    && is_compound_end(&text)
                {
                    break;
                }
                if let Some(w) = self.take_word() {
                    words.push(w);
                }
            } else {
                break;
            }
        }

        Ok(SimpleCommand {
            assignments,
            words,
            redirections,
        })
    }

    fn try_parse_assignment(&mut self) -> Option<Assignment> {
        if let Token::Word(parts) = &self.current
            && !parts.is_empty()
            && let WordPart::Literal(s) = &parts[0]
        {
            // Check for name= or name+= pattern
            if let Some(eq_pos) = s.find('=') {
                let before_eq = &s[..eq_pos];
                let (name, append) = if let Some(stripped) = before_eq.strip_suffix('+') {
                    (stripped, true)
                } else {
                    (before_eq, false)
                };

                // Validate name
                if !name.is_empty()
                    && name.chars().all(|c| c.is_alphanumeric() || c == '_')
                    && !name.chars().next().unwrap().is_ascii_digit()
                {
                    let name = name.to_string();
                    let after_eq = &s[eq_pos + 1..];

                    // Build the value word
                    let mut value_parts = Vec::new();
                    if !after_eq.is_empty() {
                        value_parts.push(WordPart::Literal(after_eq.to_string()));
                    }
                    // Add remaining parts of the token
                    for part in &parts[1..] {
                        value_parts.push(part.clone());
                    }

                    self.advance();
                    let value = if value_parts.is_empty() {
                        None
                    } else {
                        Some(value_parts)
                    };

                    return Some(Assignment {
                        name,
                        value,
                        append,
                    });
                }
            }
        }
        None
    }

    fn try_parse_redirection(&mut self) -> Result<Option<Redirection>, String> {
        let fd = self.try_parse_io_number();

        let kind = match &self.current {
            Token::Less => Some(RedirectKind::Input),
            Token::Great => Some(RedirectKind::Output),
            Token::DGreat => Some(RedirectKind::Append),
            Token::Clobber => Some(RedirectKind::Clobber),
            Token::LessAnd => Some(RedirectKind::DupInput),
            Token::GreatAnd => Some(RedirectKind::DupOutput),
            Token::LessGreat => Some(RedirectKind::ReadWrite),
            Token::DLess => Some(RedirectKind::HereDoc(false)),
            Token::DLessDash => Some(RedirectKind::HereDoc(true)),
            Token::TripleLess => Some(RedirectKind::HereString),
            _ => {
                if fd.is_some() {
                    // We consumed an IO number but there's no redirect operator.
                    // This shouldn't happen with our lexer design, but handle it.
                }
                None
            }
        };

        if let Some(kind) = kind {
            self.advance();
            match &kind {
                RedirectKind::HereDoc(_) => {
                    let target = self
                        .lexer
                        .take_heredoc_body()
                        .unwrap_or_else(|| vec![WordPart::Literal(String::new())]);
                    Ok(Some(Redirection { fd, kind, target }))
                }
                _ => {
                    let target = self
                        .take_word()
                        .ok_or_else(|| "expected word after redirection".to_string())?;
                    Ok(Some(Redirection { fd, kind, target }))
                }
            }
        } else {
            Ok(None)
        }
    }

    fn try_parse_io_number(&mut self) -> Option<i32> {
        if let Token::Word(parts) = &self.current
            && parts.len() == 1
            && let WordPart::Literal(s) = &parts[0]
            && s.len() == 1
            && s.chars().next().unwrap().is_ascii_digit()
        {
            // Check if next token is a redirect operator
            match self.peek_next_operator() {
                Some(true) => {
                    let n: i32 = s.parse().unwrap();
                    self.advance();
                    return Some(n);
                }
                _ => return None,
            }
        }
        None
    }

    fn peek_next_operator(&self) -> Option<bool> {
        // We need to check if the very next character after the current word
        // is a redirect operator. This is a simplification - in a real shell,
        // the IO_NUMBER token is recognized by the lexer when a digit immediately
        // precedes <, >, etc. with no whitespace.
        // For now, we don't handle this perfectly - just return None to avoid
        // consuming numbers that aren't IO numbers.
        None
    }

    fn parse_redirections(&mut self) -> Result<Vec<Redirection>, String> {
        let mut redirections = Vec::new();
        while let Some(redir) = self.try_parse_redirection()? {
            redirections.push(redir);
        }
        Ok(redirections)
    }
}

fn is_reserved_word(s: &str) -> bool {
    matches!(
        s,
        "if" | "then"
            | "else"
            | "elif"
            | "fi"
            | "for"
            | "while"
            | "until"
            | "do"
            | "done"
            | "case"
            | "esac"
            | "in"
            | "function"
            | "{"
            | "}"
            | "!"
            | "[["
            | "]]"
            | "select"
    )
}

fn is_compound_end(s: &str) -> bool {
    matches!(
        s,
        "}" | "fi" | "done" | "esac" | "then" | "else" | "elif" | "do" | ")" | "]]"
    )
}
