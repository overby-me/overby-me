//! Attribute parsing, including the spellings that differ inside a group.

use crate::globals::{MAXIMUM_ALIGNMENT, MAXIMUM_STACK_ALIGNMENT};
use crate::lexer::Token;
use crate::{ParseError, Parser};
use llvm_ir::TypeId;
use llvm_ir::attribute::{Attribute, AttributeSet, EnumAttr, IntAttr, StructuredAttr, TypeAttr};
use llvm_ir::types::TypeKind;
use llvm_support::ApInt;

/// Where a function may touch memory, as the attributes that predate
/// `memory(...)` said it.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
struct Locations {
    argmem: bool,
    inaccessiblemem: bool,
    other: bool,
}

impl Locations {
    const ALL: Locations = Locations {
        argmem: true,
        inaccessiblemem: true,
        other: true,
    };
}

/// The legacy spelling of a memory effect, which upstream reads and then
/// prints as `memory(...)`. `argmemonly` and its two friends say where, and
/// `readnone`, `readonly` and `writeonly` say how, so a function carrying one
/// of each means the intersection: `argmemonly readonly` is
/// `memory(argmem: read)`.
///
/// The same three access keywords are ordinary attributes on a parameter,
/// where they keep their spelling, so only the function and call-site
/// positions upgrade.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct LegacyMemory {
    locations: Option<Locations>,
    access: Option<&'static str>,
}

impl LegacyMemory {
    pub(crate) fn take(&mut self, word: &str) -> bool {
        match word {
            "argmemonly" => {
                self.locations = Some(Locations {
                    argmem: true,
                    ..Locations::default()
                });
            }
            "inaccessiblememonly" => {
                self.locations = Some(Locations {
                    inaccessiblemem: true,
                    ..Locations::default()
                });
            }
            "inaccessiblemem_or_argmemonly" => {
                self.locations = Some(Locations {
                    argmem: true,
                    inaccessiblemem: true,
                    ..Locations::default()
                });
            }
            // `readnone` wins over the others, being the strongest claim.
            "readnone" => self.access = Some("none"),
            "readonly" if self.access != Some("none") => self.access = Some("read"),
            "writeonly" if self.access != Some("none") => self.access = Some("write"),
            "readonly" | "writeonly" => {}
            _ => return false,
        }
        true
    }

    fn is_empty(self) -> bool {
        self.locations.is_none() && self.access.is_none()
    }

    /// What `memory(...)` says the same thing.
    fn arguments(self) -> String {
        let access = self.access.unwrap_or("readwrite");
        if access == "none" {
            return "none".to_string();
        }
        let locations = self.locations.unwrap_or(Locations::ALL);
        if locations == Locations::ALL {
            return access.to_string();
        }
        let mut parts = Vec::new();
        if locations.argmem {
            parts.push(format!("argmem: {access}"));
        }
        if locations.inaccessiblemem {
            parts.push(format!("inaccessiblemem: {access}"));
        }
        if locations.other {
            parts.push(format!("other: {access}"));
        }
        parts.join(", ")
    }
}

/// Whether a word is one of the six the upgrade reads.
pub(crate) fn is_legacy_memory(word: &str) -> bool {
    matches!(
        word,
        "argmemonly"
            | "inaccessiblememonly"
            | "inaccessiblemem_or_argmemonly"
            | "readnone"
            | "readonly"
            | "writeonly"
    )
}

impl Parser {
    // ------------------------------------------------------------ attributes

    /// An alignment written as an attribute is capped the way one written as
    /// a clause is, and upstream refuses an oversized one in the parser
    /// rather than the verifier, so the message points at the literal. The
    /// stack cap is one bit lower than the rest.
    fn alignment_fits(&mut self, kind: IntAttr, bytes: u64) -> Result<(), ParseError> {
        let ceiling = match kind {
            IntAttr::Align => MAXIMUM_ALIGNMENT,
            IntAttr::AlignStack => MAXIMUM_STACK_ALIGNMENT,
            _ => return Ok(()),
        };
        if bytes > ceiling {
            return self.error(format!("huge alignment values are unsupported: {bytes}"));
        }
        Ok(())
    }

