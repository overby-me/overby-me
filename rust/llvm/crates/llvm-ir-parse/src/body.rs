//! Functions, basic blocks and instructions.

use crate::lexer::Token;
use crate::{FunctionState, ParseError, Parser};
use llvm_ir::constant::{CastOp, GepFlags};
use llvm_ir::function::{Function, Param};
use llvm_ir::instruction::{
    AtomicOrdering, AtomicRmwOp, BinOp, CallArg, CallData, CallingConv, FastMathFlags,
    FloatPredicate, InstKind, Instruction, IntFlags, IntPredicate, LandingPadClause,
    NamedCallingConv, OperandBundle, SyncScope, TailKind, UnwindTarget,
};
use llvm_ir::types::TypeKind;
use llvm_ir::value::{BlockId, InstId, Name};
use llvm_ir::{TypeId, Value};

impl Parser {
    // ------------------------------------------------------------- functions

    pub(crate) fn parse_function(&mut self, is_define: bool) -> Result<(), ParseError> {
        // `declare !dbg !12 i32 @f()` puts the attachments before everything
        // else, which is the one place they lead rather than trail.
        let leading_metadata = self.parse_metadata_attachments()?;
        let qualifiers = self.parse_global_qualifiers()?;
        let calling_conv = self.parse_calling_conv()?;
        let return_attrs = self.parse_attribute_set(false)?;
        let return_type = self.parse_type_atom()?;

        let name = match self.advance() {
            Token::GlobalName(name) => Name::Named(name),
            Token::GlobalNumber(number) => Name::Number(number),
            other => {
                self.index -= 1;
                return self.error(format!(
                    "expected a function name, found {}",
                    other.describe()
                ));
            }
        };

        let mut function = Function::new(name, return_type);
        function.metadata = leading_metadata;
        function.qualifiers = qualifiers;
        function.calling_conv = calling_conv;
        function.return_attrs = return_attrs;

        let mut state = FunctionState::default();
        self.require(Token::LeftParen)?;
        while !self.eat(&Token::RightParen) {
            if !function.params.is_empty() || function.is_var_arg {
                self.require(Token::Comma)?;
            }
            if self.eat(&Token::Ellipsis) {
                function.is_var_arg = true;
                continue;
            }
            let ty = self.parse_type()?;
            let attrs = self.parse_attribute_set(false)?;
            let name = match self.peek().clone() {
                Token::LocalName(name) => {
                    self.advance();
                    state
                        .named_params
                        .insert(name.clone(), function.params.len() as u32);
                    Some(Name::Named(name))
                }
                Token::LocalNumber(number) => {
                    self.advance();
                    state
                        .numbered_params
                        .insert(number, function.params.len() as u32);
                    state.next_number = state.next_number.max(number + 1);
                    None
                }
                _ => {
                    if is_define {
                        state
                            .numbered_params
                            .insert(state.next_number, function.params.len() as u32);
                        state.next_number += 1;
                    }
                    None
                }
            };
            function.params.push(Param { ty, attrs, name });
        }

        // The trailing clause soup, in the order upstream writes it but
        // accepted in any order because hand-written input varies. Every arm
        // has to consume something; the progress check at the bottom turns a
        // future arm that does not into an error instead of a hang.
        loop {
            let before = self.index;
            match self.peek().clone() {
                Token::Word(word) => match word.as_str() {
                    "unnamed_addr" | "local_unnamed_addr" => {
                        self.advance();
                        function.qualifiers.unnamed_addr =
                            llvm_ir::global::UnnamedAddr::from_keyword(&word);
                    }
                    "addrspace" => {
                        function.qualifiers.address_space = self.parse_optional_address_space()?;
                    }
                    "section" => {
                        self.advance();
                        function.section = Some(self.require_quoted()?);
                    }
                    "partition" => {
                        self.advance();
                        function.partition = Some(self.require_quoted()?);
                    }
                    "comdat" => {
                        self.advance();
                        function.comdat = Some(self.parse_comdat_ref()?);
                    }
                    "align" => {
                        self.advance();
                        function.align = Some(self.parse_align()?);
                    }
                    "gc" => {
                        self.advance();
                        function.gc = Some(self.require_quoted()?);
                    }
                    "prefix" => {
                        self.advance();
                        function.prefix = Some(self.parse_typed_constant()?);
                    }
                    "prologue" => {
                        self.advance();
                        function.prologue = Some(self.parse_typed_constant()?);
                    }
                    "personality" => {
                        self.advance();
                        function.personality = Some(self.parse_typed_constant()?);
                    }
                    _ => {
                        let before = self.index;
                        let attrs = self.parse_attribute_set(false)?;
                        if self.index == before {
                            break;
                        }
                        function.attrs.attributes.extend(attrs.attributes);
                        function.attrs.groups.extend(attrs.groups);
                    }
                },
                Token::AttributeGroup(number) => {
                    self.advance();
                    function.attrs.groups.push(number);
                }
                Token::Quoted(_) => {
                    let attrs = self.parse_attribute_set(false)?;
                    function.attrs.attributes.extend(attrs.attributes);
                }
                // `!dbg !0` is an attachment; `!name = !{...}` on the next
                // line is the start of the next top-level item and this
                // function is over.
                Token::MetadataName(_) if matches!(self.peek_at(1), Token::MetadataNumber(_)) => {
                    let attachments = self.parse_metadata_attachments()?;
                    function.metadata.extend(attachments);
                }
                _ => break,
            }
            if self.index == before {
                return self.error("this clause was not understood");
            }
        }

        if is_define {
            self.require(Token::LeftBrace)?;
            self.parse_function_body(&mut function, &mut state)?;
            self.require(Token::RightBrace)?;
        }

        let expected = self.symbols.get(&function.name).copied();
        let id = self.module.add_function(function);
        debug_assert_eq!(
            expected,
            Some(llvm_ir::GlobalRef::Function(id)),
            "the pre-scan and the parse disagree about function ids"
        );
        Ok(())
    }

