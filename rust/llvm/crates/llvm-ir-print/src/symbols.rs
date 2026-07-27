//! Printing everything that lives at module scope with a name: global
//! variables, aliases, ifuncs and functions.

use std::fmt::Write as _;

use llvm_ir::BlockId;
use llvm_ir::attribute::Attribute;
use llvm_ir::function::Function;
use llvm_ir::global::{Alias, GlobalQualifiers, GlobalVariable, IFunc, Linkage};

use crate::slots::FunctionSlots;
use crate::{
    LABEL_COMMENT_COLUMN, Printer, attribute_list, attribute_text, escape_string, identifier,
    name_text, predecessors,
};

impl Printer<'_> {
    // --------------------------------------------------------------- globals

    pub(crate) fn global(&mut self, global: &GlobalVariable) {
        let _ = write!(self.out, "@{} = ", name_text(&global.name));
        if global.initializer.is_none()
            && matches!(global.qualifiers.linkage, None | Some(Linkage::External))
        {
            self.push("external ");
        }
        self.qualifiers(&global.qualifiers, true, true);
        if global.externally_initialized {
            self.push("externally_initialized ");
        }
        self.push(if global.is_constant {
            "constant "
        } else {
            "global "
        });
        self.ty(global.value_type);
        if let Some(initializer) = global.initializer {
            self.push(" ");
            self.constant(initializer);
        }
        for (set, word) in [
            (global.sanitizer.no_address, "no_sanitize_address"),
            (global.sanitizer.no_hwaddress, "no_sanitize_hwaddress"),
            (global.sanitizer.address_dyninit, "sanitize_address_dyninit"),
            (global.sanitizer.memtag, "sanitize_memtag"),
        ] {
            if set {
                let _ = write!(self.out, ", {word}");
            }
        }
        if let Some(section) = &global.section {
            let _ = write!(self.out, ", section \"{}\"", escape_string(section));
        }
        if let Some(partition) = &global.partition {
            let _ = write!(self.out, ", partition \"{}\"", escape_string(partition));
        }
        if let Some(model) = &global.code_model {
            let _ = write!(self.out, ", code_model \"{model}\"");
        }
        if let Some(comdat) = &global.comdat {
            self.comdat_clause(comdat);
        }
        if let Some(align) = global.align {
            let _ = write!(self.out, ", align {}", align.bytes());
        }
        self.metadata_attachments(&global.metadata, ", ");
        if let Some(group) = self.group_for(&global.attrs) {
            let _ = write!(self.out, " #{group}");
        }
    }

    pub(crate) fn alias(&mut self, alias: &Alias) {
        let _ = write!(self.out, "@{} = ", name_text(&alias.name));
        self.qualifiers(&alias.qualifiers, true, true);
        self.push("alias ");
        self.ty(alias.value_type);
        self.push(", ");
        // A constant expression aliasee writes what it produces itself, so
        // upstream puts no type in front of it. A bare symbol does need one.
        if matches!(
            self.module.ctx.constant(alias.aliasee),
            llvm_ir::constant::Constant::Expression(_)
        ) {
            self.constant(alias.aliasee);
        } else {
            self.constant_with_type(alias.aliasee);
        }
        if let Some(partition) = &alias.partition {
            let _ = write!(self.out, ", partition \"{}\"", escape_string(partition));
        }
        self.metadata_attachments(&alias.metadata, ", ");
        self.push("\n");
    }

    pub(crate) fn ifunc(&mut self, ifunc: &IFunc) {
        let _ = write!(self.out, "@{} = ", name_text(&ifunc.name));
        self.qualifiers(&ifunc.qualifiers, true, true);
        self.push("ifunc ");
        self.ty(ifunc.value_type);
        self.push(", ");
        self.constant_with_type(ifunc.resolver);
        if let Some(partition) = &ifunc.partition {
            let _ = write!(self.out, ", partition \"{}\"", escape_string(partition));
        }
        self.metadata_attachments(&ifunc.metadata, ", ");
        self.push("\n");
    }

    /// The qualifier run every global-scope symbol starts with. Each piece
    /// prints with a trailing space, and `external` linkage prints as nothing.
    pub(crate) fn qualifiers(
        &mut self,
        qualifiers: &GlobalQualifiers,
        include_address_space: bool,
        include_unnamed_addr: bool,
    ) {
        if let Some(linkage) = qualifiers.linkage
            && linkage != Linkage::External
        {
            let _ = write!(self.out, "{} ", linkage.keyword());
        }
        if let Some(preemption) = qualifiers.preemption {
            let _ = write!(self.out, "{} ", preemption.keyword());
        }
        if let Some(visibility) = qualifiers.visibility {
            let _ = write!(self.out, "{} ", visibility.keyword());
        }
        if let Some(storage) = qualifiers.dll_storage {
            let _ = write!(self.out, "{} ", storage.keyword());
        }
        if let Some(model) = &qualifiers.thread_local {
            match model {
                None => self.push("thread_local "),
                Some(model) => {
                    let _ = write!(self.out, "thread_local({}) ", model.keyword());
                }
            }
        }
        if include_unnamed_addr && let Some(unnamed) = qualifiers.unnamed_addr {
            let _ = write!(self.out, "{} ", unnamed.keyword());
        }
        if include_address_space
            && let Some(address_space) = qualifiers.address_space
            && address_space != 0
        {
            let _ = write!(self.out, "addrspace({address_space}) ");
        }
    }

    pub(crate) fn comdat_clause(&mut self, comdat: &llvm_ir::global::ComdatRef) {
        match &comdat.name {
            None => self.push(", comdat"),
            Some(name) => {
                let _ = write!(self.out, ", comdat(${})", identifier(name));
            }
        }
    }

    // ------------------------------------------------------------- functions

    pub(crate) fn function(&mut self, function: &Function) {
        let resolved = self.resolved_attributes(&function.attrs);
        let comment: Vec<String> = resolved
            .iter()
            .filter(|attribute| !matches!(attribute, Attribute::String { .. }))
            .map(|attribute| attribute_text(self.module, attribute, false))
            .collect();
        if !comment.is_empty() {
            let _ = writeln!(self.out, "; Function Attrs: {}", comment.join(" "));
        }

        self.push(if function.is_definition() {
            "define "
        } else {
            "declare "
        });
        self.qualifiers(&function.qualifiers, false, false);
        self.calling_conv(function.calling_conv);
        if !function.return_attrs.is_empty() {
            let _ = write!(
                self.out,
                "{} ",
                attribute_list(self.module, &function.return_attrs, false)
            );
        }
        self.ty(function.return_type);
        let _ = write!(self.out, " @{}(", name_text(&function.name));

        self.slots = FunctionSlots::compute(self.module, function);
        for (index, param) in function.params.iter().enumerate() {
            if index > 0 {
                self.push(", ");
            }
            self.ty(param.ty);
            if !param.attrs.is_empty() {
                let _ = write!(
                    self.out,
                    " {}",
                    attribute_list(self.module, &param.attrs, false)
                );
            }
            match &param.name {
                Some(name) => {
                    let _ = write!(self.out, " %{}", name_text(name));
                }
                None => {
                    if function.is_definition() {
                        let slot = self
                            .slots
                            .argument(index as u32)
                            .expect("every unnamed argument gets a slot");
                        let _ = write!(self.out, " %{slot}");
                    }
                }
            }
        }
        if function.is_var_arg {
            if function.params.is_empty() {
                self.push("...");
            } else {
                self.push(", ...");
            }
        }
        self.push(")");

        if let Some(unnamed) = function.qualifiers.unnamed_addr {
            let _ = write!(self.out, " {}", unnamed.keyword());
        }
        if let Some(address_space) = function.qualifiers.address_space
            && address_space != 0
        {
            let _ = write!(self.out, " addrspace({address_space})");
        }
        if let Some(group) = self.group_for(&function.attrs) {
            let _ = write!(self.out, " #{group}");
        }
        if let Some(section) = &function.section {
            let _ = write!(self.out, " section \"{}\"", escape_string(section));
        }
        if let Some(partition) = &function.partition {
            let _ = write!(self.out, " partition \"{}\"", escape_string(partition));
        }
        if let Some(comdat) = &function.comdat {
            match &comdat.name {
                None => self.push(" comdat"),
                Some(name) => {
                    let _ = write!(self.out, " comdat(${})", identifier(name));
                }
            }
        }
        if let Some(align) = function.align {
            let _ = write!(self.out, " align {}", align.bytes());
        }
        if let Some(gc) = &function.gc {
            let _ = write!(self.out, " gc \"{}\"", escape_string(gc));
        }
        if let Some((ty, value)) = function.prefix {
            self.push(" prefix ");
            self.ty(ty);
            self.push(" ");
            self.constant(value);
        }
        if let Some((ty, value)) = function.prologue {
            self.push(" prologue ");
            self.ty(ty);
            self.push(" ");
            self.constant(value);
        }
        if let Some((ty, value)) = function.personality {
            self.push(" personality ");
            self.ty(ty);
            self.push(" ");
            self.constant(value);
        }
        self.metadata_attachments(&function.metadata, " ");

        if !function.is_definition() {
            self.push("\n");
            return;
        }

        self.push(" {");
        let predecessors = predecessors(function);
        let blocks: Vec<BlockId> = function.block_order.clone();
        for (index, id) in blocks.iter().enumerate() {
            self.basic_block(function, *id, index == 0, &predecessors);
        }
        self.push("}\n");
    }

    pub(crate) fn basic_block(
        &mut self,
        function: &Function,
        id: BlockId,
        is_entry: bool,
        predecessors: &[Vec<BlockId>],
    ) {
        let block = function.block(id);
        match &block.name {
            Some(name) => {
                let _ = write!(self.out, "\n{}:", name_text(name));
            }
            None if !is_entry => {
                let slot = self.slots.block(id).expect("unnamed blocks get slots");
                let _ = write!(self.out, "\n{slot}:");
            }
            None => {}
        }

        if !is_entry {
            // Upstream pads to a fixed column and falls back to a single
            // space when the label has already passed it.
            let pad = LABEL_COMMENT_COLUMN.saturating_sub(self.column()).max(1);
            self.push(&" ".repeat(pad));
            let preds = &predecessors[id.0 as usize];
            if preds.is_empty() {
                self.push("; No predecessors!");
            } else {
                self.push("; preds = ");
                for (index, pred) in preds.iter().enumerate() {
                    if index > 0 {
                        self.push(", ");
                    }
                    let text = self.block_label(function, *pred);
                    self.push(&text);
                }
            }
        }
        self.push("\n");

        let instructions: Vec<_> = function.block(id).instructions.clone();
        for inst in instructions {
            if function.try_instruction(inst).is_some() {
                self.instruction(function, inst);
            }
        }
    }

    pub(crate) fn block_label(&self, function: &Function, id: BlockId) -> String {
        match &function.block(id).name {
            Some(name) => format!("%{}", name_text(name)),
            None => match self.slots.block(id) {
                Some(slot) => format!("%{slot}"),
                None => "%<badref>".to_string(),
            },
        }
    }
}
