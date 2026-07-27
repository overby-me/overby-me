//! Global variables, aliases, ifuncs and the qualifiers they share.

use crate::lexer::Token;
use crate::{ParseError, Parser};
use llvm_ir::attribute::AttributeSet;
use llvm_ir::global::{
    Alias, ComdatRef, DllStorageClass, GlobalQualifiers, GlobalVariable, IFunc, Linkage,
    RuntimePreemption, Sanitizers, TlsModel, UnnamedAddr, Visibility,
};
use llvm_ir::value::Name;
use llvm_support::Align;

/// The largest alignment upstream accepts, and the largest that fits its
/// encoding.
const MAXIMUM_ALIGNMENT: u64 = 1 << 32;

impl Parser {
    // --------------------------------------------------------------- globals

    pub(crate) fn parse_global_definition(&mut self, name: Name) -> Result<(), ParseError> {
        self.require(Token::Equals)?;
        let external = self.eat_word("external");
        let qualifiers = self.parse_global_qualifiers()?;

        if self.eat_word("alias") {
            let value_type = self.parse_type()?;
            self.require(Token::Comma)?;
            // `alias i32, getelementptr (...)`: the aliasee is written
            // without a type of its own when it is an expression, because the
            // expression says what it produces.
            let aliasee = if self.starts_a_typeless_expression() {
                self.parse_untyped_constant_expression()?
            } else {
                self.parse_typed_constant()?.1
            };
            let mut alias = Alias {
                name,
                qualifiers,
                value_type,
                aliasee,
                partition: None,
                metadata: Vec::new(),
            };
            while self.eat(&Token::Comma) {
                if self.eat_word("partition") {
                    alias.partition = Some(self.require_quoted()?);
                } else {
                    return self.error("unexpected clause after an alias");
                }
            }
            alias.metadata = self.parse_metadata_attachments()?;
            self.module.add_alias(alias);
            return Ok(());
        }

        if self.eat_word("ifunc") {
            let value_type = self.parse_type()?;
            self.require(Token::Comma)?;
            let (_, resolver) = self.parse_typed_constant()?;
            let mut ifunc = IFunc {
                name,
                qualifiers,
                value_type,
                resolver,
                partition: None,
                metadata: Vec::new(),
            };
            while self.eat(&Token::Comma) {
                if self.eat_word("partition") {
                    ifunc.partition = Some(self.require_quoted()?);
                } else {
                    return self.error("unexpected clause after an ifunc");
                }
            }
            ifunc.metadata = self.parse_metadata_attachments()?;
            self.module.add_ifunc(ifunc);
            return Ok(());
        }

        let externally_initialized = self.eat_word("externally_initialized");
        let is_constant = if self.eat_word("constant") {
            true
        } else if self.eat_word("global") {
            false
        } else {
            return self.error("expected 'global', 'constant', 'alias' or 'ifunc'");
        };

        let value_type = self.parse_type()?;
        let initializer = if external
            || matches!(
                self.peek(),
                Token::Comma | Token::Eof | Token::Word(_) | Token::GlobalName(_)
            ) && !self.starts_a_constant()
        {
            None
        } else {
            Some(self.parse_constant(value_type)?)
        };

        let mut global = GlobalVariable {
            name,
            qualifiers,
            externally_initialized,
            is_constant,
            value_type,
            initializer,
            section: None,
            partition: None,
            comdat: None,
            align: None,
            metadata: Vec::new(),
            attrs: AttributeSet::default(),
            code_model: None,
            sanitizer: Sanitizers::default(),
        };

        while self.eat(&Token::Comma) {
            if self.eat_word("section") {
                global.section = Some(self.require_quoted()?);
            } else if self.eat_word("partition") {
                global.partition = Some(self.require_quoted()?);
            } else if self.eat_word("code_model") {
                global.code_model = Some(self.require_quoted()?);
            } else if self.eat_word("comdat") {
                global.comdat = Some(self.parse_comdat_ref()?);
            } else if self.eat_word("align") {
                global.align = Some(self.parse_align()?);
            } else if self.eat_word("no_sanitize_address") {
                global.sanitizer.no_address = true;
            } else if self.eat_word("no_sanitize_hwaddress") {
                global.sanitizer.no_hwaddress = true;
            } else if self.eat_word("sanitize_address_dyninit") {
                global.sanitizer.address_dyninit = true;
            } else if self.eat_word("sanitize_memtag") {
                global.sanitizer.memtag = true;
            } else if matches!(self.peek(), Token::MetadataName(_))
                && matches!(self.peek_at(1), Token::MetadataNumber(_))
            {
                let attachments = self.parse_metadata_attachments()?;
                global.metadata.extend(attachments);
            } else {
                return self.error("unexpected clause after a global");
            }
        }
        // `@g = global i32 7 "key" = "value" #0`: a global's own attributes
        // come last, with no comma, and can be either strings or groups.
        loop {
            match self.peek().clone() {
                Token::AttributeGroup(number) => {
                    self.advance();
                    global.attrs.groups.push(number);
                }
                Token::Quoted(_) => {
                    let attribute = self.parse_attribute(false)?;
                    global.attrs.push(attribute);
                }
                _ => break,
            }
        }
        self.module.add_global(global);
        Ok(())
    }

