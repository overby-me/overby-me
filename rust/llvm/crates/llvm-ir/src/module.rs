//! Modules.

use std::collections::HashMap;

use crate::attribute::Attribute;
use crate::constant::{ConstId, Constant};
use crate::context::Context;
use crate::function::Function;
use crate::global::{Alias, Comdat, GlobalVariable, IFunc};
use crate::metadata::{Metadata, NamedMetadata};
use crate::types::TypeId;
use crate::value::{AliasId, FunctionId, GlobalRef, GlobalVarId, IFuncId, MdId, Name, Value};
use llvm_support::{Align, DataLayout, Triple};

/// A translation unit: everything one `.ll` file holds, plus the context its
/// types and constants are interned in.
#[derive(Clone, Debug)]
pub struct Module {
    pub ctx: Context,
    /// The `; ModuleID = '...'` comment, which is a comment upstream but
    /// carries the module's identity and is worth keeping.
    pub module_id: Option<String>,
    pub source_filename: Option<String>,
    pub data_layout: Option<DataLayout>,
    pub triple: Option<Triple>,
    /// Each `module asm "..."` line, in order.
    pub module_asm: Vec<String>,
    pub comdats: Vec<Comdat>,
    pub globals: Vec<GlobalVariable>,
    pub aliases: Vec<Alias>,
    pub ifuncs: Vec<IFunc>,
    pub functions: Vec<Function>,
    /// The order the functions print in, when it is not the order they were
    /// built in. Empty means the arena order, which is what a module read
    /// straight through has. A function this names none of is one that does
    /// not print at all.
    ///
    /// Two things put an entry here. Upstream does not rename an intrinsic
    /// declaration in place, it builds a new function and erases the old, so
    /// a declaration whose name gained the types it was instantiated at
    /// prints after everything the module wrote. And a module writing two
    /// spellings of one intrinsic has two declarations where upstream has one
    /// function, so the second is left out of the order once its calls have
    /// been pointed at the first.
    ///
    /// Both are recorded here rather than performed on the arena, because
    /// moving or removing a function would move its id, which every constant
    /// naming it holds.
    pub function_order: Vec<FunctionId>,
    /// `attributes #0 = { ... }`, kept with its number so that references to
    /// it print unchanged.
    pub attribute_groups: Vec<(u32, Vec<Attribute>)>,
    pub named_metadata: Vec<NamedMetadata>,
    /// Metadata indexed by the number it prints as. Holes are numbers nothing
    /// defined, which the verifier reports as unresolved references.
    pub metadata: Vec<Option<Metadata>>,
    /// The ThinLTO summary index, `^0 = module: (...)`, written after the
    /// module body and printed back as it was written.
    pub summary: Vec<crate::summary::SummaryEntry>,
    symbols: HashMap<Name, GlobalRef>,
}

impl Default for Module {
    fn default() -> Self {
        Self::new()
    }
}

impl Module {
    pub fn new() -> Module {
        Module {
            ctx: Context::new(),
            module_id: None,
            source_filename: None,
            data_layout: None,
            triple: None,
            module_asm: Vec::new(),
            comdats: Vec::new(),
            globals: Vec::new(),
            aliases: Vec::new(),
            ifuncs: Vec::new(),
            functions: Vec::new(),
            function_order: Vec::new(),
            attribute_groups: Vec::new(),
            named_metadata: Vec::new(),
            metadata: Vec::new(),
            summary: Vec::new(),
            symbols: HashMap::new(),
        }
    }

    /// The functions in the order they print, as indexes into the arena.
    ///
    /// No recorded order means the order they were built in. A recorded one
    /// says what prints as well as in what order, so a function it leaves out
    /// is one upstream merged away and does not write back.
    pub fn function_print_order(&self) -> Vec<usize> {
        if self.function_order.is_empty() {
            return (0..self.functions.len()).collect();
        }
        self.function_order
            .iter()
            .map(|FunctionId(id)| *id as usize)
            .collect()
    }

    pub fn add_function(&mut self, function: Function) -> FunctionId {
        let id = FunctionId(self.functions.len() as u32);
        self.symbols
            .insert(function.name.clone(), GlobalRef::Function(id));
        self.functions.push(function);
        id
    }

    pub fn add_global(&mut self, global: GlobalVariable) -> GlobalVarId {
        let id = GlobalVarId(self.globals.len() as u32);
        self.symbols
            .insert(global.name.clone(), GlobalRef::Variable(id));
        self.globals.push(global);
        id
    }

