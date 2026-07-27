//! Constant parsing, and the symbol pre-scan that makes forward references to
//! globals work.
//!
//! A function body routinely calls a function declared at the bottom of the
//! file, so `@name` has to resolve before the definition is read. Rather than
//! building placeholder globals and patching them later, the parser walks the
//! token stream once up front and works out which id every global-scope name
//! will get. The ids match because both passes assign them in file order.

use std::collections::HashMap;

use crate::lexer::Token;
use crate::{ParseError, Parser};
use llvm_ir::TypeId;
use llvm_ir::attribute::AsmDialect;
use llvm_ir::constant::{CastOp, ConstExpr, ConstId, Constant, GepFlags, InlineAsm};
use llvm_ir::types::TypeKind;
use llvm_ir::value::{AliasId, FunctionId, GlobalRef, GlobalVarId, IFuncId, Name};
use llvm_support::{ApFloat, FloatSemantics};

impl Parser {
    /// Works out the id of every global-scope symbol before parsing anything.
    pub(crate) fn prescan_symbols(&mut self) -> Result<(), ParseError> {
        let mut symbols: HashMap<Name, GlobalRef> = HashMap::new();
        let mut globals = 0u32;
        let mut aliases = 0u32;
        let mut ifuncs = 0u32;
        let mut functions = 0u32;

        for index in 0..self.tokens.len() {
            match &self.tokens[index].token {
                token @ (Token::GlobalName(_) | Token::GlobalNumber(_))
                    if matches!(
                        self.tokens.get(index + 1).map(|t| &t.token),
                        Some(Token::Equals)
                    ) =>
                {
                    let Some(name) = global_name(token) else {
                        continue;
                    };
                    let mut kind = "global";
                    for ahead in index + 2..(index + 24).min(self.tokens.len()) {
                        if let Token::Word(word) = &self.tokens[ahead].token
                            && matches!(word.as_str(), "alias" | "ifunc" | "global" | "constant")
                        {
                            kind = if word == "alias" {
                                "alias"
                            } else if word == "ifunc" {
                                "ifunc"
                            } else {
                                "global"
                            };
                            break;
                        }
                    }
                    let id = match kind {
                        "alias" => {
                            aliases += 1;
                            GlobalRef::Alias(AliasId(aliases - 1))
                        }
                        "ifunc" => {
                            ifuncs += 1;
                            GlobalRef::IFunc(IFuncId(ifuncs - 1))
                        }
                        _ => {
                            globals += 1;
                            GlobalRef::Variable(GlobalVarId(globals - 1))
                        }
                    };
                    if symbols.contains_key(&name) {
                        return Err(ParseError {
                            position: self.tokens[index].position,
                            message: format!("redefinition of global '@{}'", describe(&name)),
                        });
                    }
                    symbols.insert(name, id);
                }
                Token::Word(word) if word == "define" || word == "declare" => {
                    for ahead in index + 1..self.tokens.len() {
                        let Some(name) = global_name(&self.tokens[ahead].token) else {
                            continue;
                        };
                        if symbols.contains_key(&name) {
                            return Err(ParseError {
                                position: self.tokens[ahead].position,
                                message: format!("redefinition of global '@{}'", describe(&name)),
                            });
                        }
                        symbols.insert(name, GlobalRef::Function(FunctionId(functions)));
                        functions += 1;
                        break;
                    }
                }
                _ => {}
            }
        }

        self.symbols = symbols;
        Ok(())
    }

    /// The id a global-scope name was given by the pre-scan.
    pub(crate) fn global_ref(&mut self, name: &Name) -> Result<GlobalRef, ParseError> {
        match self.symbols.get(name) {
            Some(id) => Ok(*id),
            None => {
                let text = match name {
                    Name::Named(text) => text.clone(),
                    Name::Number(number) => number.to_string(),
                };
                self.error(format!("reference to undefined symbol @{text}"))
            }
        }
    }

    /// A reference to a global as a constant of the given pointer type.
    pub(crate) fn global_constant(
        &mut self,
        name: &Name,
        ty: TypeId,
    ) -> Result<ConstId, ParseError> {
        let target = self.global_ref(name)?;
        Ok(self
            .module
            .ctx
            .intern_constant(Constant::Global { ty, target }))
    }

    pub(crate) fn parse_typed_constant(&mut self) -> Result<(TypeId, ConstId), ParseError> {
        let ty = self.parse_type()?;
        let constant = self.parse_constant(ty)?;
        Ok((ty, constant))
    }

