//! Metadata parsing.
//!
//! Specialized nodes keep their tag and their fields as written. The one
//! lexical oddity is that `line:` inside `!DILocation(line: 1)` arrives as a
//! label token, because a word followed by a colon is a label everywhere else
//! in the grammar; the parser reads it back as a field name.

use crate::lexer::Token;
use crate::{FunctionState, ParseError, Parser};
use llvm_ir::function::Function;
use llvm_ir::metadata::{MdAttachment, MdField, MdOperand, Metadata, SpecializedArgs};
use llvm_ir::value::MdId;

/// Where a metadata operand is being read, which decides whether local values
/// are allowed in it.
type ValueContext<'a> = Option<(&'a mut Function, &'a mut FunctionState)>;

impl Parser {
    pub(crate) fn parse_metadata_definition(&mut self) -> Result<Metadata, ParseError> {
        let distinct = self.eat_word("distinct");
        match self.peek().clone() {
            Token::MetadataString(text) => {
                self.advance();
                if distinct {
                    return self.error("a metadata string cannot be distinct");
                }
                Ok(Metadata::String(text))
            }
            Token::Exclaim => {
                self.advance();
                self.require(Token::LeftBrace)?;
                let mut operands = Vec::new();
                while !self.eat(&Token::RightBrace) {
                    if !operands.is_empty() {
                        self.require(Token::Comma)?;
                    }
                    if self.peek() == &Token::RightBrace {
                        self.advance();
                        break;
                    }
                    operands.push(self.parse_metadata_operand(None)?);
                }
                Ok(Metadata::Tuple { distinct, operands })
            }
            Token::MetadataName(tag) => {
                self.advance();
                let args = self.parse_specialized_args()?;
                Ok(Metadata::Specialized {
                    distinct,
                    tag,
                    args,
                })
            }
            other => self.error(format!(
                "expected a metadata node, found {}",
                other.describe()
            )),
        }
    }

    fn parse_specialized_args(&mut self) -> Result<SpecializedArgs, ParseError> {
        self.require(Token::LeftParen)?;
        if self.eat(&Token::RightParen) {
            return Ok(SpecializedArgs::Positional(Vec::new()));
        }
        let named = matches!(self.peek(), Token::Label(_));
        if named {
            let mut fields = Vec::new();
            loop {
                let Token::Label(key) = self.advance() else {
                    self.index -= 1;
                    return self.error("expected a field name in a specialized node");
                };
                let value = self.parse_metadata_field()?;
                fields.push((key, value));
                if !self.eat(&Token::Comma) {
                    break;
                }
            }
            self.require(Token::RightParen)?;
            Ok(SpecializedArgs::Named(fields))
        } else {
            let mut fields = Vec::new();
            loop {
                fields.push(self.parse_metadata_field()?);
                if !self.eat(&Token::Comma) {
                    break;
                }
            }
            self.require(Token::RightParen)?;
            Ok(SpecializedArgs::Positional(fields))
        }
    }