    /// One attribute. `in_group` selects between the two spellings upstream
    /// uses for the same attribute: `align 8` in a parameter list and
    /// `align=8` inside an attribute group.
    pub(crate) fn parse_attribute(&mut self, in_group: bool) -> Result<Attribute, ParseError> {
        if let Token::Quoted(bytes) = self.peek().clone() {
            self.advance();
            let Ok(key) = String::from_utf8(bytes) else {
                return self.error("an attribute key has to be valid UTF-8");
            };
            let value = if self.eat(&Token::Equals) {
                Some(self.require_quoted()?)
            } else {
                None
            };
            return Ok(Attribute::String { key, value });
        }
        let word = self.require_word()?;

        if word == "range" {
            self.require(Token::LeftParen)?;
            let ty = self.parse_type()?;
            let bits = self.integer_width(ty)?;
            let lower = self.parse_ap_int(bits)?;
            self.require(Token::Comma)?;
            let upper = self.parse_ap_int(bits)?;
            self.require(Token::RightParen)?;
            return Ok(Attribute::Range { ty, lower, upper });
        }

        if let Some(kind) = TypeAttr::from_keyword(&word) {
            self.require(Token::LeftParen)?;
            let ty = self.parse_type()?;
            self.require(Token::RightParen)?;
            return Ok(Attribute::Type { kind, ty });
        }

        // `uwtable` is both a bare keyword and a parenthesised one, so the
        // structured reading only applies when an argument list follows.
        if let Some(kind) = StructuredAttr::from_keyword(&word)
            && self.peek() == &Token::LeftParen
        {
            let arguments = self.collect_parenthesised()?;
            return Ok(Attribute::Structured { kind, arguments });
        }

        if let Some(kind) = IntAttr::from_keyword(&word) {
            // Upstream spells the same attribute three ways depending on
            // where it sits: `align 8` in a parameter list, `align=8` and
            // `alignstack=4` inside a group, `alignstack(4)` everywhere else.
            if in_group && matches!(kind, IntAttr::Align | IntAttr::AlignStack) {
                self.require(Token::Equals)?;
                let first = self.require_unsigned()?;
                self.alignment_fits(kind, first)?;
                return Ok(Attribute::Int {
                    kind,
                    first,
                    second: None,
                });
            }
            // Upstream prints `align 4` in a parameter list but reads
            // `align(4)` there too, and its own tests use both.
            if !in_group && kind == IntAttr::Align && self.peek() != &Token::LeftParen {
                let first = self.require_unsigned()?;
                self.alignment_fits(kind, first)?;
                return Ok(Attribute::Int {
                    kind,
                    first,
                    second: None,
                });
            }
            self.require(Token::LeftParen)?;
            let first = self.require_unsigned()?;
            self.alignment_fits(kind, first)?;
            let second = if self.eat(&Token::Comma) {
                Some(self.require_unsigned()?)
            } else {
                None
            };
            self.require(Token::RightParen)?;
            return Ok(Attribute::Int {
                kind,
                first,
                second,
            });
        }

        if let Some(kind) = EnumAttr::from_keyword(&word) {
            // `nocapture` is the older spelling of `captures(none)`, and
            // upstream reads one and prints the other. It is the only
            // parameter attribute that upgrades this way: `readonly`,
            // `writeonly` and the rest keep their spelling here, where on a
            // function they do not.
            if kind == EnumAttr::NoCapture {
                return Ok(Attribute::Structured {
                    kind: StructuredAttr::Captures,
                    arguments: "none".to_string(),
                });
            }
            return Ok(Attribute::Enum(kind));
        }

        self.error(format!(
            "unknown attribute '{word}'; add it to llvm_ir::attribute rather than ignoring it"
        ))
    }

