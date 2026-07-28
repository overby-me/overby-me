//! Metadata parsing.
//!
//! Specialized nodes keep their tag and their fields as written. The one
//! lexical oddity is that `line:` inside `!DILocation(line: 1)` arrives as a
//! label token, because a word followed by a colon is a label everywhere else
//! in the grammar; the parser reads it back as a field name.

use crate::lexer::Token;
use crate::md_schema;
use crate::{FunctionState, ParseError, Parser};
use llvm_ir::TypeId;
use llvm_ir::function::Function;
use llvm_ir::metadata::{MdAttachment, MdField, MdOperand, MdRef, Metadata, SpecializedArgs};
use llvm_ir::value::MdId;

/// Where a metadata operand is being read, which decides whether local values
/// are allowed in it.
type ValueContext<'a> = Option<(&'a mut Function, &'a mut FunctionState)>;

impl Parser {
    pub(crate) fn parse_metadata_definition(
        &mut self,
        context: ValueContext<'_>,
    ) -> Result<Metadata, ParseError> {
        let distinct = self.eat_word("distinct");
        match self.peek().clone() {
            // `!0 = !"text"` looks reasonable and upstream rejects it: a
            // string is an operand, and a node definition is a tuple or a
            // specialized node. Accepting it would let us print something
            // llvm-as will not read back.
            Token::MetadataString(_) => {
                self.error("a metadata definition must be a node, not a string")
            }
            Token::Exclaim => {
                self.advance();
                self.require(Token::LeftBrace)?;
                let mut operands = Vec::new();
                while !self.eat(&Token::RightBrace) {
                    if !operands.is_empty() {
                        self.require(Token::Comma)?;
                    }
                    if self.peek() == &Token::RightBrace {
                        self.advance();
                        break;
                    }
                    operands.push(self.parse_metadata_operand(None)?);
                }
                Ok(Metadata::Tuple { distinct, operands })
            }
            Token::MetadataName(bytes) => {
                self.advance();
                // A node kind is a word. A name that is not even UTF-8 is
                // certainly not one of the thirty-two.
                let tag = String::from_utf8(bytes).unwrap_or_default();
                let Some(schema) = md_schema::node(&tag) else {
                    self.index -= 1;
                    return self.error("expected metadata type");
                };
                let args = self.parse_specialized_args(context)?;
                // Read before the grammar is checked, so that a word the
                // vocabulary has is a number by the time its range is asked
                // about and one it does not have is what is left over.
                let args = read_vocabulary_words(&tag, args);
                self.check_specialized(schema, &tag, distinct, &args)?;
                let args = drop_defaulted_fields(schema, &tag, args);
                let args = fill_compile_unit_defaults(&tag, args);
                let args = upgrade_subprogram_flags(&tag, args);
                // Sorted here rather than at print time, because two nodes
                // that differ only in the order their fields were written are
                // one node upstream, and uniquing compares what is stored.
                let args = match args {
                    SpecializedArgs::Named(mut fields) => {
                        fields.sort_by_key(|(name, _)| llvm_ir::metadata::field_rank(&tag, name));
                        SpecializedArgs::Named(fields)
                    }
                    other => other,
                };
                Ok(Metadata::Specialized {
                    distinct,
                    tag,
                    args,
                })
            }
            other => self.error(format!(
                "expected a metadata node, found {}",
                other.describe()
            )),
        }
    }

    /// Checks a specialized node against its grammar: field names, repeats,
    /// required fields, null and empty values, numeric ranges, and whether
    /// the node has to be `distinct`.
    fn check_specialized(
        &mut self,
        schema: &'static md_schema::Node,
        tag: &str,
        distinct: bool,
        args: &SpecializedArgs,
    ) -> Result<(), ParseError> {
        let fields = match args {
            SpecializedArgs::Named(fields) => fields.as_slice(),
            SpecializedArgs::Positional(elements) => {
                if !schema.positional {
                    // The only positional form a keyed node has is the empty
                    // one, and then every required field is missing.
                    return self.check_required(schema, &[]);
                }
                for element in elements {
                    if let MdField::Unsigned(value) = element
                        && *value > u64::MAX as u128
                    {
                        return self.error(format!("element too large, limit is {}", u64::MAX));
                    }
                }
                return Ok(());
            }
        };

        let mut seen: Vec<&str> = Vec::new();
        for (name, value) in fields {
            let Some(spec) = schema.fields.iter().find(|spec| spec.name == name) else {
                return self.error(format!("invalid field '{name}'"));
            };
            if seen.contains(&spec.name) {
                return self.error(format!("field '{name}' cannot be specified more than once"));
            }
            seen.push(spec.name);
            self.check_field(tag, spec, value)?;
        }

        match schema.distinct {
            md_schema::Distinct::Optional => {}
            md_schema::Distinct::Always if !distinct => {
                return self.error(format!("missing 'distinct', required for !{tag}"));
            }
            md_schema::Distinct::WhenDefinition if !distinct && is_a_definition(fields) => {
                return self.error(format!(
                    "missing 'distinct', required for !{tag} that is a Definition"
                ));
            }
            _ => {}
        }

        self.check_required(schema, &seen)
    }

