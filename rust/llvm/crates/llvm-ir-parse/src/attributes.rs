//! Attribute parsing, including the spellings that differ inside a group.

use crate::lexer::Token;
use crate::{ParseError, Parser};
use llvm_ir::TypeId;
use llvm_ir::attribute::{Attribute, AttributeSet, EnumAttr, IntAttr, StructuredAttr, TypeAttr};
use llvm_ir::types::TypeKind;
use llvm_support::ApInt;

impl Parser {
    // ------------------------------------------------------------ attributes

    /// One attribute. `in_group` selects between the two spellings upstream
    /// uses for the same attribute: `align 8` in a parameter list and
    /// `align=8` inside an attribute group.
    pub(crate) fn parse_attribute(&mut self, in_group: bool) -> Result<Attribute, ParseError> {
        if let Token::Quoted(key) = self.peek().clone() {
            self.advance();
            let value = if self.eat(&Token::Equals) {
                Some(self.require_quoted()?)
            } else {
                None
            };
            return Ok(Attribute::String { key, value });
        }
        let word = self.require_word()?;

        if word == "range" {
            self.require(Token::LeftParen)?;
            let ty = self.parse_type()?;
            let bits = self.integer_width(ty)?;
            let lower = self.parse_ap_int(bits)?;
            self.require(Token::Comma)?;
            let upper = self.parse_ap_int(bits)?;
            self.require(Token::RightParen)?;
            return Ok(Attribute::Range { ty, lower, upper });
        }

        if let Some(kind) = TypeAttr::from_keyword(&word) {
            self.require(Token::LeftParen)?;
            let ty = self.parse_type()?;
            self.require(Token::RightParen)?;
            return Ok(Attribute::Type { kind, ty });
        }

        // `uwtable` is both a bare keyword and a parenthesised one, so the
        // structured reading only applies when an argument list follows.
        if let Some(kind) = StructuredAttr::from_keyword(&word)
            && self.peek() == &Token::LeftParen
        {
            let arguments = self.collect_parenthesised()?;
            return Ok(Attribute::Structured { kind, arguments });
        }

        if let Some(kind) = IntAttr::from_keyword(&word) {
            // Upstream spells the same attribute three ways depending on
            // where it sits: `align 8` in a parameter list, `align=8` and
            // `alignstack=4` inside a group, `alignstack(4)` everywhere else.
            if in_group && matches!(kind, IntAttr::Align | IntAttr::AlignStack) {
                self.require(Token::Equals)?;
                let first = self.require_unsigned()?;
                return Ok(Attribute::Int {
                    kind,
                    first,
                    second: None,
                });
            }
            if !in_group && kind == IntAttr::Align {
                let first = self.require_unsigned()?;
                return Ok(Attribute::Int {
                    kind,
                    first,
                    second: None,
                });
            }
            self.require(Token::LeftParen)?;
            let first = self.require_unsigned()?;
            let second = if self.eat(&Token::Comma) {
                Some(self.require_unsigned()?)
            } else {
                None
            };
            self.require(Token::RightParen)?;
            return Ok(Attribute::Int {
                kind,
                first,
                second,
            });
        }

        if let Some(kind) = EnumAttr::from_keyword(&word) {
            return Ok(Attribute::Enum(kind));
        }

        self.error(format!(
            "unknown attribute '{word}'; add it to llvm_ir::attribute rather than ignoring it"
        ))
    }

    /// The text between a matching pair of parentheses, kept verbatim.
    pub(crate) fn collect_parenthesised(&mut self) -> Result<String, ParseError> {
        self.require(Token::LeftParen)?;
        let mut depth = 1usize;
        let mut parts: Vec<String> = Vec::new();
        let mut previous_was_word = false;
        loop {
            let token = self.advance();
            match &token {
                Token::LeftParen => depth += 1,
                Token::RightParen => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(parts.join(""));
                    }
                }
                Token::Eof => return self.error("unterminated attribute argument list"),
                _ => {}
            }
            let text = match &token {
                Token::Word(word) => word.clone(),
                // `memory(argmem: readwrite)` reaches the lexer as a label,
                // because a word followed by a colon is a label everywhere
                // else in the grammar.
                Token::Label(word) => format!("{word}: "),
                Token::Integer { negative, digits } => {
                    format!("{}{digits}", if *negative { "-" } else { "" })
                }
                Token::Comma => ",".to_string(),
                Token::LeftParen => "(".to_string(),
                Token::RightParen => ")".to_string(),
                Token::Quoted(text) => format!("\"{text}\""),
                other => other.describe(),
            };
            let is_word = matches!(token, Token::Word(_) | Token::Integer { .. });
            // `memory(argmem: readwrite)` and `allockind("alloc,zeroed")`
            // both print with a space after each comma and after each colon.
            if previous_was_word && is_word {
                parts.push(" ".to_string());
            }
            if text == "," {
                parts.push(",".to_string());
                parts.push(" ".to_string());
                previous_was_word = false;
                continue;
            }
            parts.push(text);
            previous_was_word = is_word;
        }
    }

    /// A run of attributes, stopping at the first token that is not one.
    pub(crate) fn parse_attribute_set(
        &mut self,
        in_group: bool,
    ) -> Result<AttributeSet, ParseError> {
        let mut set = AttributeSet::default();
        loop {
            match self.peek().clone() {
                Token::AttributeGroup(number) => {
                    self.advance();
                    set.groups.push(number);
                }
                Token::Quoted(_) => set.attributes.push(self.parse_attribute(in_group)?),
                Token::Word(word) => {
                    let known = EnumAttr::from_keyword(&word).is_some()
                        || IntAttr::from_keyword(&word).is_some()
                        || TypeAttr::from_keyword(&word).is_some()
                        || StructuredAttr::from_keyword(&word).is_some()
                        || word == "range";
                    if !known {
                        return Ok(set);
                    }
                    set.attributes.push(self.parse_attribute(in_group)?);
                }
                _ => return Ok(set),
            }
        }
    }

    pub(crate) fn integer_width(&mut self, ty: TypeId) -> Result<u32, ParseError> {
        match self.module.ctx.type_kind(ty) {
            TypeKind::Integer(bits) => Ok(*bits),
            _ => self.error("expected an integer type"),
        }
    }

    pub(crate) fn parse_ap_int(&mut self, bits: u32) -> Result<ApInt, ParseError> {
        match self.advance() {
            Token::Integer { negative, digits } => {
                let text = if negative {
                    format!("-{digits}")
                } else {
                    digits
                };
                ApInt::parse_sized(&text, 10, bits).map_or_else(
                    |error| {
                        self.index -= 1;
                        self.error(error.to_string())
                    },
                    Ok,
                )
            }
            Token::Word(word) if word == "true" => Ok(ApInt::from_u64(bits, 1)),
            Token::Word(word) if word == "false" => Ok(ApInt::new(bits)),
            other => {
                self.index -= 1;
                self.error(format!(
                    "expected an integer literal, found {}",
                    other.describe()
                ))
            }
        }
    }
}
