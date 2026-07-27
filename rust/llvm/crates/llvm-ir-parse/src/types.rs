//! Type parsing.

use crate::lexer::Token;
use crate::{ParseError, Parser};
use llvm_ir::TypeId;
use llvm_ir::types::TypeKind;
use llvm_support::FloatSemantics;

impl Parser {
    // ----------------------------------------------------------------- types

    pub(crate) fn parse_type(&mut self) -> Result<TypeId, ParseError> {
        let base = self.parse_type_atom()?;
        self.parse_type_suffix(base)
    }

    /// A function type is written as a result type followed by a parameter
    /// list, so any type can grow one.
    pub(crate) fn parse_type_suffix(&mut self, base: TypeId) -> Result<TypeId, ParseError> {
        let mut current = base;
        loop {
            if self.peek() == &Token::Star {
                return self
                    .error("typed pointers are not accepted; this dialect uses opaque 'ptr' only");
            }
            if self.peek() != &Token::LeftParen {
                return Ok(current);
            }
            self.advance();
            let mut params = Vec::new();
            let mut is_var_arg = false;
            while !self.eat(&Token::RightParen) {
                if !params.is_empty() || is_var_arg {
                    self.require(Token::Comma)?;
                }
                if self.eat(&Token::Ellipsis) {
                    is_var_arg = true;
                    continue;
                }
                params.push(self.parse_type()?);
            }
            current = self.module.ctx.function_type(current, params, is_var_arg);
        }
    }

    pub(crate) fn parse_type_atom(&mut self) -> Result<TypeId, ParseError> {
        match self.advance() {
            Token::IntType(bits) => Ok(self.module.ctx.int_type(bits)),
            Token::Word(word) => match word.as_str() {
                "void" => Ok(self.module.ctx.void_type()),
                "label" => Ok(self.module.ctx.label_type()),
                "metadata" => Ok(self.module.ctx.metadata_type()),
                "token" => Ok(self.module.ctx.token_type()),
                "x86_amx" => Ok(self.module.ctx.intern_type(TypeKind::X86Amx)),
                "half" => Ok(self.module.ctx.float_type(FloatSemantics::Half)),
                "bfloat" => Ok(self.module.ctx.float_type(FloatSemantics::BFloat)),
                "float" => Ok(self.module.ctx.float_type(FloatSemantics::Single)),
                "double" => Ok(self.module.ctx.float_type(FloatSemantics::Double)),
                "fp128" => Ok(self.module.ctx.float_type(FloatSemantics::Quad)),
                "x86_fp80" => Ok(self
                    .module
                    .ctx
                    .float_type(FloatSemantics::X87DoubleExtended)),
                "ppc_fp128" => Ok(self.module.ctx.float_type(FloatSemantics::PpcDoubleDouble)),
                "ptr" => {
                    let address_space = self.parse_optional_address_space()?;
                    Ok(self.module.ctx.pointer_type(address_space.unwrap_or(0)))
                }
                "opaque" => self.error("'opaque' is only a type definition body"),
                "target" => self.parse_target_type(),
                other => self.error(format!("unknown type '{other}'")),
            },
            Token::LeftBrace => {
                self.index -= 1;
                let (fields, packed) = self.parse_struct_body()?;
                Ok(self.module.ctx.struct_type(fields, packed))
            }
            Token::Less => {
                if self.peek() == &Token::LeftBrace {
                    self.index -= 1;
                    let (fields, packed) = self.parse_struct_body()?;
                    return Ok(self.module.ctx.struct_type(fields, packed));
                }
                let scalable = self.eat_word("vscale");
                if scalable && !self.eat_word("x") {
                    return self.error("expected 'x' after 'vscale'");
                }
                let count = self.require_unsigned()?;
                if !self.eat_word("x") {
                    return self.error("expected 'x' in a vector type");
                }
                let element = self.parse_type()?;
                self.require(Token::Greater)?;
                if !self.is_valid_vector_element(element) {
                    return self.error("invalid vector element type");
                }
                Ok(self.module.ctx.vector_type(element, count, scalable))
            }
            Token::LeftBracket => {
                let count = self.require_unsigned()?;
                if !self.eat_word("x") {
                    return self.error("expected 'x' in an array type");
                }
                let element = self.parse_type()?;
                self.require(Token::RightBracket)?;
                if !self.is_valid_aggregate_element(element) {
                    return self.error("invalid array element type");
                }
                Ok(self.module.ctx.array_type(element, count))
            }
            Token::LocalName(name) => {
                if let Some(alias) = self.module.ctx.lookup_type_alias(&name) {
                    return Ok(alias);
                }
                let id = self.module.ctx.named_struct(&name);
                Ok(self.module.ctx.named_struct_type(id))
            }
            // `%0` in type position is a struct named by number.
            Token::LocalNumber(number) => {
                let name = number.to_string();
                if let Some(alias) = self.module.ctx.lookup_type_alias(&name) {
                    return Ok(alias);
                }
                let id = self.module.ctx.named_struct(&name);
                self.module.ctx.set_struct_numbered(id);
                Ok(self.module.ctx.named_struct_type(id))
            }
            other => {
                self.index -= 1;
                self.error(format!("expected a type, found {}", other.describe()))
            }
        }
    }