    /// A constant of a known type.
    pub(crate) fn parse_constant(&mut self, ty: TypeId) -> Result<ConstId, ParseError> {
        if let Some(shared) = self.parse_shared_constant(ty)? {
            return Ok(shared);
        }
        // A constant expression can produce any type, not just a pointer:
        // `i64 ptrtoint (ptr @g to i64)` is an integer constant.
        if let Some(word) = self.peek_word()
            && (matches!(
                word,
                "getelementptr"
                    | "extractelement"
                    | "insertelement"
                    | "shufflevector"
                    | "add"
                    | "sub"
                    | "xor"
            ) || CastOp::from_keyword(word).is_some())
        {
            return self.parse_constant_expression(ty);
        }
        match self.module.ctx.type_kind(ty).clone() {
            TypeKind::Integer(bits) => {
                let value = self.parse_ap_int(bits)?;
                Ok(self.module.ctx.const_int(ty, value))
            }
            TypeKind::Float(semantics) => {
                let value = self.parse_float(semantics)?;
                Ok(self
                    .module
                    .ctx
                    .intern_constant(Constant::Float { ty, value }))
            }
            TypeKind::Pointer { .. } => self.parse_pointer_constant(ty),
            TypeKind::Array { element, .. } => match self.peek().clone() {
                Token::ByteString(bytes) => {
                    self.advance();
                    Ok(self
                        .module
                        .ctx
                        .intern_constant(Constant::String { ty, bytes }))
                }
                _ => {
                    self.require(Token::LeftBracket)?;
                    let elements = self.parse_constant_list(Token::RightBracket, element)?;
                    Ok(self
                        .module
                        .ctx
                        .intern_constant(Constant::Array { ty, elements }))
                }
            },
            TypeKind::Vector { element, .. } => {
                self.require(Token::Less)?;
                let elements = self.parse_constant_list(Token::Greater, element)?;
                Ok(self
                    .module
                    .ctx
                    .intern_constant(Constant::Vector { ty, elements }))
            }
            TypeKind::Struct { fields, packed } => self.parse_struct_constant(ty, &fields, packed),
            TypeKind::NamedStruct(id) => {
                let def = self.module.ctx.struct_def(id);
                let packed = def.packed;
                let Some(fields) = def.fields.clone() else {
                    return self.error("an opaque struct has no constants");
                };
                self.parse_struct_constant(ty, &fields, packed)
            }
            TypeKind::Token => {
                if self.eat_word("none") {
                    Ok(self.module.ctx.intern_constant(Constant::NoneToken(ty)))
                } else {
                    self.error("the only token constant is 'none'")
                }
            }
            // `metadata ptr %s` and `metadata !DIExpression()` are both
            // legal where a value of type metadata is wanted, so this is a
            // whole metadata operand rather than only a reference.
            TypeKind::Metadata => {
                let operand = self.parse_metadata_operand(None)?;
                Ok(self.module.ctx.intern_constant(Constant::Metadata {
                    ty,
                    operand: Box::new(operand),
                }))
            }
            other => self.error(format!("{other:?} has no constants")),
        }
    }

