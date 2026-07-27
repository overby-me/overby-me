//! The top-level loop: everything a module holds outside a function body.

use crate::attributes::LegacyMemory;
use crate::lexer::Token;
use crate::{ParseError, Parser};
use llvm_ir::TypeId;
use llvm_ir::global::{Comdat, ComdatKind};
use llvm_ir::metadata::NamedMetadata;
use llvm_ir::summary::{SummaryEntry, SummaryField, SummaryValue};
use llvm_ir::value::{GlobalRef, MdId, Name};
use llvm_support::{DataLayout, Triple};

impl Parser {
    // ------------------------------------------------------------- top level

    pub(crate) fn parse_top_level(&mut self) -> Result<(), ParseError> {
        loop {
            match self.peek().clone() {
                Token::Eof => return Ok(()),
                Token::Word(word) => match word.as_str() {
                    "source_filename" => {
                        self.advance();
                        self.require(Token::Equals)?;
                        self.module.source_filename = Some(self.require_quoted()?);
                    }
                    "target" => {
                        self.advance();
                        let which = self.require_word()?;
                        self.require(Token::Equals)?;
                        let text = self.require_quoted()?;
                        match which.as_str() {
                            "datalayout" => {
                                let layout =
                                    DataLayout::parse(&text).map_err(|error| ParseError {
                                        position: self.position(),
                                        message: error.to_string(),
                                    })?;
                                self.module.data_layout = Some(layout);
                            }
                            "triple" => self.module.triple = Some(Triple::parse(&text)),
                            other => {
                                return self
                                    .error(format!("unknown target specification '{other}'"));
                            }
                        }
                    }
                    "module" => {
                        self.advance();
                        if !self.eat_word("asm") {
                            return self.error("expected 'asm' after 'module'");
                        }
                        let text = self.require_quoted()?;
                        self.module.module_asm.push(text);
                    }
                    "define" => {
                        self.advance();
                        self.parse_function(true)?;
                    }
                    "declare" => {
                        self.advance();
                        self.parse_function(false)?;
                    }
                    "attributes" => {
                        self.advance();
                        self.parse_attribute_group()?;
                    }
                    "uselistorder" | "uselistorder_bb" => {
                        self.parse_use_list_order()?;
                    }
                    other => {
                        return self.error(format!("unexpected top-level keyword '{other}'"));
                    }
                },
                Token::LocalName(name) => {
                    self.advance();
                    self.parse_type_definition(&name)?;
                }
                // `%0 = type { ... }` names a struct by number. Upstream
                // prints those back by number, so the name is the digits.
                Token::LocalNumber(number) => {
                    self.advance();
                    self.parse_type_definition(&number.to_string())?;
                    let id = self.module.ctx.named_struct(&number.to_string());
                    self.module.ctx.set_struct_numbered(id);
                }
                Token::SummaryNumber(number) => {
                    self.advance();
                    self.parse_summary_entry(number)?;
                }
                Token::GlobalName(name) => {
                    self.advance();
                    self.parse_global_definition(Name::Named(name))?;
                }
                Token::GlobalNumber(number) => {
                    self.advance();
                    self.parse_global_definition(Name::Number(number))?;
                }
                Token::ComdatName(name) => {
                    self.advance();
                    self.require(Token::Equals)?;
                    if !self.eat_word("comdat") {
                        return self.error("expected 'comdat'");
                    }
                    let kind_word = self.require_word()?;
                    let Some(kind) = ComdatKind::from_keyword(&kind_word) else {
                        return self.error(format!("unknown comdat kind '{kind_word}'"));
                    };
                    self.module.comdats.push(Comdat { name, kind });
                }
                Token::MetadataName(name) => {
                    self.advance();
                    self.require(Token::Equals)?;
                    self.require(Token::Exclaim)?;
                    self.require(Token::LeftBrace)?;
                    let mut operands = Vec::new();
                    while !self.eat(&Token::RightBrace) {
                        if !operands.is_empty() {
                            self.require(Token::Comma)?;
                        }
                        // A named list holds references, and the two kinds
                        // that are written at every use rather than numbered
                        // may be written here in place. An ordinary node may
                        // not: upstream refuses `!named = !{!{i32 1}}`.
                        match self.peek().clone() {
                            Token::MetadataNumber(number) => {
                                self.advance();
                                operands.push(MdId(number));
                            }
                            Token::Exclaim | Token::MetadataName(_) => {
                                let node = self.parse_metadata_definition(None)?;
                                if !crate::metadata::prints_in_place(&node) {
                                    return self.error(
                                        "a named metadata list holds references, not nodes",
                                    );
                                }
                                let id = MdId(self.next_inline_metadata);
                                self.next_inline_metadata += 1;
                                self.module.set_metadata(id, node);
                                operands.push(id);
                            }
                            other => {
                                return self.error(format!(
                                    "expected a metadata reference, found {}",
                                    other.describe()
                                ));
                            }
                        }
                    }
                    self.module.named_metadata.push(NamedMetadata {
                        name: name.into(),
                        operands,
                    });
                }
                Token::MetadataNumber(number) => {
                    self.advance();
                    self.require(Token::Equals)?;
                    let node = self.parse_metadata_definition(None)?;
                    self.module.set_metadata(MdId(number), node);
                }
                other => {
                    return self.error(format!("unexpected {} at top level", other.describe()));
                }
            }
        }
    }

