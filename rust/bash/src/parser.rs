use crate::ast::*;
use crate::lexer::{Lexer, Token};
use std::collections::HashMap;

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

    pub fn new_with_aliases(
        input: &str,
        aliases: HashMap<String, String>,
        expand_aliases: bool,
        posix_mode: bool,
    ) -> Self {
        let mut lexer = Lexer::new(input);
        lexer.aliases = aliases;
        lexer.shopt_expand_aliases = expand_aliases;
        lexer.posix_mode = posix_mode;
        let current = lexer.next_token();
        Self { lexer, current }
    }

    /// Update the alias table (called between parse-execute cycles)
    pub fn update_aliases(
        &mut self,
        aliases: HashMap<String, String>,
        expand_aliases: bool,
        posix_mode: bool,
    ) {
        self.lexer.aliases = aliases;
        self.lexer.shopt_expand_aliases = expand_aliases;
        self.lexer.posix_mode = posix_mode;
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

    pub fn heredoc_overflow_line(&self) -> Option<usize> {
        self.lexer.heredoc_overflow_line
    }

    pub fn take_heredoc_eof_warnings(&mut self) -> Vec<(usize, usize, String)> {
        std::mem::take(&mut self.lexer.heredoc_eof_warnings)
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

    fn token_to_str(&self) -> String {
        match &self.current {
            Token::Word(parts) => parts
                .iter()
                .map(|p| match p {
                    WordPart::Literal(s) => s.clone(),
                    _ => String::new(),
                })
                .collect(),
            Token::Newline => "newline".to_string(),
            Token::Pipe => "|".to_string(),
            Token::AndIf => "&&".to_string(),
            Token::OrIf => "||".to_string(),
            Token::Semi => ";".to_string(),
            Token::Amp => "&".to_string(),
            Token::DSemi => ";;".to_string(),
            Token::SemiAmp => ";&".to_string(),
            Token::DSemiAmp => ";;&".to_string(),
            Token::LParen => "(".to_string(),
            Token::RParen => ")".to_string(),
            Token::Eof => "EOF".to_string(),
            _ => format!("{:?}", self.current),
        }
    }

    fn expect_keyword(&mut self, kw: &str) -> Result<(), String> {
        if self.eat_keyword(kw) {
            Ok(())
        } else {
            let token_str = self.token_to_str();
            Err(format!(
                "syntax error near unexpected token `{}'",
                token_str
            ))
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

    pub fn is_at_eof(&self) -> bool {
        self.current == Token::Eof
    }

    /// Get current lexer position (for stuck detection)
    pub fn current_pos(&self) -> usize {
        self.lexer.current_pos()
    }

    /// Skip newlines and semicolons (for incremental parsing)
    pub fn skip_newlines_and_semis(&mut self) {
        while matches!(self.current, Token::Newline | Token::Semi) {
            self.advance();
        }
    }

    /// Parse a single complete command (public wrapper for incremental loop).
    /// Does NOT consume the trailing terminator, so the caller can sync aliases
    /// before the next token is read.
    pub fn parse_complete_command_pub(&mut self) -> Result<CompleteCommand, String> {
        self.parse_complete_command_inner(false)
    }

    /// Skip tokens until the next command boundary (newline or semicolon)
    pub fn skip_to_next_command(&mut self) {
        let mut limit = 1000; // safety limit
        while !matches!(self.current, Token::Eof | Token::Newline) && limit > 0 {
            self.advance();
            limit -= 1;
        }
        // Consume the newline
        if self.current == Token::Newline {
            self.advance();
        }
    }

    pub fn current_line(&self) -> usize {
        self.lexer.line
    }

    pub fn current_token_str(&self) -> String {
        match &self.current {
            Token::Word(parts) => parts
                .iter()
                .map(|p| match p {
                    WordPart::Literal(s) => s.clone(),
                    WordPart::SingleQuoted(s) => format!("'{}'", s),
                    _ => String::new(),
                })
                .collect(),
            Token::RParen => ")".to_string(),
            Token::LParen => "(".to_string(),
            Token::Semi => ";".to_string(),
            Token::DSemi => ";;".to_string(),
            Token::Pipe => "|".to_string(),
            Token::Amp => "&".to_string(),
            Token::AndIf => "&&".to_string(),
            Token::OrIf => "||".to_string(),
            Token::Newline => "newline".to_string(),
            Token::Eof => "EOF".to_string(),
            _ => "unknown".to_string(),
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
                || self.current == Token::SemiAmp
                || self.current == Token::DSemiAmp
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
        self.parse_complete_command_inner(true)
    }

    /// Parse a single complete command.
    /// If `consume_terminator` is true, advance past the trailing newline/semi.
    /// If false, leave the terminator in `self.current` so the caller can handle it
    /// (used by incremental parse-execute loop to sync aliases before reading the next token).
    fn parse_complete_command_inner(
        &mut self,
        consume_terminator: bool,
    ) -> Result<CompleteCommand, String> {
        let line = self.current_line();
        let mut list = self.parse_and_or_list()?;

        let background = match self.current {
            Token::Amp => {
                self.advance();
                true
            }
            Token::Semi | Token::Newline => {
                if consume_terminator {
                    self.advance();
                }
                false
            }
            _ => false,
        };

        // Resolve any deferred heredoc bodies (for pipeline heredocs like cmd <<EOF | cmd2)
        self.resolve_heredoc_bodies(&mut list);

        Ok(CompleteCommand {
            list,
            background,
            line,
        })
    }

    /// Fill in empty heredoc bodies that couldn't be resolved during parsing
    /// (happens when heredoc is in a pipeline: cmd <<EOF | cmd2)
    fn resolve_heredoc_bodies(&mut self, list: &mut AndOrList) {
        self.resolve_heredoc_in_pipeline(&mut list.first);
        for (_, pipeline) in &mut list.rest {
            self.resolve_heredoc_in_pipeline(pipeline);
        }
    }

    fn resolve_heredoc_in_pipeline(&mut self, pipeline: &mut Pipeline) {
        for cmd in &mut pipeline.commands {
            match cmd {
                Command::Simple(sc) => {
                    for redir in &mut sc.redirections {
                        if matches!(redir.kind, RedirectKind::HereDoc(_))
                            && (redir.target.is_empty()
                                || (redir.target.len() == 1
                                    && matches!(&redir.target[0], WordPart::Literal(s) if s.is_empty())))
                            && let Some(body) = self.lexer.take_heredoc_body()
                        {
                            redir.target = body;
                        }
                    }
                }
                Command::Compound(_, redirections) => {
                    for redir in redirections {
                        if matches!(redir.kind, RedirectKind::HereDoc(_))
                            && (redir.target.is_empty()
                                || (redir.target.len() == 1
                                    && matches!(&redir.target[0], WordPart::Literal(s) if s.is_empty())))
                            && let Some(body) = self.lexer.take_heredoc_body()
                        {
                            redir.target = body;
                        }
                    }
                }
                _ => {}
            }
        }
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
        let mut timed = self.eat_keyword("time");
        let mut negated = self.eat_keyword("!");
        // time can also come after !
        if !timed {
            timed = self.eat_keyword("time");
        }
        // Handle multiple ! (each additional one toggles negation)
        while self.eat_keyword("!") {
            negated = !negated;
        }
        // Consume additional time keywords (time time ... is valid)
        while self.eat_keyword("time") {
            timed = true;
        }
        // Consume `time -p` flag (POSIX time format) and `--`
        let mut time_posix = false;
        if timed {
            loop {
                if let Token::Word(ref w) = self.current {
                    let s: String = w
                        .iter()
                        .map(|p| match p {
                            WordPart::Literal(s) => s.as_str(),
                            _ => "",
                        })
                        .collect();
                    if s == "-p" {
                        time_posix = true;
                        self.advance();
                        continue;
                    }
                    if s == "--" {
                        self.advance();
                        continue;
                    }
                }
                // Check for another time keyword after -p
                if self.eat_keyword("time") {
                    continue;
                }
                break;
            }
        }

        let first = self.parse_command()?;
        let mut commands = vec![first];
        let mut pipe_stderr = Vec::new();

        while self.current == Token::Pipe || self.current == Token::PipeAmp {
            let is_pipe_amp = self.current == Token::PipeAmp;
            pipe_stderr.push(is_pipe_amp);
            self.advance();
            self.skip_newlines();
            commands.push(self.parse_command()?);
        }

        Ok(Pipeline {
            negated,
            timed,
            time_posix,
            commands,
            pipe_stderr,
        })
    }

    fn parse_command(&mut self) -> Result<Command, String> {
        // Check for function definition: name () compound_command
        if let Some(name) = self.word_text()
            && !is_reserved_word(&name)
            && !name.contains('=')
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
            || self.is_keyword("select")
            || self.is_keyword("while")
            || self.is_keyword("until")
            || self.is_keyword("case")
            || self.is_keyword("[[")
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
            // Check for (( — arithmetic command or C-style for
            let saved_pos = self.lexer.save_position();
            let saved_tok = self.current.clone();
            self.advance(); // consume first (
            if self.current == Token::LParen {
                // (( expression )) — don't consume second ( as token;
                // instead, use raw lexer to read until ))
                // Backtrack the lexer to just after the first ( (before second ()
                // The second ( is the current token, so lexer position is after it.
                // We need to read from including the content after the second (.
                // Actually, the lexer already read past the second ( to produce the
                // LParen token. So we read_until_double_paren from the current lexer pos.
                let expr = self.read_arith_command()?;
                // Sync the parser's current token
                self.current = self.lexer.next_token();
                return Ok(CompoundCommand::Arithmetic(expr));
            }
            // Not ((, backtrack to regular subshell
            self.lexer.restore_position(saved_pos);
            self.current = saved_tok;
            self.parse_subshell()
        } else if self.is_keyword("if") {
            self.parse_if()
        } else if self.is_keyword("for") || self.is_keyword("select") {
            self.parse_for()
        } else if self.is_keyword("while") {
            self.parse_while()
        } else if self.is_keyword("until") {
            self.parse_until()
        } else if self.is_keyword("case") {
            self.parse_case()
        } else if self.is_keyword("[[") {
            self.parse_conditional()
        } else {
            Err(format!("expected compound command, got {:?}", self.current))
        }
    }

    /// Read the body of `(( expr ))` — collect until matching `))`.
    fn read_arith_command(&mut self) -> Result<String, String> {
        // We've already consumed `((`; now read raw chars until `))`.
        let expr = self.lexer.read_until_double_paren()?;
        Ok(expr)
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
        // Accept both 'for' and 'select'
        if !self.eat_keyword("for") {
            self.expect_keyword("select")?;
        }

        // Check for C-style: for (( init; cond; step ))
        if self.current == Token::LParen {
            let saved_pos = self.lexer.save_position();
            let saved_tok = self.current.clone();
            self.advance(); // consume first (
            if self.current == Token::LParen {
                // Don't consume second ( as token — use raw lexer
                // Lexer position is right after the second ( was read
                return self.parse_arith_for();
            }
            self.lexer.restore_position(saved_pos);
            self.current = saved_tok;
        }

        let var = self.word_text().ok_or_else(|| {
            let token = self.token_to_str();
            format!("syntax error near unexpected token `{}'", token)
        })?;
        // Validate variable name
        if !var.chars().all(|c| c.is_alphanumeric() || c == '_')
            || var.chars().next().is_none_or(|c| c.is_ascii_digit())
        {
            return Err(format!("RUNTIME:`{}': not a valid identifier", var));
        }
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

    /// Parse `for (( init; cond; step )) do body done` — already consumed `((`
    fn parse_arith_for(&mut self) -> Result<CompoundCommand, String> {
        let init = self.lexer.read_until_char(';')?;
        let cond = self.lexer.read_until_char(';')?;
        let step = self.lexer.read_until_double_paren()?;
        // Sync parser — skip ; or newline before 'do'
        self.current = self.lexer.next_token();
        if matches!(self.current, Token::Semi | Token::Newline) {
            self.current = self.lexer.next_token();
        }
        self.skip_newlines();
        // Accept either do...done or { ... } for arith-for body
        let body = if self.eat_keyword("do") {
            self.skip_newlines();
            let body = self.parse_program()?;
            self.expect_keyword("done")?;
            body
        } else if let Token::Word(ref w) = self.current
            && w.len() == 1
            && matches!(&w[0], WordPart::Literal(s) if s == "{")
        {
            self.advance();
            self.skip_newlines();
            let body = self.parse_program()?;
            // Expect closing }
            if let Token::Word(ref w) = self.current
                && w.len() == 1
                && matches!(&w[0], WordPart::Literal(s) if s == "}")
            {
                self.advance();
            } else {
                return Err("syntax error near unexpected token".to_string());
            }
            body
        } else {
            return Err(format!(
                "syntax error near unexpected token `{}'",
                self.token_to_str()
            ));
        };
        Ok(CompoundCommand::ArithFor(ArithForClause {
            init,
            cond,
            step,
            body,
        }))
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
        // Suppress alias expansion in case patterns
        self.lexer.in_case_pattern = true;
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
            self.lexer.in_case_pattern = false;

            if !self.eat(&Token::RParen) {
                // Try to recover
                if patterns.is_empty() {
                    break;
                }
            }

            self.skip_newlines();
            let body = self.parse_program()?;

            let terminator = if self.current == Token::DSemi {
                self.advance();
                CaseTerminator::Break
            } else if self.current == Token::SemiAmp {
                self.advance();
                CaseTerminator::FallThrough
            } else if self.current == Token::DSemiAmp {
                self.advance();
                CaseTerminator::TestNext
            } else {
                CaseTerminator::Break
            };
            // Re-suppress alias expansion for next pattern
            self.lexer.in_case_pattern = true;
            self.skip_newlines();

            items.push(CaseItem {
                patterns,
                body,
                terminator,
            });
        }

        self.lexer.in_case_pattern = false;
        self.expect_keyword("esac")?;
        Ok(CompoundCommand::Case(CaseClause { word, items }))
    }

    /// Parse `[[ expression ]]`
    fn parse_conditional(&mut self) -> Result<CompoundCommand, String> {
        self.expect_keyword("[[")?;
        let expr = self.parse_cond_or()?;
        self.expect_keyword("]]")?;
        Ok(CompoundCommand::Conditional(expr))
    }

    fn parse_cond_or(&mut self) -> Result<CondExpr, String> {
        let mut left = self.parse_cond_and()?;
        while self.current == Token::OrIf {
            self.advance();
            self.skip_newlines();
            let right = self.parse_cond_and()?;
            left = CondExpr::Or(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_cond_and(&mut self) -> Result<CondExpr, String> {
        let mut left = self.parse_cond_primary()?;
        while self.current == Token::AndIf {
            self.advance();
            self.skip_newlines();
            let right = self.parse_cond_primary()?;
            left = CondExpr::And(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_cond_primary(&mut self) -> Result<CondExpr, String> {
        // Handle ! (negation)
        if self.is_keyword("!") {
            self.advance();
            let expr = self.parse_cond_primary()?;
            return Ok(CondExpr::Not(Box::new(expr)));
        }

        // Handle ( expr )
        if self.current == Token::LParen {
            self.advance();
            let expr = self.parse_cond_or()?;
            if !self.eat(&Token::RParen) {
                return Err("expected ')' in conditional".to_string());
            }
            return Ok(expr);
        }

        // Check for unary operators: -n, -z, -e, -f, -d, etc.
        if let Some(text) = self.word_text()
            && is_cond_unary_op(&text)
        {
            let op = text;
            self.advance();
            let operand = self
                .take_word()
                .ok_or_else(|| format!("expected operand after '{}'", op))?;
            return Ok(CondExpr::Unary(op, operand));
        }

        // Must be a word — check for binary operator after it
        let left = self
            .take_word()
            .ok_or_else(|| "expected expression in [[ ]]".to_string())?;

        // Check for ]]
        if self.is_keyword("]]") || self.current == Token::Eof {
            return Ok(CondExpr::Word(left));
        }

        // Check for && or || (handled by caller)
        if matches!(self.current, Token::AndIf | Token::OrIf) {
            return Ok(CondExpr::Word(left));
        }

        // Check for binary operator (as word token)
        if let Some(text) = self.word_text()
            && is_cond_binary_op(&text)
        {
            let op = text;
            self.advance();
            // For =~ (regex match), read the pattern as raw text.
            // For == != = (glob match), try normal word first but handle
            // extglob patterns by backtracking if LParen follows.
            let right = if op == "=~" {
                self.read_cond_pattern()?
            } else if op == "==" || op == "!=" || op == "=" {
                // Save state to backtrack if extglob
                let saved_lexer = self.lexer.save_position();
                let saved_tok = self.current.clone();
                if let Some(word) = self.take_word() {
                    if self.current == Token::LParen {
                        // This is an extglob pattern like +(foo) — backtrack
                        self.lexer.restore_position(saved_lexer);
                        self.current = saved_tok;
                        self.read_cond_pattern()?
                    } else {
                        word
                    }
                } else if matches!(self.current, Token::LParen) {
                    self.read_cond_pattern()?
                } else {
                    return Err(format!("expected operand after '{}'", op));
                }
            } else {
                self.take_word()
                    .ok_or_else(|| format!("expected operand after '{}'", op))?
            };
            return Ok(CondExpr::Binary(left, op, right));
        }

        // Check for < and > which are Token::Less/Token::Great inside [[ ]]
        if matches!(self.current, Token::Less | Token::Great) {
            let op = if self.current == Token::Less {
                "<".to_string()
            } else {
                ">".to_string()
            };
            self.advance();
            let right = self
                .take_word()
                .ok_or_else(|| format!("expected operand after '{}'", op))?;
            return Ok(CondExpr::Binary(left, op, right));
        }

        // Just a word
        Ok(CondExpr::Word(left))
    }

    /// Read a regex pattern for `[[ x =~ pattern ]]`.
    /// Regex patterns can contain ( ) | which are normally special tokens,
    /// so we read raw text from the lexer until we hit ]], &&, or ||.
    fn read_cond_pattern(&mut self) -> Result<Word, String> {
        let mut text = String::new();
        // Consume tokens and raw text until ]], &&, ||
        // This handles extglob patterns like +(foo|bar) and regex patterns
        loop {
            if self.is_keyword("]]") || self.current == Token::Eof {
                break;
            }
            if matches!(self.current, Token::AndIf | Token::OrIf) {
                break;
            }
            match &self.current {
                Token::Word(parts) => {
                    text.push_str(&word_to_string(parts));
                    self.advance();
                }
                Token::LParen => {
                    text.push('(');
                    self.advance();
                }
                Token::RParen => {
                    text.push(')');
                    self.advance();
                }
                Token::Pipe => {
                    text.push('|');
                    self.advance();
                }
                _ => break,
            }
        }
        let trimmed = text.trim().to_string();
        if trimmed.is_empty() {
            return Err("expected pattern in conditional".to_string());
        }
        Ok(vec![WordPart::Literal(trimmed)])
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

            // Assignments can appear interspersed with redirections before command words
            if words.is_empty()
                && let Some(assign) = self.try_parse_assignment()
            {
                assignments.push(assign);
                continue;
            }

            // Check for inline array assignment: if we see word ending with = followed by (
            // This handles `declare -a arr=(one two three)` etc.
            if self.current == Token::LParen && !words.is_empty() {
                // Check if last word ends with =
                let last_ends_with_eq = {
                    let last: &Word = &words[words.len() - 1];
                    if let Some(WordPart::Literal(s)) = last.last() {
                        s.ends_with('=') || s.ends_with("+=")
                    } else {
                        false
                    }
                };
                if last_ends_with_eq {
                    self.advance(); // consume (
                    let elements = self.parse_array_elements();
                    // For array assignments in command args (declare/local),
                    // expand each element individually and join with \x01 separator.
                    // This preserves the structure for the builtin to split.
                    let last = words.last_mut().unwrap();
                    last.push(WordPart::Literal("(".to_string()));
                    for (i, elem) in elements.iter().enumerate() {
                        if i > 0 {
                            last.push(WordPart::Literal("\x01".to_string()));
                        }
                        // Include [key]= prefix for associative array elements
                        if let Some(idx) = &elem.index {
                            last.push(WordPart::Literal("[".to_string()));
                            for part in idx {
                                last.push(part.clone());
                            }
                            last.push(WordPart::Literal("]=".to_string()));
                        }
                        for part in &elem.value {
                            last.push(part.clone());
                        }
                    }
                    last.push(WordPart::Literal(")".to_string()));
                    continue;
                }
            }

            // Check for word
            if let Token::Word(_) = &self.current {
                // Don't consume keywords that end compound commands,
                // but only at command position (first word).
                // In argument position, these are regular words.
                if words.is_empty()
                    && let Some(text) = self.word_text()
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
        // Extract all info from the current token without holding a borrow
        let info = if let Token::Word(parts) = &self.current {
            if parts.is_empty() {
                return None;
            }
            if let WordPart::Literal(s) = &parts[0] {
                if let Some(eq_pos) = s.find('=') {
                    let before_eq = &s[..eq_pos];
                    let (name_str, append) = if let Some(stripped) = before_eq.strip_suffix('+') {
                        (stripped, true)
                    } else {
                        (before_eq, false)
                    };
                    let base_name = if let Some(bracket) = name_str.find('[') {
                        &name_str[..bracket]
                    } else {
                        name_str
                    };
                    if !base_name.is_empty()
                        && base_name.chars().all(|c| c.is_alphanumeric() || c == '_')
                        && !base_name.chars().next().unwrap().is_ascii_digit()
                    {
                        let full_name = name_str.to_string();
                        let after_eq = s[eq_pos + 1..].to_string();
                        let num_parts = parts.len();
                        // Build value parts eagerly
                        let mut value_parts = Vec::new();
                        if !after_eq.is_empty() {
                            value_parts.push(WordPart::Literal(after_eq.clone()));
                        }
                        for part in &parts[1..] {
                            value_parts.push(part.clone());
                        }
                        Some((full_name, append, after_eq, num_parts, value_parts))
                    } else {
                        None
                    }
                } else if s.ends_with('[') || s.contains('[') {
                    // Handle array assignment with quoted subscript: name[quoted_key]=value
                    // The = is in a later part (e.g., BASH_ALIASES['\$']=xx)
                    let base_end = s.find('[').unwrap();
                    let base_name = &s[..base_end];
                    if !base_name.is_empty()
                        && base_name.chars().all(|c| c.is_alphanumeric() || c == '_')
                        && !base_name.chars().next().unwrap().is_ascii_digit()
                    {
                        // Find ]=  or ]+=  in a later Literal part
                        let mut found_eq = false;
                        let mut name_text = s.to_string();
                        let mut value_parts = Vec::new();
                        let mut eq_part_idx = 0;
                        for (idx, part) in parts[1..].iter().enumerate() {
                            if found_eq {
                                value_parts.push(part.clone());
                                continue;
                            }
                            match part {
                                WordPart::Literal(lit) => {
                                    if let Some(eq_pos) = lit.find("]=") {
                                        let before = &lit[..eq_pos + 1]; // include ]
                                        name_text.push_str(before);
                                        let after_eq = &lit[eq_pos + 2..];
                                        if !after_eq.is_empty() {
                                            value_parts
                                                .push(WordPart::Literal(after_eq.to_string()));
                                        }
                                        found_eq = true;
                                        eq_part_idx = idx + 1;
                                    } else {
                                        name_text.push_str(lit);
                                    }
                                }
                                WordPart::SingleQuoted(sq) => {
                                    name_text.push_str(sq);
                                }
                                WordPart::DoubleQuoted(dq) => {
                                    for dp in dq {
                                        if let WordPart::Literal(l) = dp {
                                            name_text.push_str(l);
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                        if found_eq {
                            let append = name_text.ends_with("+=");
                            let _ = eq_part_idx;
                            Some((name_text, append, String::new(), parts.len(), value_parts))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        let (full_name, append, after_eq, num_parts, value_parts) = info?;

        // Case 1: "name=(" all in one token
        if after_eq == "(" && num_parts == 1 {
            self.advance();
            let elements = self.parse_array_elements();
            return Some(Assignment {
                name: full_name,
                value: AssignValue::Array(elements),
                append,
            });
        }

        // Case 2: "name=" as one token, then LParen as next token
        if after_eq.is_empty() && num_parts == 1 {
            let saved_pos = self.lexer.save_position();
            let saved_tok = self.current.clone();
            self.advance();
            if self.current == Token::LParen {
                self.advance();
                let elements = self.parse_array_elements();
                return Some(Assignment {
                    name: full_name,
                    value: AssignValue::Array(elements),
                    append,
                });
            }
            // Not an array — backtrack
            self.lexer.restore_position(saved_pos);
            self.current = saved_tok;
        }

        // Scalar assignment — even empty value is a Scalar (a= sets to "")
        self.advance();
        let value = AssignValue::Scalar(value_parts);

        Some(Assignment {
            name: full_name,
            value,
            append,
        })
    }

    /// Parse array elements: `word1 [n]=word2 word3 ...` until `)`
    fn parse_array_elements(&mut self) -> Vec<ArrayElement> {
        let mut elements = Vec::new();
        self.skip_newlines();

        while self.current != Token::RParen && self.current != Token::Eof {
            // Check for [index]=value syntax by extracting info first
            let indexed_info = if let Token::Word(parts) = &self.current {
                if let Some(WordPart::Literal(s)) = parts.first() {
                    if s.starts_with('[') {
                        if let Some(close) = s.find("]+=") {
                            // [idx]+=value — per-element append
                            let idx_str = s[1..close].to_string();
                            let after = s[close + 3..].to_string();
                            let rest_parts: Vec<WordPart> = parts[1..].to_vec();
                            Some((idx_str, after, rest_parts, true))
                        } else if let Some(close) = s.find("]=") {
                            let idx_str = s[1..close].to_string();
                            let after = s[close + 2..].to_string();
                            let rest_parts: Vec<WordPart> = parts[1..].to_vec();
                            Some((idx_str, after, rest_parts, false))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };

            if let Some((idx_str, after, rest_parts, elem_append)) = indexed_info {
                self.advance();
                let mut value_parts = Vec::new();
                if !after.is_empty() {
                    value_parts.push(WordPart::Literal(after));
                }
                value_parts.extend(rest_parts);
                elements.push(ArrayElement {
                    index: Some(vec![WordPart::Literal(idx_str)]),
                    value: value_parts,
                    append: elem_append,
                });
                self.skip_newlines();
                continue;
            }

            if let Token::Word(_) = &self.current {
                if let Some(w) = self.take_word() {
                    elements.push(ArrayElement {
                        index: None,
                        value: w,
                        append: false,
                    });
                }
            } else {
                break;
            }
            self.skip_newlines();
        }

        // Consume the closing )
        self.eat(&Token::RParen);
        elements
    }

    fn try_parse_redirection(&mut self) -> Result<Option<Redirection>, String> {
        // Check for {varname}> style redirections
        let fd = self.try_parse_redir_fd();

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
            Token::AmpGreat => Some(RedirectKind::OutputAll),
            Token::AmpDGreat => Some(RedirectKind::AppendAll),
            _ => {
                if fd.is_some() {
                    // We consumed an IO number but there's no redirect operator.
                }
                None
            }
        };

        if let Some(kind) = kind {
            self.advance();
            match &kind {
                RedirectKind::HereDoc(_) => {
                    // Heredoc body is read when the next newline is tokenized.
                    // Use empty placeholder if not available yet (pipeline case).
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

    fn try_parse_redir_fd(&mut self) -> Option<RedirFd> {
        // Extract info from current token without holding borrow
        let token_info = if let Token::Word(parts) = &self.current {
            if parts.len() == 1 {
                if let WordPart::Literal(s) = &parts[0] {
                    Some(s.clone())
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        let s = token_info?;

        // Check for {varname} before redirect operator
        if s.starts_with('{') && s.ends_with('}') && s.len() > 2 {
            let varname = s[1..s.len() - 1].to_string();
            if varname.chars().all(|c| c.is_alphanumeric() || c == '_') {
                let saved_pos = self.lexer.save_position();
                let saved_tok = self.current.clone();
                self.advance();
                if matches!(
                    self.current,
                    Token::Less
                        | Token::Great
                        | Token::DGreat
                        | Token::LessAnd
                        | Token::GreatAnd
                        | Token::LessGreat
                ) {
                    return Some(RedirFd::Var(varname));
                }
                self.lexer.restore_position(saved_pos);
                self.current = saved_tok;
            }
        }

        // Check for numeric fd — only if the next token is a redirect operator
        // and there was no whitespace between the digit word and the redirect.
        // The lexer records whether whitespace preceded the current token.
        if !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()) {
            let saved_pos = self.lexer.save_position();
            let saved_tok = self.current.clone();
            self.advance();
            let is_redir = !self.lexer.had_whitespace_before_token
                && matches!(
                    self.current,
                    Token::Less
                        | Token::Great
                        | Token::DGreat
                        | Token::LessAnd
                        | Token::GreatAnd
                        | Token::LessGreat
                        | Token::Clobber
                        | Token::AmpGreat
                        | Token::AmpDGreat
                );
            if is_redir {
                let n: i32 = s.parse().unwrap();
                return Some(RedirFd::Number(n));
            }
            // Not a redirect — backtrack
            self.lexer.restore_position(saved_pos);
            self.current = saved_tok;
        }

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
        "}" | "fi" | "done" | "esac" | "then" | "else" | "elif" | "do" | "]]"
    )
}

fn is_cond_unary_op(s: &str) -> bool {
    matches!(
        s,
        "-n" | "-z"
            | "-e"
            | "-f"
            | "-d"
            | "-r"
            | "-w"
            | "-x"
            | "-s"
            | "-L"
            | "-h"
            | "-a"
            | "-b"
            | "-c"
            | "-g"
            | "-k"
            | "-p"
            | "-t"
            | "-u"
            | "-G"
            | "-N"
            | "-O"
            | "-S"
            | "-o"
            | "-v"
            | "-R"
    )
}

fn is_cond_binary_op(s: &str) -> bool {
    matches!(
        s,
        "=" | "=="
            | "!="
            | "<"
            | ">"
            | "-eq"
            | "-ne"
            | "-lt"
            | "-le"
            | "-gt"
            | "-ge"
            | "-nt"
            | "-ot"
            | "-ef"
            | "=~"
    )
}
