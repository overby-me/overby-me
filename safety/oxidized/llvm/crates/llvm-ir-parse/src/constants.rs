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
use llvm_ir::instruction::BinOp;
use llvm_ir::types::TypeKind;
use llvm_ir::value::{AliasId, FunctionId, GlobalRef, GlobalVarId, IFuncId, Name};
use llvm_support::{ApFloat, FloatSemantics};

impl Parser {
    /// The name a definition takes, with the slot it consumes.
    ///
    /// An unnamed symbol is rewritten to the slot it took, so that the parse
    /// after this sees one spelling for it and a reference by number finds
    /// it. A named one takes no slot and is left alone.
    fn take_slot(&mut self, index: usize, slot: &mut u32) -> Result<Option<Name>, ParseError> {
        let number = match &self.tokens[index].token {
            Token::GlobalName(text) if text.is_empty() => *slot,
            Token::GlobalName(text) => return Ok(Some(Name::Named(text.clone()))),
            Token::GlobalNumber(number) => {
                // A written number says which slot to start from, so it may
                // skip ahead and may not go back over one already taken.
                if *number < *slot {
                    return Err(ParseError {
                        position: self.tokens[index].position,
                        message: format!("global number @{number} is out of order"),
                    });
                }
                *number
            }
            _ => return Ok(None),
        };
        *slot = number + 1;
        self.tokens[index].token = Token::GlobalNumber(number);
        Ok(Some(Name::Number(number)))
    }