    pub fn add_alias(&mut self, alias: Alias) -> AliasId {
        let id = AliasId(self.aliases.len() as u32);
        self.symbols
            .insert(alias.name.clone(), GlobalRef::Alias(id));
        self.aliases.push(alias);
        id
    }

    pub fn add_ifunc(&mut self, ifunc: IFunc) -> IFuncId {
        let id = IFuncId(self.ifuncs.len() as u32);
        self.symbols
            .insert(ifunc.name.clone(), GlobalRef::IFunc(id));
        self.ifuncs.push(ifunc);
        id
    }

    pub fn function(&self, id: FunctionId) -> &Function {
        &self.functions[id.0 as usize]
    }

    pub fn function_mut(&mut self, id: FunctionId) -> &mut Function {
        &mut self.functions[id.0 as usize]
    }

    pub fn global(&self, id: GlobalVarId) -> &GlobalVariable {
        &self.globals[id.0 as usize]
    }

    pub fn alias(&self, id: AliasId) -> &Alias {
        &self.aliases[id.0 as usize]
    }

    pub fn ifunc(&self, id: IFuncId) -> &IFunc {
        &self.ifuncs[id.0 as usize]
    }

    /// Looks a global-scope symbol up by the name it is written with.
    pub fn symbol(&self, name: &Name) -> Option<GlobalRef> {
        self.symbols.get(name).copied()
    }

    /// The name of anything spelled with a leading `@`.
    pub fn global_name(&self, target: GlobalRef) -> &Name {
        match target {
            GlobalRef::Function(id) => &self.function(id).name,
            GlobalRef::Variable(id) => &self.global(id).name,
            GlobalRef::Alias(id) => &self.alias(id).name,
            GlobalRef::IFunc(id) => &self.ifunc(id).name,
        }
    }

    /// The address space a reference to this symbol points into.
    pub fn global_address_space(&self, target: GlobalRef) -> u32 {
        let qualifiers = match target {
            GlobalRef::Function(id) => &self.function(id).qualifiers,
            GlobalRef::Variable(id) => &self.global(id).qualifiers,
            GlobalRef::Alias(id) => &self.alias(id).qualifiers,
            GlobalRef::IFunc(id) => &self.ifunc(id).qualifiers,
        };
        qualifiers.address_space.unwrap_or(0)
    }

    /// Stores a metadata node under a number, growing the table as needed.
    pub fn set_metadata(&mut self, id: MdId, node: Metadata) {
        let index = id.0 as usize;
        if self.metadata.len() <= index {
            self.metadata.resize(index + 1, None);
        }
        self.metadata[index] = Some(node);
    }

    pub fn metadata_node(&self, id: MdId) -> Option<&Metadata> {
        self.metadata.get(id.0 as usize)?.as_ref()
    }

    /// The next unused metadata number.
    pub fn next_metadata_id(&self) -> MdId {
        MdId(self.metadata.len() as u32)
    }

    pub fn add_metadata(&mut self, node: Metadata) -> MdId {
        let id = self.next_metadata_id();
        self.metadata.push(Some(node));
        id
    }

    /// The alignment an unwritten `align` clause means for this type.
    ///
    /// Upstream does not leave the field empty: it computes one from the data
    /// layout and prints it, so `load i32` and `load i32, align 4` are the
    /// same instruction. An `alloca` takes the preferred alignment and the
    /// memory operations take the ABI one. `None` when the type has no size,
    /// which the verifier reports separately.
    pub fn default_align(&self, ty: TypeId, preferred: bool) -> Option<Align> {
        let layout = self.data_layout.clone().unwrap_or_default();
        if preferred {
            crate::layout::preferred_align(&self.ctx, &layout, ty).ok()
        } else {
            crate::layout::abi_align(&self.ctx, &layout, ty).ok()
        }
    }

    pub fn attribute_group(&self, number: u32) -> Option<&[Attribute]> {
        self.attribute_groups
            .iter()
            .find(|(id, _)| *id == number)
            .map(|(_, attributes)| attributes.as_slice())
    }

    /// Every metadata node that is defined, with its number.
    pub fn metadata_nodes(&self) -> impl Iterator<Item = (MdId, &Metadata)> {
        self.metadata
            .iter()
            .enumerate()
            .filter_map(|(index, node)| Some((MdId(index as u32), node.as_ref()?)))
    }

