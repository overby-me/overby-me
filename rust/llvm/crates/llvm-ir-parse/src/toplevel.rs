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

/// What upstream says about a use list and the indexes written for it, or
/// `None` when it says nothing.
///
/// The indexes are a permutation of the list, so there is one per use and
/// each is a position in it. Upstream has a message of its own for the two
/// counts that cannot be permuted at all. Both a value's use list and a
/// block's are read this way; only the counting differs.
fn use_list_verdict(uses: usize, indexes: &[u64]) -> Option<String> {
    if uses == 0 {
        return Some("value has no uses".to_string());
    }
    if uses == 1 {
        return Some("value only has one use".to_string());
    }
    if indexes.len() != uses {
        return Some(format!("wrong number of indexes, expected {uses}"));
    }
    if indexes.iter().any(|index| *index as usize >= uses) {
        return Some("expected distinct uselistorder indexes in range [0, size)".to_string());
    }
    None
}

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
            // A name nothing defines has no uses by construction, which is
            // what upstream says about it rather than reporting the symbol
            // as undefined.
            Token::GlobalName(name) if !self.symbols.contains_key(&Name::Named(name.clone())) => {
                return self.error("value has no uses");
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
                .find(|id| target.block(**id).name.as_ref() == Some(&Name::Named(block.clone())))
                .copied();
            let Some(defined) = defined else {
                return self.error("uselistorder_bb names a block its function does not define");
            };
            self.require(Token::Comma)?;
            let at = self.position();
            let indexes = self.parse_use_list_indexes()?;
            // Checked after the module rather than here: a block whose
            // address is taken below this directive is not used yet.
            self.block_use_list_orders
                .push((at, function, defined, indexes));
            return Ok(());
        }
        // The type is read, because it has to be the type the value was
        // defined with.
        let written = self.parse_type()?;
        self.check_use_list_type(written, context)?;
        let named = self.peek().clone();
        let at = self.position();
        // At the top level the value is a constant, and reading it is what
        // lets the indexes be checked against its use count afterwards. A
        // form the constant parser does not take is skipped the way the
        // whole reference used to be, so a shape this does not model costs
        // the check rather than the module.
        let target = if context.is_none() {
            let saved = self.index;
            match self.parse_constant(written) {
                Ok(id) => Some(id),
                Err(_) => {
                    self.index = saved;
                    None
                }
            }
        } else {
            None
        };
        let items = usize::from(target.is_none());
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
        let indexes = self.parse_use_list_indexes()?;
        // Checked after the module rather than here: a global used by a
        // later function is not yet used while the text is being read.
        if let Some(target) = target {
            self.use_list_orders.push((at, target, indexes));
        } else if let Some((function, state)) = context {
            self.check_local_use_list(function, state, written, &named, &indexes)?;
        }
        Ok(())
    }

    /// A local named by a directive, against the uses its own function
    /// makes of it.
    ///
    /// This needs no waiting the way a constant does: a body's directives
    /// come after every instruction in it, so the function is whole by the
    /// time one is read, and nothing outside the function can use a local.
    fn check_local_use_list(
        &mut self,
        function: &Function,
        state: &FunctionState,
        written: TypeId,
        name: &Token,
        indexes: &[u64],
    ) -> Result<(), ParseError> {
        // `uselistorder label %block` names a block, whose use list holds
        // the terminator slots that reach it rather than the operand slots
        // that read a value.
        //
        // A body's directive is checked where it is written, so it counts
        // what the module has read so far: a `blockaddress` below the
        // function is not a use yet, and upstream answers the same file the
        // same way.
        if matches!(self.module.ctx.type_kind(written), TypeKind::Label) {
            // The name to look a `blockaddress` up by is the one the
            // directive wrote, not the one the block stored: a block
            // labelled `1:` keeps no name and answers to its slot number,
            // which is what an address naming it writes too.
            let (block, label) = match name {
                Token::LocalName(text) => (
                    state.named_blocks.get(text).copied(),
                    Some(Name::Named(text.clone())),
                ),
                Token::LocalNumber(number) => (
                    state.numbered_blocks.get(number).copied(),
                    Some(Name::Number(*number)),
                ),
                _ => (None, None),
            };
            let Some(block) = block else {
                return self.error("value has no uses");
            };
            let mut uses = function.block_uses(block);
            let symbol = self.symbols.get(&function.name).copied();
            if let (Some(symbol), Some(label)) = (symbol, label)
                && self
                    .module
                    .block_address_used(symbol, &label, Some(function))
            {
                uses += 1;
            }
            return match use_list_verdict(uses, indexes) {
                Some(message) => self.error(message),
                None => Ok(()),
            };
        }
        let target = match name {
            Token::LocalName(text) => state
                .named_params
                .get(text)
                .map(|index| Value::Argument(*index))
                .or_else(|| {
                    state
                        .named_values
                        .get(text)
                        .map(|id| Value::Instruction(*id))
                }),
            Token::LocalNumber(number) => state
                .numbered_params
                .get(number)
                .map(|index| Value::Argument(*index))
                .or_else(|| {
                    state
                        .numbered_values
                        .get(number)
                        .map(|id| Value::Instruction(*id))
                }),
            _ => None,
        };
        let Some(target) = target else {
            // A local nothing defines has no uses, which is what upstream
            // says about it. Only a local: anything else here is a constant
            // and the module-wide count answers for it.
            if matches!(name, Token::LocalName(_) | Token::LocalNumber(_)) {
                return self.error("value has no uses");
            }
            return Ok(());
        };
        match use_list_verdict(function.value_uses(target), indexes) {
            Some(message) => self.error(message),
            None => Ok(()),
        }
    }

    /// Every directive that named a constant, against the use count the
    /// whole module gives it.
    ///
    /// The indexes are a permutation of the use list, so there is one per
    /// use and each is a position in it. Upstream has a message of its own
    /// for the two counts that cannot be permuted at all.
    pub(crate) fn check_use_list_orders(&mut self) -> Result<(), ParseError> {
        for (at, target, indexes) in std::mem::take(&mut self.use_list_orders) {
            // Constant data is shared across the context rather than owned
            // by the module, so it has no use list to permute and upstream
            // takes any indexes at all for one.
            if !self.module.ctx.constant(target).has_use_list() {
                continue;
            }
            if let Some(message) = use_list_verdict(self.module.use_count(target), &indexes) {
                return Err(ParseError {
                    position: at,
                    message,
                });
            }
        }
        // `uselistorder_bb` is written at the top level, after every
        // function, so its block is counted against the whole module: a
        // `blockaddress` anywhere reaches it, wherever it was written.
        for (at, function, block, indexes) in std::mem::take(&mut self.block_use_list_orders) {
            let target = self.module.function(function);
            let mut uses = target.block_uses(block);
            if let Some(label) = target.block(block).name.clone()
                && self
                    .module
                    .block_address_used(GlobalRef::Function(function), &label, None)
            {
                uses += 1;
            }
            if let Some(message) = use_list_verdict(uses, &indexes) {
                return Err(ParseError {
                    position: at,
                    message,
                });
            }
        }
        Ok(())
    }

    /// The `{ 1, 0 }` half of a directive.
    ///
    /// The indexes are a permutation of a use list, so they are distinct and
    /// they change the order: upstream refuses `{ 0, 1 }` for saying nothing.
    /// Whether they cover the list is checked afterwards, the count not being
    /// available until the whole module is read.
    fn parse_use_list_indexes(&mut self) -> Result<Vec<u64>, ParseError> {
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
        Ok(indexes)
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

    /// Gives a module that names a target the data layout that target uses.
    ///
    /// A module writing a triple and no layout gets one anyway: upstream
    /// fills in the target's, so `target triple = "x86_64-unknown-linux-gnu"`
    /// alone comes back carrying `target datalayout = "e-m:e-p270:32:32-..."`.
    /// That matters twice over, because the layout is also what the default
    /// alignments are read from, so a module without one prints different
    /// alignments as well as a missing line.
    ///
    /// `corpus/target-data-layouts.nu` measures which layout each triple
    /// implies, one module per triple. A triple with no row is left alone,
    /// as is a module that wrote a layout of its own: upstream does not
    /// replace one that is already there, which is what the differential
    /// check records about `target datalayout = "e"` surviving `opt -S`.
    pub(crate) fn fill_data_layout_from_triple(&mut self) {
        if self.module.data_layout.is_some() {
            return;
        }
        let Some(triple) = &self.module.triple else {
            return;
        };
        let Some(text) = llvm_ir::target_layout::layout_for(triple.as_str()) else {
            return;
        };
        // A layout that came out of upstream and will not parse is a bug in
        // the reader rather than in the module, and dropping it leaves the
        // module exactly as it was rather than refusing it.
        if let Ok(layout) = DataLayout::parse(text) {
            self.module.data_layout = Some(layout);
        }
    }

    /// Gives an intrinsic declaration the name upstream gives it.
    ///
    /// An overloaded intrinsic carries the types it was instantiated at in
    /// its own name, and a module may write the name without them: upstream
    /// reads `declare void @llvm.lifetime.start(i64, ptr)` and prints
    /// `@llvm.lifetime.start.p0`. The ones written before opaque pointers are
    /// the common case, those names carrying every component except the ones
    /// a typed pointer used to imply, which is why this rebuilds the whole
    /// suffix rather than only filling an empty one in.
    ///
    /// `corpus/intrinsic-mangling.nu` measures which positions go in, one
    /// intrinsic at a time, and holds every row against the thirty-seven
    /// thousand intrinsic declarations in upstream's own tests. A name with
    /// no row keeps whatever the module wrote.
    ///
    /// A renamed declaration also moves: upstream builds a new function and
    /// erases the old, so it prints after everything the module wrote and
    /// after the declarations the calls implied, which is measured. The move
    /// is recorded as a print order rather than performed on the arena,
    /// because an id is what every constant naming the function holds.
    pub(crate) fn remangle_intrinsics(&mut self) {
        // Every canonical name first, because whether one is taken depends on
        // what the others end up called rather than on what they are called
        // now. `Assembler/remangle.ll` is two declarations whose canonical
        // names are each other's, and upstream swaps them; a module writing
        // two spellings of one intrinsic instead has one of them merged away.
        // What tells those apart is whether the name is held by a declaration
        // that keeps it.
        let wanted: Vec<Option<Name>> = (0..self.module.functions.len())
            .map(|index| {
                let canonical = Name::Named(self.canonical_intrinsic_name(index)?);
                (canonical != self.module.functions[index].name).then_some(canonical)
            })
            .collect();
        let keeps: Vec<Name> = (0..self.module.functions.len())
            .map(|index| {
                wanted[index]
                    .clone()
                    .unwrap_or_else(|| self.module.functions[index].name.clone())
            })
            .collect();
        let mut moved = Vec::new();
        let mut merged = Vec::new();
        for index in 0..self.module.functions.len() {
            let Some(canonical) = wanted[index].clone() else {
                continue;
            };
            // A declaration that already has the name and is keeping it is
            // the one upstream would have found, so this one is merged into
            // it. Failing that, an earlier declaration wanting the same name
            // gets it and the later ones merge into that.
            let holder = (0..self.module.functions.len()).find(|other| {
                *other != index
                    && wanted[*other].is_none()
                    && self.module.functions[*other].name == canonical
            });
            let claimant =
                holder.or_else(|| (0..index).find(|earlier| keeps[*earlier] == canonical));
            if claimant.is_some() {
                merged.push((index, canonical));
                continue;
            }
            self.module.functions[index].name = canonical;
            // One upstream rebuilt for its arity is where that pass put it:
            // a declaration is built anew once, however many things about it
            // the reader changed.
            if !self
                .rebuilt_declarations
                .contains(&llvm_ir::value::FunctionId(index as u32))
            {
                moved.push(llvm_ir::value::FunctionId(index as u32));
            }
        }
        for (index, canonical) in &merged {
            self.merge_intrinsic_declaration(*index, canonical);
        }
        if moved.is_empty() && merged.is_empty() {
            return;
        }
        let dropped: Vec<llvm_ir::value::FunctionId> = merged
            .iter()
            .map(|(index, _)| llvm_ir::value::FunctionId(*index as u32))
            .collect();
        let mut order: Vec<llvm_ir::value::FunctionId> = self
            .module
            .function_print_order()
            .into_iter()
            .map(|index| llvm_ir::value::FunctionId(index as u32))
            .filter(|id| !moved.contains(id) && !dropped.contains(id))
            .collect();
        order.extend(moved);
        self.module.function_order = order;
    }

    /// Gives a call written before its intrinsic gained a parameter the
    /// argument upstream gives it, and drops the intrinsics upstream drops.
    ///
    /// `declare i8 @llvm.ctlz.i8(i8)` is the spelling from before that
    /// intrinsic took a second argument, and upstream reads it as the current
    /// one: the declaration gains the parameter and every call gains an
    /// `i1 false`. `llvm.stackprotectorcheck` goes the other way and is
    /// removed, call and declaration together.
    ///
    /// `crates/llvm-ir/src/intrinsic/arity.rs` is the table and
    /// `corpus/intrinsic-arity.nu` explains how it was measured. This runs
    /// before the renaming, because the mangling table is keyed on an
    /// intrinsic's current arity and would leave a declaration at the older
    /// one alone.
    pub(crate) fn upgrade_intrinsic_arity(&mut self) {
        let mut dropped = Vec::new();
        let mut moved = Vec::new();
        for index in 0..self.module.functions.len() {
            let function = &self.module.functions[index];
            if function.blocks().next().is_some() {
                continue;
            }
            let Name::Named(name) = function.name.clone() else {
                continue;
            };
            if llvm_ir::intrinsic::arity::is_dropped(&name) {
                dropped.push(llvm_ir::value::FunctionId(index as u32));
                continue;
            }
            let Some(added) = llvm_ir::intrinsic::arity::upgrade(&name, function.params.len())
            else {
                continue;
            };
            let mut arguments = Vec::with_capacity(added.len());
            for text in added {
                // A row whose value this cannot build leaves the declaration
                // as it was, which is the same answer as having no row.
                let Some(argument) = self.appended_argument(text) else {
                    arguments.clear();
                    break;
                };
                arguments.push(argument);
            }
            if arguments.is_empty() {
                continue;
            }
            for (ty, _) in &arguments {
                self.module.functions[index]
                    .params
                    .push(llvm_ir::function::Param {
                        ty: *ty,
                        attrs: llvm_ir::attribute::AttributeSet::default(),
                        name: None,
                    });
            }
            self.append_call_arguments(llvm_ir::value::FunctionId(index as u32), &arguments);
            moved.push(llvm_ir::value::FunctionId(index as u32));
            self.rebuilt_declarations
                .push(llvm_ir::value::FunctionId(index as u32));
        }
        for id in &dropped {
            self.drop_calls_to(*id);
        }
        if dropped.is_empty() && moved.is_empty() {
            return;
        }
        // Upstream rebuilds a declaration it upgrades rather than editing it,
        // so it prints after everything the module wrote, the way a renamed
        // one does. One it drops prints nowhere at all.
        let mut order: Vec<llvm_ir::value::FunctionId> = self
            .module
            .function_print_order()
            .into_iter()
            .map(|index| llvm_ir::value::FunctionId(index as u32))
            .filter(|id| !dropped.contains(id) && !moved.contains(id))
            .collect();
        order.extend(moved);
        self.module.function_order = order;
    }

    /// One argument upstream appends, read out of the table's own text.
    ///
    /// The shapes measured are an integer and a token, which is every value
    /// upstream synthesises: `i1 false`, `i1 0`, `i32 0`, `token none`.
    fn appended_argument(&mut self, text: &str) -> Option<(llvm_ir::TypeId, Value)> {
        let (spelling, value) = text.split_once(' ')?;
        if spelling == "token" && value == "none" {
            let ty = self.module.ctx.token_type();
            let constant = self.module.ctx.intern_constant(Constant::NoneToken(ty));
            return Some((ty, Value::Constant(constant)));
        }
        let bits: u32 = spelling.strip_prefix('i')?.parse().ok()?;
        let number: u64 = match value {
            "false" => 0,
            "true" => 1,
            other => other.parse().ok()?,
        };
        let ty = self.module.ctx.int_type(bits);
        let constant = self.module.ctx.intern_constant(Constant::Integer {
            ty,
            value: llvm_support::ApInt::from_u64(bits, number),
        });
        Some((ty, Value::Constant(constant)))
    }

    /// Appends the arguments to every call of one function.
    fn append_call_arguments(
        &mut self,
        callee: llvm_ir::value::FunctionId,
        arguments: &[(llvm_ir::TypeId, Value)],
    ) {
        // The call carries the callee's function type as well as its
        // arguments, and a call whose two disagree is one nothing reads.
        let mut types: Vec<(llvm_ir::TypeId, llvm_ir::TypeId)> = Vec::new();
        for index in 0..self.module.functions.len() {
            let blocks: Vec<llvm_ir::BlockId> = self.module.functions[index]
                .blocks()
                .map(|(id, _)| id)
                .collect();
            for block in blocks {
                let instructions = self.module.functions[index]
                    .block(block)
                    .instructions
                    .clone();
                for id in instructions {
                    if self.callee_pointer_type(index, id, callee).is_none() {
                        continue;
                    }
                    let written = match &self.module.functions[index].instruction(id).kind {
                        llvm_ir::instruction::InstKind::Call(call)
                        | llvm_ir::instruction::InstKind::Invoke { call, .. }
                        | llvm_ir::instruction::InstKind::CallBr { call, .. } => call.function_type,
                        _ => continue,
                    };
                    if types.iter().any(|(from, _)| *from == written) {
                        continue;
                    }
                    let TypeKind::Function {
                        result,
                        params,
                        is_var_arg,
                    } = self.module.ctx.type_kind(written).clone()
                    else {
                        continue;
                    };
                    let mut params = params;
                    params.extend(arguments.iter().map(|(ty, _)| *ty));
                    let widened = self.module.ctx.function_type(result, params, is_var_arg);
                    types.push((written, widened));
                }
            }
        }
        self.rewrite_calls_to(callee, &mut |call| {
            if let Some((_, widened)) = types.iter().find(|(from, _)| *from == call.function_type) {
                call.function_type = *widened;
            }
            for (ty, value) in arguments {
                call.args.push(llvm_ir::instruction::CallArg {
                    ty: *ty,
                    attrs: llvm_ir::attribute::AttributeSet::default(),
                    value: *value,
                });
            }
        });
    }

    /// Removes every call to one function, which is what upstream does with
    /// an intrinsic it no longer has. Only a call returning nothing is ever
    /// removed, so there is no result left with nothing to read.
    fn drop_calls_to(&mut self, callee: llvm_ir::value::FunctionId) {
        for index in 0..self.module.functions.len() {
            let blocks: Vec<llvm_ir::BlockId> = self.module.functions[index]
                .blocks()
                .map(|(id, _)| id)
                .collect();
            for block in blocks {
                let instructions = self.module.functions[index]
                    .block(block)
                    .instructions
                    .clone();
                let kept: Vec<llvm_ir::InstId> = instructions
                    .iter()
                    .filter(|id| self.callee_pointer_type(index, **id, callee).is_none())
                    .copied()
                    .collect();
                if kept.len() != instructions.len() {
                    self.module.functions[index].block_mut(block).instructions = kept;
                }
            }
        }
    }

    /// Runs a change over every call to one function.
    fn rewrite_calls_to(
        &mut self,
        callee: llvm_ir::value::FunctionId,
        change: &mut dyn FnMut(&mut llvm_ir::instruction::CallData),
    ) {
        for index in 0..self.module.functions.len() {
            let blocks: Vec<llvm_ir::BlockId> = self.module.functions[index]
                .blocks()
                .map(|(id, _)| id)
                .collect();
            for block in blocks {
                let instructions = self.module.functions[index]
                    .block(block)
                    .instructions
                    .clone();
                for id in instructions {
                    if self.callee_pointer_type(index, id, callee).is_none() {
                        continue;
                    }
                    let instruction = self.module.functions[index].instruction_mut(id);
                    match &mut instruction.kind {
                        llvm_ir::instruction::InstKind::Call(call)
                        | llvm_ir::instruction::InstKind::Invoke { call, .. }
                        | llvm_ir::instruction::InstKind::CallBr { call, .. } => change(call),
                        _ => {}
                    }
                }
            }
        }
    }

    /// Points every call to one spelling of an intrinsic at the function the
    /// other spelling made.
    ///
    /// Upstream builds one function per name, so a module declaring both
    /// `@llvm.aarch64.thread.pointer` and `@llvm.arm.thread.pointer` comes
    /// back with a single `@llvm.thread.pointer.p0` that both call sites
    /// name. Only calls are redirected, which is all there is: an intrinsic's
    /// address may not be taken, so nothing else can hold one.
    ///
    /// The merged declaration stays in the arena and is left out of the print
    /// order. Removing the function would move every id after it, and an id
    /// is what a constant naming one holds.
    fn merge_intrinsic_declaration(&mut self, from: usize, canonical: &Name) {
        let Some(to) = self
            .module
            .functions
            .iter()
            .position(|function| function.name == *canonical)
        else {
            return;
        };
        let from = llvm_ir::value::FunctionId(from as u32);
        let to = llvm_ir::value::FunctionId(to as u32);
        for index in 0..self.module.functions.len() {
            let blocks: Vec<llvm_ir::BlockId> = self.module.functions[index]
                .blocks()
                .map(|(id, _)| id)
                .collect();
            for block in blocks {
                let instructions = self.module.functions[index]
                    .block(block)
                    .instructions
                    .clone();
                for id in instructions {
                    let Some(ty) = self.callee_pointer_type(index, id, from) else {
                        continue;
                    };
                    let constant = self.module.ctx.intern_constant(Constant::Global {
                        target: GlobalRef::Function(to),
                        ty,
                    });
                    let instruction = self.module.functions[index].instruction_mut(id);
                    match &mut instruction.kind {
                        llvm_ir::instruction::InstKind::Call(call)
                        | llvm_ir::instruction::InstKind::Invoke { call, .. }
                        | llvm_ir::instruction::InstKind::CallBr { call, .. } => {
                            call.callee = Value::Constant(constant);
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    /// The type of the constant a call names its callee through, when that
    /// callee is this function and nothing else.
    fn callee_pointer_type(
        &self,
        index: usize,
        id: llvm_ir::InstId,
        callee: llvm_ir::value::FunctionId,
    ) -> Option<llvm_ir::TypeId> {
        let instruction = self.module.functions[index].instruction(id);
        let call = match &instruction.kind {
            llvm_ir::instruction::InstKind::Call(call)
            | llvm_ir::instruction::InstKind::Invoke { call, .. }
            | llvm_ir::instruction::InstKind::CallBr { call, .. } => call,
            _ => return None,
        };
        let Value::Constant(constant) = call.callee else {
            return None;
        };
        let Constant::Global { target, ty } = self.module.ctx.constant(constant) else {
            return None;
        };
        (*target == GlobalRef::Function(callee)).then_some(*ty)
    }

    /// Reads the calls upstream reads as an instruction rather than as a
    /// call.
    ///
    /// `@llvm.nvvm.atomic.load.inc.32.p0(ptr %p, i32 %v)` is
    /// `atomicrmw uinc_wrap ptr %p, i32 %v seq_cst`, and the declaration goes
    /// with it. `crates/llvm-ir/src/intrinsic/rewrites.rs` holds the four and
    /// how each was measured.
    ///
    /// The declaration's own types are not consulted, which is the whole
    /// point: `auto_upgrade_nvvm_intrinsics.ll` declares
    /// `i32 @llvm.nvvm.atomic.load.add.f32.p0(ptr, float)` and calls it
    /// returning `float`, and by the time anything checks, the call is an
    /// `atomicrmw` whose type came from the value it was given. That module
    /// was the last one upstream reads and we refused.
    ///
    /// The result loses its name, upstream building a fresh instruction
    /// rather than editing the one that was there, so `%r = call ...` comes
    /// back as `%1 = atomicrmw ...`.
    pub(crate) fn rewrite_intrinsic_calls(&mut self) {
        let layout = self.module.data_layout.clone().unwrap_or_default();
        for index in 0..self.module.functions.len() {
            let blocks: Vec<llvm_ir::BlockId> = self.module.functions[index]
                .blocks()
                .map(|(id, _)| id)
                .collect();
            for block in blocks {
                let instructions = self.module.functions[index]
                    .block(block)
                    .instructions
                    .clone();
                for id in instructions {
                    let Some((op, pointer, value, value_type)) = self.rewritten_call(index, id)
                    else {
                        continue;
                    };
                    // Upstream writes the alignment out rather than leaving it to
                    // the reader, and it is the value type's own.
                    let align = llvm_ir::layout::abi_align(&self.module.ctx, &layout, value_type)
                        .unwrap_or(llvm_support::Align::ONE);
                    let instruction = self.module.functions[index].instruction_mut(id);
                    instruction.name = None;
                    instruction.ty = value_type;
                    instruction.kind = llvm_ir::instruction::InstKind::AtomicRmw {
                        op,
                        pointer,
                        value_type,
                        value,
                        volatile: false,
                        scope: llvm_ir::instruction::SyncScope::system(),
                        ordering: llvm_ir::instruction::AtomicOrdering::SeqCst,
                        align: Some(align),
                    };
                }
            }
        }
    }

    /// What one call is read as, when it is read as a read-modify-write.
    fn rewritten_call(
        &self,
        index: usize,
        id: llvm_ir::InstId,
    ) -> Option<(
        llvm_ir::instruction::AtomicRmwOp,
        Value,
        Value,
        llvm_ir::TypeId,
    )> {
        let instruction = self.module.functions[index].instruction(id);
        let call = match &instruction.kind {
            llvm_ir::instruction::InstKind::Call(call) => call,
            _ => return None,
        };
        let Value::Constant(callee) = call.callee else {
            return None;
        };
        let Constant::Global { target, .. } = self.module.ctx.constant(callee) else {
            return None;
        };
        let GlobalRef::Function(function) = target else {
            return None;
        };
        let Name::Named(name) = &self.module.function(*function).name else {
            return None;
        };
        let op = llvm_ir::intrinsic::rewrites::atomic_rmw_op(name)?;
        // A pointer and the value to combine with what it holds, and nothing
        // else. A call written with any other shape is not one of these.
        let [pointer, value] = call.args.as_slice() else {
            return None;
        };
        Some((op, pointer.value, value.value, value.ty))
    }

    /// Puts the declarations the calls implied in the order upstream prints
    /// them: by the name the module wrote, and among the ones that wrote the
    /// same name, by the name each ended up with.
    ///
    /// Both halves are measured. Five intrinsics called in reverse
    /// alphabetical order come back alphabetical, which is the first. And
    /// `@llvm.umax` called at `i8` and at `i16` beside `@llvm.umax.i32`
    /// comes back `i16`, `i8`, `i32`: the two that wrote `llvm.umax` sort
    /// before the one that wrote `llvm.umax.i32`, and between themselves
    /// `llvm.umax.i16` sorts before `llvm.umax.i8`.
    ///
    /// This runs after the renaming, which moves what it renames to the end.
    /// The two sets never overlap, an implied declaration being built with
    /// the name it will keep, so the implied ones are sorted where they
    /// already sit rather than moved.
    pub(crate) fn sort_implied_declarations(&mut self) {
        let mut written: Vec<(llvm_ir::value::FunctionId, Name)> = self
            .implied_intrinsics
            .iter()
            .enumerate()
            .map(|(offset, name)| {
                (
                    llvm_ir::value::FunctionId(self.first_implied_id.0 + offset as u32),
                    name.clone(),
                )
            })
            .collect();
        written.extend(
            self.extra_implied
                .iter()
                .map(|(name, id, _, _)| (*id, name.clone())),
        );
        if written.len() < 2 {
            return;
        }
        let mut order = self.module.function_print_order();
        let mut places: Vec<usize> = Vec::new();
        for (place, index) in order.iter().enumerate() {
            if written.iter().any(|(id, _)| id.0 as usize == *index) {
                places.push(place);
            }
        }
        if places.len() < 2 {
            return;
        }
        let mut sorted: Vec<usize> = places.iter().map(|place| order[*place]).collect();
        sorted.sort_by_cached_key(|index| {
            let wrote = written
                .iter()
                .find(|(id, _)| id.0 as usize == *index)
                .map(|(_, name)| name.clone());
            let ended = self.module.functions[*index].name.clone();
            (name_text(wrote), name_text(Some(ended)))
        });
        for (place, index) in places.into_iter().zip(sorted) {
            order[place] = index;
        }
        self.module.function_order = order
            .into_iter()
            .map(|index| llvm_ir::value::FunctionId(index as u32))
            .collect();
    }

    /// The name upstream reads this one as, when it reads it as another.
    ///
    /// Some intrinsics are read under an older spelling and written under
    /// the current one: `llvm.aarch64.thread.pointer` becomes
    /// `llvm.thread.pointer` and `llvm.arm.neon.vclz` becomes `llvm.ctlz`.
    /// The components the name was written with come along, the rename being
    /// of the intrinsic rather than of the instantiation.
    ///
    /// `corpus/intrinsic-renames.nu` measures it, and had to be told the
    /// difference between a rename and a remangling: both look the same from
    /// outside, one name going in and another coming out, so it compares the
    /// two names with their instantiation types dropped. Before it did,
    /// `llvm.smax.v4i32` counted as renamed, twice and differently.
    fn upgraded_intrinsic_name(&self, name: &str) -> Option<String> {
        let base = llvm_ir::intrinsic::base_name(name);
        let upgraded = llvm_ir::intrinsic::renames::renamed(base)?;
        Some(format!("{upgraded}{}", &name[base.len()..]))
    }

    /// The name the mangling table builds for this declaration, or `None`
    /// when nothing measured says what it should be.
    pub(crate) fn canonical_intrinsic_name(&self, index: usize) -> Option<String> {
        let function = &self.module.functions[index];
        // A definition is not an intrinsic: upstream refuses a body on one.
        if function.is_var_arg || function.blocks().next().is_some() {
            return None;
        }
        let Name::Named(name) = &function.name else {
            return None;
        };
        // An older spelling becomes the current one first, the components
        // hanging off whatever the intrinsic ends up being called.
        // `llvm.wasm.laneselect.v16i8` is `llvm.wasm.relaxed.laneselect`
        // instantiated at `v16i8`, and asking the mangling table about the
        // name as written would be asking about an intrinsic that no longer
        // exists under it.
        let upgraded = self.upgraded_intrinsic_name(name);
        let written = upgraded.as_deref().unwrap_or(name);
        let Some((base, _, positions)) =
            llvm_ir::intrinsic::mangling::positions(written, function.params.len() + 1)
        else {
            // A rename with no mangling row still stands on its own, and so
            // does one whose row was measured at another arity.
            return upgraded;
        };
        // The same gate the attributes go through: a declaration whose types
        // do not fit the documented ones is not the intrinsic upstream would
        // recognise, and upstream leaves what it does not recognise alone.
        if !self.intrinsic_declaration_fits(index, function.params.len()) {
            return upgraded;
        }
        let mut name = base.to_string();
        for position in positions {
            let ty = match position {
                0 => function.return_type,
                other => function.params.get(other - 1)?.ty,
            };
            name.push('.');
            name.push_str(&llvm_ir::intrinsic::mangle::mangled_type(
                &self.module.ctx,
                ty,
            )?);
        }
        Some(name)
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
            // LangRef and the assembler disagree about how many arguments
            // three intrinsics take, and the assembler is what upstream is.
            // `llvm.ptr.annotation` and `llvm.var.annotation` are documented
            // with four arguments and upstream refuses that form outright,
            // "Callsite was not defined with variable arguments!", while it
            // renames the five-argument one to `llvm.ptr.annotation.p0.p0`
            // and gives it its attributes. `llvm.donothing` is the other way
            // round, documented with an argument and recognised with none.
            // A signature of another arity than the measured one is stale,
            // so it says nothing rather than refusing.
            return llvm_ir::intrinsic::attributes::attributes(name)
                .is_some_and(|measured| measured.params.len() == arity);
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
        let mut swift = Vec::new();
        for id in &flags {
            let Some(name) = self.flag_name(*id) else {
                continue;
            };
            match name.as_str() {
                "Objective-C Image Info Version" => has_version = true,
                "Objective-C Class Properties" => has_properties = true,
                "Objective-C Garbage Collection" => swift = self.upgrade_objc_collection(*id),
                _ => {}
            }
        }
        for (name, bits, value) in swift {
            let node = self.module_flag(1, name, bits, value);
            self.add_module_flag(node);
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

    /// The Objective-C collector flag, which is eight bits wide and was once
    /// where a Swift compiler wrote its own version too.
    ///
    /// Measured by sweeping the value. The low byte is the collector's own
    /// configuration and stays; bits 8 to 15 are the Swift ABI version, 16 to
    /// 23 the minor and 24 to 31 the major, and upstream writes those out as
    /// three flags of their own when there is anything above the low byte.
    /// `0x05010700` comes back as an `i8 0` collector beside ABI 7, major 5
    /// and minor 1, and `0x00000001` comes back as an `i8 1` collector and
    /// nothing else.
    ///
    /// The upgrade fires on a value wider than eight bits, whatever the
    /// behaviour says, and the behaviour it leaves is always `Error`: a flag
    /// written `i8` is left alone entirely, keeping the behaviour it had.
    fn upgrade_objc_collection(&mut self, id: MdId) -> Vec<(&'static str, u32, u64)> {
        let Some(Metadata::Tuple { operands, .. }) = self.module.metadata_node(id) else {
            return Vec::new();
        };
        let [
            _,
            _,
            MdOperand::Value {
                ty,
                value: Value::Constant(written),
            },
        ] = operands.as_slice()
        else {
            return Vec::new();
        };
        let TypeKind::Integer(bits) = *self.module.ctx.type_kind(*ty) else {
            return Vec::new();
        };
        if bits <= 8 {
            return Vec::new();
        }
        let Some(value) = self
            .module
            .ctx
            .constant(*written)
            .as_integer()
            .and_then(ApInt::to_u64)
        else {
            return Vec::new();
        };
        self.narrow_flag_value(id, 8);
        self.set_flag_behaviour(id, 1);
        if value >> 8 == 0 {
            return Vec::new();
        }
        vec![
            ("Swift ABI Version", 32, (value >> 8) & 0xff),
            ("Swift Major Version", 8, (value >> 24) & 0xff),
            ("Swift Minor Version", 8, (value >> 16) & 0xff),
        ]
    }

    /// One module flag, built rather than read.
    fn module_flag(&mut self, behaviour: u64, name: &'static str, bits: u32, value: u64) -> MdId {
        let i32_type = self.module.ctx.int_type(32);
        let behaviour = self
            .module
            .ctx
            .const_int(i32_type, ApInt::from_u64(32, behaviour));
        let ty = self.module.ctx.int_type(bits);
        let value = self.module.ctx.const_int(ty, ApInt::from_u64(bits, value));
        self.module.add_metadata(Metadata::Tuple {
            distinct: false,
            operands: vec![
                MdOperand::Value {
                    ty: i32_type,
                    value: Value::Constant(behaviour),
                },
                MdOperand::String(name.into()),
                MdOperand::Value {
                    ty,
                    value: Value::Constant(value),
                },
            ],
        })
    }

    /// Adds a flag to `llvm.module.flags`, which is where upstream puts one
    /// it worked out: at the end, after everything the module wrote.
    fn add_module_flag(&mut self, node: MdId) {
        for named in &mut self.module.named_metadata {
            if named.name == "llvm.module.flags" {
                named.operands.push(node);
                return;
            }
        }
    }

    /// Rewrites the behaviour a flag was written with.
    fn set_flag_behaviour(&mut self, id: MdId, behaviour: u64) {
        let Some(Metadata::Tuple { distinct, operands }) = self.module.metadata_node(id).cloned()
        else {
            return;
        };
        let i32_type = self.module.ctx.int_type(32);
        let written = self
            .module
            .ctx
            .const_int(i32_type, ApInt::from_u64(32, behaviour));
        let mut operands = operands;
        operands[0] = MdOperand::Value {
            ty: i32_type,
            value: Value::Constant(written),
        };
        self.module
            .set_metadata(id, Metadata::Tuple { distinct, operands });
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

/// A name as text, for sorting. An unnamed function sorts before every named
/// one, which is where nothing puts it in practice: an implied declaration
/// always has a name.
fn name_text(name: Option<Name>) -> String {
    match name {
        Some(Name::Named(text)) => text,
        Some(Name::Number(number)) => number.to_string(),
        None => String::new(),
    }
}
