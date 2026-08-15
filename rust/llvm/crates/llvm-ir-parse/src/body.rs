//! Functions, basic blocks and instructions.

use crate::lexer::Token;
use crate::{FunctionState, ParseError, Parser};
use llvm_ir::attribute::AttributeSet;
use llvm_ir::constant::{CastOp, Constant, GepFlags};
use llvm_ir::function::{Function, Param};
use llvm_ir::instruction::{
    AtomicOrdering, AtomicRmwOp, BinOp, CallArg, CallData, CallingConv, FastMathFlags,
    FloatPredicate, InstKind, Instruction, IntFlags, IntPredicate, LandingPadClause,
    NamedCallingConv, OperandBundle, SyncScope, TailKind, UnwindTarget,
};
use llvm_ir::metadata::{MdOperand, MdRef};
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
        let return_type = self.parse_pointer_suffix(return_type)?;

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
                        let attrs = self.parse_function_attribute_set()?;
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
                // A run that starts with a quoted key is the same run: it may
                // name a group and it may hold the older memory spellings,
                // and reading only the attributes here dropped both.
                Token::Quoted(_) => {
                    let attrs = self.parse_function_attribute_set()?;
                    function.attrs.attributes.extend(attrs.attributes);
                    function.attrs.groups.extend(attrs.groups);
                }
                // `!dbg !0` is an attachment, and so is `!prof !{...}` with
                // its node written in place; `!name = !{...}` on the next
                // line is the start of the next top-level item and this
                // function is over.
                Token::MetadataName(_)
                    if matches!(
                        self.peek_at(1),
                        Token::MetadataNumber(_) | Token::MetadataName(_) | Token::Exclaim
                    ) =>
                {
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
                // The block takes a slot from the same counter unnamed values
                // use, and `%N` elsewhere in the body names it by that
                // number. Going through `block_by_name` reuses the placeholder
                // a forward reference already made rather than shadowing it.
                let slot = state.next_number;
                let id = self.block_by_name(function, state, &Name::Number(slot))?;
                function.place_block(id);
                state.next_number += 1;
                Some(id)
            }
        };

        // A body's use-list order directives come last, in a run: once one
        // has been written, an instruction or a label after it is
        // "expected uselistorder directive" upstream. Measured both ways,
        // including that a second directive is fine.
        let mut in_directives = false;
        loop {
            let next = self.peek().clone();
            if in_directives
                && !matches!(next, Token::RightBrace | Token::Eof)
                && !matches!(&next, Token::Word(word) if word == "uselistorder")
            {
                return self.error("expected uselistorder directive");
            }
            match next {
                Token::RightBrace | Token::Eof => break,
                Token::Label(name) => {
                    self.advance();
                    let id = self.block_by_name(function, state, &Name::Named(name.clone()))?;
                    if function.block_order.contains(&id) {
                        return self.error(format!("redefinition of block '%{name}'"));
                    }
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
                // A use-list order directive sits among the instructions and
                // is not one: it says what order a value's uses were in, and
                // upstream drops it on the way out. Only `uselistorder`:
                // `uselistorder_bb` is a top-level directive, and written
                // here it is an opcode upstream does not know.
                Token::Word(word) if word == "uselistorder" => {
                    self.parse_use_list_order(Some((function, state)))?;
                    in_directives = true;
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
                        let slot = state.next_number;
                        let fresh = self.block_by_name(function, state, &Name::Number(slot))?;
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
                if !state.defined_values.insert(name.clone()) {
                    return self.error(format!("redefinition of '%{name}'"));
                }
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
            None if produces_value => {
                // An instruction written without a `%N =` still takes the
                // next number, and `%N` elsewhere may already have made a
                // placeholder for it. Reusing that placeholder rather than
                // reserving a second slot is what keeps a forward reference
                // pointing at the instruction that arrives.
                let id = match state.numbered_values.get(&state.next_number) {
                    Some(id) => *id,
                    None => function.reserve_instruction(),
                };
                state.numbered_values.insert(state.next_number, id);
                state.next_number += 1;
                id
            }
            None => function.reserve_instruction(),
        };

        let mut metadata = self.parse_metadata_attachments_after_comma()?;
        // A debug record built from a call to one of the `llvm.dbg.*`
        // intrinsics takes its location from the `!dbg` the call carried,
        // which is the last operand rather than an attachment.
        let mut kind = kind;
        let one_short = match &kind {
            InstKind::DebugRecord { operands, .. } => {
                operands.len() + 1 == debug_record_arity(&kind).unwrap_or(0)
            }
            _ => false,
        };
        if one_short {
            let location = metadata
                .iter()
                .position(|attachment| attachment.kind == "dbg")
                .map(|at| metadata.remove(at).node);
            let location = match location {
                Some(MdRef::Id(id)) => MdOperand::Ref(id),
                Some(MdRef::Inline(node)) => MdOperand::Inline(node),
                None => MdOperand::Null,
            };
            if let InstKind::DebugRecord { operands, .. } = &mut kind {
                operands.push(location);
            }
        }
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
            self.wrote_debug_record = true;
            self.require(Token::LeftParen)?;
            let mut operands = Vec::new();
            while !self.eat(&Token::RightParen) {
                if !operands.is_empty() {
                    self.require(Token::Comma)?;
                }
                operands.push(self.parse_metadata_operand(Some((function, state)))?);
            }
            // Each record has a fixed shape: a value, then metadata for
            // everything else. Upstream reads the `!` as punctuation rather
            // than as part of an operand, so a value where metadata belongs
            // is a syntax error there and a shape error here.
            let wanted = match name.as_str() {
                "dbg_declare" | "dbg_value" => Some(4),
                "dbg_assign" => Some(7),
                "dbg_label" => Some(2),
                // There are four kinds of debug record and no more, so an
                // unrecognised `#dbg_` name is a misspelling rather than a
                // record this parser has not met yet.
                other => {
                    return self.error(format!("#{other} is not a debug record"));
                }
            };
            if let Some(wanted) = wanted {
                if operands.len() != wanted {
                    return self.error(format!(
                        "#{name} takes {wanted} operands, not {}",
                        operands.len()
                    ));
                }
                // `#dbg_assign` carries two values, the assigned one and the
                // address it was assigned to; everything else is metadata.
                let values: &[usize] = if name == "dbg_assign" { &[0, 4] } else { &[0] };
                if operands.iter().enumerate().any(|(position, operand)| {
                    !values.contains(&position)
                        && matches!(operand, llvm_ir::metadata::MdOperand::Value { .. })
                }) {
                    return self.error(format!("#{name} takes metadata where it expects it"));
                }
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
            // A promise that a value is no NaN survives a change of precision
            // and nothing else: `fptosi` produces an integer, which has no
            // NaN to be promised about, and `sitofp` starts from one. So the
            // other casts never read a fast-math word, and upstream reports
            // the type it was looking for where the word stands.
            self.parse_operation_flags(&mut flags, &mut fast_math);
            if !fast_math.is_empty() && !matches!(op, CastOp::FpExt | CastOp::FpTrunc) {
                return self.error("expected type");
            }
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
                    fast_math,
                    operand,
                    source_type,
                },
            ));
        }

        match opcode.as_str() {
            "ret" => {
                // `ret void` returns nothing and `ret void ()* null` returns
                // a pointer to a function, so the word alone does not settle
                // it: what follows does.
                let returns_nothing = self.peek_word() == Some("void")
                    && !matches!(self.peek_at(1), Token::LeftParen | Token::Star)
                    && self.peek_at(1) != &Token::Word("addrspace".to_string());
                if returns_nothing {
                    self.advance();
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
                let swifterror = self.eat_word("swifterror");
                let allocated_type = self.parse_type()?;
                let mut count = None;
                let mut align = None;
                let mut address_space = None;
                while self.eat(&Token::Comma) {
                    if self.eat_word("align") {
                        align = Some(self.parse_align()?);
                    } else if self.peek_word() == Some("addrspace") {
                        address_space = self.parse_optional_address_space()?;
                        // The address space is the last clause the grammar
                        // has, so anything after it is an error rather than
                        // another clause.
                        if self.peek() == &Token::Comma
                            && !matches!(self.peek_at(1), Token::MetadataName(_))
                        {
                            return self.error("an alloca writes nothing after its address space");
                        }
                        break;
                    } else if matches!(self.peek(), Token::MetadataName(_)) {
                        self.index -= 1;
                        break;
                    } else if count.is_some() {
                        return self.error("an alloca counts its elements once");
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
                        swifterror,
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
                // An ordinary load takes its alignment from the data layout
                // when it writes none, but an atomic one says what it is:
                // the target has to know the access is single-copy before it
                // picks an instruction for it.
                if atomic.is_some() && align.is_none() {
                    return self.error("atomic load needs an alignment of its own".to_string());
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
                if atomic.is_some() && align.is_none() {
                    return self.error("atomic store needs an alignment of its own".to_string());
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
                let (second_type, second) = self.parse_typed_value(function, state)?;
                self.require(Token::Comma)?;
                let (mask_type, mask) = self.parse_typed_value(function, state)?;
                // Both halves are shuffled together, so they are the same
                // vector; only one type is kept because there is only one.
                if second_type != vector_type {
                    return self.error("shufflevector shuffles two vectors of different types");
                }
                let ty = self.shuffle_result_type(vector_type, mask_type)?;
                // A mask lane that picks nothing is a lane nobody reads, and
                // upstream spells that `poison` rather than `undef`: the two
                // say the same thing here and it writes back the one that
                // says it of a value that was never chosen.
                let mask = self.poison_the_undef_lanes(mask);
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
                    // The comma before an attachment looks exactly like the
                    // comma before another edge, so a `!dbg` here ends the
                    // list rather than starting one.
                    if !incoming.is_empty() && self.peek() != &Token::LeftBracket {
                        self.index -= 1;
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
                // Upstream reads a call to one of the four `llvm.dbg.*`
                // intrinsics as the record it is the older spelling of, and
                // prints the record back. The location comes from the call's
                // `!dbg` attachment, which the caller moves across once the
                // attachments have been read.
                if let Some(name) = self.debug_record_name(&call) {
                    self.wrote_debug_intrinsic = true;
                    // `llvm.dbg.value` and `llvm.dbg.declare` once took an
                    // offset into the variable between the value and the
                    // variable itself, and the expression took its place.
                    // Upstream drops the argument as it reads, so a call
                    // written the older way becomes the same record as one
                    // written the newer.
                    let args: Vec<_> = call
                        .args
                        .iter()
                        .enumerate()
                        .filter(|(position, arg)| {
                            !(*position == 1
                                && call.args.len() == 4
                                && matches!(
                                    self.module.ctx.type_kind(arg.ty),
                                    TypeKind::Integer(_)
                                ))
                        })
                        .map(|(_, arg)| arg)
                        .collect();
                    let operands = args
                        .iter()
                        .map(|arg| match arg.value {
                            Value::Constant(id) => match self.module.ctx.constant(id).clone() {
                                Constant::Metadata { operand, .. } => *operand,
                                _ => MdOperand::Value {
                                    ty: arg.ty,
                                    value: arg.value,
                                },
                            },
                            value => MdOperand::Value { ty: arg.ty, value },
                        })
                        .collect();
                    let void = self.module.ctx.void_type();
                    return Ok((void, InstKind::DebugRecord { name, operands }));
                }
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

    /// The name of the callee about to be read, when it is an `llvm.*` name
    /// nothing declares and LangRef documents. Upstream materialises a
    /// declaration for exactly that set: an undocumented `llvm.*` name is
    /// still "use of undefined value", and a name it recognises is declared
    /// from the call's own signature.
    fn undeclared_intrinsic(&self) -> Option<Name> {
        let Token::GlobalName(text) = self.peek() else {
            return None;
        };
        let name = Name::Named(text.clone());
        self.implied_intrinsics.contains(&name).then_some(name)
    }

    /// Whether a signature agrees with itself about the types an intrinsic
    /// overloads on.
    ///
    /// LangRef documents an overloaded intrinsic once per instantiation, so
    /// the positions whose types vary together across all of them are one
    /// type. `corpus/intrinsic-overloads.nu` measures which, counting the
    /// result as position nought, and a signature giving two of them
    /// different types describes no instantiation there is.
    fn check_tied_positions(
        &mut self,
        name: &Name,
        result: TypeId,
        params: &[TypeId],
    ) -> Result<(), ParseError> {
        let Name::Named(text) = name else {
            return Ok(());
        };
        let Some((arity, classes)) = llvm_ir::intrinsic::overloads::tied(text) else {
            return Ok(());
        };
        // Measured at one arity, and a call with another is one upstream
        // upgrades from an older spelling rather than one to check here.
        if arity != params.len() + 1 {
            return Ok(());
        }
        let positions: Vec<TypeId> = std::iter::once(result)
            .chain(params.iter().copied())
            .collect();
        for class in classes {
            let mut wanted = None;
            for position in *class {
                let ty = positions[*position];
                match wanted {
                    None => wanted = Some(ty),
                    Some(first) if first == ty => {}
                    Some(_) => return self.error("invalid intrinsic signature"),
                }
            }
        }
        Ok(())
    }

    /// Adds the declarations the calls implied, after everything the module
    /// writes, which is where upstream puts them. The attributes upstream
    /// gives an intrinsic go on afterwards, in `apply_intrinsic_attributes`,
    /// which is the same pass that puts them on a declaration the module
    /// wrote out itself.
    ///
    /// One of these is named the way upstream names it as it is built, and
    /// not renamed afterwards by `remangle_intrinsics`, because the two land
    /// in different places. Upstream materialises a declaration where the
    /// call is read and rewrites a written one at the end of the module, so
    /// an implied `llvm.smax.i8` prints before a renamed
    /// `llvm.lifetime.start.p0` even when the module wrote the second one
    /// first, which is measured.
    pub(crate) fn add_implied_intrinsics(&mut self) {
        for name in self.implied_intrinsics.clone() {
            let Some((return_type, params)) = self.implied_signatures.get(&name).cloned() else {
                continue;
            };
            let mut declaration = Function::new(name, return_type);
            declaration.params = params
                .into_iter()
                .map(|ty| Param {
                    ty,
                    attrs: AttributeSet::default(),
                    name: None,
                })
                .collect();
            let id = self.module.add_function(declaration);
            let index = id.0 as usize;
            if let Some(canonical) = self.canonical_intrinsic_name(index) {
                self.module.functions[index].name = llvm_ir::value::Name::Named(canonical);
            }
        }
    }

    /// The debug record a call is the older spelling of, if it is one.
    fn debug_record_name(&self, call: &CallData) -> Option<String> {
        let Value::Constant(id) = call.callee else {
            return None;
        };
        let Constant::Global { target, .. } = self.module.ctx.constant(id) else {
            return None;
        };
        // The symbol table rather than the function arena: the declaration
        // may sit after the call in the text, so its slot in the arena is
        // reserved and not yet filled.
        [
            ("llvm.dbg.declare", "dbg_declare"),
            ("llvm.dbg.value", "dbg_value"),
            ("llvm.dbg.assign", "dbg_assign"),
            ("llvm.dbg.label", "dbg_label"),
        ]
        .into_iter()
        .find(|(intrinsic, _)| {
            self.symbols.get(&Name::Named((*intrinsic).to_string())) == Some(target)
        })
        .map(|(_, record)| record.to_string())
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
        // An intrinsic needs no declaration: upstream builds one from the
        // call when it recognises the name. The signature is not known until
        // the arguments have been read, so the callee is set aside and
        // resolved after them.
        let undeclared = self.undeclared_intrinsic();
        let callee = self.parse_value(function, state, callee_type)?;

        self.require(Token::LeftParen)?;
        let mut args = Vec::new();
        let mut forwards = false;
        while !self.eat(&Token::RightParen) {
            if !args.is_empty() {
                self.require(Token::Comma)?;
            }
            // `f(%a, ...)` hands the caller's own variable arguments
            // straight through, which only a `musttail` call does and only a
            // function that has some can do.
            if self.eat(&Token::Ellipsis) {
                forwards = true;
                if tail != TailKind::MustTail {
                    return self
                        .error("unexpected ellipsis in argument list for non-musttail call");
                }
                if !function.is_var_arg {
                    return self.error(
                        "unexpected ellipsis in argument list for musttail call in non-varargs \
                         function",
                    );
                }
                continue;
            }
            let ty = self.parse_type()?;
            let attrs = self.parse_attribute_set(false)?;
            let value = self.parse_value(function, state, ty)?;
            args.push(CallArg { ty, attrs, value });
        }
        // The other direction: a musttail call hands the frame over whole, so
        // a caller that has variable arguments has to hand those over too.
        if tail == TailKind::MustTail && function.is_var_arg && !forwards {
            return self.error(
                "expected '...' at end of argument list for musttail call in varargs function",
            );
        }

        // The declaration upstream builds takes its shape from the first
        // call, so the first one to arrive is the one recorded.
        if let Some(name) = undeclared
            && !self.implied_signatures.contains_key(&name)
        {
            let result = match self.module.ctx.type_kind(written_type) {
                TypeKind::Function { result, .. } => *result,
                _ => written_type,
            };
            let params: Vec<TypeId> = args.iter().map(|arg| arg.ty).collect();
            // There has to be a declaration to build, and positions that
            // share one overloaded type disagreeing about what it is leaves
            // none: `llvm.umax(i8, i16)` names no instantiation of
            // `llvm.umax`, which upstream reports here rather than later.
            self.check_tied_positions(&name, result, &params)?;
            self.implied_signatures.insert(name, (result, params));
        }

        let fn_attrs = self.parse_function_attribute_set()?;

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

/// How many operands each kind of debug record takes, counting the location
/// it ends with. A record built from a call to the matching intrinsic is one
/// short until that location is moved across from the call's `!dbg`.
fn debug_record_arity(kind: &InstKind) -> Option<usize> {
    let InstKind::DebugRecord { name, .. } = kind else {
        return None;
    };
    match name.as_str() {
        "dbg_declare" | "dbg_value" => Some(4),
        "dbg_assign" => Some(7),
        "dbg_label" => Some(2),
        _ => None,
    }
}

impl Parser {
    /// Rewrites every `undef` lane of a constant vector to `poison`.
    fn poison_the_undef_lanes(&mut self, mask: Value) -> Value {
        let Value::Constant(id) = mask else {
            return mask;
        };
        let Constant::Vector { ty, elements } = self.module.ctx.constant(id).clone() else {
            return mask;
        };
        let mut lanes = Vec::with_capacity(elements.len());
        let mut changed = false;
        for element in elements {
            match self.module.ctx.constant(element).clone() {
                Constant::Undef(lane) => {
                    changed = true;
                    lanes.push(self.module.ctx.intern_constant(Constant::Poison(lane)));
                }
                _ => lanes.push(element),
            }
        }
        if !changed {
            return mask;
        }
        Value::Constant(self.module.ctx.intern_constant(Constant::Vector {
            ty,
            elements: lanes,
        }))
    }
}