    /// Whether the next token could begin a constant, which is how a global
    /// with no initialiser is told apart from one with a `zeroinitializer`.
    /// The constant-expression opcodes that need no type in front.
    pub(crate) fn starts_a_typeless_expression(&self) -> bool {
        matches!(self.peek(), Token::Word(word) if matches!(
            word.as_str(),
            "getelementptr"
                | "bitcast"
                | "inttoptr"
                | "ptrtoint"
                | "ptrtoaddr"
                | "addrspacecast"
                | "trunc"
                | "extractelement"
                | "insertelement"
                | "shufflevector"
                | "add"
                | "sub"
                | "xor"
        ))
    }

    pub(crate) fn starts_a_constant(&self) -> bool {
        match self.peek() {
            Token::Word(word) => matches!(
                word.as_str(),
                "zeroinitializer"
                    | "null"
                    | "none"
                    | "undef"
                    | "poison"
                    | "true"
                    | "false"
                    | "getelementptr"
                    | "bitcast"
                    | "inttoptr"
                    | "ptrtoint"
                    | "ptrtoaddr"
                    | "addrspacecast"
                    | "trunc"
                    | "extractelement"
                    | "insertelement"
                    | "shufflevector"
                    | "splat"
                    | "ptrauth"
                    | "add"
                    | "sub"
                    | "xor"
                    | "blockaddress"
                    | "dso_local_equivalent"
                    | "no_cfi"
                    | "asm"
            ),
            Token::Integer { .. }
            | Token::Float(_)
            | Token::HexFloat { .. }
            | Token::ByteString(_)
            | Token::GlobalName(_)
            | Token::GlobalNumber(_)
            | Token::LeftBrace
            | Token::LeftBracket
            | Token::Less => true,
            _ => false,
        }
    }

    pub(crate) fn parse_comdat_ref(&mut self) -> Result<ComdatRef, ParseError> {
        if !self.eat(&Token::LeftParen) {
            return Ok(ComdatRef { name: None });
        }
        let name = match self.advance() {
            Token::ComdatName(name) => name,
            other => {
                self.index -= 1;
                return self.error(format!(
                    "expected a comdat name, found {}",
                    other.describe()
                ));
            }
        };
        self.require(Token::RightParen)?;
        Ok(ComdatRef { name: Some(name) })
    }

    pub(crate) fn parse_align(&mut self) -> Result<Align, ParseError> {
        let bytes = self.require_unsigned()?;
        // Upstream caps alignment at 2^32 and rejects anything larger in the
        // parser rather than the verifier, so the message points at the
        // literal.
        if bytes > MAXIMUM_ALIGNMENT {
            return self.error(format!("huge alignment values are unsupported: {bytes}"));
        }
        Align::from_bytes(bytes).map_or_else(
            || self.error(format!("alignment {bytes} is not a power of two")),
            Ok,
        )
    }

    pub(crate) fn parse_global_qualifiers(&mut self) -> Result<GlobalQualifiers, ParseError> {
        let mut qualifiers = GlobalQualifiers::default();
        while let Some(word) = self.peek_word() {
            if let Some(linkage) = Linkage::from_keyword(word) {
                qualifiers.linkage = Some(linkage);
                self.advance();
                continue;
            }
            if let Some(preemption) = RuntimePreemption::from_keyword(word) {
                qualifiers.preemption = Some(preemption);
                self.advance();
                continue;
            }
            if let Some(visibility) = Visibility::from_keyword(word) {
                qualifiers.visibility = Some(visibility);
                self.advance();
                continue;
            }
            if let Some(storage) = DllStorageClass::from_keyword(word) {
                qualifiers.dll_storage = Some(storage);
                self.advance();
                continue;
            }
            if let Some(unnamed) = UnnamedAddr::from_keyword(word) {
                qualifiers.unnamed_addr = Some(unnamed);
                self.advance();
                continue;
            }
            if word == "thread_local" {
                self.advance();
                if self.eat(&Token::LeftParen) {
                    let model_word = self.require_word()?;
                    let Some(model) = TlsModel::from_keyword(&model_word) else {
                        return self.error(format!("unknown TLS model '{model_word}'"));
                    };
                    self.require(Token::RightParen)?;
                    qualifiers.thread_local = Some(Some(model));
                } else {
                    qualifiers.thread_local = Some(None);
                }
                continue;
            }
            if word == "addrspace" {
                qualifiers.address_space = self.parse_optional_address_space()?;
                continue;
            }
            break;
        }
        Ok(qualifiers)
    }
}