    /// The constants any type can have.
    fn parse_shared_constant(&mut self, ty: TypeId) -> Result<Option<ConstId>, ParseError> {
        let Some(word) = self.peek_word() else {
            return Ok(None);
        };
        if word == "splat" {
            self.advance();
            self.require(Token::LeftParen)?;
            let (_, element) = self.parse_typed_constant()?;
            self.require(Token::RightParen)?;
            return Ok(Some(
                self.module
                    .ctx
                    .intern_constant(Constant::Splat { ty, element }),
            ));
        }
        if word == "ptrauth" {
            self.advance();
            self.require(Token::LeftParen)?;
            let (_, pointer) = self.parse_typed_constant()?;
            self.require(Token::Comma)?;
            let (_, key) = self.parse_typed_constant()?;
            let discriminator = if self.eat(&Token::Comma) {
                Some(self.parse_typed_constant()?.1)
            } else {
                None
            };
            let address_discriminator = if self.eat(&Token::Comma) {
                Some(self.parse_typed_constant()?.1)
            } else {
                None
            };
            self.require(Token::RightParen)?;
            // Each operand of a signed pointer says something specific, and
            // upstream checks all four where it reads them.
            let is_pointer = |parser: &Self, id: ConstId| {
                matches!(
                    parser
                        .module
                        .ctx
                        .type_kind(parser.module.ctx.constant(id).ty()),
                    llvm_ir::TypeKind::Pointer { .. }
                )
            };
            let is_int = |parser: &Self, id: ConstId, bits: u32| {
                matches!(parser.module.ctx.constant(id), Constant::Integer { ty, .. }
                    if matches!(parser.module.ctx.type_kind(*ty), llvm_ir::TypeKind::Integer(width) if *width == bits))
            };
            if !is_pointer(self, pointer) {
                return self.error("a ptrauth base pointer has to be a pointer");
            }
            if !is_int(self, key, 32) {
                return self.error("a ptrauth key has to be an i32 constant");
            }
            if let Some(id) = discriminator
                && !is_int(self, id, 64)
            {
                return self.error("a ptrauth integer discriminator has to be an i64 constant");
            }
            if let Some(id) = address_discriminator
                && !is_pointer(self, id)
            {
                return self.error("a ptrauth address discriminator has to be a pointer");
            }
            return Ok(Some(self.module.ctx.intern_constant(Constant::PtrAuth {
                ty,
                pointer,
                key,
                discriminator,
                address_discriminator,
            })));
        }
        let constant = match word {
            "zeroinitializer" => Constant::ZeroInitializer(ty),
            "undef" => Constant::Undef(ty),
            "poison" => Constant::Poison(ty),
            _ => return Ok(None),
        };
        self.advance();
        Ok(Some(self.module.ctx.intern_constant(constant)))
    }

    fn parse_struct_constant(
        &mut self,
        ty: TypeId,
        fields: &[TypeId],
        packed: bool,
    ) -> Result<ConstId, ParseError> {
        if packed {
            self.require(Token::Less)?;
        }
        self.require(Token::LeftBrace)?;
        let mut values = Vec::new();
        for (index, field) in fields.iter().enumerate() {
            if index > 0 {
                self.require(Token::Comma)?;
            }
            let written = self.parse_type()?;
            if written != *field {
                return self.error("struct constant field has the wrong type");
            }
            values.push(self.parse_constant(*field)?);
        }
        self.require(Token::RightBrace)?;
        if packed {
            self.require(Token::Greater)?;
        }
        Ok(self
            .module
            .ctx
            .intern_constant(Constant::Struct { ty, fields: values }))
    }

    fn parse_constant_list(
        &mut self,
        closer: Token,
        element: TypeId,
    ) -> Result<Vec<ConstId>, ParseError> {
        let mut elements = Vec::new();
        while !self.eat(&closer) {
            if !elements.is_empty() {
                self.require(Token::Comma)?;
            }
            let written = self.parse_type()?;
            if written != element {
                return self.error("aggregate element has the wrong type");
            }
            elements.push(self.parse_constant(element)?);
        }
        Ok(elements)
    }

    fn parse_pointer_constant(&mut self, ty: TypeId) -> Result<ConstId, ParseError> {
        match self.peek().clone() {
            Token::Word(word) => match word.as_str() {
                "null" => {
                    self.advance();
                    Ok(self.module.ctx.const_null(ty))
                }
                "blockaddress" => {
                    self.advance();
                    self.require(Token::LeftParen)?;
                    let function = match self.advance() {
                        Token::GlobalName(name) => Name::Named(name),
                        Token::GlobalNumber(number) => Name::Number(number),
                        other => {
                            self.index -= 1;
                            return self
                                .error(format!("expected a function, found {}", other.describe()));
                        }
                    };
                    let function = self.global_ref(&function)?;
                    self.require(Token::Comma)?;
                    let block = match self.advance() {
                        Token::LocalName(name) => Name::Named(name),
                        Token::LocalNumber(number) => Name::Number(number),
                        other => {
                            self.index -= 1;
                            return self
                                .error(format!("expected a block, found {}", other.describe()));
                        }
                    };
                    self.require(Token::RightParen)?;
                    Ok(self.module.ctx.intern_constant(Constant::BlockAddress {
                        ty,
                        function,
                        block,
                    }))
                }
                "dso_local_equivalent" | "no_cfi" => {
                    self.advance();
                    let target = match self.advance() {
                        Token::GlobalName(name) => Name::Named(name),
                        Token::GlobalNumber(number) => Name::Number(number),
                        other => {
                            self.index -= 1;
                            return self
                                .error(format!("expected a symbol, found {}", other.describe()));
                        }
                    };
                    let target = self.global_ref(&target)?;
                    let constant = if word == "no_cfi" {
                        Constant::NoCfiValue { ty, target }
                    } else {
                        Constant::DsoLocalEquivalent { ty, target }
                    };
                    Ok(self.module.ctx.intern_constant(constant))
                }
                "asm" => {
                    self.advance();
                    let asm = self.parse_inline_asm(ty)?;
                    Ok(self
                        .module
                        .ctx
                        .intern_constant(Constant::InlineAsm(Box::new(asm))))
                }
                _ => self.parse_constant_expression(ty),
            },
            Token::GlobalName(name) => {
                self.advance();
                self.global_constant(&Name::Named(name), ty)
            }
            Token::GlobalNumber(number) => {
                self.advance();
                self.global_constant(&Name::Number(number), ty)
            }
            other => self.error(format!(
                "expected a pointer constant, found {}",
                other.describe()
            )),
        }
    }