    fn check_required(
        &mut self,
        schema: &'static md_schema::Node,
        seen: &[&str],
    ) -> Result<(), ParseError> {
        for spec in schema.fields {
            if spec.required && !seen.contains(&spec.name) {
                return self.error(format!("missing required field '{}'", spec.name));
            }
        }
        Ok(())
    }

    fn check_field(
        &mut self,
        tag: &str,
        spec: &'static md_schema::Field,
        value: &MdField,
    ) -> Result<(), ParseError> {
        let name = spec.name;
        // `operands:` holds the node's own operands, written with braces, so
        // a reference to a node that holds them is not what it takes.
        if name == "operands"
            && !matches!(value, MdField::Inline(node) if matches!(**node, Metadata::Tuple { .. }))
        {
            return self.error("expected '{' here");
        }
        // A `flags:` is a set of words joined by `|`, and each of them has
        // to be one: the words were swept out the same way the numbered
        // vocabularies were, and no mask name is among them.
        if name == "flags"
            && let MdField::Words(words) = value
        {
            for word in words {
                if !llvm_ir::metadata::dwarf::FLAGS.contains(&word.as_str()) {
                    return self.error(format!("invalid debug info flag '{word}'"));
                }
            }
        }
        // A word is read into the number it stands for, so one still written
        // as a word here is one no vocabulary has. Three of the nine tables
        // are not complete and refuse nothing.
        if let MdField::Words(words) = value
            && let [word] = words.as_slice()
            && let Some(vocabulary) = llvm_ir::metadata::vocabulary_name(tag, name)
        {
            return self.error(format!("invalid {vocabulary} '{word}'"));
        }
        if spec.non_null && matches!(value, MdField::Null) {
            return self.error(format!("'{name}' cannot be null"));
        }
        if spec.non_empty && matches!(value, MdField::Str(text) if text.is_empty()) {
            return self.error(format!("'{name}' cannot be empty"));
        }
        match (spec.shape, value) {
            (md_schema::Shape::Any, _) => Ok(()),
            (md_schema::Shape::Unsigned(limit), MdField::Unsigned(written)) => {
                if *written > u128::from(limit) {
                    return self.error(format!("value for '{name}' too large, limit is {limit}"));
                }
                Ok(())
            }
            (md_schema::Shape::Unsigned(_), MdField::Signed(_)) => {
                self.error("expected unsigned integer")
            }
            (md_schema::Shape::Enumerator(limit, _), MdField::Unsigned(written)) => {
                if *written > u128::from(limit) {
                    return self.error(format!("value for '{name}' too large, limit is {limit}"));
                }
                Ok(())
            }
            (md_schema::Shape::Enumerator(_, what), MdField::Str(_) | MdField::Signed(_)) => {
                self.error(format!("expected {what}"))
            }

            (md_schema::Shape::SmallEnumerator(limit), MdField::Unsigned(written)) => {
                if *written > u128::from(limit) {
                    return self.error(format!("value for '{name}' too large"));
                }
                Ok(())
            }
            (md_schema::Shape::Bounded(_, max), MdField::Unsigned(written)) => {
                if *written > max as u128 {
                    return self.error(format!("value for '{name}' too large, limit is {max}"));
                }
                Ok(())
            }
            (md_schema::Shape::Bounded(min, _), MdField::Signed(written)) => {
                if *written < min {
                    return self.error(format!("value for '{name}' too small, limit is {min}"));
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn parse_specialized_args(
        &mut self,
        mut context: ValueContext<'_>,
    ) -> Result<SpecializedArgs, ParseError> {
        self.require(Token::LeftParen)?;
        if self.eat(&Token::RightParen) {
            // An empty argument list says nothing about which of the two
            // spellings it is, and a node whose fields all dropped ends up
            // with an empty named list, so the two have to be the same thing
            // or uniquing sees two nodes where upstream sees one.
            return Ok(SpecializedArgs::Named(Vec::new()));
        }
        let named = matches!(self.peek(), Token::Label(_));
        if named {
            let mut fields = Vec::new();
            loop {
                let Token::Label(key) = self.advance() else {
                    self.index -= 1;
                    return self.error("expected a field name in a specialized node");
                };
                let value = self.parse_metadata_field(reborrow(&mut context))?;
                fields.push((key, value));
                if !self.eat(&Token::Comma) {
                    break;
                }
            }
            self.require(Token::RightParen)?;
            Ok(SpecializedArgs::Named(fields))
        } else {
            let mut fields = Vec::new();
            loop {
                fields.push(self.parse_metadata_field(reborrow(&mut context))?);
                if !self.eat(&Token::Comma) {
                    break;
                }
            }
            self.require(Token::RightParen)?;
            Ok(SpecializedArgs::Positional(fields))
        }
    }

    /// A value inside a metadata node: a constant, or an SSA value when a
    /// function is being read. `DIArgList` is the node that holds the latter,
    /// which is why the context is threaded this far down.
    fn parse_metadata_value(
        &mut self,
        ty: TypeId,
        context: ValueContext<'_>,
    ) -> Result<llvm_ir::Value, ParseError> {
        match context {
            Some((function, state)) => self.parse_value(function, state, ty),
            None => Ok(llvm_ir::Value::Constant(self.parse_constant(ty)?)),
        }
    }

    fn parse_metadata_field(&mut self, context: ValueContext<'_>) -> Result<MdField, ParseError> {
        match self.peek().clone() {
            Token::Integer { negative, digits } => {
                self.advance();
                // A `DIEnumerator` value is arbitrary-precision, so anything
                // wider than 128 bits is kept as written rather than refused.
                let Ok(magnitude) = digits.parse::<u128>() else {
                    return Ok(MdField::BigInt { negative, digits });
                };
                if negative {
                    // `-170141183460469231731687303715884105728` is i128::MIN,
                    // whose magnitude does not fit i128. Negate through u128.
                    match i128::try_from(magnitude) {
                        Ok(value) => Ok(MdField::Signed(-value)),
                        Err(_) if magnitude == 1u128 << 127 => Ok(MdField::Signed(i128::MIN)),
                        Err(_) => Ok(MdField::BigInt { negative, digits }),
                    }
                } else {
                    Ok(MdField::Unsigned(magnitude))
                }
            }
            Token::Quoted(text) => {
                self.advance();
                Ok(MdField::Str(text.into()))
            }
            Token::MetadataNumber(number) => {
                self.advance();
                Ok(MdField::Ref(MdId(number)))
            }
            Token::MetadataString(text) => {
                self.advance();
                Ok(MdField::Inline(Box::new(Metadata::String(text.into()))))
            }
            Token::MetadataName(_) | Token::Exclaim => {
                let node = self.parse_metadata_definition(context)?;
                // The same rule a reference follows: everything but the two
                // kinds written at every use is hoisted out and numbered, so
                // `types: !{}` becomes `types: !5` with `!5 = !{}`.
                if prints_in_place(&node) {
                    return Ok(MdField::Inline(Box::new(node)));
                }
                let id = MdId(self.next_inline_metadata);
                self.next_inline_metadata += 1;
                self.module.set_metadata(id, node);
                Ok(MdField::Ref(id))
            }
            // `operands: {!0, !3}` writes the tuple with braces and no `!`.
            Token::LeftBrace => {
                self.advance();
                let mut operands = Vec::new();
                while !self.eat(&Token::RightBrace) {
                    if !operands.is_empty() {
                        self.require(Token::Comma)?;
                    }
                    if self.eat(&Token::RightBrace) {
                        break;
                    }
                    operands.push(self.parse_metadata_operand(None)?);
                }
                Ok(MdField::Inline(Box::new(Metadata::Tuple {
                    distinct: false,
                    operands,
                })))
            }
            Token::Word(word) => {
                if word == "null" {
                    self.advance();
                    return Ok(MdField::Null);
                }
                if word == "true" || word == "false" {
                    self.advance();
                    return Ok(MdField::Bool(word == "true"));
                }
                // A type keyword here means a typed value, as in a DIArgList.
                if self.looks_like_a_type() {
                    let ty = self.parse_type()?;
                    return Ok(MdField::Value {
                        ty,
                        value: self.parse_metadata_value(ty, context)?,
                    });
                }
                let mut words = vec![word];
                self.advance();
                while self.eat(&Token::Pipe) {
                    words.push(self.require_word()?);
                }
                Ok(MdField::Words(words))
            }
            Token::IntType(_) => {
                let ty = self.parse_type()?;
                let value = self.parse_metadata_value(ty, context)?;
                Ok(MdField::Value { ty, value })
            }
            // An array or vector type opens a field as well: a
            // `DIDerivedType` may carry `extraData: [4 x i32] [i32 23, ...]`.
            // A brace cannot, `{...}` there being the tuple form above.
            Token::LeftBracket | Token::Less => {
                let ty = self.parse_type()?;
                let value = self.parse_metadata_value(ty, context)?;
                Ok(MdField::Value { ty, value })
            }
            other => self.error(format!(
                "expected a metadata field, found {}",
                other.describe()
            )),
        }
    }

    fn looks_like_a_type(&self) -> bool {
        matches!(
            self.peek(),
            // An array or vector type opens a field too.
            Token::IntType(_) | Token::LeftBracket | Token::Less
        ) || matches!(self.peek(), Token::Word(word) if matches!(
            word.as_str(),
            "void" | "half" | "bfloat" | "float" | "double" | "fp128" | "x86_fp80"
                | "ppc_fp128" | "ptr" | "label" | "metadata" | "token"
        ))
    }

    pub(crate) fn parse_metadata_operand(
        &mut self,
        context: ValueContext<'_>,
    ) -> Result<MdOperand, ParseError> {
        match self.peek().clone() {
            Token::Word(word) if word == "null" => {
                self.advance();
                Ok(MdOperand::Null)
            }
            Token::MetadataNumber(number) => {
                self.advance();
                Ok(MdOperand::Ref(MdId(number)))
            }
            Token::MetadataString(text) => {
                self.advance();
                Ok(MdOperand::String(text.into()))
            }
            Token::MetadataName(_) | Token::Exclaim => match self.parse_metadata_ref(context)? {
                MdRef::Id(id) => Ok(MdOperand::Ref(id)),
                MdRef::Inline(node) => Ok(MdOperand::Inline(node)),
            },
            _ => {
                let ty = self.parse_type()?;
                match context {
                    Some((function, state)) => {
                        let value = self.parse_value(function, state, ty)?;
                        Ok(MdOperand::Value { ty, value })
                    }
                    None => {
                        let constant = self.parse_constant(ty)?;
                        Ok(MdOperand::Value {
                            ty,
                            value: llvm_ir::Value::Constant(constant),
                        })
                    }
                }
            }
        }
    }

    /// `!kind !7` attachments written one after another, which is how they
    /// appear on functions.
    pub(crate) fn parse_metadata_attachments(&mut self) -> Result<Vec<MdAttachment>, ParseError> {
        let mut attachments = Vec::new();
        while let Token::MetadataName(kind) = self.peek().clone() {
            if !self.attachment_follows(1) {
                break;
            }
            self.advance();
            attachments.push(MdAttachment {
                kind: kind.into(),
                node: self.parse_metadata_ref(None)?,
            });
        }
        Ok(attachments)
    }

    /// Whether a metadata attachment, rather than another operand, follows
    /// the comma the parser is sitting on.
    pub(crate) fn attachment_after_comma(&self) -> bool {
        self.peek() == &Token::Comma
            && matches!(self.peek_at(1), Token::MetadataName(_))
            && self.attachment_follows(2)
    }

    /// Whether the token this far ahead starts the node of an attachment: a
    /// number, or a node written in place.
    fn attachment_follows(&self, ahead: usize) -> bool {
        matches!(
            self.peek_at(ahead),
            Token::MetadataNumber(_) | Token::MetadataName(_) | Token::Exclaim
        )
    }

    fn parse_metadata_ref(&mut self, context: ValueContext<'_>) -> Result<MdRef, ParseError> {
        if let Token::MetadataNumber(number) = self.peek().clone() {
            self.advance();
            return Ok(MdRef::Id(MdId(number)));
        }
        let node = self.parse_metadata_definition(context)?;
        // `DIExpression` and `DIArgList` are written at every use and never
        // numbered. Everything else upstream hoists out and numbers, so a
        // node written in place here becomes one written once and referred
        // to, which is what gets printed back.
        if prints_in_place(&node) {
            return Ok(MdRef::Inline(Box::new(node)));
        }
        let id = MdId(self.next_inline_metadata);
        self.next_inline_metadata += 1;
        self.module.set_metadata(id, node);
        Ok(MdRef::Id(id))
    }

    /// `, !dbg !7` attachments, which is how they appear on instructions and
    /// globals.
    pub(crate) fn parse_metadata_attachments_after_comma(
        &mut self,
    ) -> Result<Vec<MdAttachment>, ParseError> {
        let mut attachments = Vec::new();
        loop {
            if self.peek() != &Token::Comma {
                break;
            }
            let Token::MetadataName(kind) = self.peek_at(1).clone() else {
                break;
            };
            if !self.attachment_follows(2) {
                break;
            }
            self.advance();
            self.advance();
            attachments.push(MdAttachment {
                kind: kind.into(),
                node: self.parse_metadata_ref(None)?,
            });
        }
        Ok(attachments)
    }
}

/// Whether a `!DISubprogram`'s fields say it describes a definition, which is
/// written either as `isDefinition: true` or as a `DISPFlagDefinition` bit.
fn is_a_definition(fields: &[(String, MdField)]) -> bool {
    fields
        .iter()
        .any(|(name, value)| match (name.as_str(), value) {
            ("isDefinition", MdField::Bool(set)) => *set,
            ("spFlags", MdField::Words(words)) => {
                words.iter().any(|word| word == "DISPFlagDefinition")
            }
            _ => false,
        })
}

/// `DIExpression` and `DIArgList` print at every use rather than once, so
/// they stay where they are written.
pub(crate) fn prints_in_place(node: &Metadata) -> bool {
    matches!(
        node,
        Metadata::Specialized { tag, .. } if tag == "DIExpression" || tag == "DIArgList"
    )
}

/// Hands a borrowed context down one level. `Option<(&mut _, &mut _)>` is not
/// `Copy`, so each level has to reborrow rather than move.
fn reborrow<'a>(context: &'a mut ValueContext<'_>) -> ValueContext<'a> {
    context
        .as_mut()
        .map(|(function, state)| (&mut **function, &mut **state))
}

/// Removes the fields written with the value they would have had anyway,
/// which upstream does not write back and which unique two nodes that differ
/// only in one. The table is an allow-list from
/// `corpus/md-field-defaults.nu`, the exceptions having no shape.
fn drop_defaulted_fields(
    schema: &'static md_schema::Node,
    tag: &str,
    args: SpecializedArgs,
) -> SpecializedArgs {
    let SpecializedArgs::Named(fields) = args else {
        return args;
    };
    SpecializedArgs::Named(
        fields
            .into_iter()
            .filter(|(name, value)| {
                let required = schema
                    .fields
                    .iter()
                    .find(|spec| spec.name == name)
                    .is_some_and(|spec| spec.required);
                let Some(default) = default_value(tag, name) else {
                    return true;
                };
                // A size or an offset is held whether or not it is written
                // back, so dropping it here would unique two nodes upstream
                // keeps apart. The printer is what leaves it out.
                if required || llvm_ir::metadata::stored_at_zero(tag, name) {
                    return true;
                }
                !is_default(tag, name, default, value)
            })
            .collect(),
    )
}

fn default_value(tag: &str, name: &str) -> Option<&'static str> {
    DEFAULTED
        .binary_search_by_key(&(tag, name), |(kind, field, _)| (kind, field))
        .ok()
        .map(|index| DEFAULTED[index].2)
}

/// A field that takes a word, holding the number the word stands for, so
/// that `tag: 3` and `tag: DW_TAG_entry_point` are one node rather than two.
fn read_vocabulary_words(tag: &str, args: SpecializedArgs) -> SpecializedArgs {
    let SpecializedArgs::Named(fields) = args else {
        return args;
    };
    SpecializedArgs::Named(
        fields
            .into_iter()
            .map(|(name, value)| {
                let MdField::Words(words) = &value else {
                    return (name, value);
                };
                // A flag set is several words joined by `|` and stands for no
                // single number, so only a lone word is read.
                let [word] = words.as_slice() else {
                    return (name, value);
                };
                match llvm_ir::metadata::number(tag, &name, word) {
                    Some(number) => (name, MdField::Unsigned(u128::from(number))),
                    None => (name, value),
                }
            })
            .collect(),
    )
}

fn is_default(tag: &str, name: &str, default: &str, value: &MdField) -> bool {
    // The one field whose default is not its type's zero, and the string
    // fields whose default is no text at all.
    match default {
        "true" => return matches!(value, MdField::Bool(true)),
        "empty" => return matches!(value, MdField::Str(text) if text.is_empty()),
        // An operand list with nothing in it lists nothing, which is the
        // same as not writing one.
        "{}" => {
            return matches!(value, MdField::Inline(node)
                if matches!(&**node, Metadata::Tuple { operands, .. } if operands.is_empty()));
        }
        _ => {}
    }
    // A word-valued field holds the number the word stands for by the time
    // this runs, so its default is looked up the same way.
    if let Some(number) = llvm_ir::metadata::number(tag, name, default) {
        return matches!(value, MdField::Unsigned(written) if *written == u128::from(number));
    }
    match value {
        MdField::Unsigned(0) | MdField::Signed(0) | MdField::Bool(false) => true,
        MdField::Str(text) => text.is_empty(),
        // A field that names a node names none when it is written `null`,
        // which is the same as not writing it.
        MdField::Null => true,
        // A word the vocabulary does not have is kept as it was written,
        // and a default spelled the same way still matches it.
        MdField::Words(words) => words == &[default],
        _ => false,
    }
}

/// Sorted, so the lookup can be a binary search. Generated by
/// `corpus/md-field-defaults.nu`.
/// Each pair with the value that counts as its default, which is the type's
/// zero everywhere but one: a compile unit inlines its split debug info
/// unless it says otherwise.
static DEFAULTED: &[(&str, &str, &str)] = &[
    ("DIBasicType", "align", "0"),
    ("DIBasicType", "encoding", "0"),
    ("DIBasicType", "flags", "0"),
    ("DIBasicType", "name", "empty"),
    ("DIBasicType", "num_extra_inhabitants", "0"),
    ("DIBasicType", "size", "0"),
    ("DIBasicType", "tag", "DW_TAG_base_type"),
    ("DICommonBlock", "file", "null"),
    ("DICommonBlock", "line", "0"),
    ("DICompileUnit", "debugInfoForProfiling", "false"),
    ("DICompileUnit", "dwoId", "0"),
    ("DICompileUnit", "enums", "null"),
    ("DICompileUnit", "flags", "empty"),
    ("DICompileUnit", "globals", "null"),
    ("DICompileUnit", "imports", "null"),
    ("DICompileUnit", "macros", "null"),
    ("DICompileUnit", "producer", "empty"),
    ("DICompileUnit", "rangesBaseAddress", "false"),
    ("DICompileUnit", "retainedTypes", "null"),
    ("DICompileUnit", "sdk", "empty"),
    ("DICompileUnit", "splitDebugFilename", "empty"),
    ("DICompileUnit", "splitDebugInlining", "true"),
    ("DICompileUnit", "sysroot", "empty"),
    ("DICompositeType", "align", "0"),
    ("DICompositeType", "annotations", "null"),
    ("DICompositeType", "baseType", "null"),
    ("DICompositeType", "elements", "null"),
    ("DICompositeType", "file", "null"),
    ("DICompositeType", "flags", "0"),
    ("DICompositeType", "identifier", "empty"),
    ("DICompositeType", "line", "0"),
    ("DICompositeType", "name", "empty"),
    ("DICompositeType", "offset", "0"),
    ("DICompositeType", "runtimeLang", "0"),
    ("DICompositeType", "scope", "null"),
    ("DICompositeType", "size", "0"),
    ("DICompositeType", "templateParams", "null"),
    ("DICompositeType", "vtableHolder", "null"),
    ("DIDerivedType", "align", "0"),
    ("DIDerivedType", "annotations", "null"),
    ("DIDerivedType", "extraData", "null"),
    ("DIDerivedType", "file", "null"),
    ("DIDerivedType", "flags", "0"),
    ("DIDerivedType", "line", "0"),
    ("DIDerivedType", "name", "empty"),
    ("DIDerivedType", "offset", "0"),
    ("DIDerivedType", "scope", "null"),
    ("DIDerivedType", "size", "0"),
    ("DIEnumerator", "isUnsigned", "false"),
    ("DIFile", "source", "empty"),
    ("DIGlobalVariable", "align", "0"),
    ("DIGlobalVariable", "annotations", "null"),
    ("DIGlobalVariable", "declaration", "null"),
    ("DIGlobalVariable", "file", "null"),
    ("DIGlobalVariable", "line", "0"),
    ("DIGlobalVariable", "linkageName", "empty"),
    ("DIGlobalVariable", "templateParams", "null"),
    ("DIGlobalVariable", "type", "null"),
    ("DIImportedEntity", "elements", "null"),
    ("DIImportedEntity", "entity", "null"),
    ("DIImportedEntity", "file", "null"),
    ("DIImportedEntity", "line", "0"),
    ("DIImportedEntity", "name", "empty"),
    ("DILabel", "column", "0"),
    ("DILabel", "file", "null"),
    ("DILexicalBlock", "column", "0"),
    ("DILexicalBlock", "file", "null"),
    ("DILexicalBlock", "line", "0"),
    ("DILexicalBlockFile", "file", "null"),
    ("DILocalVariable", "align", "0"),
    ("DILocalVariable", "annotations", "null"),
    ("DILocalVariable", "arg", "0"),
    ("DILocalVariable", "file", "null"),
    ("DILocalVariable", "flags", "0"),
    ("DILocalVariable", "line", "0"),
    ("DILocalVariable", "name", "empty"),
    ("DILocalVariable", "type", "null"),
    ("DILocation", "column", "0"),
    ("DILocation", "inlinedAt", "null"),
    ("DILocation", "isImplicitCode", "false"),
    ("DIMacro", "line", "0"),
    ("DIMacro", "value", "empty"),
    ("DIMacroFile", "line", "0"),
    ("DIMacroFile", "nodes", "null"),
    ("DIModule", "apinotes", "empty"),
    ("DIModule", "configMacros", "empty"),
    ("DIModule", "file", "null"),
    ("DIModule", "includePath", "empty"),
    ("DIModule", "isDecl", "false"),
    ("DIModule", "line", "0"),
    ("DINamespace", "exportSymbols", "false"),
    ("DINamespace", "name", "empty"),
    ("DIObjCProperty", "attributes", "0"),
    ("DIObjCProperty", "file", "null"),
    ("DIObjCProperty", "getter", "empty"),
    ("DIObjCProperty", "line", "0"),
    ("DIObjCProperty", "name", "empty"),
    ("DIObjCProperty", "setter", "empty"),
    ("DIObjCProperty", "type", "null"),
    ("DIStringType", "tag", "DW_TAG_string_type"),
    ("DISubprogram", "annotations", "null"),
    ("DISubprogram", "containingType", "null"),
    ("DISubprogram", "declaration", "null"),
    ("DISubprogram", "file", "null"),
    ("DISubprogram", "flags", "0"),
    ("DISubprogram", "line", "0"),
    ("DISubprogram", "linkageName", "empty"),
    ("DISubprogram", "name", "empty"),
    ("DISubprogram", "retainedNodes", "null"),
    ("DISubprogram", "scopeLine", "0"),
    ("DISubprogram", "templateParams", "null"),
    ("DISubprogram", "thisAdjustment", "0"),
    ("DISubprogram", "thrownTypes", "null"),
    ("DISubprogram", "type", "null"),
    ("DISubprogram", "virtualIndex", "0"),
    ("DISubroutineType", "cc", "0"),
    ("DISubroutineType", "flags", "0"),
    ("DITemplateTypeParameter", "defaulted", "false"),
    ("DITemplateTypeParameter", "name", "empty"),
    ("DITemplateValueParameter", "defaulted", "false"),
    ("DITemplateValueParameter", "name", "empty"),
    (
        "DITemplateValueParameter",
        "tag",
        "DW_TAG_template_value_parameter",
    ),
    ("DITemplateValueParameter", "type", "null"),
    ("GenericDINode", "header", "empty"),
    ("GenericDINode", "operands", "{}"),
];

/// The fields upstream writes whether or not the module did. They are the
/// ones whose absence says nothing rather than saying the default: what a
/// compile unit was built for, whether a global is local to its unit and
/// whether it is defined here at all. Every other field goes when it holds
/// what it would have held anyway.
fn always_written(tag: &str) -> &'static [(&'static str, &'static str)] {
    match tag {
        "DICompileUnit" => &[
            ("isOptimized", "false"),
            ("runtimeVersion", "0"),
            ("emissionKind", "NoDebug"),
        ],
        // A global variable is a definition unless it says otherwise, where
        // local-to-its-unit is false unless it says otherwise: the two
        // booleans next to each other default opposite ways.
        "DIGlobalVariable" => &[("isLocal", "false"), ("isDefinition", "true")],
        "DILocation" => &[("line", "0")],
        _ => &[],
    }
}

fn fill_compile_unit_defaults(tag: &str, args: SpecializedArgs) -> SpecializedArgs {
    let filled = always_written(tag);
    if filled.is_empty() {
        return args;
    }
    let SpecializedArgs::Named(mut fields) = args else {
        return args;
    };
    for (name, default) in filled {
        if fields.iter().any(|(written, _)| written == name) {
            continue;
        }
        let value = match *default {
            "false" => MdField::Bool(false),
            "true" => MdField::Bool(true),
            "0" => MdField::Unsigned(0),
            word => MdField::Words(vec![word.to_string()]),
        };
        fields.push(((*name).to_string(), value));
    }
    SpecializedArgs::Named(fields)
}

/// The older spelling of a subprogram's flags, which upstream reads and
/// replaces. `isLocal`, `isDefinition`, `isOptimized` and `virtuality` were
/// four fields saying four things; they are one `spFlags` set now, and a node
/// writing any of them is written back with the set instead.
///
/// `isDefinition` is the one whose absence does not mean false: a subprogram
/// written in the old format is a definition unless it says otherwise, which
/// is why `isLocal: true` alone comes back as a definition too.
fn upgrade_subprogram_flags(tag: &str, args: SpecializedArgs) -> SpecializedArgs {
    if tag != "DISubprogram" {
        return args;
    }
    let SpecializedArgs::Named(fields) = args else {
        return args;
    };
    const OLD: [&str; 4] = ["isLocal", "isDefinition", "isOptimized", "virtuality"];
    if !fields.iter().any(|(name, _)| OLD.contains(&name.as_str())) {
        let mut fields = fields;
        // A `unit:` belongs to a subprogram that has a body, so one carrying
        // it is a definition however it was spelled.
        if fields.iter().any(|(name, _)| name == "unit")
            && !fields.iter().any(|(name, _)| name == "spFlags")
        {
            fields.push((
                "spFlags".to_string(),
                MdField::Words(vec!["DISPFlagDefinition".to_string()]),
            ));
        }
        // And every subprogram says what it is scoped to, even when that is
        // nothing.
        if !fields.iter().any(|(name, _)| name == "scope") {
            fields.push(("scope".to_string(), MdField::Null));
        }
        return virtual_index_survives(SpecializedArgs::Named(fields));
    }
    let written = |wanted: &str| {
        fields
            .iter()
            .find(|(name, _)| name == wanted)
            .map(|(_, v)| v)
    };
    let is_set = |wanted: &str| matches!(written(wanted), Some(MdField::Bool(true)));
    let virtuality = match written("virtuality") {
        Some(MdField::Words(words)) => words.first().map(String::as_str),
        _ => None,
    };

    // The order upstream writes them in, which is not the order the four
    // fields were.
    let mut flags = Vec::new();
    match virtuality {
        Some("DW_VIRTUALITY_virtual") => flags.push("DISPFlagVirtual"),
        Some("DW_VIRTUALITY_pure_virtual") => flags.push("DISPFlagPureVirtual"),
        _ => {}
    }
    if is_set("isLocal") {
        flags.push("DISPFlagLocalToUnit");
    }
    if !matches!(written("isDefinition"), Some(MdField::Bool(false))) {
        flags.push("DISPFlagDefinition");
    }
    if is_set("isOptimized") {
        flags.push("DISPFlagOptimized");
    }

    let mut kept: Vec<(String, MdField)> = fields
        .into_iter()
        .filter(|(name, _)| !OLD.contains(&name.as_str()))
        .collect();
    let value = if flags.is_empty() {
        MdField::Unsigned(0)
    } else {
        MdField::Words(flags.iter().map(|flag| (*flag).to_string()).collect())
    };
    kept.push(("spFlags".to_string(), value));
    if !kept.iter().any(|(name, _)| name == "scope") {
        kept.push(("scope".to_string(), MdField::Null));
    }
    virtual_index_survives(SpecializedArgs::Named(kept))
}

/// Which slot in the vtable a subprogram occupies is nought for most of them,
/// and a virtual subprogram writes it anyway: the number means something
/// there, where on a subprogram that is not virtual it means nothing and goes.
fn virtual_index_survives(args: SpecializedArgs) -> SpecializedArgs {
    let SpecializedArgs::Named(mut fields) = args else {
        return args;
    };
    let is_virtual = fields.iter().any(|(name, value)| {
        name == "spFlags"
            && matches!(value, MdField::Words(words)
                if words.iter().any(|word| matches!(word.as_str(), "DISPFlagVirtual" | "DISPFlagPureVirtual")))
    });
    if is_virtual && !fields.iter().any(|(name, _)| name == "virtualIndex") {
        fields.push(("virtualIndex".to_string(), MdField::Unsigned(0)));
    }
    SpecializedArgs::Named(fields)
}