    /// A `uselistorder` or `uselistorder_bb` directive, read and dropped.
    ///
    /// The directive records what order a value's uses were in, so that
    /// bitcode round-trips through the textual form without reordering them.
    /// `llvm-dis` prints none of them unless asked to, so reading one and
    /// keeping nothing is what reproduces upstream's output; keeping them
    /// would print something upstream does not.
    ///
    /// What is not checked is whether the indexes are a permutation of the
    /// use list, which upstream does check. That needs the def-use chains
    /// this does not build, so a module with nonsense indexes is read where
    /// upstream refuses it.
    pub(crate) fn parse_use_list_order(&mut self) -> Result<(), ParseError> {
        // `uselistorder_bb` names a function and a block, so it has one more
        // comma-separated item before the index list than `uselistorder`.
        let by_block = self.require_word()? == "uselistorder_bb";
        if by_block {
            let name = match self.advance() {
                Token::GlobalName(name) => Name::Named(name),
                Token::GlobalNumber(number) => Name::Number(number),
                other => {
                    self.index -= 1;
                    return self.error(format!(
                        "expected a function name in uselistorder_bb, found {}",
                        other.describe()
                    ));
                }
            };
            let function = match self.symbols.get(&name) {
                Some(GlobalRef::Function(id)) => *id,
                _ => {
                    return self
                        .error("uselistorder_bb names a function this module does not have");
                }
            };
            self.require(Token::Comma)?;
            let block = match self.advance() {
                Token::LocalName(text) => text,
                // A block is named here even when it prints as a number
                // elsewhere: upstream has no way to reach a numbered one.
                Token::LocalNumber(_) => {
                    self.index -= 1;
                    return self.error("uselistorder_bb needs a named block, not a numbered one");
                }
                other => {
                    self.index -= 1;
                    return self.error(format!(
                        "expected a block in uselistorder_bb, found {}",
                        other.describe()
                    ));
                }
            };
            let target = self.module.function(function);
            if target.block_order.is_empty() {
                return self.error("uselistorder_bb names a function with no body");
            }
            let defined = target
                .block_order
                .iter()
                .any(|id| target.block(*id).name.as_ref() == Some(&Name::Named(block.clone())));
            if !defined {
                return self.error("uselistorder_bb names a block its function does not define");
            }
            self.require(Token::Comma)?;
            return self.parse_use_list_indexes();
        }
        let items = 1;
        // What names the value is skipped rather than read: it may be a
        // constant expression with commas and parentheses of its own, and
        // nothing here needs to know which value it was. The scan stops at
        // the comma before the index list, which is the first one outside
        // any bracket.
        for item in 0..items {
            if item > 0 {
                self.require(Token::Comma)?;
            }
            let mut depth = 0i32;
            loop {
                match self.peek() {
                    Token::LeftParen | Token::LeftBracket | Token::Less | Token::LeftBrace => {
                        depth += 1;
                    }
                    Token::RightParen
                    | Token::RightBracket
                    | Token::Greater
                    | Token::RightBrace => depth -= 1,
                    Token::Comma if depth == 0 => break,
                    Token::Eof => return self.error("unterminated uselistorder directive"),
                    _ => {}
                }
                self.advance();
            }
        }
        self.require(Token::Comma)?;
        self.parse_use_list_indexes()
    }

