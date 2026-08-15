//! The printer's state and the module-level driver.

use std::collections::HashMap;
use std::fmt::Write as _;

use llvm_ir::attribute::{Attribute, AttributeSet, EnumAttr, IntAttr};
use llvm_ir::function::Function;
use llvm_ir::instruction::{CallData, InstKind};
use llvm_ir::value::{Name, escape_name, needs_quotes};
use llvm_ir::{BlockId, Module, StructId, TypeId, TypeKind};
use llvm_support::Align;

use crate::md_slots::MetadataSlots;
use crate::print_type;
use crate::slots::{FunctionSlots, ModuleSlots};
use llvm_ir::value::GlobalRef;

/// The column upstream pads a block label to before its predecessor comment.
pub(crate) const LABEL_COMMENT_COLUMN: usize = 50;
/// The indent upstream uses for the continuation lines of `invoke`,
/// `landingpad`, `catchswitch` and friends.
pub(crate) const CONTINUATION: &str = "          ";

pub(crate) struct Printer<'m> {
    pub(crate) module: &'m Module,
    pub(crate) out: String,
    pub(crate) slots: FunctionSlots,
    /// Distinct function-attribute sets, in the order upstream first meets
    /// them. Upstream never prints function attributes inline: it hoists
    /// every set into a numbered group and writes a reference, so the printer
    /// has to build the same table rather than echo whichever groups the
    /// input happened to have.
    groups: Vec<Vec<Attribute>>,
    group_numbers: HashMap<Vec<Attribute>, u32>,
    pub(crate) metadata: MetadataSlots,
    pub(crate) module_slots: ModuleSlots,
}

