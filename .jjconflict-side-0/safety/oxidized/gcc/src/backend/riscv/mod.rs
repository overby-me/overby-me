#[cfg_attr(feature = "gcc_assembler", allow(dead_code))]
// Built-in assembler unused when gcc handles assembly
pub(crate) mod assembler;
pub(crate) mod codegen;
#[cfg_attr(feature = "gcc_linker", allow(dead_code, unused_imports))]
// Built-in linker unused when gcc handles linking; unused_imports: mod.rs has pub use re-exports
pub(crate) mod linker;

pub(crate) use codegen::emit::RiscvCodegen;