    /// How many operand slots in the module read this constant.
    ///
    /// One per slot rather than one per reader, which is what upstream
    /// counts: `icmp eq ptr @g, @g` uses `@g` twice. Constants are interned,
    /// so an expression written into two globals is one constant and reads
    /// what it reads once; the two globals then read the expression rather
    /// than what it names.
    ///
    /// This is what a `uselistorder` directive is checked against, and it
    /// can only be taken once the whole module is read: a global used by a
    /// later function is not yet used while the text is still being parsed.
    pub fn use_count(&self, target: ConstId) -> usize {
        let mut count = 0;
        for index in 0..self.ctx.constant_count() {
            for operand in self.ctx.constant(ConstId(index as u32)).operand_constants() {
                count += usize::from(operand == target);
            }
        }
        for global in &self.globals {
            count += usize::from(global.initializer == Some(target));
        }
        for alias in &self.aliases {
            count += usize::from(alias.aliasee == target);
        }
        for ifunc in &self.ifuncs {
            count += usize::from(ifunc.resolver == target);
        }
        for function in &self.functions {
            for slot in [function.personality, function.prefix, function.prologue] {
                count += usize::from(slot.map(|(_, id)| id) == Some(target));
            }
            for (id, _) in function.blocks() {
                for (_, instruction) in function.block_instructions(id) {
                    for value in instruction.kind.use_count_values() {
                        count += usize::from(value == Value::Constant(target));
                    }
                    // A debug record is a call to one of the four
                    // `llvm.dbg.*` intrinsics in its older spelling, read
                    // into a record as upstream reads it. The call is gone
                    // from the model and the use it made is not: upstream
                    // still counts it, so a module permuting that
                    // declaration's use list is counting these.
                    if let crate::instruction::InstKind::DebugRecord { name, .. } =
                        &instruction.kind
                        && let Constant::Global {
                            target: GlobalRef::Function(callee),
                            ..
                        } = self.ctx.constant(target)
                        && let Name::Named(intrinsic) = &self.function(*callee).name
                    {
                        count +=
                            usize::from(intrinsic == &format!("llvm.{}", name.replace('_', ".")));
                    }
                }
            }
        }
        count
    }

    /// Whether a `blockaddress` naming this block is read anywhere.
    ///
    /// This is the half of a block's use list that its own function cannot
    /// answer. Upstream uniques the constant per block, so however many
    /// times its text appears it is one entry in the list, and a constant
    /// nothing reads is not an entry at all: a `uselistorder` directive can
    /// name a blockaddress without using it.
    ///
    /// `pending` is a function that is being read and has not joined the
    /// module yet, whose own operands count too: a block can take its own
    /// address, as an `indirectbr` written against a label in the same
    /// function does.
    pub fn block_address_used(
        &self,
        function: GlobalRef,
        block: &Name,
        pending: Option<&Function>,
    ) -> bool {
        let address = (0..self.ctx.constant_count())
            .map(|index| ConstId(index as u32))
            .find(|id| {
                matches!(
                    self.ctx.constant(*id),
                    Constant::BlockAddress { function: named, block: label, .. }
                        if *named == function && label == block
                )
            });
        let Some(address) = address else {
            return false;
        };
        self.use_count(address) > 0
            || pending.is_some_and(|function| function.value_uses(Value::Constant(address)) > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::function::Function;
    use crate::metadata::Metadata;

    #[test]
    fn symbols_resolve_to_what_defined_them() {
        let mut module = Module::new();
        let void = module.ctx.void_type();
        let id = module.add_function(Function::new(Name::named("main"), void));
        assert_eq!(
            module.symbol(&Name::named("main")),
            Some(GlobalRef::Function(id))
        );
        assert_eq!(module.symbol(&Name::named("absent")), None);
        assert_eq!(
            module.global_name(GlobalRef::Function(id)),
            &Name::named("main")
        );
        assert_eq!(module.global_address_space(GlobalRef::Function(id)), 0);
    }

    #[test]
    fn metadata_is_stored_under_the_number_it_prints_as() {
        let mut module = Module::new();
        // A forward reference writes a high number first; the table grows and
        // the gap stays empty until something fills it.
        module.set_metadata(MdId(3), Metadata::String("late".into()));
        assert_eq!(module.metadata.len(), 4);
        assert!(module.metadata_node(MdId(0)).is_none());
        assert_eq!(
            module.metadata_node(MdId(3)).and_then(Metadata::as_string),
            Some("late")
        );
        assert_eq!(module.metadata_nodes().count(), 1);

        module.set_metadata(MdId(0), Metadata::String("early".into()));
        assert_eq!(module.metadata_nodes().count(), 2);
        assert_eq!(module.next_metadata_id(), MdId(4));
    }

    #[test]
    fn a_module_is_send_and_sync() {
        // The one structural advantage over the C++ original, so it gets a
        // test rather than a comment.
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Module>();
    }
}