impl<'m> Printer<'m> {
    pub(crate) fn new(module: &'m Module) -> Printer<'m> {
        Printer {
            module,
            out: String::new(),
            slots: FunctionSlots::default(),
            groups: Vec::new(),
            group_numbers: HashMap::new(),
            metadata: MetadataSlots::default(),
            module_slots: ModuleSlots::compute(module),
        }
    }

    /// What a module-scope symbol prints as, which for an unnamed one is its
    /// slot rather than the number it was written with.
    pub(crate) fn global_text(&self, id: GlobalRef) -> String {
        match self.module_slots.get(id) {
            Some(number) => number.to_string(),
            None => name_text(self.module.global_name(id)),
        }
    }

    pub(crate) fn push(&mut self, text: &str) {
        self.out.push_str(text);
    }

    /// Length of the current output line, for the label padding rule.
    pub(crate) fn column(&self) -> usize {
        match self.out.rfind('\n') {
            Some(index) => self.out.len() - index - 1,
            None => self.out.len(),
        }
    }

    // ---------------------------------------------------------------- module

    /// Numbers every function-attribute set, in upstream's discovery order:
    /// globals, aliases and ifuncs first, then each function, then the call
    /// sites of each function body in turn.
    ///
    /// "Each function" is in print order rather than arena order, a group's
    /// number being its first use as printed. A renamed intrinsic
    /// declaration prints last and takes the last number with it, which is
    /// measured: `declare void @llvm.lifetime.start(i64, ptr)` written above
    /// a definition comes back below it and numbered after it.
    fn assign_attribute_groups(&mut self) {
        let module = self.module;
        let order = module.function_print_order();
        let mut sets: Vec<Vec<Attribute>> = Vec::new();
        for global in &module.globals {
            sets.push(self.resolved_attributes(&global.attrs));
        }
        for index in &order {
            let function = &module.functions[*index];
            // A debug-info intrinsic's declaration is not printed, so the
            // group it names has one fewer user, and a group with none is
            // not printed either.
            if is_debug_intrinsic(function) {
                continue;
            }
            sets.push(self.resolved_attributes(&function.attrs));
        }
        for index in &order {
            let function = &module.functions[*index];
            for (id, _) in function.blocks() {
                for (_, instruction) in function.block_instructions(id) {
                    if let Some(call) = call_data(&instruction.kind) {
                        sets.push(self.resolved_attributes(&call.fn_attrs));
                    }
                }
            }
        }
        for set in sets {
            if set.is_empty() || self.group_numbers.contains_key(&set) {
                continue;
            }
            let number = self.groups.len() as u32;
            self.group_numbers.insert(set.clone(), number);
            self.groups.push(set);
        }
    }

    /// The group number an attribute set prints as, if it has one.
    pub(crate) fn group_for(&self, set: &AttributeSet) -> Option<u32> {
        let resolved = self.resolved_attributes(set);
        if resolved.is_empty() {
            return None;
        }
        self.group_numbers.get(&resolved).copied()
    }

    pub(crate) fn module(&mut self) {
        self.assign_attribute_groups();
        self.metadata = MetadataSlots::compute(self.module);
        if let Some(id) = &self.module.module_id {
            let _ = writeln!(self.out, "; ModuleID = '{id}'");
        }
        if let Some(name) = &self.module.source_filename {
            let _ = writeln!(self.out, "source_filename = \"{}\"", escape_string(name));
        }
        if let Some(layout) = &self.module.data_layout {
            let _ = writeln!(self.out, "target datalayout = \"{}\"", layout.as_str());
        }
        if let Some(triple) = &self.module.triple {
            let _ = writeln!(self.out, "target triple = \"{}\"", triple.as_str());
        }

        if !self.module.module_asm.is_empty() {
            self.push("\n");
            for line in &self.module.module_asm {
                let _ = writeln!(self.out, "module asm \"{}\"", escape_string(line));
            }
        }

        self.type_identities();

        // A comdat is a group for symbols to join, so one nothing joins says
        // nothing and is not written back.
        let joined = self.comdats_in_use();
        // A comdat stands on its own, so each is preceded by a blank line
        // rather than the group being preceded by one.
        for comdat in &self.module.comdats {
            if !joined.contains(&comdat.name) {
                continue;
            }
            self.push("\n");
            let _ = writeln!(
                self.out,
                "${} = comdat {}",
                identifier(&comdat.name),
                comdat.kind.keyword()
            );
        }

        if !self.module.globals.is_empty() {
            self.push("\n");
            for index in 0..self.module.globals.len() {
                self.global(index, &self.module.globals[index]);
                self.push("\n");
            }
        }

        if !self.module.aliases.is_empty() {
            self.push("\n");
            for index in 0..self.module.aliases.len() {
                self.alias(index, &self.module.aliases[index]);
            }
        }

        if !self.module.ifuncs.is_empty() {
            self.push("\n");
            for index in 0..self.module.ifuncs.len() {
                self.ifunc(index, &self.module.ifuncs[index]);
            }
        }

        for index in self.module.function_print_order() {
            // The four debug-info intrinsics are the older spelling of the
            // debug records, and upstream's reader replaces every call to
            // one and drops the declaration, whether it was called or not.
            // The declaration is kept in the model so that a constant built
            // during parsing still resolves, and left unprinted here.
            if is_debug_intrinsic(&self.module.functions[index]) {
                continue;
            }
            self.push("\n");
            self.function(index, &self.module.functions[index]);
        }

        if !self.groups.is_empty() {
            self.push("\n");
            for number in 0..self.groups.len() {
                let set = AttributeSet {
                    attributes: self.groups[number].clone(),
                    groups: Vec::new(),
                };
                let _ = writeln!(
                    self.out,
                    "attributes #{number} = {{ {} }}",
                    attribute_list(self.module, &set, true)
                );
            }
        }

        // The summary index is read and not written back, which is what
        // upstream's `opt -S` does with one: a module carrying `^0 = module:
        // (...)` comes back without it, body and all else intact. The index
        // is a thing beside the module rather than part of it, and the
        // textual writer does not hold it.
        //
        // `llvm-dis` is the tool that does write one, and it writes the index
        // the bitcode reader built rather than the text that was read: the
        // path and hash come from the file it opened and a `; guid = N`
        // comment is appended. Printing back what the module wrote was
        // neither of those, and it was ours rather than measured.

        if !self.module.named_metadata.is_empty() {
            self.push("\n");
            for index in 0..self.module.named_metadata.len() {
                let named = &self.module.named_metadata[index];
                let name = metadata_name(&named.name);
                let operands = named.operands.clone();
                let _ = write!(self.out, "!{name} = !{{");
                for (position, operand) in operands.iter().enumerate() {
                    if position > 0 {
                        self.push(", ");
                    }
                    self.metadata_reference(*operand);
                }
                self.push("}\n");
            }
        }

        if !self.metadata.order.is_empty() {
            self.push("\n");
            let ids: Vec<_> = self.metadata.order.clone();
            for (number, id) in ids.into_iter().enumerate() {
                let node = self
                    .module
                    .metadata_node(id)
                    .expect("id came from the traversal")
                    .clone();
                let _ = write!(self.out, "!{number} = ");
                // A composite type that claimed an identifier prints
                // `distinct` whether or not the module wrote it, and only the
                // one that claimed it does, so the slots answer rather than
                // the node. A node that wrote the keyword itself prints it
                // below, hence the guard: the two answers overlap.
                if !node.is_distinct() && self.metadata.claimed_an_identifier(id) {
                    self.push("distinct ");
                }
                self.metadata_definition(&node);
                self.push("\n");
            }
        }
    }

    /// `%name = type { ... }` for the identified structs the module still
    /// reaches, in the order the walk meets them. A name nothing refers to is
    /// not written, which is what upstream's type finder decides.
    pub(crate) fn type_identities(&mut self) {
        let ids: Vec<StructId> = crate::type_finder::reachable_named_structs(self.module);
        if ids.is_empty() {
            return;
        }
        self.push("\n");
        for id in ids {
            let def = self.module.ctx.struct_def(id);
            let name = struct_name(def);
            let fields = def.fields.clone();
            let packed = def.packed;
            let _ = write!(self.out, "%{name} = type ");
            match fields {
                None => self.push("opaque"),
                Some(fields) => self.struct_body(&fields, packed),
            }
            self.push("\n");
        }
    }

    pub(crate) fn struct_body(&mut self, fields: &[TypeId], packed: bool) {
        if packed {
            self.push("<");
        }
        if fields.is_empty() {
            self.push("{}");
        } else {
            self.push("{ ");
            for (index, field) in fields.iter().enumerate() {
                if index > 0 {
                    self.push(", ");
                }
                self.ty(*field);
            }
            self.push(" }");
        }
        if packed {
            self.push(">");
        }
    }

    // ----------------------------------------------------------------- types

    pub(crate) fn ty(&mut self, id: TypeId) {
        match self.module.ctx.type_kind(id).clone() {
            TypeKind::Void => self.push("void"),
            TypeKind::Label => self.push("label"),
            TypeKind::Metadata => self.push("metadata"),
            TypeKind::Token => self.push("token"),
            TypeKind::X86Amx => self.push("x86_amx"),
            TypeKind::Integer(bits) => {
                let _ = write!(self.out, "i{bits}");
            }
            TypeKind::Float(semantics) => self.push(semantics.type_name()),
            TypeKind::Pointer { address_space } => {
                self.push("ptr");
                if address_space != 0 {
                    let _ = write!(self.out, " addrspace({address_space})");
                }
            }
            TypeKind::Array { element, count } => {
                let _ = write!(self.out, "[{count} x ");
                self.ty(element);
                self.push("]");
            }
            TypeKind::Vector {
                element,
                count,
                scalable,
            } => {
                self.push("<");
                if scalable {
                    self.push("vscale x ");
                }
                let _ = write!(self.out, "{count} x ");
                self.ty(element);
                self.push(">");
            }
            TypeKind::Struct { fields, packed } => self.struct_body(&fields, packed),
            TypeKind::NamedStruct(struct_id) => {
                let name = struct_name(self.module.ctx.struct_def(struct_id));
                let _ = write!(self.out, "%{name}");
            }
            TypeKind::Function {
                result,
                params,
                is_var_arg,
            } => {
                self.ty(result);
                self.push(" (");
                for (index, param) in params.iter().enumerate() {
                    if index > 0 {
                        self.push(", ");
                    }
                    self.ty(*param);
                }
                if is_var_arg {
                    if params.is_empty() {
                        self.push("...");
                    } else {
                        self.push(", ...");
                    }
                }
                self.push(")");
            }
            TypeKind::Target { name, types, ints } => {
                let _ = write!(self.out, "target(\"{}\"", escape_string(&name));
                for ty in types {
                    self.push(", ");
                    self.ty(ty);
                }
                for int in ints {
                    let _ = write!(self.out, ", {int}");
                }
                self.push(")");
            }
        }
    }
}

/// Predecessor lists, indexed by block.
///
/// Upstream walks a block's use list, which grows at the front, so the
/// predecessors come out in reverse order of the terminators that name them.
/// A terminator naming the same block twice contributes two entries, exactly
/// as upstream does.
pub(crate) fn predecessors(function: &Function) -> Vec<Vec<BlockId>> {
    let mut preds = vec![Vec::new(); function.block_count()];
    for (id, _) in function.blocks() {
        for (_, instruction) in function.block_instructions(id) {
            for successor in instruction.kind.successors() {
                preds[successor.0 as usize].push(id);
            }
        }
    }
    for list in &mut preds {
        list.reverse();
    }
    preds
}

/// Escapes a string the way upstream's `printEscapedString` does: anything
/// outside printable ASCII, plus the backslash and the double quote, becomes
/// a backslash and two uppercase hex digits.
pub(crate) fn escape_string(text: &str) -> String {
    escape_bytes(text.as_bytes())
}

/// The same rule over raw bytes, which is what a `c"..."` string needs.
///
/// A backslash prints as a doubled backslash rather than as `\5C`, while a
/// double quote prints as `\22`. That asymmetry is upstream's.
pub(crate) fn escape_bytes(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for byte in bytes {
        match byte {
            b'\\' => out.push_str("\\\\"),
            b'"' => out.push_str("\\22"),
            0x20..=0x7e => out.push(*byte as char),
            _ => {
                let _ = write!(out, "\\{byte:02X}");
            }
        }
    }
    out
}

/// An identifier, quoted when it has to be.
///
/// Upstream's printer allows only alphanumerics, `-`, `.` and `_` bare, which
/// is narrower than what its lexer accepts: a `$` in a mangled Rust symbol
/// reads back fine unquoted but always prints quoted.
pub(crate) fn identifier(name: &str) -> String {
    if needs_quotes(name) || name.contains('$') {
        format!("\"{}\"", escape_name(name))
    } else {
        name.to_string()
    }
}

/// A struct's name as it prints: bare when the struct is numbered, quoted
/// when the word needs it.
fn struct_name(def: &llvm_ir::types::StructDef) -> String {
    if def.numbered {
        def.name.clone()
    } else {
        identifier(&def.name)
    }
}

/// A metadata name prints with `\\xx` for anything the bare grammar has no
/// room for, rather than being quoted the way a value's name is.
pub(crate) fn metadata_name(name: &llvm_ir::ByteString) -> String {
    let mut out = String::new();
    for (index, byte) in name.as_bytes().iter().copied().enumerate() {
        // A name that opens with a digit reads as a number, so `!111` would
        // be node 111 and `!42abc` a node followed by nothing that parses.
        // Escaping the first character alone is enough to say it is a name,
        // and what upstream does: `!\3111` is the name `111`.
        let first_is_a_digit = index == 0 && byte.is_ascii_digit();
        if !first_is_a_digit
            && (byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'$' | b'-'))
        {
            out.push(byte as char);
        } else {
            let _ = write!(out, "\\{byte:02X}");
        }
    }
    out
}

