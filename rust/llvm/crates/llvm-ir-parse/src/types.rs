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
    /// The `*` half of the suffix and nothing else.
    ///
    /// A return type is read without the parenthesised half, `void (i32)`
    /// being a signature rather than a function type there, but `i8*` is
    /// still the older spelling of `ptr` and folds the same way.
    /// Whether the parenthesised group starting here is followed by a `*`,
    /// which is what makes it a function type rather than a parameter list.
    fn star_follows_group(&self) -> bool {
        let mut depth = 0i32;
        let mut ahead = 0;
        loop {
            match self.peek_at(ahead) {
                Token::LeftParen => depth += 1,
                Token::RightParen => {
                    depth -= 1;
                    if depth == 0 {
                        return self.peek_at(ahead + 1) == &Token::Star;
                    }
                }
                Token::Eof => return false,
                _ => {}
            }
            ahead += 1;
        }
    }

    pub(crate) fn parse_pointer_suffix(&mut self, base: TypeId) -> Result<TypeId, ParseError> {
        let mut current = base;
        loop {
            // `define void ()* @f()` returns a pointer to a function. The
            // parenthesised half is a parameter list here unless a `*`
            // follows it, which is the only thing that tells the two apart,
            // so the group is measured before it is read.
            if self.peek() == &Token::LeftParen && self.star_follows_group() {
                current = self.parse_type_suffix(current)?;
                continue;
            }
            let space = if self.peek_word() == Some("addrspace") && self.peek_at(4) == &Token::Star
            {
                self.parse_optional_address_space()?
            } else {
                None
            };
            if self.peek() == &Token::Star {
                // The older spelling is read; writing it around the newer one
                // is not. `ptr*` is a module that has both dialects in it at
                // once, and upstream says which to use.
                if matches!(self.module.ctx.type_kind(current), TypeKind::Pointer { .. })
                    && current == base
                {
                    return self.error("ptr* is invalid - use ptr instead");
                }
                self.advance();
                current = self.module.ctx.pointer_type(space.unwrap_or(0));
                continue;
            }
            if space.is_some() {
                return self.error("an address space here belongs to a pointer");
            }
            return Ok(current);
        }
    }

    pub(crate) fn parse_type_suffix(&mut self, base: TypeId) -> Result<TypeId, ParseError> {
        let mut current = base;
        loop {
            // `i8*` and `i8 addrspace(3)*` are the older spellings of `ptr`
            // and `ptr addrspace(3)`. Upstream folds them as it reads, the
            // pointee having no meaning since opaque pointers, so this reads
            // them the same way and the model never holds a typed pointer.
            // That is a spelling this accepts, not a dialect it supports:
            // nothing downstream can tell `i8*` from `ptr`, which is exactly
            // what upstream arranges too.
            let space = if self.peek_word() == Some("addrspace") && self.peek_at(4) == &Token::Star
            {
                self.parse_optional_address_space()?
            } else {
                None
            };
            if self.peek() == &Token::Star {
                // The older spelling is read; writing it around the newer one
                // is not. `ptr*` has both dialects in it at once, and
                // upstream says which to use.
                if matches!(self.module.ctx.type_kind(current), TypeKind::Pointer { .. })
                    && current == base
                {
                    return self.error("ptr* is invalid - use ptr instead");
                }
                self.advance();
                current = self.module.ctx.pointer_type(space.unwrap_or(0));
                continue;
            }
            if let Some(space) = space {
                let _ = space;
                return self.error("an address space here belongs to a pointer");
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
                // A lane count is held in thirty-two bits, scalable or not,
                // so `<4294967296 x i8>` names a vector upstream has no way
                // to write down.
                if count > u64::from(u32::MAX) {
                    return self.error("size too large for vector");
                }
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
        // A registered name may insist on a shape, and upstream says so where
        // it is written rather than leaving it to the verifier:
        // `target("aarch64.svcount", i32)` is "should have no parameters".
        // `corpus/target-extension-types.nu` measures which names have one,
        // and it is three of the forty its tests spell; everything else takes
        // whatever it is given.
        if let Some((wanted_types, wanted_ints)) =
            llvm_ir::target_extension::properties(&name).params
            && (types.len() != usize::from(wanted_types) || ints.len() != usize::from(wanted_ints))
        {
            return self.error(format!(
                "target extension type {name} should have {}",
                describe_parameters(wanted_types, wanted_ints)
            ));
        }
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
        // A target extension type may be a vector element only where the
        // target registered it as one, which is a property of its own rather
        // than something the other four imply: `spirv.Image` is sized and
        // allocatable and still not a vector element, while
        // `llvm.test.vectorelement` is. Both spellings appear in upstream's
        // own tests, one in `target-ext-vector.ll` and one in
        // `target-ext-vector-invalid.ll`.
        if let TypeKind::Target { name, .. } = self.module.ctx.type_kind(ty) {
            return llvm_ir::target_extension::properties(name.as_str()).vector;
        }
        matches!(
            self.module.ctx.type_kind(ty),
            TypeKind::Integer(_) | TypeKind::Float(_) | TypeKind::Pointer { .. }
        )
    }
}

/// How upstream words the shape a target extension type insists on, which is
/// three phrasings rather than a count: "no parameters", "no type parameters
/// and one integer parameter", "one type parameter and one integer
/// parameter".
fn describe_parameters(types: u8, ints: u8) -> String {
    let plural = |count: u8, what: &str| match count {
        0 => format!("no {what}s"),
        1 => format!("one {what}"),
        many => format!("{many} {what}s"),
    };
    if types == 0 && ints == 0 {
        return "no parameters".to_string();
    }
    format!(
        "{} and {}",
        plural(types, "type parameter"),
        plural(ints, "integer parameter")
    )
}