    /// Works out the id of every global-scope symbol before parsing anything.
    pub(crate) fn prescan_symbols(&mut self) -> Result<(), ParseError> {
        let mut symbols: HashMap<Name, GlobalRef> = HashMap::new();
        let mut globals = 0u32;
        let mut aliases = 0u32;
        let mut ifuncs = 0u32;
        let mut functions = 0u32;

        // The slot an unnamed module symbol takes. A written number is a
        // slot rather than a name, and so is an empty quoted name: upstream
        // reads `@""` as unnamed and gives it the next one, which is how
        // `skip-value-numbers-globals.ll` refers to the `@""` after `@5` as
        // `@6`. One counter serves every kind, in the order they are
        // written, and a number only ever skips ahead.
        let mut slot = 0u32;

        for index in 0..self.tokens.len() {
            match &self.tokens[index].token {
                Token::GlobalName(_) | Token::GlobalNumber(_)
                    if matches!(
                        self.tokens.get(index + 1).map(|t| &t.token),
                        Some(Token::Equals)
                    ) =>
                {
                    let Some(name) = self.take_slot(index, &mut slot)? else {
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
                        if global_name(&self.tokens[ahead].token).is_none() {
                            continue;
                        }
                        let Some(name) = self.take_slot(ahead, &mut slot)? else {
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

        // An intrinsic needs no declaration: upstream builds one from the
        // call when it recognises the name, and appends it after everything
        // the module writes. Its id has to be reserved here so that the
        // pre-scan and the parse agree about which function is which.
        let mut implied = Vec::new();
        let mut mentions = 0usize;
        for spanned in &self.tokens {
            let Token::GlobalName(text) = &spanned.token else {
                continue;
            };
            if !text.starts_with("llvm.") {
                continue;
            }
            let name = Name::Named(text.clone());
            if symbols.contains_key(&name) {
                continue;
            }
            if implied.contains(&name) {
                // Counted again: one written name can stand for more than
                // one function, `@llvm.umax` called at `i8` and at `i16`
                // being two of them, and each needs an id of its own.
                if llvm_ir::intrinsic::is_known(text) {
                    mentions += 1;
                }
                continue;
            }
            // Recognising the name is all that is needed to build the
            // declaration from the call, and upstream recognises far more
            // names than LangRef documents: the coroutine and
            // exception-handling intrinsics are documented in other files,
            // `llvm.vector.interleave4` in none, and every target's in the
            // target backend. `is_known` asks the documented tables and the
            // set measured from the modules upstream reads, at the whole
            // name and at the one it instantiates.
            if llvm_ir::intrinsic::is_known(text) {
                implied.push(name);
            }
        }
        // Sorted by the name the module wrote, which is not the order the
        // calls come in and not the order the finished names sort in either.
        // Five intrinsics called in reverse alphabetical order come back
        // alphabetical, so it is a sort; and `@llvm.umax` called at `i8`
        // prints before `@llvm.umax.i32` though `llvm.umax.i8` sorts after
        // it, so the key is what was written. That is the shape of a
        // forward reference held in a map until the module is finished
        // rather than a declaration built where the call was read.
        implied.sort_by(|left, right| match (left, right) {
            (Name::Named(left), Name::Named(right)) => left.cmp(right),
            _ => std::cmp::Ordering::Equal,
        });
        self.first_implied_id = FunctionId(functions);
        for name in implied {
            symbols.insert(name.clone(), GlobalRef::Function(FunctionId(functions)));
            functions += 1;
            self.implied_intrinsics.push(name);
        }
        // And a block after those for the names that stand for more than one
        // function. A call site names an instantiation, so `@llvm.umax` at
        // `i8` and at `i16` are two declarations from one written name, and
        // the second cannot have the first's id.
        //
        // The block is sized by how many times such a name is mentioned
        // beyond its first, which is an upper bound rather than a count: two
        // calls at the same type want one function between them. What is
        // left over is never referred to and never built, and sits past the
        // end of the arena where nothing looks.
        self.extra_implied_ids = FunctionId(functions);
        self.extra_implied_room = mentions;

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

    /// The address space a pointer type names, looking through a vector so
    /// that a cast between vectors of pointers answers the same way a cast
    /// between the pointers themselves does.
    fn address_space(&self, ty: TypeId) -> Option<u32> {
        let inner = match self.module.ctx.type_kind(ty) {
            TypeKind::Vector { element, .. } => *element,
            _ => ty,
        };
        self.module.ctx.type_kind(inner).pointer_address_space()
    }

    /// The indices of a walk that answers with a vector, each written the way
    /// upstream writes it: as a vector, except where it names a struct field.
    fn widen_gep_indices(
        &mut self,
        result: TypeId,
        source_type: TypeId,
        indices: Vec<ConstId>,
    ) -> Vec<ConstId> {
        // The type the next index steps into, or `None` once the walk has
        // lost it, after which the rest are left as they were written.
        let mut current = Some(source_type);
        let mut widened = Vec::with_capacity(indices.len());
        for (position, index) in indices.into_iter().enumerate() {
            // The first index strides over the source type rather than
            // stepping into it, so the walk starts at the second.
            let fields = match (position, current) {
                (0, _) | (_, None) => None,
                (_, Some(ty)) => self.struct_fields_of(ty),
            };
            if let Some(fields) = fields {
                let index = self.narrow_struct_index(index);
                widened.push(index);
                current = self
                    .constant_index(index)
                    .and_then(|field| fields.get(field).copied());
                continue;
            }
            widened.push(self.widen_to(result, index));
            // The first index walks through the pointer to the source type,
            // which is where the second index starts, so it steps into
            // nothing.
            if position == 0 {
                continue;
            }
            current = match current.map(|ty| self.module.ctx.type_kind(ty)) {
                Some(TypeKind::Array { element, .. } | TypeKind::Vector { element, .. }) => {
                    Some(*element)
                }
                _ => None,
            };
        }
        widened
    }

    /// A scalable vector every lane of which holds one value, written the
    /// way upstream writes one: the value put in the first lane and shuffled
    /// across every other.
    fn built_splat(&mut self, ty: TypeId, element: ConstId, count: u64) -> ConstId {
        let poison = self.module.ctx.intern_constant(Constant::Poison(ty));
        let index = self.module.ctx.const_int_of(64, 0);
        let inserted = self
            .module
            .ctx
            .intern_constant(Constant::Expression(Box::new(ConstExpr::InsertElement {
                vector: poison,
                element,
                index,
                ty,
            })));
        let i32_type = self.module.ctx.int_type(32);
        let mask_type = self.module.ctx.vector_type(i32_type, count, true);
        let mask = self
            .module
            .ctx
            .intern_constant(Constant::ZeroInitializer(mask_type));
        self.module
            .ctx
            .intern_constant(Constant::Expression(Box::new(ConstExpr::ShuffleVector {
                first: inserted,
                second: poison,
                mask,
                ty,
            })))
    }

    /// A struct field named lane by lane, as the one field it names. Every
    /// lane picks the same field or the walk has no type at the end of it,
    /// so upstream writes the scalar whatever the module wrote.
    fn narrow_struct_index(&mut self, index: ConstId) -> ConstId {
        let element = match self.module.ctx.constant(index) {
            Constant::Splat { element, .. } => Some(*element),
            Constant::Vector { elements, .. } => match elements.first().copied() {
                Some(first) if elements.iter().all(|lane| *lane == first) => Some(first),
                _ => None,
            },
            Constant::ZeroInitializer(ty) => {
                let TypeKind::Vector { element, .. } = self.module.ctx.type_kind(*ty) else {
                    return index;
                };
                let element = *element;
                let TypeKind::Integer(bits) = *self.module.ctx.type_kind(element) else {
                    return index;
                };
                return self.module.ctx.const_int_of(bits, 0);
            }
            _ => None,
        };
        element.unwrap_or(index)
    }

    /// A struct's fields, when the type is one.
    fn struct_fields_of(&self, ty: TypeId) -> Option<Vec<TypeId>> {
        match self.module.ctx.type_kind(ty) {
            TypeKind::Struct { fields, .. } => Some(fields.clone()),
            TypeKind::NamedStruct(id) => self.module.ctx.struct_def(*id).fields.clone(),
            _ => None,
        }
    }

    /// The number a constant index carries, when it carries one.
    fn constant_index(&self, index: ConstId) -> Option<usize> {
        match self.module.ctx.constant(index) {
            Constant::Integer { value, .. } => usize::try_from(value.to_u64()?).ok(),
            Constant::ZeroInitializer(_) => Some(0),
            _ => None,
        }
    }

    /// A scalar written where the result has lanes, as the vector it stands
    /// for. Anything already a vector, and anything at all when the result is
    /// scalar or the count is not known, is left as it was.
    fn widen_to(&mut self, result: TypeId, value: ConstId) -> ConstId {
        let TypeKind::Vector {
            count,
            scalable: false,
            ..
        } = *self.module.ctx.type_kind(result)
        else {
            return value;
        };
        let element = self.module.ctx.constant(value).ty();
        if matches!(self.module.ctx.type_kind(element), TypeKind::Vector { .. }) {
            return value;
        }
        let Ok(count) = usize::try_from(count) else {
            return value;
        };
        let ty = self.module.ctx.vector_type(element, count as u64, false);
        self.fold_aggregate(ty, &vec![value; count], true)
    }

    /// The shorthand upstream folds an aggregate into when every element
    /// says the same thing.
    ///
    /// All zero is `zeroinitializer`, all undef is `undef` and all poison is
    /// `poison`, for a vector, an array or a struct alike. A vector whose
    /// lanes are all the same and not one of those is `splat (T v)`, which an
    /// array of the same shape is not: upstream folds the splat form for
    /// vectors only.
    fn fold_aggregate(&mut self, ty: TypeId, elements: &[ConstId], is_vector: bool) -> ConstId {
        // An array with no elements is written `[]` and printed back as
        // `poison`, where a struct with no fields is printed back as
        // `zeroinitializer`. Measured, both, and neither is a guess a reader
        // would make.
        if elements.is_empty() {
            return self.module.ctx.intern_constant(if is_vector {
                Constant::Vector {
                    ty,
                    elements: Vec::new(),
                }
            } else {
                Constant::Poison(ty)
            });
        }
        // Zero is checked element by element rather than by identity, because
        // a struct's fields need not be the same constant to be all zero:
        // `{ i16, [2 x i16] } { i16 0, [2 x i16] zeroinitializer }` is one.
        if elements
            .iter()
            .all(|element| self.is_zero_constant(*element))
        {
            return self
                .module
                .ctx
                .intern_constant(Constant::ZeroInitializer(ty));
        }
        let same = elements.windows(2).all(|pair| pair[0] == pair[1]);
        if same {
            let first = elements[0];
            match self.module.ctx.constant(first) {
                Constant::Undef(_) => {
                    return self.module.ctx.intern_constant(Constant::Undef(ty));
                }
                Constant::Poison(_) => {
                    return self.module.ctx.intern_constant(Constant::Poison(ty));
                }
                _ => {}
            }
            // A splat is how upstream writes a vector of repeated *data*.
            // A vector of the same symbol is written out lane by lane, the
            // lanes being addresses a linker fills in rather than data.
            if is_vector
                && matches!(
                    self.module.ctx.constant(first),
                    Constant::Integer { .. } | Constant::Float { .. }
                )
            {
                return self
                    .module
                    .ctx
                    .intern_constant(Constant::Splat { ty, element: first });
            }
        }
        let elements = elements.to_vec();
        self.module.ctx.intern_constant(if is_vector {
            Constant::Vector { ty, elements }
        } else {
            Constant::Array { ty, elements }
        })
    }

    /// Whether a constant is the all-zero bit pattern, which `zeroinitializer`
    /// stands for. A negative zero is not: it has a bit set.
    fn is_zero_constant(&self, id: ConstId) -> bool {
        match self.module.ctx.constant(id) {
            Constant::ZeroInitializer(_) | Constant::Null(_) => true,
            Constant::Integer { value, .. } => value.is_zero(),
            Constant::Float { value, .. } => value.bits().is_zero(),
            _ => false,
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
                    Ok(self.fold_aggregate(ty, &elements, false))
                }
            },
            TypeKind::Vector { element, .. } => {
                self.require(Token::Less)?;
                let elements = self.parse_constant_list(Token::Greater, element)?;
                Ok(self.fold_aggregate(ty, &elements, true))
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
            // A splat is the shorthand for repeated *data*, so a vector of
            // the same symbol is written out lane by lane whichever way the
            // module spelled it. Reading the shorthand and printing the long
            // form is what upstream does with one.
            if let TypeKind::Vector {
                count, scalable, ..
            } = *self.module.ctx.type_kind(ty)
                && !matches!(
                    self.module.ctx.constant(element),
                    Constant::Integer { .. } | Constant::Float { .. }
                )
            {
                // A scalable vector has no lane count to write out, so
                // upstream writes the construction that makes one instead:
                // put the value in the first lane and shuffle it across.
                if scalable {
                    return Ok(Some(self.built_splat(ty, element, count)));
                }
                if let Ok(width) = usize::try_from(count) {
                    let lanes = vec![element; width];
                    return Ok(Some(self.fold_aggregate(ty, &lanes, true)));
                }
            }
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
        // `zeroinitializer` is a value of a target extension type only where
        // the target registered it as one, and `undef` and `poison` are
        // values of every one of them. Upstream says so while reading rather
        // than when verifying, so the rule sits here and covers a global's
        // initializer and an instruction's operand alike.
        if word == "zeroinitializer"
            && let TypeKind::Target { name, .. } = self.module.ctx.type_kind(ty)
            && !llvm_ir::target_extension::properties(name.as_str()).zeroinit
        {
            return self.error("invalid type for null constant");
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
        // A struct folds to `zeroinitializer` the same way, but never to a
        // splat: its fields need not have one type. A struct with no fields
        // folds too, there being nothing in it that is not zero.
        if values.iter().all(|field| self.is_zero_constant(*field)) {
            return Ok(self
                .module
                .ctx
                .intern_constant(Constant::ZeroInitializer(ty)));
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
                let ty = expected.unwrap_or(base_type);
                // A walk that answers with a vector answers lane by lane, so
                // an index written once stands for the same index in every
                // lane and upstream writes it out as one: `i32 0` alongside a
                // `<4 x i32>` index comes back `<4 x i32> zeroinitializer`.
                // A struct field is the exception: every lane picks the same
                // field or the walk has no type, so that index stays as it
                // was written.
                let indices = self.widen_gep_indices(ty, source_type, indices);
                ConstExpr::GetElementPtr {
                    source_type,
                    base,
                    indices,
                    flags,
                    inrange,
                    ty,
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
                    let (source, operand) = self.parse_typed_constant()?;
                    if !self.eat_word("to") {
                        return self.error("expected 'to' in a cast expression");
                    }
                    let target = self.parse_type()?;
                    self.require(Token::RightParen)?;
                    // Crossing address spaces is the whole of what this cast
                    // does, so one that stays in its own space is not this
                    // cast at all. Upstream says so while reading rather than
                    // when verifying, an expression having to fold as it is
                    // read.
                    if op == CastOp::AddrSpaceCast
                        && self.address_space(source) == self.address_space(target)
                    {
                        return self.error("invalid cast opcode for cast");
                    }
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
        // A getelementptr that moves nowhere is the pointer it started from,
        // and upstream folds it away as it reads: `getelementptr (i32, ptr
        // @foo)` and `getelementptr inbounds ([4 x i32], ptr @a, i64 0, i64
        // 0)` both print back as the base.
        if let ConstExpr::GetElementPtr {
            base,
            indices,
            ty,
            inrange,
            ..
        } = &expr
            && inrange.is_none()
            && indices.iter().all(|index| self.is_zero_constant(*index))
        {
            // Where the walk answers with a vector, every lane arrives back
            // at the pointer it started from, so the answer is that pointer
            // in every lane rather than the pointer itself.
            let (base, ty) = (*base, *ty);
            return Ok(self.widen_to(ty, base));
        }
        // Walking from a pointer nobody chose arrives nowhere in particular,
        // so the answer is the same nothing the walk started from.
        if let ConstExpr::GetElementPtr { base, ty, .. } = &expr {
            let (base, ty) = (*base, *ty);
            match self.module.ctx.constant(base) {
                Constant::Undef(_) => {
                    return Ok(self.module.ctx.intern_constant(Constant::Undef(ty)));
                }
                Constant::Poison(_) => {
                    return Ok(self.module.ctx.intern_constant(Constant::Poison(ty)));
                }
                _ => {}
            }
        }
        if let ConstExpr::Binary { op, lhs, rhs, .. } = &expr {
            let (op, lhs, rhs) = (*op, *lhs, *rhs);
            if let Some(folded) = self.computed_binary(op, lhs, rhs) {
                return Ok(folded);
            }
            if let Some(folded) = self.folded_binary(op, lhs, rhs) {
                return Ok(folded);
            }
        }
        if let ConstExpr::Cast { op, operand, ty } = &expr
            && let Some(folded) = self.folded_cast(*op, *operand, *ty)
        {
            return Ok(folded);
        }
        Ok(self
            .module
            .ctx
            .intern_constant(Constant::Expression(Box::new(expr))))
    }

    /// The three arithmetic constant expressions left, at the values where
    /// they answer with one of their operands. `add` and `xor` do that with
    /// nought on either side; `sub` only on the right, subtraction not being
    /// commutative.
    fn folded_binary(&mut self, op: BinOp, lhs: ConstId, rhs: ConstId) -> Option<ConstId> {
        let identity = match op {
            BinOp::Add | BinOp::Xor => {
                if self.is_zero_constant(rhs) {
                    Some(lhs)
                } else if self.is_zero_constant(lhs) {
                    Some(rhs)
                } else {
                    None
                }
            }
            BinOp::Sub => self.is_zero_constant(rhs).then_some(lhs),
            _ => None,
        }?;
        // Only when the answer is already the type the expression produces:
        // a widening one would be a cast rather than a fold.
        (self.module.ctx.constant(identity).ty() == self.module.ctx.constant(lhs).ty())
            .then_some(identity)
    }

    /// The same three expressions where both sides are known, which upstream
    /// works out rather than carries. A vector answers lane by lane.
    fn computed_binary(&mut self, op: BinOp, lhs: ConstId, rhs: ConstId) -> Option<ConstId> {
        let ty = self.module.ctx.constant(lhs).ty();
        if ty != self.module.ctx.constant(rhs).ty() {
            return None;
        }
        if let TypeKind::Vector {
            element,
            count,
            scalable: false,
        } = self.module.ctx.type_kind(ty).clone()
        {
            let width = usize::try_from(count).ok()?;
            let lanes = |id: ConstId, ctx: &llvm_ir::Context| match ctx.constant(id).clone() {
                Constant::Vector { elements, .. } => Some(elements),
                Constant::Splat { element, .. } => Some(vec![element; width]),
                _ => None,
            };
            let left = lanes(lhs, &self.module.ctx)?;
            let right = lanes(rhs, &self.module.ctx)?;
            let mut folded = Vec::with_capacity(width);
            for (a, b) in left.into_iter().zip(right) {
                folded.push(self.computed_binary(op, a, b)?);
            }
            let _ = element;
            return Some(self.fold_aggregate(ty, &folded, true));
        }
        let (Constant::Integer { value: left, .. }, Constant::Integer { value: right, .. }) = (
            self.module.ctx.constant(lhs).clone(),
            self.module.ctx.constant(rhs).clone(),
        ) else {
            return None;
        };
        let value = match op {
            BinOp::Add => left.wrapping_add(&right),
            BinOp::Sub => left.wrapping_sub(&right),
            BinOp::Xor => left.xor(&right),
            _ => return None,
        };
        Some(
            self.module
                .ctx
                .intern_constant(Constant::Integer { ty, value }),
        )
    }

    /// What upstream computes rather than carries. A cast of a literal to a
    /// type the target can express is a literal, and upstream folds one as it
    /// reads, so a module that writes the cast prints back the answer.
    fn folded_cast(&mut self, op: CastOp, operand: ConstId, ty: TypeId) -> Option<ConstId> {
        // A cast that changes nothing is the thing it was given.
        if self.module.ctx.constant(operand).ty() == ty {
            return Some(operand);
        }
        // A cast that undoes the one under it, when nothing was lost in
        // between. The width upstream compares against is a fixed sixty-four
        // rather than the module's own pointer size, which a `p:32:32` layout
        // is what shows: `i32` is not folded there either.
        if let Constant::Expression(inner) = self.module.ctx.constant(operand).clone()
            && let ConstExpr::Cast {
                op: under,
                operand: original,
                ..
            } = *inner
        {
            let width =
                |id: ConstId, ctx: &llvm_ir::Context| match ctx.type_kind(ctx.constant(id).ty()) {
                    TypeKind::Integer(bits) => Some(*bits),
                    _ => None,
                };
            let undone = match (op, under) {
                (CastOp::IntToPtr, CastOp::PtrToInt) => {
                    width(operand, &self.module.ctx) == Some(64)
                }
                (CastOp::PtrToInt, CastOp::IntToPtr) => {
                    width(original, &self.module.ctx).is_some_and(|bits| bits <= 64)
                }
                _ => false,
            };
            if undone && self.module.ctx.constant(original).ty() == ty {
                return Some(original);
            }
        }
        // A bitcast between vectors keeps the bits and changes where the
        // lane boundaries fall. Two shapes of that are answerable without
        // laying the bits out: a lane count that does not change, where each
        // lane is bitcast on its own, and a pattern that reads the same at
        // any lane width, which all-zero, all-one, undef and poison are.
        let target = self.module.ctx.type_kind(ty).clone();
        if op == CastOp::BitCast
            && let TypeKind::Vector {
                element: to_element,
                count: to_count,
                scalable: false,
            } = target
            && let TypeKind::Vector {
                count: from_count,
                scalable: false,
                ..
            } = self
                .module
                .ctx
                .type_kind(self.module.ctx.constant(operand).ty())
                .clone()
        {
            let source = self.module.ctx.constant(operand).clone();
            match source {
                Constant::ZeroInitializer(_) => {
                    return Some(
                        self.module
                            .ctx
                            .intern_constant(Constant::ZeroInitializer(ty)),
                    );
                }
                Constant::Undef(_) => {
                    return Some(self.module.ctx.intern_constant(Constant::Undef(ty)));
                }
                Constant::Poison(_) => {
                    return Some(self.module.ctx.intern_constant(Constant::Poison(ty)));
                }
                // Every bit set stays every bit set however the lanes fall.
                Constant::Splat { element, .. }
                    if matches!(self.module.ctx.constant(element),
                        Constant::Integer { value, .. } if value.is_all_ones()) =>
                {
                    let TypeKind::Integer(bits) = *self.module.ctx.type_kind(to_element) else {
                        return None;
                    };
                    let ones = llvm_support::ApInt::from_u64(bits, 0).not();
                    let lane = self.module.ctx.intern_constant(Constant::Integer {
                        ty: to_element,
                        value: ones,
                    });
                    return Some(
                        self.module
                            .ctx
                            .intern_constant(Constant::Splat { ty, element: lane }),
                    );
                }
                Constant::Vector { elements, .. } if from_count == to_count => {
                    let mut folded = Vec::with_capacity(elements.len());
                    for lane in elements {
                        folded.push(self.folded_cast(CastOp::BitCast, lane, to_element)?);
                    }
                    return Some(self.fold_aggregate(ty, &folded, true));
                }
                _ => return None,
            }
        }
        if op != CastOp::BitCast
            && let TypeKind::Vector { element, .. } = target
        {
            // The operand may be written out lane by lane, or folded to a
            // splat or to zero; all three describe the same lanes.
            let TypeKind::Vector {
                count,
                scalable: false,
                ..
            } = self.module.ctx.type_kind(ty).clone()
            else {
                return None;
            };
            let width = usize::try_from(count).ok()?;
            let lanes = match self.module.ctx.constant(operand).clone() {
                Constant::Vector { elements, .. } => elements,
                Constant::Splat { element, .. } => vec![element; width],
                Constant::ZeroInitializer(source) => {
                    let TypeKind::Vector {
                        element: source, ..
                    } = self.module.ctx.type_kind(source).clone()
                    else {
                        return None;
                    };
                    let zero = self
                        .module
                        .ctx
                        .intern_constant(Constant::ZeroInitializer(source));
                    vec![zero; width]
                }
                _ => return None,
            };
            let mut folded = Vec::with_capacity(lanes.len());
            for lane in lanes {
                folded.push(self.folded_cast(op, lane, element)?);
            }
            return Some(self.fold_aggregate(ty, &folded, true));
        }
        let bits = match &target {
            TypeKind::Integer(bits) => Some(*bits),
            TypeKind::Float(semantics) => Some(semantics.bit_width()),
            _ => None,
        };
        match (op, self.module.ctx.constant(operand).clone()) {
            // Narrowing keeps the low bits, which is what `ApInt` does when
            // it is asked for fewer of them.
            (CastOp::Trunc, Constant::Integer { value, .. }) => {
                let bits = bits?;
                let narrowed = value.trunc(bits);
                Some(self.module.ctx.intern_constant(Constant::Integer {
                    ty,
                    value: narrowed,
                }))
            }
            // A bitcast keeps the bits and changes what reads them.
            (CastOp::BitCast, Constant::Integer { value, .. }) => match target {
                TypeKind::Float(semantics) => {
                    let float = ApFloat::from_bits(semantics, value);
                    Some(
                        self.module
                            .ctx
                            .intern_constant(Constant::Float { ty, value: float }),
                    )
                }
                _ => None,
            },
            (CastOp::BitCast, Constant::Float { value, .. }) => match target {
                TypeKind::Integer(_) => Some(self.module.ctx.intern_constant(Constant::Integer {
                    ty,
                    value: value.bits().clone(),
                })),
                _ => None,
            },
            // The two that cross between an address and a number, at the one
            // value both spell the same way.
            (CastOp::PtrToInt, Constant::Null(_) | Constant::ZeroInitializer(_)) => {
                let bits = bits?;
                Some(self.module.ctx.intern_constant(Constant::Integer {
                    ty,
                    value: llvm_support::ApInt::from_u64(bits, 0),
                }))
            }
            (CastOp::IntToPtr, Constant::Integer { value, .. }) if value.is_zero() => {
                Some(self.module.ctx.intern_constant(Constant::Null(ty)))
            }
            (CastOp::IntToPtr, Constant::ZeroInitializer(_)) => {
                Some(self.module.ctx.intern_constant(Constant::Null(ty)))
            }
            _ => None,
        }
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
