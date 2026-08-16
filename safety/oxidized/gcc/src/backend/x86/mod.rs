#[cfg_attr(feature = "gcc_assembler", allow(dead_code))]
// Built-in assembler unused when gcc handles assembly
pub(crate) mod assembler;
pub(crate) mod codegen;
#[cfg_attr(feature = "gcc_linker", allow(dead_code))]
// Built-in linker unused when gcc handles linking
pub(crate) mod linker;

pub(crate) use codegen::emit::X86Codegen;