    fn parse_metadata_field(&mut self) -> Result<MdField, ParseError> {
        match self.peek().clone() {
            Token::Integer { negative, digits } => {
                self.advance();
                if negative {
                    let value: i64 = digits.parse().map_err(|_| {
                        self.error::<()>(format!("{digits} does not fit"))
                            .unwrap_err()
                    })?;
                    Ok(MdField::Signed(-value))
                } else {
                    let value: u64 = digits.parse().map_err(|_| {
                        self.error::<()>(format!("{digits} does not fit"))
                            .unwrap_err()
                    })?;
                    Ok(MdField::Unsigned(value))
                }
            }
            Token::Quoted(text) => {
                self.advance();
                Ok(MdField::Str(text))
            }
            Token::MetadataNumber(number) => {
                self.advance();
                Ok(MdField::Ref(MdId(number)))
            }
            Token::MetadataString(text) => {
                self.advance();
                Ok(MdField::Inline(Box::new(Metadata::String(text))))
            }
            Token::MetadataName(_) | Token::Exclaim => {
                let node = self.parse_metadata_definition()?;
                Ok(MdField::Inline(Box::new(node)))
            }
            Token::Word(word) => {
                if word == "null" {
                    self.advance();
                    return Ok(MdField::Null);
                }
                if word == "true" || word == "false" {
                    self.advance();
                    return Ok(MdField::Bool(word == "true"));
                }
                // A type keyword here means a typed value, as in a DIArgList.
                if self.looks_like_a_type() {
                    let ty = self.parse_type()?;
                    let constant = self.parse_constant(ty)?;
                    return Ok(MdField::Value {
                        ty,
                        value: llvm_ir::Value::Constant(constant),
                    });
                }
                let mut words = vec![word];
                self.advance();
                while self.eat(&Token::Pipe) {
                    words.push(self.require_word()?);
                }
                Ok(MdField::Words(words))
            }
            Token::IntType(_) => {
                let ty = self.parse_type()?;
                let constant = self.parse_constant(ty)?;
                Ok(MdField::Value {
                    ty,
                    value: llvm_ir::Value::Constant(constant),
                })
            }
            other => self.error(format!(
                "expected a metadata field, found {}",
                other.describe()
            )),
        }
    }

    fn looks_like_a_type(&self) -> bool {
        matches!(self.peek(), Token::IntType(_))
            || matches!(self.peek(), Token::Word(word) if matches!(
                word.as_str(),
                "void" | "half" | "bfloat" | "float" | "double" | "fp128" | "x86_fp80"
                    | "ppc_fp128" | "ptr" | "label" | "metadata" | "token"
            ))
    }

    pub(crate) fn parse_metadata_operand(
        &mut self,
        context: ValueContext<'_>,
    ) -> Result<MdOperand, ParseError> {
        match self.peek().clone() {
            Token::Word(word) if word == "null" => {
                self.advance();
                Ok(MdOperand::Null)
            }
            Token::MetadataNumber(number) => {
                self.advance();
                Ok(MdOperand::Ref(MdId(number)))
            }
            Token::MetadataString(text) => {
                self.advance();
                Ok(MdOperand::String(text))
            }
            Token::MetadataName(_) | Token::Exclaim => {
                let node = self.parse_metadata_definition()?;
                Ok(MdOperand::Inline(Box::new(node)))
            }
            _ => {
                let ty = self.parse_type()?;
                match context {
                    Some((function, state)) => {
                        let value = self.parse_value(function, state, ty)?;
                        Ok(MdOperand::Value { ty, value })
                    }
                    None => {
                        let constant = self.parse_constant(ty)?;
                        Ok(MdOperand::Value {
                            ty,
                            value: llvm_ir::Value::Constant(constant),
                        })
                    }
                }
            }
        }
    }

    /// `!kind !7` attachments written one after another, which is how they
    /// appear on functions.
    pub(crate) fn parse_metadata_attachments(&mut self) -> Result<Vec<MdAttachment>, ParseError> {
        let mut attachments = Vec::new();
        while let Token::MetadataName(kind) = self.peek().clone() {
            let Token::MetadataNumber(number) = self.peek_at(1).clone() else {
                break;
            };
            self.advance();
            self.advance();
            attachments.push(MdAttachment {
                kind,
                node: MdId(number),
            });
        }
        Ok(attachments)
    }

    /// `, !dbg !7` attachments, which is how they appear on instructions and
    /// globals.
    pub(crate) fn parse_metadata_attachments_after_comma(
        &mut self,
    ) -> Result<Vec<MdAttachment>, ParseError> {
        let mut attachments = Vec::new();
        loop {
            if self.peek() != &Token::Comma {
                break;
            }
            let Token::MetadataName(kind) = self.peek_at(1).clone() else {
                break;
            };
            let Token::MetadataNumber(number) = self.peek_at(2).clone() else {
                break;
            };
            self.advance();
            self.advance();
            self.advance();
            attachments.push(MdAttachment {
                kind,
                node: MdId(number),
            });
        }
        Ok(attachments)
    }
}