pub(crate) fn name_text(name: &Name) -> String {
    match name {
        Name::Named(text) => identifier(text),
        Name::Number(number) => number.to_string(),
    }
}

pub(crate) fn align_text(align: Option<Align>) -> String {
    match align {
        Some(align) => format!(", align {}", align.bytes()),
        None => String::new(),
    }
}

/// The attributes of a set, space separated.
///
/// `in_group` picks between the two spellings upstream uses for the same
/// attribute: `align 8` in a parameter list, `align=8` inside an attribute
/// group. Getting this wrong is invisible until a round trip fails.
/// A run of attributes as they appear on a parameter or a return value,
/// which upstream writes in its own order the way it does a function's set.
pub(crate) fn attribute_list(module: &Module, set: &AttributeSet, in_group: bool) -> String {
    let mut attributes = set.attributes.clone();
    attributes.sort_by(compare_attributes);
    attributes
        .iter()
        .map(|attribute| attribute_text(module, attribute, in_group))
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn attribute_text(module: &Module, attribute: &Attribute, in_group: bool) -> String {
    match attribute {
        Attribute::Enum(kind) => kind.keyword().to_string(),
        Attribute::Int {
            kind,
            first,
            second,
        } => match (kind, in_group, second) {
            (IntAttr::Align, true, _) => format!("align={first}"),
            (IntAttr::Align, false, _) => format!("align {first}"),
            (IntAttr::AlignStack, true, _) => format!("alignstack={first}"),
            (_, _, Some(second)) => format!("{}({first},{second})", kind.keyword()),
            (_, _, None) => format!("{}({first})", kind.keyword()),
        },
        Attribute::Type { kind, ty } => {
            format!("{}({})", kind.keyword(), print_type(module, *ty))
        }
        Attribute::Range { ty, lower, upper } => format!(
            "range({} {}, {})",
            print_type(module, *ty),
            lower.to_string_signed(),
            upper.to_string_signed()
        ),
        Attribute::Structured { kind, arguments } => {
            format!("{}({arguments})", kind.keyword())
        }
        Attribute::String { key, value } => match value {
            Some(value) => format!("\"{}\"=\"{}\"", escape_string(key), escape_string(value)),
            None => format!("\"{}\"", escape_string(key)),
        },
    }
}

impl Printer<'_> {
    /// A function's attributes, with any groups it names expanded in place.
    pub(crate) fn resolved_attributes(&self, set: &AttributeSet) -> Vec<Attribute> {
        let mut all = set.attributes.clone();
        for group in &set.groups {
            if let Some(attributes) = self.module.attribute_group(*group) {
                all.extend(attributes.iter().cloned());
            }
        }
        // Upstream writes a set in its own order rather than the one the text
        // used, so two functions that carry the same attributes in different
        // orders share a group. Plain keywords come first and alphabetically,
        // then the ones that take an argument in the order upstream numbers
        // them, then the quoted ones by key.
        all.sort_by(compare_attributes);
        all
    }
}