    pub(crate) fn parse_inline_asm(
        &mut self,
        function_type: TypeId,
    ) -> Result<InlineAsm, ParseError> {
        let mut asm = InlineAsm {
            function_type,
            assembly: String::new(),
            constraints: String::new(),
            has_side_effects: false,
            align_stack: false,
            dialect: AsmDialect::Att,
            can_unwind: false,
        };
        loop {
            if self.eat_word("sideeffect") {
                asm.has_side_effects = true;
            } else if self.eat_word("alignstack") {
                asm.align_stack = true;
            } else if self.eat_word("inteldialect") {
                asm.dialect = AsmDialect::Intel;
            } else if self.eat_word("unwind") {
                asm.can_unwind = true;
            } else {
                break;
            }
        }
        asm.assembly = self.require_quoted()?;
        self.require(Token::Comma)?;
        asm.constraints = self.require_quoted()?;
        Ok(asm)
    }

    /// The constant expressions LLVM 21 still accepts. Anything else, notably
    /// the arithmetic ones upstream removed, is an error naming the opcode.
    pub(crate) fn parse_constant_expression(&mut self, ty: TypeId) -> Result<ConstId, ParseError> {
        self.parse_constant_expression_of(Some(ty))
    }

    /// The same, where the type is not written in front and the expression
    /// has to say what it produces. An alias writes its aliasee that way:
    /// `@a = alias i32, getelementptr inbounds (i32, ptr @b, i64 1)`.
    pub(crate) fn parse_untyped_constant_expression(&mut self) -> Result<ConstId, ParseError> {
        self.parse_constant_expression_of(None)
    }