    fn parse_calling_conv(&mut self) -> Result<CallingConv, ParseError> {
        let Some(word) = self.peek_word() else {
            return Ok(CallingConv::C);
        };
        if word == "ccc" {
            self.advance();
            return Ok(CallingConv::C);
        }
        if let Some(named) = NamedCallingConv::from_keyword(word) {
            self.advance();
            return Ok(CallingConv::Named(named));
        }
        // `cc42`, with no space, which is how upstream prints the ones it has
        // no name for.
        if let Some(digits) = word.strip_prefix("cc")
            && !digits.is_empty()
            && digits.chars().all(|c| c.is_ascii_digit())
        {
            let number = digits.parse().map_err(|_| {
                self.error::<()>("calling convention number is too large")
                    .unwrap_err()
            })?;
            self.advance();
            return Ok(CallingConv::Numbered(number));
        }
        if word == "cc" {
            self.advance();
            let number = self.require_unsigned()? as u32;
            return Ok(CallingConv::Numbered(number));
        }
        // The one convention that takes an argument.
        if word == "riscv_vls_cc" && self.peek_at(1) == &Token::LeftParen {
            self.advance();
            self.advance();
            let number = self.require_unsigned()? as u32;
            self.require(Token::RightParen)?;
            return Ok(CallingConv::RiscvVls(number));
        }
        Ok(CallingConv::C)
    }

    /// Whether a block label starts here, in any of its spellings.
    fn starts_a_block_label(&self) -> bool {
        match self.peek() {
            Token::Label(_) | Token::LabelNumber(_) => true,
            Token::Quoted(_) | Token::Integer { .. } => self.peek_at(1) == &Token::Colon,
            _ => false,
        }
    }

    fn parse_function_body(
        &mut self,
        function: &mut Function,
        state: &mut FunctionState,
    ) -> Result<(), ParseError> {
        // An unnamed entry block has no label at all, so the body may start
        // straight into instructions.
        let mut current = match self.peek().clone() {
            _ if self.starts_a_block_label() => None,
            _ => {
                let id = function.reserve_block();
                function.place_block(id);
                state.next_number += 1;
                Some(id)
            }
        };

        loop {
            match self.peek().clone() {
                Token::RightBrace | Token::Eof => break,
                Token::Label(name) => {
                    self.advance();
                    let id = self.block_by_name(function, state, &Name::Named(name.clone()))?;
                    function.block_mut(id).name = Some(Name::Named(name));
                    function.place_block(id);
                    current = Some(id);
                }
                Token::LabelNumber(number) => {
                    self.advance();
                    let id = self.block_by_name(function, state, &Name::Number(number))?;
                    function.place_block(id);
                    state.next_number = state.next_number.max(number + 1);
                    current = Some(id);
                }
                // `"2":` and `-3:` are labels whose names only look like
                // numbers. The lexer cannot tell before the colon, so the
                // block loop reads the two tokens together.
                Token::Quoted(bytes) if self.peek_at(1) == &Token::Colon => {
                    self.advance();
                    self.advance();
                    let Ok(text) = String::from_utf8(bytes) else {
                        return self.error("a block label has to be valid UTF-8");
                    };
                    let id = self.block_by_name(function, state, &Name::Named(text.clone()))?;
                    function.block_mut(id).name = Some(Name::Named(text));
                    function.place_block(id);
                    current = Some(id);
                }
                Token::Integer { negative, digits } if self.peek_at(1) == &Token::Colon => {
                    self.advance();
                    self.advance();
                    let name = if negative {
                        format!("-{digits}")
                    } else {
                        digits
                    };
                    let id = self.block_by_name(function, state, &Name::Named(name.clone()))?;
                    function.block_mut(id).name = Some(Name::Named(name));
                    function.place_block(id);
                    current = Some(id);
                }
                _ => {
                    let Some(block) = current else {
                        return self.error("an instruction outside any block");
                    };
                    // A terminator ends its block. An instruction after one,
                    // with no label between, opens a fresh anonymous block:
                    // upstream's parser does this, which is why five invokes
                    // written in a row parse as five blocks.
                    let block = if function
                        .block(block)
                        .terminator()
                        .and_then(|last| function.try_instruction(last))
                        .is_some_and(|last| last.kind.is_terminator())
                    {
                        let fresh = function.reserve_block();
                        function.place_block(fresh);
                        state.next_number += 1;
                        current = Some(fresh);
                        fresh
                    } else {
                        block
                    };
                    let id = self.parse_instruction(function, state)?;
                    function.block_mut(block).instructions.push(id);
                }
            }
        }
        Ok(())
    }

    /// The block a label names, reserving it if the name has only been used
    /// as a branch target so far.
    fn block_by_name(
        &mut self,
        function: &mut Function,
        state: &mut FunctionState,
        name: &Name,
    ) -> Result<BlockId, ParseError> {
        let existing = match name {
            Name::Named(text) => state.named_blocks.get(text).copied(),
            Name::Number(number) => state.numbered_blocks.get(number).copied(),
        };
        if let Some(id) = existing {
            return Ok(id);
        }
        let id = function.reserve_block();
        match name {
            Name::Named(text) => {
                state.named_blocks.insert(text.clone(), id);
                function.block_mut(id).name = Some(Name::Named(text.clone()));
            }
            Name::Number(number) => {
                state.numbered_blocks.insert(*number, id);
            }
        }
        Ok(id)
    }

    /// The instruction a local name refers to, reserving a slot when the
    /// definition has not been read yet.
    fn value_by_name(
        &mut self,
        function: &mut Function,
        state: &mut FunctionState,
        name: &Name,
    ) -> Value {
        match name {
            Name::Named(text) => {
                if let Some(index) = state.named_params.get(text) {
                    return Value::Argument(*index);
                }
                if let Some(id) = state.named_values.get(text) {
                    return Value::Instruction(*id);
                }
                let id = function.reserve_instruction();
                state.named_values.insert(text.clone(), id);
                Value::Instruction(id)
            }
            Name::Number(number) => {
                if let Some(index) = state.numbered_params.get(number) {
                    return Value::Argument(*index);
                }
                if let Some(id) = state.numbered_values.get(number) {
                    return Value::Instruction(*id);
                }
                let id = function.reserve_instruction();
                state.numbered_values.insert(*number, id);
                Value::Instruction(id)
            }
        }
    }