/// The call data of whichever instruction carries some.
fn call_data(kind: &InstKind) -> Option<&CallData> {
    match kind {
        InstKind::Call(call) | InstKind::Invoke { call, .. } | InstKind::CallBr { call, .. } => {
            Some(call)
        }
        _ => None,
    }
}

/// Whether a function is one of the declarations upstream reads and never
/// writes back.
///
/// Two kinds. The four debug-info intrinsics are read as records, and the
/// handful that are read as an instruction rather than as a call go the same
/// way: upstream drops
/// `declare i32 @llvm.nvvm.atomic.load.inc.32.p0(ptr, i32)` whether or not
/// anything called it, which was measured on its own because it is not the
/// same question as what a call becomes.
///
/// Both are kept in the model rather than removed from it, so that a
/// constant built while parsing still resolves and an id still means what it
/// meant. Leaving them unwritten is what upstream's output shows.
fn is_debug_intrinsic(function: &llvm_ir::function::Function) -> bool {
    if !function.block_order.is_empty() {
        return false;
    }
    let llvm_ir::value::Name::Named(name) = &function.name else {
        return false;
    };
    matches!(
        name.as_str(),
        "llvm.dbg.declare" | "llvm.dbg.value" | "llvm.dbg.assign" | "llvm.dbg.label"
    ) || llvm_ir::intrinsic::rewrites::is_rewritten(name)
}

