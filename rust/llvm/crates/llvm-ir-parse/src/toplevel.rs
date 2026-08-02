//! The top-level loop: everything a module holds outside a function body.

use crate::attributes::LegacyMemory;
use crate::lexer::Token;
use crate::{FunctionState, ParseError, Parser};
use llvm_ir::TypeId;
use llvm_ir::constant::Constant;
use llvm_ir::function::Function;
use llvm_ir::global::{Comdat, ComdatKind};
use llvm_ir::intrinsic::table::Parameter;
use llvm_ir::metadata::{MdField, MdOperand, Metadata, NamedMetadata, SpecializedArgs};
use llvm_ir::summary::{SummaryEntry, SummaryField, SummaryValue};
use llvm_ir::types::TypeKind;
use llvm_ir::value::{GlobalRef, MdId, Name, Value};
use llvm_support::{ApInt, DataLayout, Triple};

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
                        self.parse_use_list_order(None)?;
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
    /// A directive names its value with the type that value was defined
    /// with, and upstream says so in two different ways: a local is reported
    /// against its own definition, and a global is reported for not being a
    /// pointer, a symbol reference having the symbol's own pointer type.
    ///
    /// Only the type is checked here. Whether the indexes match the use list
    /// needs the count, which cannot be taken while the module is still
    /// being read.
    fn check_use_list_type(
        &mut self,
        written: TypeId,
        context: Option<(&Function, &FunctionState)>,
    ) -> Result<(), ParseError> {
        match self.peek().clone() {
            Token::GlobalName(_) | Token::GlobalNumber(_)
                if !matches!(self.module.ctx.type_kind(written), TypeKind::Pointer { .. }) =>
            {
                return self.error("global variable reference must have pointer type");
            }
            Token::LocalName(name) => {
                let Some((function, state)) = context else {
                    return Ok(());
                };
                // A name the body has only referred to has a slot reserved
                // and nothing in it yet, so there is no type to compare and
                // nothing to say.
                let defined = state
                    .named_params
                    .get(&name)
                    .and_then(|index| function.params.get(*index as usize))
                    .map(|param| param.ty)
                    .or_else(|| {
                        let id = state.named_values.get(&name)?;
                        Some(function.try_instruction(*id)?.ty)
                    });
                if let Some(defined) = defined
                    && defined != written
                {
                    return self.error(format!(
                        "'%{name}' is defined with a type a uselistorder directive does not name"
                    ));
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn parse_use_list_order(
        &mut self,
        context: Option<(&Function, &FunctionState)>,
    ) -> Result<(), ParseError> {
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
        // The type is read, because it has to be the type the value was
        // defined with, and the rest of what names the value is skipped: it
        // may be a constant expression with commas and parentheses of its
        // own, and nothing else here needs to know which value it was.
        let written = self.parse_type()?;
        self.check_use_list_type(written, context)?;
        let items = 1;
        // The scan stops at the comma before the index list, which is the
        // first one outside any bracket.
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

/// The four module flags whose behaviour upstream rewrites as it reads. Each
/// was once written `Error`, meaning two modules being linked had to agree
/// exactly, and each turned out to want a rule that picks rather than one
/// that refuses: the larger of two PIC levels, the smaller of two PIE levels,
/// the larger of two branch-protection settings.
const REWRITTEN_BEHAVIOUR: &[(&str, u64)] = &[
    ("PIC Level", 8),
    ("PIE Level", 7),
    ("branch-target-enforcement", 8),
    ("sign-return-address", 8),
    ("sign-return-address-all", 8),
    ("sign-return-address-with-bkey", 8),
];

impl Parser {
    /// The attributes upstream gives an intrinsic, put on every declaration
    /// of one.
    ///
    /// They are the intrinsic's rather than the module's: upstream reads
    /// whatever was written and puts its own set there instead, parameter
    /// attributes included, so `declare void @llvm.assume(i1 nonnull) #7`
    /// and a bare `declare void @llvm.assume(i1)` both come back as
    /// `declare void @llvm.assume(i1 noundef)` with the same five function
    /// attributes. `corpus/intrinsic-attributes.nu` measures the table.
    ///
    /// The declaration has to be one upstream recognises, which is where
    /// this is narrower than upstream. A name whose types do not fit is left
    /// alone entirely: `declare void @llvm.assume(i32)` keeps its own
    /// attributes and its own name. Only the positions LangRef pins can be
    /// checked here, so where the fit cannot be told the attributes are left
    /// off rather than guessed at, which leaves a difference where upstream
    /// would have written one and never writes one upstream would not.
    pub(crate) fn apply_intrinsic_attributes(&mut self) -> Result<(), ParseError> {
        for index in 0..self.module.functions.len() {
            let Name::Named(name) = self.module.functions[index].name.clone() else {
                continue;
            };
            let Some(wanted) = llvm_ir::intrinsic::attributes::attributes(&name) else {
                continue;
            };
            if !self.intrinsic_declaration_fits(index, wanted.params.len()) {
                continue;
            }
            let function = self.attribute_set_from_text(wanted.function, true)?;
            let ret = self.attribute_set_from_text(wanted.ret, false)?;
            let mut params = Vec::with_capacity(wanted.params.len());
            for text in wanted.params {
                params.push(self.attribute_set_from_text(text, false)?);
            }
            let target = &mut self.module.functions[index];
            target.attrs = function;
            target.return_attrs = ret;
            for (param, attrs) in target.params.iter_mut().zip(params) {
                param.attrs = attrs;
            }
        }
        Ok(())
    }

    /// Whether a declaration is one upstream would recognise as the
    /// intrinsic its name begins with, which is what decides whether the
    /// attributes are attached at all.
    ///
    /// Arity has to match, an extra argument being enough for upstream to
    /// leave the declaration alone, and so does every position whose type
    /// LangRef states the same way in every instantiation. A position it
    /// leaves open is open here too.
    fn intrinsic_declaration_fits(&self, index: usize, arity: usize) -> bool {
        let function = &self.module.functions[index];
        if function.is_var_arg || function.params.len() != arity {
            return false;
        }
        let Name::Named(name) = &function.name else {
            return false;
        };
        let Some(documented) = llvm_ir::intrinsic::table::signature(name) else {
            // No signature to check against is not a reason to refuse: the
            // arity came from the attribute table, which is measured against
            // the same `declare` lines.
            return true;
        };
        if documented.len() != arity {
            return false;
        }
        documented
            .iter()
            .zip(function.params.iter())
            .all(|(wanted, param)| {
                let kind = self.module.ctx.type_kind(param.ty);
                match wanted {
                    Parameter::Any => true,
                    Parameter::Int(bits) => {
                        matches!(kind, TypeKind::Integer(width) if width == bits)
                    }
                    Parameter::Pointer => matches!(kind, TypeKind::Pointer { .. }),
                    Parameter::Metadata => matches!(kind, TypeKind::Metadata),
                    Parameter::Float => matches!(kind, TypeKind::Float(_)),
                }
            })
    }

    /// Applies that rewrite, after the whole module, `!llvm.module.flags`
    /// being able to name a node the text has not reached yet.
    /// Debug info a reader of this version cannot make sense of, taken out.
    ///
    /// A module says which debug-info format it holds with a module flag, and
    /// upstream drops the lot rather than reading an older one: the `!dbg`
    /// attachments, the debug records and the `llvm.dbg.cu` list. What is
    /// left is ordinary metadata, so a node some other list still names
    /// survives and one only the debug info reached does not.
    pub(crate) fn drop_invalid_debug_info(&mut self) {
        const CURRENT: u64 = 3;
        if self.debug_info_version() == Some(CURRENT) {
            return;
        }
        // An attachment naming a node the module never defines is left
        // where it is, because that is a reference upstream refuses rather
        // than debug info it drops.
        self.module
            .named_metadata
            .retain(|named| named.name != "llvm.dbg.cu");
        // An attachment naming a node the module never defines is left where
        // it is, that being a reference upstream refuses rather than debug
        // info it drops.
        let known: Vec<bool> = self
            .module
            .metadata
            .iter()
            .map(|node| node.is_some())
            .collect();
        let module = known.as_slice();
        for function in &mut self.module.functions {
            function.metadata.retain(|attachment| {
                attachment.kind != "dbg" || !defined(module, &attachment.node)
            });
            let records: Vec<(llvm_ir::BlockId, llvm_ir::InstId)> = function
                .blocks()
                .flat_map(|(block, _)| {
                    function
                        .block_instructions(block)
                        .filter(|(_, instruction)| {
                            matches!(
                                instruction.kind,
                                llvm_ir::instruction::InstKind::DebugRecord { .. }
                            )
                        })
                        .map(move |(id, _)| (block, id))
                        .collect::<Vec<_>>()
                })
                .collect();
            for (block, id) in records {
                function.remove_instruction(block, id);
            }
            let ids: Vec<llvm_ir::InstId> = function
                .blocks()
                .flat_map(|(block, _)| {
                    function
                        .block_instructions(block)
                        .map(|(id, _)| id)
                        .collect::<Vec<_>>()
                })
                .collect();
            for id in ids {
                function.instruction_mut(id).metadata.retain(|attachment| {
                    attachment.kind != "dbg" || !defined(module, &attachment.node)
                });
            }
        }
    }

    /// The version the `Debug Info Version` module flag names, if it names one.
    fn debug_info_version(&self) -> Option<u64> {
        let flags: Vec<MdId> = self
            .module
            .named_metadata
            .iter()
            .filter(|named| named.name == "llvm.module.flags")
            .flat_map(|named| named.operands.clone())
            .collect();
        for id in flags {
            let Some(Metadata::Tuple { operands, .. }) = self.module.metadata_node(id) else {
                continue;
            };
            let [
                _,
                MdOperand::String(name),
                MdOperand::Value {
                    value: Value::Constant(version),
                    ..
                },
            ] = operands.as_slice()
            else {
                continue;
            };
            if name.as_str() != Some("Debug Info Version") {
                continue;
            }
            if let Constant::Integer { value, .. } = self.module.ctx.constant(*version) {
                return value.to_u64();
            }
        }
        None
    }

    /// The two Objective-C module flags upstream rewrites as it reads.
    ///
    /// A module that says which image-info version it was built against is
    /// also saying it has no class properties unless it says otherwise, so
    /// the flag that says so is added. And how the collector is configured
    /// is eight bits wide however wide the module wrote it.
    pub(crate) fn upgrade_objc_module_flags(&mut self) {
        let flags: Vec<MdId> = self.module_flags();
        let mut has_version = false;
        let mut has_properties = false;
        for id in &flags {
            let Some(name) = self.flag_name(*id) else {
                continue;
            };
            match name.as_str() {
                "Objective-C Image Info Version" => has_version = true,
                "Objective-C Class Properties" => has_properties = true,
                "Objective-C Garbage Collection" => self.narrow_flag_value(*id, 8),
                _ => {}
            }
        }
        if !has_version || has_properties {
            return;
        }
        let i32_type = self.module.ctx.int_type(32);
        let behaviour = self.module.ctx.const_int(i32_type, ApInt::from_u64(32, 4));
        let value = self.module.ctx.const_int(i32_type, ApInt::from_u64(32, 0));
        let node = self.module.add_metadata(Metadata::Tuple {
            distinct: false,
            operands: vec![
                MdOperand::Value {
                    ty: i32_type,
                    value: Value::Constant(behaviour),
                },
                MdOperand::String("Objective-C Class Properties".into()),
                MdOperand::Value {
                    ty: i32_type,
                    value: Value::Constant(value),
                },
            ],
        });
        for named in &mut self.module.named_metadata {
            if named.name == "llvm.module.flags" {
                named.operands.push(node);
                return;
            }
        }
    }

    /// The nodes `llvm.module.flags` names.
    fn module_flags(&self) -> Vec<MdId> {
        self.module
            .named_metadata
            .iter()
            .filter(|named| named.name == "llvm.module.flags")
            .flat_map(|named| named.operands.clone())
            .collect()
    }

    /// The name a flag node carries, when it has the three-operand shape.
    fn flag_name(&self, id: MdId) -> Option<String> {
        let Metadata::Tuple { operands, .. } = self.module.metadata_node(id)? else {
            return None;
        };
        let [_, MdOperand::String(name), _] = operands.as_slice() else {
            return None;
        };
        name.as_str().map(str::to_string)
    }

    /// A flag whose value is held in fewer bits than the module wrote it in.
    fn narrow_flag_value(&mut self, id: MdId, bits: u32) {
        let Some(Metadata::Tuple { distinct, operands }) = self.module.metadata_node(id).cloned()
        else {
            return;
        };
        let [
            _,
            _,
            MdOperand::Value {
                value: Value::Constant(written),
                ..
            },
        ] = operands.as_slice()
        else {
            return;
        };
        let Some(value) = self.module.ctx.constant(*written).as_integer().cloned() else {
            return;
        };
        let ty = self.module.ctx.int_type(bits);
        let narrowed = self.module.ctx.const_int(ty, value.trunc(bits));
        let mut operands = operands;
        operands[2] = MdOperand::Value {
            ty,
            value: Value::Constant(narrowed),
        };
        self.module
            .set_metadata(id, Metadata::Tuple { distinct, operands });
    }

    pub(crate) fn upgrade_module_flags(&mut self) {
        let flags: Vec<MdId> = self
            .module
            .named_metadata
            .iter()
            .filter(|named| named.name == "llvm.module.flags")
            .flat_map(|named| named.operands.clone())
            .collect();
        for id in flags {
            let Some(Metadata::Tuple { distinct, operands }) =
                self.module.metadata_node(id).cloned()
            else {
                continue;
            };
            if operands.len() != 3 {
                continue;
            }
            let MdOperand::String(name) = &operands[1] else {
                continue;
            };
            let Some((_, wanted)) = REWRITTEN_BEHAVIOUR
                .iter()
                .find(|(flag, _)| name.as_str() == Some(*flag))
            else {
                continue;
            };
            let MdOperand::Value {
                ty,
                value: Value::Constant(behaviour),
            } = operands[0].clone()
            else {
                continue;
            };
            let Some(written) = self
                .module
                .ctx
                .constant(behaviour)
                .as_integer()
                .and_then(ApInt::to_u64)
            else {
                continue;
            };
            // `Error` is the one that is rewritten, and `PIC Level` takes
            // `Min` as well: two PIC levels are picked between by taking the
            // larger, whichever of the two rules a module wrote.
            let picks_the_larger = name.as_str() == Some("PIC Level");
            let rewrite = written == 1 || (written == 7 && picks_the_larger);
            if !rewrite {
                continue;
            }
            let replacement = self.module.ctx.intern_constant(Constant::Integer {
                ty,
                value: ApInt::from_u64(32, *wanted),
            });
            let mut operands = operands;
            operands[0] = MdOperand::Value {
                ty,
                value: Value::Constant(replacement),
            };
            self.module
                .set_metadata(id, Metadata::Tuple { distinct, operands });
        }
    }
}

impl Parser {
    /// A node is uniqued by what it holds, so a node that holds itself cannot
    /// be: asking whether two of them are equal would never finish. Upstream
    /// makes one distinct whether or not the module wrote the word. Two nodes
    /// that name each other are not this and stay as they were.
    pub(crate) fn mark_self_referencing_distinct(&mut self) {
        let ids: Vec<MdId> = self.module.metadata_nodes().map(|(id, _)| id).collect();
        for id in ids {
            let Some(node) = self.module.metadata_node(id).cloned() else {
                continue;
            };
            if node.is_distinct() || !names_itself(&node, id) {
                continue;
            }
            let replacement = match node {
                Metadata::Tuple { operands, .. } => Metadata::Tuple {
                    distinct: true,
                    operands,
                },
                Metadata::Specialized { tag, args, .. } => Metadata::Specialized {
                    distinct: true,
                    tag,
                    args,
                },
                Metadata::String(_) => continue,
            };
            self.module.set_metadata(id, replacement);
        }
    }
}

/// Whether a node names the id it is stored under.
fn names_itself(node: &Metadata, id: MdId) -> bool {
    match node {
        Metadata::Tuple { operands, .. } => operands
            .iter()
            .any(|operand| matches!(operand, MdOperand::Ref(named) if *named == id)),
        Metadata::Specialized { args, .. } => {
            let fields: Vec<&MdField> = match args {
                SpecializedArgs::Named(fields) => fields.iter().map(|(_, value)| value).collect(),
                SpecializedArgs::Positional(values) => values.iter().collect(),
            };
            fields
                .iter()
                .any(|field| matches!(field, MdField::Ref(named) if *named == id))
        }
        Metadata::String(_) => false,
    }
}

/// Whether an attachment names a node the module defines. A node written in
/// place is its own definition; a number has to have one.
fn defined(known: &[bool], node: &llvm_ir::metadata::MdRef) -> bool {
    match node {
        llvm_ir::metadata::MdRef::Inline(_) => true,
        llvm_ir::metadata::MdRef::Id(id) => known.get(id.0 as usize).copied().unwrap_or(false),
    }
}