    /// The `{ 1, 0 }` half of a directive.
    ///
    /// The indexes are a permutation of a use list, so they are distinct and
    /// they change the order: upstream refuses `{ 0, 1 }` for saying nothing.
    /// Whether they cover the list is not checked, that needing the use lists
    /// this does not build.
    fn parse_use_list_indexes(&mut self) -> Result<(), ParseError> {
        self.require(Token::LeftBrace)?;
        let mut indexes = Vec::new();
        while !self.eat(&Token::RightBrace) {
            if !indexes.is_empty() {
                self.require(Token::Comma)?;
            }
            indexes.push(self.require_unsigned()?);
        }
        let mut sorted = indexes.clone();
        sorted.sort_unstable();
        sorted.dedup();
        if sorted.len() != indexes.len() {
            return self.error("uselistorder indexes are a permutation, so they are distinct");
        }
        if indexes.windows(2).all(|pair| pair[0] < pair[1]) {
            return self.error("uselistorder indexes that are already in order say nothing");
        }
        Ok(())
    }

    fn parse_type_definition(&mut self, name: &str) -> Result<(), ParseError> {
        self.require(Token::Equals)?;
        if !self.eat_word("type") {
            return self.error("expected 'type' in a type definition");
        }
        if self.eat_word("opaque") {
            self.module.ctx.named_struct(name);
            return Ok(());
        }
        // Only a struct body makes an identified type. Anything else is an
        // alias that gets expanded where it is used, which is why upstream
        // prints `byval([8 x i8])` for a parameter written `byval(%alias)`.
        let is_struct = self.peek() == &Token::LeftBrace
            || (self.peek() == &Token::Less && self.peek_at(1) == &Token::LeftBrace);
        if !is_struct {
            let ty = self.parse_type()?;
            self.module.ctx.set_type_alias(name, ty);
            return Ok(());
        }
        let id = self.module.ctx.named_struct(name);
        let (fields, packed) = self.parse_struct_body()?;
        self.module.ctx.set_struct_body(id, fields, packed);
        Ok(())
    }

    pub(crate) fn parse_struct_body(&mut self) -> Result<(Vec<TypeId>, bool), ParseError> {
        let packed = if self.peek() == &Token::Less && self.peek_at(1) == &Token::LeftBrace {
            self.advance();
            true
        } else {
            false
        };
        self.require(Token::LeftBrace)?;
        let mut fields = Vec::new();
        while !self.eat(&Token::RightBrace) {
            if !fields.is_empty() {
                self.require(Token::Comma)?;
            }
            if self.peek() == &Token::RightBrace {
                self.advance();
                break;
            }
            let field = self.parse_type()?;
            if !self.is_valid_struct_field(field) {
                return self.error("invalid structure element type");
            }
            fields.push(field);
        }
        if packed {
            self.require(Token::Greater)?;
        }
        Ok((fields, packed))
    }