/// The order upstream writes a set in: plain keywords, then the ones taking
/// an argument, then the quoted ones by key. Neither of the first two is
/// alphabetical; `EnumAttr` is declared in LLVM's order and the second run's
/// order was measured.
fn compare_attributes(left: &Attribute, right: &Attribute) -> std::cmp::Ordering {
    run_of(left)
        .cmp(&run_of(right))
        .then_with(|| match (left, right) {
            (Attribute::Enum(a), Attribute::Enum(b)) => a.cmp(b),
            (Attribute::String { key: a, .. }, Attribute::String { key: b, .. }) => a.cmp(b),
            _ => structured_place(left).cmp(&structured_place(right)),
        })
}

fn run_of(attribute: &Attribute) -> u8 {
    match attribute {
        // `uwtable` is written bare or with a kind, and upstream sorts it
        // with the ones that take an argument either way: the bare spelling
        // is the same attribute carrying its default.
        Attribute::Enum(EnumAttr::UwTable) => 1,
        Attribute::Enum(_) => 0,
        Attribute::String { .. } => 2,
        _ => 1,
    }
}

fn structured_place(attribute: &Attribute) -> u8 {
    let keyword = match attribute {
        Attribute::Structured { kind, .. } => kind.keyword(),
        Attribute::Int { kind, .. } => kind.keyword(),
        Attribute::Type { kind, .. } => kind.keyword(),
        Attribute::Range { .. } => "range",
        Attribute::Enum(EnumAttr::UwTable) => "uwtable",
        Attribute::Enum(_) | Attribute::String { .. } => return u8::MAX,
    };
    [
        "allockind",
        "allocsize",
        "memory",
        "alignstack",
        "uwtable",
        "vscale_range",
    ]
    .iter()
    .position(|known| *known == keyword)
    .map_or(u8::MAX, |place| place as u8)
}