    // ---------------------------------------------------------------- values

    pub(crate) fn parse_value(
        &mut self,
        function: &mut Function,
        state: &mut FunctionState,
        ty: TypeId,
    ) -> Result<Value, ParseError> {
        // A value of type metadata is a whole metadata operand, and inside a
        // function it can name a local: `metadata ptr %s` is how a debug
        // intrinsic is handed the variable it describes.
        if matches!(self.module.ctx.type_kind(ty), TypeKind::Metadata) {
            let operand = self.parse_metadata_operand(Some((function, state)))?;
            let constant = self
                .module
                .ctx
                .intern_constant(llvm_ir::constant::Constant::Metadata {
                    ty,
                    operand: Box::new(operand),
                });
            return Ok(Value::Constant(constant));
        }
        match self.peek().clone() {
            Token::LocalName(name) => {
                self.advance();
                Ok(self.value_by_name(function, state, &Name::Named(name)))
            }
            Token::LocalNumber(number) => {
                self.advance();
                Ok(self.value_by_name(function, state, &Name::Number(number)))
            }
            Token::MetadataNumber(number) => {
                self.advance();
                Ok(Value::Metadata(llvm_ir::MdId(number)))
            }
            _ => Ok(Value::Constant(self.parse_constant(ty)?)),
        }
    }

    pub(crate) fn parse_typed_value(
        &mut self,
        function: &mut Function,
        state: &mut FunctionState,
    ) -> Result<(TypeId, Value), ParseError> {
        let ty = self.parse_type()?;
        let value = self.parse_value(function, state, ty)?;
        Ok((ty, value))
    }

    fn parse_block_operand(
        &mut self,
        function: &mut Function,
        state: &mut FunctionState,
    ) -> Result<BlockId, ParseError> {
        if !self.eat_word("label") {
            return self.error("expected 'label' before a block reference");
        }
        let name = match self.advance() {
            Token::LocalName(name) => Name::Named(name),
            Token::LocalNumber(number) => Name::Number(number),
            other => {
                self.index -= 1;
                return self.error(format!("expected a block, found {}", other.describe()));
            }
        };
        self.block_by_name(function, state, &name)
    }

    // ---------------------------------------------------------- instructions

    fn parse_instruction(
        &mut self,
        function: &mut Function,
        state: &mut FunctionState,
    ) -> Result<InstId, ParseError> {
        // A result name, if there is one.
        let mut result: Option<Name> = None;
        let mut reserved: Option<InstId> = None;
        match self.peek().clone() {
            Token::LocalName(name) if self.peek_at(1) == &Token::Equals => {
                self.advance();
                self.advance();
                reserved = Some(match state.named_values.get(&name) {
                    Some(id) => *id,
                    None => {
                        let id = function.reserve_instruction();
                        state.named_values.insert(name.clone(), id);
                        id
                    }
                });
                result = Some(Name::Named(name));
            }
            Token::LocalNumber(number) if self.peek_at(1) == &Token::Equals => {
                self.advance();
                self.advance();
                reserved = Some(match state.numbered_values.get(&number) {
                    Some(id) => *id,
                    None => {
                        let id = function.reserve_instruction();
                        state.numbered_values.insert(number, id);
                        id
                    }
                });
                state.next_number = state.next_number.max(number + 1);
            }
            _ => {}
        }

        let (ty, kind) = self.parse_instruction_kind(function, state)?;
        let produces_value = !matches!(self.module.ctx.type_kind(ty), TypeKind::Void);

        let id = match reserved {
            Some(id) => id,
            None => {
                let id = function.reserve_instruction();
                if produces_value {
                    state.numbered_values.insert(state.next_number, id);
                    state.next_number += 1;
                }
                id
            }
        };

        let metadata = self.parse_metadata_attachments_after_comma()?;
        function.define_instruction(
            id,
            Instruction {
                name: result,
                ty,
                kind,
                metadata,
            },
        );
        Ok(id)
    }