    fn parse_target_type(&mut self) -> Result<TypeId, ParseError> {
        self.require(Token::LeftParen)?;
        let name = self.require_quoted()?;
        let mut types = Vec::new();
        let mut ints = Vec::new();
        // The type parameters come first and the integer ones after, so a
        // type following an integer is a parameter list in the wrong order
        // rather than another type.
        while self.eat(&Token::Comma) {
            if matches!(self.peek(), Token::Integer { .. }) {
                ints.push(self.require_unsigned()? as u32);
            } else if ints.is_empty() {
                types.push(self.parse_type()?);
            } else {
                return self.error("a target extension type writes its types before its integers");
            }
        }
        self.require(Token::RightParen)?;
        Ok(self
            .module
            .ctx
            .intern_type(TypeKind::Target { name, types, ints }))
    }

    pub(crate) fn parse_optional_address_space(&mut self) -> Result<Option<u32>, ParseError> {
        if self.peek_word() != Some("addrspace") {
            return Ok(None);
        }
        self.advance();
        self.require(Token::LeftParen)?;
        let space = self.require_unsigned()? as u32;
        self.require(Token::RightParen)?;
        Ok(Some(space))
    }

    /// What an array or a struct may hold: anything with a size. A `token`
    /// has no representation, `x86_amx` may not be nested, and the rest have
    /// no size at all.
    pub(crate) fn is_valid_aggregate_element(&self, ty: TypeId) -> bool {
        !matches!(
            self.module.ctx.type_kind(ty),
            TypeKind::Void
                | TypeKind::Label
                | TypeKind::Metadata
                | TypeKind::Token
                | TypeKind::X86Amx
                | TypeKind::Function { .. }
        )
    }

    /// What a struct may hold, which is everything an array may hold and
    /// `x86_amx` besides. An intrinsic returning two tiles returns them in a
    /// struct, and upstream reads `{ x86_amx, x86_amx }` while refusing
    /// `[2 x x86_amx]`.
    pub(crate) fn is_valid_struct_field(&self, ty: TypeId) -> bool {
        self.is_valid_aggregate_element(ty)
            || matches!(self.module.ctx.type_kind(ty), TypeKind::X86Amx)
    }

    /// What a vector may hold, which is narrower still: only the types a
    /// lane can be.
    pub(crate) fn is_valid_vector_element(&self, ty: TypeId) -> bool {
        matches!(
            self.module.ctx.type_kind(ty),
            // A target extension type is opaque to us and may still be a
            // vector element: upstream's own target-ext-vector.ll builds
            // `<2 x target(...)>` and llvm-as reads it.
            TypeKind::Integer(_)
                | TypeKind::Float(_)
                | TypeKind::Pointer { .. }
                | TypeKind::Target { .. }
        )
    }
}