    fn parse_attribute_group(&mut self) -> Result<(), ParseError> {
        let number = match self.advance() {
            Token::AttributeGroup(number) => number,
            other => {
                self.index -= 1;
                return self.error(format!(
                    "expected an attribute group number, found {}",
                    other.describe()
                ));
            }
        };
        self.require(Token::Equals)?;
        self.require(Token::LeftBrace)?;
        // A group is function attributes by definition, so the spellings
        // that predate `memory(...)` mean the same here as on a function.
        let mut attributes = Vec::new();
        let mut legacy = LegacyMemory::default();
        while !self.eat(&Token::RightBrace) {
            if let Token::Word(word) = self.peek().clone()
                && crate::attributes::is_legacy_memory(&word)
            {
                self.advance();
                legacy.take(&word);
                continue;
            }
            attributes.push(self.parse_attribute(true)?);
        }
        crate::attributes::apply_legacy_memory(&mut attributes, legacy);
        self.module.attribute_groups.push((number, attributes));
        Ok(())
    }
}

impl Parser {
    /// `^0 = module: (path: "a.o", hash: (1, 2, 3, 4, 5))`.
    ///
    /// The grammar is uniform all the way down, so this is one recursive
    /// value parser and a keyword in front of it.
    pub(crate) fn parse_summary_entry(&mut self, id: u32) -> Result<(), ParseError> {
        self.require(Token::Equals)?;
        // A word followed by a colon lexes as a label everywhere in this
        // grammar, which is exactly what a summary keyword is.
        let kind = match self.advance() {
            Token::Label(kind) => kind,
            // A space before the colon is legal and upstream writes one.
            Token::Word(kind) if self.eat(&Token::Colon) => kind,
            _ => {
                self.index -= 1;
                return self.error("expected a summary keyword");
            }
        };
        let value = self.parse_summary_value()?;
        self.module.summary.push(SummaryEntry { id, kind, value });
        Ok(())
    }

    fn parse_summary_value(&mut self) -> Result<SummaryValue, ParseError> {
        match self.advance() {
            Token::SummaryNumber(number) => Ok(SummaryValue::Ref(number)),
            Token::Quoted(bytes) => match String::from_utf8(bytes) {
                Ok(text) => Ok(SummaryValue::String(text)),
                Err(_) => self.error("a summary string has to be valid UTF-8"),
            },
            Token::Word(word) => {
                // `writeonly ^14`: a word can qualify the value after it.
                if matches!(
                    self.peek(),
                    Token::SummaryNumber(_) | Token::Quoted(_) | Token::Integer { .. }
                ) {
                    let value = self.parse_summary_value()?;
                    return Ok(SummaryValue::Qualified(word, Box::new(value)));
                }
                Ok(SummaryValue::Word(word))
            }
            Token::Integer { negative, digits } => {
                let value: u64 = digits.parse().map_err(|_| {
                    self.error::<()>(format!("{digits} does not fit a summary value"))
                        .unwrap_err()
                })?;
                if negative {
                    return self.error("a summary value is not negative");
                }
                Ok(SummaryValue::Number(value))
            }
            Token::LeftParen => {
                let mut fields = Vec::new();
                while !self.eat(&Token::RightParen) {
                    if !fields.is_empty() {
                        self.require(Token::Comma)?;
                    }
                    if self.eat(&Token::RightParen) {
                        break;
                    }
                    fields.push(self.parse_summary_field()?);
                }
                Ok(SummaryValue::Tuple(fields))
            }
            other => {
                self.index -= 1;
                self.error(format!(
                    "expected a summary value, found {}",
                    other.describe()
                ))
            }
        }
    }

    /// A tuple item, which is `key: value` when a colon follows the word and
    /// a bare value otherwise. `(none)` and `(linkage: external)` are both
    /// tuples of one item.
    fn parse_summary_field(&mut self) -> Result<SummaryField, ParseError> {
        if let Token::Label(key) = self.peek().clone() {
            self.advance();
            return Ok(SummaryField {
                key: Some(key),
                value: self.parse_summary_value()?,
            });
        }
        if let Token::Word(key) = self.peek().clone()
            && self.peek_at(1) == &Token::Colon
        {
            self.advance();
            self.advance();
            return Ok(SummaryField {
                key: Some(key),
                value: self.parse_summary_value()?,
            });
        }
        Ok(SummaryField {
            key: None,
            value: self.parse_summary_value()?,
        })
    }
}