    fn parse_constant_expression_of(
        &mut self,
        expected: Option<TypeId>,
    ) -> Result<ConstId, ParseError> {
        let word = self.require_word()?;
        let expr = match word.as_str() {
            "getelementptr" => {
                let mut flags = GepFlags::default();
                loop {
                    if self.eat_word("inbounds") {
                        flags.inbounds = true;
                    } else if self.eat_word("nusw") {
                        flags.nusw = true;
                    } else if self.eat_word("nuw") {
                        flags.nuw = true;
                    } else {
                        break;
                    }
                }
                let inrange = if self.eat_word("inrange") {
                    self.require(Token::LeftParen)?;
                    let low = self.require_signed()?;
                    self.require(Token::Comma)?;
                    let high = self.require_signed()?;
                    self.require(Token::RightParen)?;
                    Some((low, high))
                } else {
                    None
                };
                self.require(Token::LeftParen)?;
                let source_type = self.parse_type()?;
                self.require(Token::Comma)?;
                let (base_type, base) = self.parse_typed_constant()?;
                let mut indices = Vec::new();
                while self.eat(&Token::Comma) {
                    let (_, index) = self.parse_typed_constant()?;
                    indices.push(index);
                }
                self.require(Token::RightParen)?;
                ConstExpr::GetElementPtr {
                    source_type,
                    base,
                    indices,
                    flags,
                    inrange,
                    ty: expected.unwrap_or(base_type),
                }
            }
            // Three of the arithmetic expressions survived the removals; the
            // rest are errors naming the opcode, and `select` says so itself.
            "add" | "sub" | "xor" => {
                let op = llvm_ir::instruction::BinOp::from_keyword(&word)
                    .expect("the match arm listed these");
                let mut flags = llvm_ir::instruction::IntFlags::default();
                let mut discard = llvm_ir::instruction::FastMathFlags::default();
                self.parse_operation_flags(&mut flags, &mut discard);
                self.require(Token::LeftParen)?;
                let (lhs_type, lhs) = self.parse_typed_constant()?;
                self.require(Token::Comma)?;
                let (_, rhs) = self.parse_typed_constant()?;
                self.require(Token::RightParen)?;
                ConstExpr::Binary {
                    op,
                    flags,
                    lhs,
                    rhs,
                    ty: expected.unwrap_or(lhs_type),
                }
            }
            "extractelement" => {
                self.require(Token::LeftParen)?;
                let (vector_type, vector) = self.parse_typed_constant()?;
                self.require(Token::Comma)?;
                let (_, index) = self.parse_typed_constant()?;
                self.require(Token::RightParen)?;
                let ty = match expected {
                    Some(ty) => ty,
                    None => match self.module.ctx.type_kind(vector_type).as_vector() {
                        Some((element, _, _)) => element,
                        None => return self.error("extractelement needs a vector"),
                    },
                };
                ConstExpr::ExtractElement { vector, index, ty }
            }
            "insertelement" => {
                self.require(Token::LeftParen)?;
                let (vector_type, vector) = self.parse_typed_constant()?;
                self.require(Token::Comma)?;
                let (_, element) = self.parse_typed_constant()?;
                self.require(Token::Comma)?;
                let (_, index) = self.parse_typed_constant()?;
                self.require(Token::RightParen)?;
                ConstExpr::InsertElement {
                    vector,
                    element,
                    index,
                    ty: expected.unwrap_or(vector_type),
                }
            }
            "shufflevector" => {
                self.require(Token::LeftParen)?;
                let (first_type, first) = self.parse_typed_constant()?;
                self.require(Token::Comma)?;
                let (_, second) = self.parse_typed_constant()?;
                self.require(Token::Comma)?;
                let (_, mask) = self.parse_typed_constant()?;
                self.require(Token::RightParen)?;
                ConstExpr::ShuffleVector {
                    first,
                    second,
                    mask,
                    ty: expected.unwrap_or(first_type),
                }
            }
            other => match CastOp::from_keyword(other) {
                Some(op) => {
                    self.require(Token::LeftParen)?;
                    let (_, operand) = self.parse_typed_constant()?;
                    if !self.eat_word("to") {
                        return self.error("expected 'to' in a cast expression");
                    }
                    let target = self.parse_type()?;
                    self.require(Token::RightParen)?;
                    if let Some(ty) = expected
                        && target != ty
                    {
                        return self.error("cast expression does not produce the expected type");
                    }
                    ConstExpr::Cast {
                        op,
                        operand,
                        ty: target,
                    }
                }
                None => {
                    return self.error(format!(
                        "'{other}' is not a constant expression in this dialect"
                    ));
                }
            },
        };
        Ok(self
            .module
            .ctx
            .intern_constant(Constant::Expression(Box::new(expr))))
    }

    pub(crate) fn parse_float(&mut self, semantics: FloatSemantics) -> Result<ApFloat, ParseError> {
        match self.advance() {
            Token::HexFloat { form, digits } => {
                ApFloat::parse_hex_literal(form, &digits, semantics).map_or_else(
                    |error| {
                        self.index -= 1;
                        self.error(error.to_string())
                    },
                    Ok,
                )
            }
            Token::Float(text) => ApFloat::parse_decimal(&text, semantics).map_or_else(
                |error| {
                    self.index -= 1;
                    self.error(error.to_string())
                },
                Ok,
            ),
            Token::Integer { negative, digits } => {
                // `float 1` is not legal upstream, but `double 0` shows up in
                // hand-written tests often enough to say so precisely.
                let sign = if negative { "-" } else { "" };
                self.index -= 1;
                self.error(format!(
                    "floating-point constants need a decimal point or a hexadecimal form; \
                     write {sign}{digits}.0"
                ))
            }
            other => {
                self.index -= 1;
                self.error(format!(
                    "expected a floating-point constant, found {}",
                    other.describe()
                ))
            }
        }
    }
}

/// The name a `@`-sigil token carries, if it carries one.
fn global_name(token: &Token) -> Option<Name> {
    match token {
        Token::GlobalName(name) => Some(Name::Named(name.clone())),
        Token::GlobalNumber(number) => Some(Name::Number(*number)),
        _ => None,
    }
}

fn describe(name: &Name) -> String {
    match name {
        Name::Named(text) => text.clone(),
        Name::Number(number) => number.to_string(),
    }
}
