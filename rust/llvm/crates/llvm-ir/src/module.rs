//! Modules.

use std::collections::HashMap;

use crate::attribute::Attribute;
use crate::context::Context;
use crate::function::Function;
use crate::global::{Alias, Comdat, GlobalVariable, IFunc};
use crate::metadata::{Metadata, NamedMetadata};
use crate::types::TypeId;
use crate::value::{AliasId, FunctionId, GlobalRef, GlobalVarId, IFuncId, MdId, Name};
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
    /// `attributes #0 = { ... }`, kept with its number so that references to
    /// it print unchanged.
    pub attribute_groups: Vec<(u32, Vec<Attribute>)>,
    pub named_metadata: Vec<NamedMetadata>,
    /// Metadata indexed by the number it prints as. Holes are numbers nothing
    /// defined, which the verifier reports as unresolved references.
    pub metadata: Vec<Option<Metadata>>,
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
            attribute_groups: Vec::new(),
            named_metadata: Vec::new(),
            metadata: Vec::new(),
            symbols: HashMap::new(),
        }
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
        module.set_metadata(MdId(3), Metadata::String("late".to_string()));
        assert_eq!(module.metadata.len(), 4);
        assert!(module.metadata_node(MdId(0)).is_none());
        assert_eq!(
            module.metadata_node(MdId(3)).and_then(Metadata::as_string),
            Some("late")
        );
        assert_eq!(module.metadata_nodes().count(), 1);

        module.set_metadata(MdId(0), Metadata::String("early".to_string()));
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
