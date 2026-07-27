//! The top-level loop: everything a module holds outside a function body.

use crate::lexer::Token;
use crate::{ParseError, Parser};
use llvm_ir::TypeId;
use llvm_ir::global::{Comdat, ComdatKind};
use llvm_ir::metadata::NamedMetadata;
use llvm_ir::summary::{SummaryEntry, SummaryField, SummaryValue};
use llvm_ir::value::{MdId, Name};
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
                        return self.error(
                            "use-list order directives are not modelled; \
                             re-emit without -preserve-ll-uselistorder",
                        );
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
                        match self.advance() {
                            Token::MetadataNumber(number) => operands.push(MdId(number)),
                            other => {
                                self.index -= 1;
                                return self.error(format!(
                                    "expected a metadata reference, found {}",
                                    other.describe()
                                ));
                            }
                        }
                    }
                    self.module
                        .named_metadata
                        .push(NamedMetadata { name, operands });
                }
                Token::MetadataNumber(number) => {
                    self.advance();
                    self.require(Token::Equals)?;
                    let node = self.parse_metadata_definition()?;
                    self.module.set_metadata(MdId(number), node);
                }
                other => {
                    return self.error(format!("unexpected {} at top level", other.describe()));
                }
            }
        }
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
            if !self.is_valid_aggregate_element(field) {
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
        let mut attributes = Vec::new();
        while !self.eat(&Token::RightBrace) {
            attributes.push(self.parse_attribute(true)?);
        }
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
        let Token::Label(kind) = self.advance() else {
            self.index -= 1;
            return self.error("expected a summary keyword");
        };
        let value = self.parse_summary_value()?;
        self.module.summary.push(SummaryEntry { id, kind, value });
        Ok(())
    }

    fn parse_summary_value(&mut self) -> Result<SummaryValue, ParseError> {
        match self.advance() {
            Token::SummaryNumber(number) => Ok(SummaryValue::Ref(number)),
            Token::Quoted(text) => Ok(SummaryValue::String(text)),
            Token::Word(word) => Ok(SummaryValue::Word(word)),
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
        Ok(SummaryField {
            key: None,
            value: self.parse_summary_value()?,
        })
    }
}