    /// The text between a matching pair of parentheses, kept verbatim.
    pub(crate) fn collect_parenthesised(&mut self) -> Result<String, ParseError> {
        self.require(Token::LeftParen)?;
        let mut depth = 1usize;
        let mut parts: Vec<String> = Vec::new();
        let mut previous_was_word = false;
        loop {
            let token = self.advance();
            match &token {
                Token::LeftParen => depth += 1,
                Token::RightParen => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(parts.join(""));
                    }
                }
                Token::Eof => return self.error("unterminated attribute argument list"),
                _ => {}
            }
            let text = match &token {
                Token::Word(word) => word.clone(),
                // `memory(argmem: readwrite)` reaches the lexer as a label,
                // because a word followed by a colon is a label everywhere
                // else in the grammar.
                Token::Label(word) => format!("{word}: "),
                Token::Integer { negative, digits } => {
                    format!("{}{digits}", if *negative { "-" } else { "" })
                }
                Token::Comma => ",".to_string(),
                Token::LeftParen => "(".to_string(),
                Token::RightParen => ")".to_string(),
                Token::Quoted(text) => format!("\"{}\"", String::from_utf8_lossy(text.as_slice())),
                other => other.describe(),
            };
            let is_word = matches!(token, Token::Word(_) | Token::Integer { .. });
            // `memory(argmem: readwrite)` and `allockind("alloc,zeroed")`
            // both print with a space after each comma and after each colon.
            if previous_was_word && is_word {
                parts.push(" ".to_string());
            }
            if text == "," {
                parts.push(",".to_string());
                parts.push(" ".to_string());
                previous_was_word = false;
                continue;
            }
            parts.push(text);
            previous_was_word = is_word;
        }
    }

    /// A run of attributes, stopping at the first token that is not one.
    pub(crate) fn parse_attribute_set(
        &mut self,
        in_group: bool,
    ) -> Result<AttributeSet, ParseError> {
        self.parse_attribute_set_where(in_group, false)
    }

    /// The same, in a position where the pre-`memory(...)` spellings mean
    /// what `memory(...)` means: a function's own attributes or a call
    /// site's, and not a parameter's.
    pub(crate) fn parse_function_attribute_set(&mut self) -> Result<AttributeSet, ParseError> {
        self.parse_attribute_set_where(false, true)
    }

    fn parse_attribute_set_where(
        &mut self,
        in_group: bool,
        upgrade_memory: bool,
    ) -> Result<AttributeSet, ParseError> {
        let mut set = AttributeSet::default();
        let mut legacy = LegacyMemory::default();
        loop {
            match self.peek().clone() {
                Token::AttributeGroup(number) => {
                    self.advance();
                    set.groups.push(number);
                }
                Token::Quoted(_) => set.attributes.push(self.parse_attribute(in_group)?),
                Token::Word(word) if upgrade_memory && is_legacy_memory(&word) => {
                    self.advance();
                    legacy.take(&word);
                }
                Token::Word(word) => {
                    let known = EnumAttr::from_keyword(&word).is_some()
                        || IntAttr::from_keyword(&word).is_some()
                        || TypeAttr::from_keyword(&word).is_some()
                        || StructuredAttr::from_keyword(&word).is_some()
                        || word == "range";
                    if !known {
                        break;
                    }
                    set.attributes.push(self.parse_attribute(in_group)?);
                }
                _ => break,
            }
        }
        apply_legacy_memory(&mut set.attributes, legacy);
        Ok(set)
    }

    pub(crate) fn integer_width(&mut self, ty: TypeId) -> Result<u32, ParseError> {
        match self.module.ctx.type_kind(ty) {
            TypeKind::Integer(bits) => Ok(*bits),
            _ => self.error("expected an integer type"),
        }
    }

    pub(crate) fn parse_ap_int(&mut self, bits: u32) -> Result<ApInt, ParseError> {
        match self.advance() {
            Token::Integer { negative, digits } => {
                let text = if negative {
                    format!("-{digits}")
                } else {
                    digits
                };
                ApInt::parse_sized(&text, 10, bits).map_or_else(
                    |error| {
                        self.index -= 1;
                        self.error(error.to_string())
                    },
                    Ok,
                )
            }
            Token::Word(word) if word == "true" => Ok(ApInt::from_u64(bits, 1)),
            Token::Word(word) if word == "false" => Ok(ApInt::new(bits)),
            Token::Word(word) if wide_hex(&word, bits).is_some() => {
                Ok(wide_hex(&word, bits).unwrap_or_else(|| ApInt::new(bits)))
            }
            other => {
                self.index -= 1;
                self.error(format!(
                    "expected an integer literal, found {}",
                    other.describe()
                ))
            }
        }
    }
}

/// Replaces whatever the set said about memory with what the legacy
/// attributes said, which is the order upstream resolves the two in.
pub(crate) fn apply_legacy_memory(attributes: &mut Vec<Attribute>, legacy: LegacyMemory) {
    if legacy.is_empty() {
        return;
    }
    attributes.retain(|attribute| {
        !matches!(
            attribute,
            Attribute::Structured {
                kind: StructuredAttr::Memory,
                ..
            }
        )
    });
    attributes.push(Attribute::Structured {
        kind: StructuredAttr::Memory,
        arguments: legacy.arguments(),
    });
}

/// `u0x...` and `s0x...`, for an integer too wide to write in decimal. `u0x`
/// zero-extends its bit pattern; `s0x` reads it as two's complement in the
/// narrowest width that holds its digits, so its top set bit is the sign and
/// `s0x1` is -1. Measured, not guessed.
/// The same two forms where a plain count is wanted rather than a typed
/// constant: an array's length, for one.
pub(crate) fn wide_hex_u64(word: &str) -> Option<u64> {
    wide_hex(word, 64)?.to_u64()
}

fn wide_hex(word: &str, bits: u32) -> Option<ApInt> {
    let (signed, digits) = match word.as_bytes() {
        [b'u', b'0', b'x', ..] => (false, &word[3..]),
        [b's', b'0', b'x', ..] => (true, &word[3..]),
        _ => return None,
    };
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let magnitude = ApInt::parse_magnitude(digits, 16).ok()?;
    if !signed {
        return Some(magnitude.zext_or_trunc(bits));
    }
    let leading = digits.len() - digits.trim_start_matches('0').len();
    let top = digits.as_bytes().get(leading).map_or(0, |digit| {
        let value = u32::from(char::from(*digit).to_digit(16).unwrap_or(0) as u8);
        32 - value.leading_zeros()
    });
    let width = ((digits.len() - leading) as u32).saturating_sub(1) * 4 + top;
    Some(magnitude.trunc(width.max(1)).sext_or_trunc(bits))
}