    fn parse_instruction_kind(
        &mut self,
        function: &mut Function,
        state: &mut FunctionState,
    ) -> Result<(TypeId, InstKind), ParseError> {
        if let Token::DebugRecord(name) = self.peek().clone() {
            self.advance();
            self.require(Token::LeftParen)?;
            let mut operands = Vec::new();
            while !self.eat(&Token::RightParen) {
                if !operands.is_empty() {
                    self.require(Token::Comma)?;
                }
                operands.push(self.parse_metadata_operand(Some((function, state)))?);
            }
            let void = self.module.ctx.void_type();
            return Ok((void, InstKind::DebugRecord { name, operands }));
        }

        let opcode = self.require_word()?;
        let void = self.module.ctx.void_type();

        if let Some(op) = BinOp::from_keyword(&opcode) {
            let mut flags = IntFlags::default();
            let mut fast_math = FastMathFlags::default();
            self.parse_operation_flags(&mut flags, &mut fast_math);
            let ty = self.parse_type()?;
            let lhs = self.parse_value(function, state, ty)?;
            self.require(Token::Comma)?;
            let rhs = self.parse_value(function, state, ty)?;
            return Ok((
                ty,
                InstKind::Binary {
                    op,
                    flags,
                    fast_math,
                    lhs,
                    rhs,
                },
            ));
        }

        if let Some(op) = CastOp::from_keyword(&opcode) {
            let mut flags = IntFlags::default();
            let mut fast_math = FastMathFlags::default();
            self.parse_operation_flags(&mut flags, &mut fast_math);
            let source_type = self.parse_type()?;
            let operand = self.parse_value(function, state, source_type)?;
            if !self.eat_word("to") {
                return self.error("expected 'to' in a cast");
            }
            let ty = self.parse_type()?;
            return Ok((
                ty,
                InstKind::Cast {
                    op,
                    flags,
                    operand,
                    source_type,
                },
            ));
        }

        match opcode.as_str() {
            "ret" => {
                if self.eat_word("void") {
                    return Ok((void, InstKind::Ret(None)));
                }
                let (ty, value) = self.parse_typed_value(function, state)?;
                Ok((void, InstKind::Ret(Some((ty, value)))))
            }
            "br" => {
                if self.peek_word() == Some("label") {
                    let target = self.parse_block_operand(function, state)?;
                    return Ok((void, InstKind::Br { target }));
                }
                let ty = self.parse_type()?;
                let condition = self.parse_value(function, state, ty)?;
                self.require(Token::Comma)?;
                let if_true = self.parse_block_operand(function, state)?;
                self.require(Token::Comma)?;
                let if_false = self.parse_block_operand(function, state)?;
                Ok((
                    void,
                    InstKind::CondBr {
                        condition,
                        if_true,
                        if_false,
                    },
                ))
            }
            "switch" => {
                let value_type = self.parse_type()?;
                let value = self.parse_value(function, state, value_type)?;
                self.require(Token::Comma)?;
                let default = self.parse_block_operand(function, state)?;
                self.require(Token::LeftBracket)?;
                let mut cases = Vec::new();
                while !self.eat(&Token::RightBracket) {
                    let case_type = self.parse_type()?;
                    if case_type != value_type {
                        return self.error("switch case has the wrong type");
                    }
                    let case = self.parse_value(function, state, case_type)?;
                    self.require(Token::Comma)?;
                    let block = self.parse_block_operand(function, state)?;
                    cases.push((case, block));
                }
                Ok((
                    void,
                    InstKind::Switch {
                        value_type,
                        value,
                        default,
                        cases,
                    },
                ))
            }
            "indirectbr" => {
                let (_, address) = self.parse_typed_value(function, state)?;
                self.require(Token::Comma)?;
                self.require(Token::LeftBracket)?;
                let mut destinations = Vec::new();
                while !self.eat(&Token::RightBracket) {
                    if !destinations.is_empty() {
                        self.require(Token::Comma)?;
                    }
                    destinations.push(self.parse_block_operand(function, state)?);
                }
                Ok((
                    void,
                    InstKind::IndirectBr {
                        address,
                        destinations,
                    },
                ))
            }
            "invoke" => {
                let (ty, call) = self.parse_call_data(function, state, TailKind::None)?;
                if !self.eat_word("to") {
                    return self.error("expected 'to' after an invoke");
                }
                let normal = self.parse_block_operand(function, state)?;
                if !self.eat_word("unwind") {
                    return self.error("expected 'unwind' after an invoke");
                }
                let unwind = self.parse_block_operand(function, state)?;
                Ok((
                    ty,
                    InstKind::Invoke {
                        call: Box::new(call),
                        normal,
                        unwind,
                    },
                ))
            }
            "callbr" => {
                let (ty, call) = self.parse_call_data(function, state, TailKind::None)?;
                if !self.eat_word("to") {
                    return self.error("expected 'to' after a callbr");
                }
                let fallthrough = self.parse_block_operand(function, state)?;
                self.require(Token::LeftBracket)?;
                let mut indirect = Vec::new();
                while !self.eat(&Token::RightBracket) {
                    if !indirect.is_empty() {
                        self.require(Token::Comma)?;
                    }
                    indirect.push(self.parse_block_operand(function, state)?);
                }
                Ok((
                    ty,
                    InstKind::CallBr {
                        call: Box::new(call),
                        fallthrough,
                        indirect,
                    },
                ))
            }
            "resume" => {
                let (ty, value) = self.parse_typed_value(function, state)?;
                Ok((void, InstKind::Resume { ty, value }))
            }
            "unreachable" => Ok((void, InstKind::Unreachable)),
            "catchswitch" => {
                if !self.eat_word("within") {
                    return self.error("expected 'within' in a catchswitch");
                }
                let parent = self.parse_pad_operand(function, state)?;
                self.require(Token::LeftBracket)?;
                let mut handlers = Vec::new();
                while !self.eat(&Token::RightBracket) {
                    if !handlers.is_empty() {
                        self.require(Token::Comma)?;
                    }
                    handlers.push(self.parse_block_operand(function, state)?);
                }
                if !self.eat_word("unwind") {
                    return self.error("expected 'unwind' in a catchswitch");
                }
                let unwind = self.parse_unwind_target(function, state)?;
                let token = self.module.ctx.token_type();
                Ok((
                    token,
                    InstKind::CatchSwitch {
                        parent,
                        handlers,
                        unwind,
                    },
                ))
            }
            "catchret" => {
                if !self.eat_word("from") {
                    return self.error("expected 'from' in a catchret");
                }
                let pad = self.parse_pad_operand(function, state)?;
                if !self.eat_word("to") {
                    return self.error("expected 'to' in a catchret");
                }
                let target = self.parse_block_operand(function, state)?;
                Ok((void, InstKind::CatchRet { pad, target }))
            }
            "cleanupret" => {
                if !self.eat_word("from") {
                    return self.error("expected 'from' in a cleanupret");
                }
                let pad = self.parse_pad_operand(function, state)?;
                if !self.eat_word("unwind") {
                    return self.error("expected 'unwind' in a cleanupret");
                }
                let unwind = self.parse_unwind_target(function, state)?;
                Ok((void, InstKind::CleanupRet { pad, unwind }))
            }
            "fneg" => {
                let mut flags = IntFlags::default();
                let mut fast_math = FastMathFlags::default();
                self.parse_operation_flags(&mut flags, &mut fast_math);
                let ty = self.parse_type()?;
                let operand = self.parse_value(function, state, ty)?;
                Ok((ty, InstKind::FNeg { fast_math, operand }))
            }
            "icmp" => {
                let mut flags = IntFlags::default();
                let mut fast_math = FastMathFlags::default();
                self.parse_operation_flags(&mut flags, &mut fast_math);
                let word = self.require_word()?;
                let Some(predicate) = IntPredicate::from_keyword(&word) else {
                    return self.error(format!("unknown integer predicate '{word}'"));
                };
                let operand_type = self.parse_type()?;
                let lhs = self.parse_value(function, state, operand_type)?;
                self.require(Token::Comma)?;
                let rhs = self.parse_value(function, state, operand_type)?;
                let ty = self.comparison_result_type(operand_type)?;
                Ok((
                    ty,
                    InstKind::ICmp {
                        predicate,
                        flags,
                        operand_type,
                        lhs,
                        rhs,
                    },
                ))
            }
            "fcmp" => {
                let mut flags = IntFlags::default();
                let mut fast_math = FastMathFlags::default();
                self.parse_operation_flags(&mut flags, &mut fast_math);
                let word = self.require_word()?;
                let Some(predicate) = FloatPredicate::from_keyword(&word) else {
                    return self.error(format!("unknown float predicate '{word}'"));
                };
                let operand_type = self.parse_type()?;
                let lhs = self.parse_value(function, state, operand_type)?;
                self.require(Token::Comma)?;
                let rhs = self.parse_value(function, state, operand_type)?;
                let ty = self.comparison_result_type(operand_type)?;
                Ok((
                    ty,
                    InstKind::FCmp {
                        predicate,
                        fast_math,
                        operand_type,
                        lhs,
                        rhs,
                    },
                ))
            }
            "alloca" => {
                let inalloca = self.eat_word("inalloca");
                let allocated_type = self.parse_type()?;
                let mut count = None;
                let mut align = None;
                let mut address_space = None;
                while self.eat(&Token::Comma) {
                    if self.eat_word("align") {
                        align = Some(self.parse_align()?);
                    } else if self.peek_word() == Some("addrspace") {
                        address_space = self.parse_optional_address_space()?;
                    } else if matches!(self.peek(), Token::MetadataName(_)) {
                        self.index -= 1;
                        break;
                    } else {
                        count = Some(self.parse_typed_value(function, state)?);
                    }
                }
                let ptr = self.module.ctx.pointer_type(address_space.unwrap_or(0));
                let align = align.or_else(|| self.module.default_align(allocated_type, true));
                Ok((
                    ptr,
                    InstKind::Alloca {
                        allocated_type,
                        count,
                        align,
                        address_space,
                        inalloca,
                    },
                ))
            }
            "load" => {
                let atomic_marker = self.eat_word("atomic");
                let volatile = self.eat_word("volatile");
                let loaded_type = self.parse_type()?;
                self.require(Token::Comma)?;
                let pointer_type = self.parse_type()?;
                let pointer = self.parse_value(function, state, pointer_type)?;
                let atomic = if atomic_marker {
                    Some(self.parse_atomic_suffix()?)
                } else {
                    None
                };
                let mut align = None;
                while self.eat(&Token::Comma) {
                    if self.eat_word("align") {
                        align = Some(self.parse_align()?);
                    } else {
                        self.index -= 1;
                        break;
                    }
                }
                let align = align.or_else(|| self.module.default_align(loaded_type, false));
                Ok((
                    loaded_type,
                    InstKind::Load {
                        loaded_type,
                        pointer,
                        volatile,
                        atomic,
                        align,
                    },
                ))
            }
            "store" => {
                let atomic_marker = self.eat_word("atomic");
                let volatile = self.eat_word("volatile");
                let (value_type, value) = self.parse_typed_value(function, state)?;
                self.require(Token::Comma)?;
                let pointer_type = self.parse_type()?;
                let pointer = self.parse_value(function, state, pointer_type)?;
                let atomic = if atomic_marker {
                    Some(self.parse_atomic_suffix()?)
                } else {
                    None
                };
                let mut align = None;
                while self.eat(&Token::Comma) {
                    if self.eat_word("align") {
                        align = Some(self.parse_align()?);
                    } else {
                        self.index -= 1;
                        break;
                    }
                }
                let align = align.or_else(|| self.module.default_align(value_type, false));
                Ok((
                    void,
                    InstKind::Store {
                        value_type,
                        value,
                        pointer,
                        volatile,
                        atomic,
                        align,
                    },
                ))
            }
            "fence" => {
                let (scope, ordering) = self.parse_atomic_suffix()?;
                Ok((void, InstKind::Fence { scope, ordering }))
            }
            "cmpxchg" => {
                let weak = self.eat_word("weak");
                let volatile = self.eat_word("volatile");
                let pointer_type = self.parse_type()?;
                let pointer = self.parse_value(function, state, pointer_type)?;
                self.require(Token::Comma)?;
                let (compare_type, compare) = self.parse_typed_value(function, state)?;
                self.require(Token::Comma)?;
                let (_, new) = self.parse_typed_value(function, state)?;
                let scope = self.parse_sync_scope()?;
                let success = self.parse_ordering()?;
                let failure = self.parse_ordering()?;
                let mut align = None;
                while self.eat(&Token::Comma) {
                    if self.eat_word("align") {
                        align = Some(self.parse_align()?);
                    } else {
                        self.index -= 1;
                        break;
                    }
                }
                let align = align.or_else(|| self.module.default_align(compare_type, false));
                let bool_type = self.module.ctx.int_type(1);
                let ty = self
                    .module
                    .ctx
                    .struct_type(vec![compare_type, bool_type], false);
                Ok((
                    ty,
                    InstKind::CmpXchg {
                        pointer,
                        compare_type,
                        compare,
                        new,
                        weak,
                        volatile,
                        scope,
                        success,
                        failure,
                        align,
                    },
                ))
            }
            "atomicrmw" => {
                let volatile = self.eat_word("volatile");
                let word = self.require_word()?;
                let Some(op) = AtomicRmwOp::from_keyword(&word) else {
                    return self.error(format!("unknown atomicrmw operation '{word}'"));
                };
                let pointer_type = self.parse_type()?;
                let pointer = self.parse_value(function, state, pointer_type)?;
                self.require(Token::Comma)?;
                let (value_type, value) = self.parse_typed_value(function, state)?;
                let scope = self.parse_sync_scope()?;
                let ordering = self.parse_ordering()?;
                let mut align = None;
                while self.eat(&Token::Comma) {
                    if self.eat_word("align") {
                        align = Some(self.parse_align()?);
                    } else {
                        self.index -= 1;
                        break;
                    }
                }
                let align = align.or_else(|| self.module.default_align(value_type, false));
                Ok((
                    value_type,
                    InstKind::AtomicRmw {
                        op,
                        pointer,
                        value_type,
                        value,
                        volatile,
                        scope,
                        ordering,
                        align,
                    },
                ))
            }
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
                let source_type = self.parse_type()?;
                self.require(Token::Comma)?;
                let pointer_type = self.parse_type()?;
                let pointer = self.parse_value(function, state, pointer_type)?;
                let mut indices = Vec::new();
                while self.eat(&Token::Comma) {
                    if matches!(self.peek(), Token::MetadataName(_)) {
                        self.index -= 1;
                        break;
                    }
                    indices.push(self.parse_typed_value(function, state)?);
                }
                let ty = self.gep_result_type(pointer_type, &indices)?;
                Ok((
                    ty,
                    InstKind::GetElementPtr {
                        source_type,
                        pointer_type,
                        pointer,
                        indices,
                        flags,
                        inrange,
                    },
                ))
            }
            "extractelement" => {
                let (vector_type, vector) = self.parse_typed_value(function, state)?;
                self.require(Token::Comma)?;
                let (index_type, index) = self.parse_typed_value(function, state)?;
                let ty = match self.module.ctx.type_kind(vector_type) {
                    TypeKind::Vector { element, .. } => *element,
                    _ => return self.error("extractelement needs a vector"),
                };
                Ok((
                    ty,
                    InstKind::ExtractElement {
                        vector_type,
                        vector,
                        index_type,
                        index,
                    },
                ))
            }
            "insertelement" => {
                let (vector_type, vector) = self.parse_typed_value(function, state)?;
                self.require(Token::Comma)?;
                let (element_type, element) = self.parse_typed_value(function, state)?;
                self.require(Token::Comma)?;
                let (index_type, index) = self.parse_typed_value(function, state)?;
                Ok((
                    vector_type,
                    InstKind::InsertElement {
                        vector_type,
                        vector,
                        element_type,
                        element,
                        index_type,
                        index,
                    },
                ))
            }
            "shufflevector" => {
                let (vector_type, first) = self.parse_typed_value(function, state)?;
                self.require(Token::Comma)?;
                let (_, second) = self.parse_typed_value(function, state)?;
                self.require(Token::Comma)?;
                let (mask_type, mask) = self.parse_typed_value(function, state)?;
                let ty = self.shuffle_result_type(vector_type, mask_type)?;
                Ok((
                    ty,
                    InstKind::ShuffleVector {
                        vector_type,
                        first,
                        second,
                        mask_type,
                        mask,
                    },
                ))
            }
            "extractvalue" => {
                let (aggregate_type, aggregate) = self.parse_typed_value(function, state)?;
                let mut indices = Vec::new();
                // A metadata attachment follows the same comma, so the index
                // list ends where one begins.
                while !self.attachment_after_comma() && self.eat(&Token::Comma) {
                    indices.push(self.require_unsigned()? as u32);
                }
                let ty = self.aggregate_element_type(aggregate_type, &indices)?;
                Ok((
                    ty,
                    InstKind::ExtractValue {
                        aggregate_type,
                        aggregate,
                        indices,
                    },
                ))
            }
            "insertvalue" => {
                let (aggregate_type, aggregate) = self.parse_typed_value(function, state)?;
                self.require(Token::Comma)?;
                let (element_type, element) = self.parse_typed_value(function, state)?;
                let mut indices = Vec::new();
                // A metadata attachment follows the same comma, so the index
                // list ends where one begins.
                while !self.attachment_after_comma() && self.eat(&Token::Comma) {
                    indices.push(self.require_unsigned()? as u32);
                }
                Ok((
                    aggregate_type,
                    InstKind::InsertValue {
                        aggregate_type,
                        aggregate,
                        element_type,
                        element,
                        indices,
                    },
                ))
            }
            "phi" => {
                let mut flags = IntFlags::default();
                let mut fast_math = FastMathFlags::default();
                self.parse_operation_flags(&mut flags, &mut fast_math);
                let ty = self.parse_type()?;
                let mut incoming = Vec::new();
                // A phi with no edges at all is legal: upstream accepts one
                // in a block nothing branches to.
                while self.peek() == &Token::LeftBracket || !incoming.is_empty() {
                    if !incoming.is_empty() && !self.eat(&Token::Comma) {
                        break;
                    }
                    self.require(Token::LeftBracket)?;
                    let value = self.parse_value(function, state, ty)?;
                    self.require(Token::Comma)?;
                    let name = match self.advance() {
                        Token::LocalName(name) => Name::Named(name),
                        Token::LocalNumber(number) => Name::Number(number),
                        other => {
                            self.index -= 1;
                            return self
                                .error(format!("expected a block, found {}", other.describe()));
                        }
                    };
                    let block = self.block_by_name(function, state, &name)?;
                    self.require(Token::RightBracket)?;
                    incoming.push((value, block));
                }
                Ok((
                    ty,
                    InstKind::Phi {
                        fast_math,
                        incoming,
                    },
                ))
            }
            "select" => {
                let mut flags = IntFlags::default();
                let mut fast_math = FastMathFlags::default();
                self.parse_operation_flags(&mut flags, &mut fast_math);
                let (condition_type, condition) = self.parse_typed_value(function, state)?;
                self.require(Token::Comma)?;
                let (ty, if_true) = self.parse_typed_value(function, state)?;
                self.require(Token::Comma)?;
                let (_, if_false) = self.parse_typed_value(function, state)?;
                Ok((
                    ty,
                    InstKind::Select {
                        fast_math,
                        condition_type,
                        condition,
                        if_true,
                        if_false,
                    },
                ))
            }
            "freeze" => {
                let (operand_type, operand) = self.parse_typed_value(function, state)?;
                Ok((
                    operand_type,
                    InstKind::Freeze {
                        operand_type,
                        operand,
                    },
                ))
            }
            "va_arg" => {
                let (list_type, list) = self.parse_typed_value(function, state)?;
                self.require(Token::Comma)?;
                let ty = self.parse_type()?;
                Ok((ty, InstKind::VaArg { list_type, list }))
            }
            "landingpad" => {
                let ty = self.parse_type()?;
                let mut cleanup = false;
                let mut clauses = Vec::new();
                loop {
                    if self.eat_word("cleanup") {
                        cleanup = true;
                    } else if self.eat_word("catch") {
                        let (clause_type, value) = self.parse_typed_value(function, state)?;
                        clauses.push(LandingPadClause::Catch {
                            ty: clause_type,
                            value,
                        });
                    } else if self.eat_word("filter") {
                        let (clause_type, value) = self.parse_typed_value(function, state)?;
                        clauses.push(LandingPadClause::Filter {
                            ty: clause_type,
                            value,
                        });
                    } else {
                        break;
                    }
                }
                Ok((ty, InstKind::LandingPad { cleanup, clauses }))
            }
            "catchpad" | "cleanuppad" => {
                if !self.eat_word("within") {
                    return self.error("expected 'within' in a pad");
                }
                let parent = self.parse_pad_operand(function, state)?;
                self.require(Token::LeftBracket)?;
                let mut args = Vec::new();
                while !self.eat(&Token::RightBracket) {
                    if !args.is_empty() {
                        self.require(Token::Comma)?;
                    }
                    args.push(self.parse_typed_value(function, state)?);
                }
                let token = self.module.ctx.token_type();
                let kind = if opcode == "catchpad" {
                    InstKind::CatchPad { parent, args }
                } else {
                    InstKind::CleanupPad { parent, args }
                };
                Ok((token, kind))
            }
            "call" | "tail" | "musttail" | "notail" => {
                let tail = match opcode.as_str() {
                    "tail" => TailKind::Tail,
                    "musttail" => TailKind::MustTail,
                    "notail" => TailKind::NoTail,
                    _ => TailKind::None,
                };
                if tail != TailKind::None && !self.eat_word("call") {
                    return self.error("expected 'call' after a tail marker");
                }
                let (ty, call) = self.parse_call_data(function, state, tail)?;
                Ok((ty, InstKind::Call(Box::new(call))))
            }
            other => self.error(format!("unknown instruction '{other}'")),
        }
    }

    /// A funclet operand: `none` or a local value.
    fn parse_pad_operand(
        &mut self,
        function: &mut Function,
        state: &mut FunctionState,
    ) -> Result<Value, ParseError> {
        if self.eat_word("none") {
            let token = self.module.ctx.token_type();
            return Ok(Value::Constant(
                self.module
                    .ctx
                    .intern_constant(llvm_ir::constant::Constant::NoneToken(token)),
            ));
        }
        let name = match self.advance() {
            Token::LocalName(name) => Name::Named(name),
            Token::LocalNumber(number) => Name::Number(number),
            other => {
                self.index -= 1;
                return self.error(format!("expected a funclet, found {}", other.describe()));
            }
        };
        Ok(self.value_by_name(function, state, &name))
    }

    fn parse_unwind_target(
        &mut self,
        function: &mut Function,
        state: &mut FunctionState,
    ) -> Result<UnwindTarget, ParseError> {
        if self.eat_word("to") {
            if !self.eat_word("caller") {
                return self.error("expected 'caller' after 'unwind to'");
            }
            return Ok(UnwindTarget::Caller);
        }
        Ok(UnwindTarget::Block(
            self.parse_block_operand(function, state)?,
        ))
    }

    fn parse_call_data(
        &mut self,
        function: &mut Function,
        state: &mut FunctionState,
        tail: TailKind,
    ) -> Result<(TypeId, CallData), ParseError> {
        let mut flags = IntFlags::default();
        let mut fast_math = FastMathFlags::default();
        self.parse_operation_flags(&mut flags, &mut fast_math);
        let calling_conv = self.parse_calling_conv()?;
        let return_attrs = self.parse_attribute_set(false)?;
        // `call addrspace(0) void @f()` puts the space before the type;
        // upstream also accepts it after, so both are read.
        let leading_address_space = self.parse_optional_address_space()?;
        let written_type = self.parse_type_atom()?;
        let written_type = self.parse_type_suffix(written_type)?;
        let address_space = self
            .parse_optional_address_space()?
            .or(leading_address_space);

        let callee_type = self.module.ctx.pointer_type(address_space.unwrap_or(0));
        let callee = self.parse_value(function, state, callee_type)?;

        self.require(Token::LeftParen)?;
        let mut args = Vec::new();
        while !self.eat(&Token::RightParen) {
            if !args.is_empty() {
                self.require(Token::Comma)?;
            }
            if self.eat(&Token::Ellipsis) {
                continue;
            }
            let ty = self.parse_type()?;
            let attrs = self.parse_attribute_set(false)?;
            let value = self.parse_value(function, state, ty)?;
            args.push(CallArg { ty, attrs, value });
        }

        let fn_attrs = self.parse_attribute_set(false)?;

        let mut bundles = Vec::new();
        if self.peek() == &Token::LeftBracket {
            self.advance();
            while !self.eat(&Token::RightBracket) {
                if !bundles.is_empty() {
                    self.require(Token::Comma)?;
                }
                let tag = self.require_quoted()?;
                self.require(Token::LeftParen)?;
                let mut bundle_args = Vec::new();
                while !self.eat(&Token::RightParen) {
                    if !bundle_args.is_empty() {
                        self.require(Token::Comma)?;
                    }
                    bundle_args.push(self.parse_typed_value(function, state)?);
                }
                bundles.push(OperandBundle {
                    tag,
                    args: bundle_args,
                });
            }
        }

        // A call writes either the whole function type or only the result
        // type; when only the result type is written, the argument types
        // reconstruct the rest.
        let (function_type, result) = match self.module.ctx.type_kind(written_type) {
            TypeKind::Function { result, .. } => (written_type, *result),
            _ => {
                let params: Vec<TypeId> = args.iter().map(|arg| arg.ty).collect();
                (
                    self.module.ctx.function_type(written_type, params, false),
                    written_type,
                )
            }
        };

        Ok((
            result,
            CallData {
                tail,
                fast_math,
                calling_conv,
                return_attrs,
                function_type,
                address_space,
                callee,
                args,
                fn_attrs,
                bundles,
            },
        ))
    }

    /// The flag keywords that can follow an opcode, in any order.
    pub(crate) fn parse_operation_flags(
        &mut self,
        flags: &mut IntFlags,
        fast_math: &mut FastMathFlags,
    ) {
        loop {
            let Some(word) = self.peek_word() else {
                return;
            };
            let matched = match word {
                "nuw" => {
                    flags.nuw = true;
                    true
                }
                "nsw" => {
                    flags.nsw = true;
                    true
                }
                "exact" => {
                    flags.exact = true;
                    true
                }
                "disjoint" => {
                    flags.disjoint = true;
                    true
                }
                "nneg" => {
                    flags.nneg = true;
                    true
                }
                "samesign" => {
                    flags.samesign = true;
                    true
                }
                other => fast_math.set_by_keyword(other),
            };
            if !matched {
                return;
            }
            self.advance();
        }
    }

    fn parse_sync_scope(&mut self) -> Result<SyncScope, ParseError> {
        if self.peek_word() != Some("syncscope") {
            return Ok(SyncScope::system());
        }
        self.advance();
        self.require(Token::LeftParen)?;
        let name = self.require_quoted()?;
        self.require(Token::RightParen)?;
        Ok(SyncScope(Some(name)))
    }

    fn parse_ordering(&mut self) -> Result<AtomicOrdering, ParseError> {
        let word = self.require_word()?;
        match AtomicOrdering::from_keyword(&word) {
            Some(ordering) => Ok(ordering),
            None => self.error(format!("unknown memory ordering '{word}'")),
        }
    }

    fn parse_atomic_suffix(&mut self) -> Result<(SyncScope, AtomicOrdering), ParseError> {
        let scope = self.parse_sync_scope()?;
        let ordering = self.parse_ordering()?;
        Ok((scope, ordering))
    }

    // ------------------------------------------------------------ type rules

    /// `icmp` and `fcmp` produce `i1`, or a vector of `i1` for vector operands.
    fn comparison_result_type(&mut self, operand_type: TypeId) -> Result<TypeId, ParseError> {
        let bool_type = self.module.ctx.int_type(1);
        match self.module.ctx.type_kind(operand_type) {
            TypeKind::Vector {
                count, scalable, ..
            } => {
                let (count, scalable) = (*count, *scalable);
                Ok(self.module.ctx.vector_type(bool_type, count, scalable))
            }
            _ => Ok(bool_type),
        }
    }

    /// A `getelementptr` produces a pointer, or a vector of pointers when any
    /// operand is a vector.
    fn gep_result_type(
        &mut self,
        pointer_type: TypeId,
        indices: &[(TypeId, Value)],
    ) -> Result<TypeId, ParseError> {
        if let TypeKind::Vector {
            count, scalable, ..
        } = self.module.ctx.type_kind(pointer_type)
        {
            let (count, scalable) = (*count, *scalable);
            let ptr = self.module.ctx.pointer_type(0);
            return Ok(self.module.ctx.vector_type(ptr, count, scalable));
        }
        for (ty, _) in indices {
            if let TypeKind::Vector {
                count, scalable, ..
            } = self.module.ctx.type_kind(*ty)
            {
                let (count, scalable) = (*count, *scalable);
                let ptr = self.module.ctx.pointer_type(0);
                return Ok(self.module.ctx.vector_type(ptr, count, scalable));
            }
        }
        Ok(pointer_type)
    }

    fn shuffle_result_type(
        &mut self,
        vector_type: TypeId,
        mask_type: TypeId,
    ) -> Result<TypeId, ParseError> {
        let element = match self.module.ctx.type_kind(vector_type) {
            TypeKind::Vector { element, .. } => *element,
            _ => return self.error("shufflevector needs vector operands"),
        };
        match self.module.ctx.type_kind(mask_type) {
            TypeKind::Vector {
                count, scalable, ..
            } => {
                let (count, scalable) = (*count, *scalable);
                Ok(self.module.ctx.vector_type(element, count, scalable))
            }
            _ => self.error("a shufflevector mask must be a vector"),
        }
    }

    fn aggregate_element_type(
        &mut self,
        aggregate: TypeId,
        indices: &[u32],
    ) -> Result<TypeId, ParseError> {
        let mut current = aggregate;
        for index in indices {
            current = match self.module.ctx.type_kind(current).clone() {
                TypeKind::Struct { fields, .. } => match fields.get(*index as usize) {
                    Some(field) => *field,
                    None => return self.error("struct index is out of range"),
                },
                TypeKind::NamedStruct(id) => {
                    let def = self.module.ctx.struct_def(id);
                    match def.fields.as_ref().and_then(|f| f.get(*index as usize)) {
                        Some(field) => *field,
                        None => return self.error("struct index is out of range"),
                    }
                }
                TypeKind::Array { element, .. } => element,
                _ => return self.error("extractvalue needs an aggregate"),
            };
        }
        Ok(current)
    }
}