/// The comdats some symbol joins. A bare `comdat` clause names the one the
/// symbol's own name makes, so the symbol is what says which.
impl Printer<'_> {
    fn comdats_in_use(&self) -> Vec<String> {
        let mut names = Vec::new();
        let mut join = |comdat: &Option<llvm_ir::global::ComdatRef>, symbol: &Name| {
            let Some(comdat) = comdat else {
                return;
            };
            let name = match (&comdat.name, symbol) {
                (Some(name), _) => name.clone(),
                (None, Name::Named(name)) => name.clone(),
                (None, Name::Number(number)) => number.to_string(),
            };
            if !names.contains(&name) {
                names.push(name);
            }
        };
        for global in &self.module.globals {
            join(&global.comdat, &global.name);
        }
        for function in &self.module.functions {
            join(&function.comdat, &function.name);
        }
        names
    }
}

/// Whether a function's name starts with `llvm.` and names no intrinsic
/// upstream knows.
///
/// The same question the parser's gate asks, and it has to be the same
/// answer: a name upstream recognises is one it builds a declaration for and
/// one it prints no comment above. Asking only what LangRef documents put
/// the comment above every target intrinsic, which upstream knows perfectly
/// well.
pub(crate) fn unknown_intrinsic(function: &Function) -> bool {
    let Name::Named(name) = &function.name else {
        return false;
    };
    if !name.starts_with("llvm.") {
        return false;
    }
    !llvm_ir::intrinsic::is_known(name)
}
