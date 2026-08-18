//! Input-to-output section name mapping.
//!
//! Maps input section names like `.text.foo` to their standard output section
//! names like `.text`. Used by all linker backends during section merging.

/// Map an input section name to the standard output section name.
///
/// This is the shared implementation used by all linker backends. Input sections
/// like `.text.foo` are merged into `.text`, `.rodata.bar` into `.rodata`, etc.
/// RISC-V additionally maps `.sdata`/`.sbss` (via `map_section_name_riscv()`).
pub fn map_section_name(name: &str) -> &str {
    if name.starts_with(".text.") || name == ".text" {
        return ".text";
    }
    if name.starts_with(".data.rel.ro") {
        return ".data.rel.ro";
    }
    if name.starts_with(".data.") || name == ".data" {
        return ".data";
    }
    if name.starts_with(".rodata.") || name == ".rodata" {
        return ".rodata";
    }
    if name.starts_with(".bss.") || name == ".bss" {
        return ".bss";
    }
    if name.starts_with(".init_array") {
        return ".init_array";
    }
    if name.starts_with(".fini_array") {
        return ".fini_array";
    }
    if name.starts_with(".tbss.") || name == ".tbss" {
        return ".tbss";
    }
    if name.starts_with(".tdata.") || name == ".tdata" {
        return ".tdata";
    }
    if name.starts_with(".gcc_except_table") {
        return ".gcc_except_table";
    }
    if name.starts_with(".eh_frame") {
        return ".eh_frame";
    }
    if name.starts_with(".note.") {
        return name;
    }
    name
}
