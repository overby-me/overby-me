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
                self.check_specialized(schema, &tag, distinct, &args)?;
                let args = drop_defaulted_fields(schema, &tag, args);
                let args = fill_compile_unit_defaults(&tag, args);
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
            self.check_field(spec, value)?;
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
        spec: &'static md_schema::Field,
        value: &MdField,
    ) -> Result<(), ParseError> {
        let name = spec.name;
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
                if required || !drops_at_its_default(tag, name) {
                    return true;
                }
                !is_default(name, value)
            })
            .collect(),
    )
}

fn drops_at_its_default(tag: &str, name: &str) -> bool {
    DEFAULTED.binary_search(&(tag, name)).is_ok()
}

fn is_default(name: &str, value: &MdField) -> bool {
    match value {
        MdField::Unsigned(0) | MdField::Signed(0) | MdField::Bool(false) => true,
        MdField::Str(text) => text.is_empty(),
        // A field that names a node names none when it is written `null`,
        // which is the same as not writing it.
        MdField::Null => true,
        // `tag:` is the one word-valued field with a default.
        MdField::Words(words) => name == "tag" && words == &["DW_TAG_base_type"],
        _ => false,
    }
}

/// Sorted, so the lookup can be a binary search. Generated by
/// `corpus/md-field-defaults.nu`.
static DEFAULTED: &[(&str, &str)] = &[
    ("DIBasicType", "align"),
    ("DIBasicType", "encoding"),
    ("DIBasicType", "flags"),
    ("DIBasicType", "name"),
    ("DIBasicType", "num_extra_inhabitants"),
    ("DIBasicType", "size"),
    ("DIBasicType", "tag"),
    ("DICompileUnit", "debugInfoForProfiling"),
    ("DICompileUnit", "dwoId"),
    ("DICompileUnit", "enums"),
    ("DICompileUnit", "globals"),
    ("DICompileUnit", "imports"),
    ("DICompileUnit", "macros"),
    ("DICompileUnit", "rangesBaseAddress"),
    ("DICompileUnit", "retainedTypes"),
    ("DICompileUnit", "sdk"),
    ("DICompileUnit", "splitDebugFilename"),
    ("DICompileUnit", "sysroot"),
    ("DICompositeType", "align"),
    ("DICompositeType", "annotations"),
    ("DICompositeType", "baseType"),
    ("DICompositeType", "elements"),
    ("DICompositeType", "file"),
    ("DICompositeType", "flags"),
    ("DICompositeType", "identifier"),
    ("DICompositeType", "line"),
    ("DICompositeType", "name"),
    ("DICompositeType", "offset"),
    ("DICompositeType", "runtimeLang"),
    ("DICompositeType", "scope"),
    ("DICompositeType", "size"),
    ("DICompositeType", "templateParams"),
    ("DICompositeType", "vtableHolder"),
    ("DIDerivedType", "align"),
    ("DIDerivedType", "annotations"),
    ("DIDerivedType", "extraData"),
    ("DIDerivedType", "file"),
    ("DIDerivedType", "flags"),
    ("DIDerivedType", "line"),
    ("DIDerivedType", "name"),
    ("DIDerivedType", "offset"),
    ("DIDerivedType", "scope"),
    ("DIDerivedType", "size"),
    ("DIEnumerator", "isUnsigned"),
    ("DIFile", "source"),
    ("DIGlobalVariable", "align"),
    ("DIGlobalVariable", "annotations"),
    ("DIGlobalVariable", "declaration"),
    ("DIGlobalVariable", "file"),
    ("DIGlobalVariable", "line"),
    ("DIGlobalVariable", "linkageName"),
    ("DIGlobalVariable", "templateParams"),
    ("DIGlobalVariable", "type"),
    ("DIImportedEntity", "elements"),
    ("DIImportedEntity", "entity"),
    ("DIImportedEntity", "file"),
    ("DIImportedEntity", "line"),
    ("DIImportedEntity", "name"),
    ("DILabel", "column"),
    ("DILabel", "file"),
    ("DILexicalBlock", "column"),
    ("DILexicalBlock", "file"),
    ("DILexicalBlock", "line"),
    ("DILocalVariable", "align"),
    ("DILocalVariable", "annotations"),
    ("DILocalVariable", "arg"),
    ("DILocalVariable", "file"),
    ("DILocalVariable", "flags"),
    ("DILocalVariable", "line"),
    ("DILocalVariable", "name"),
    ("DILocalVariable", "type"),
    ("DILocation", "column"),
    ("DILocation", "inlinedAt"),
    ("DILocation", "isImplicitCode"),
    ("DIMacro", "line"),
    ("DIMacro", "value"),
    ("DIMacroFile", "line"),
    ("DIMacroFile", "nodes"),
    ("DIModule", "apinotes"),
    ("DIModule", "configMacros"),
    ("DIModule", "file"),
    ("DIModule", "includePath"),
    ("DIModule", "isDecl"),
    ("DIModule", "line"),
    ("DINamespace", "exportSymbols"),
    ("DINamespace", "name"),
    ("DIObjCProperty", "attributes"),
    ("DIObjCProperty", "file"),
    ("DIObjCProperty", "getter"),
    ("DIObjCProperty", "line"),
    ("DIObjCProperty", "name"),
    ("DIObjCProperty", "setter"),
    ("DIObjCProperty", "type"),
    ("DISubprogram", "annotations"),
    ("DISubprogram", "containingType"),
    ("DISubprogram", "declaration"),
    ("DISubprogram", "file"),
    ("DISubprogram", "flags"),
    ("DISubprogram", "line"),
    ("DISubprogram", "linkageName"),
    ("DISubprogram", "name"),
    ("DISubprogram", "retainedNodes"),
    ("DISubprogram", "scopeLine"),
    ("DISubprogram", "templateParams"),
    ("DISubprogram", "thisAdjustment"),
    ("DISubprogram", "thrownTypes"),
    ("DISubprogram", "type"),
    ("DISubprogram", "virtualIndex"),
    ("DISubroutineType", "cc"),
    ("DISubroutineType", "flags"),
    ("DITemplateTypeParameter", "defaulted"),
    ("DITemplateTypeParameter", "name"),
    ("DITemplateValueParameter", "defaulted"),
    ("DITemplateValueParameter", "name"),
    ("DITemplateValueParameter", "type"),
];

/// A compile unit is the one node upstream writes fields on that the module
/// never wrote. Three of them: whether the code was optimised, which Objective-C
/// runtime it targets, and how much debug info to emit. They say something
/// about the whole translation unit, so leaving them out says nothing rather
/// than saying the default.
fn fill_compile_unit_defaults(tag: &str, args: SpecializedArgs) -> SpecializedArgs {
    if tag != "DICompileUnit" {
        return args;
    }
    let SpecializedArgs::Named(mut fields) = args else {
        return args;
    };
    for (name, value) in [
        ("isOptimized", MdField::Bool(false)),
        ("runtimeVersion", MdField::Unsigned(0)),
        ("emissionKind", MdField::Words(vec!["NoDebug".to_string()])),
    ] {
        if !fields.iter().any(|(written, _)| written == name) {
            fields.push((name.to_string(), value));
        }
    }
    SpecializedArgs::Named(fields)
}
