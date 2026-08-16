//! ELF format constants: identification bytes, section types, flags, symbol attributes,
//! program header types, dynamic tags, and standard structure sizes.
//!
//! These are the raw ELF spec constants used by all assembler and linker backends.
//! Organized by category matching the ELF specification sections.

// ── ELF identification ───────────────────────────────────────────────────────

pub const ELF_MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];

// ELF class
pub const ELFCLASS32: u8 = 1;
pub const ELFCLASS64: u8 = 2;

// Data encoding
pub const ELFDATA2LSB: u8 = 1;

// Version
pub const EV_CURRENT: u8 = 1;

// OS/ABI
pub const ELFOSABI_NONE: u8 = 0;

// ── ELF object types ─────────────────────────────────────────────────────────

pub const ET_REL: u16 = 1;
pub const ET_EXEC: u16 = 2;
pub const ET_DYN: u16 = 3;

// ── Machine types ────────────────────────────────────────────────────────────

pub const EM_386: u16 = 3;
pub const EM_X86_64: u16 = 62;
pub const EM_AARCH64: u16 = 183;
pub const EM_RISCV: u16 = 243;

// ── Section header types ─────────────────────────────────────────────────────

pub const SHT_NULL: u32 = 0;
pub const SHT_PROGBITS: u32 = 1;
pub const SHT_SYMTAB: u32 = 2;
pub const SHT_STRTAB: u32 = 3;
pub const SHT_RELA: u32 = 4;
pub const SHT_HASH: u32 = 5;
pub const SHT_DYNAMIC: u32 = 6;
pub const SHT_NOTE: u32 = 7;
pub const SHT_NOBITS: u32 = 8;
pub const SHT_REL: u32 = 9;
pub const SHT_DYNSYM: u32 = 11;
pub const SHT_INIT_ARRAY: u32 = 14;
pub const SHT_FINI_ARRAY: u32 = 15;
pub const SHT_PREINIT_ARRAY: u32 = 16;
pub const SHT_GROUP: u32 = 17;

/// COMDAT group flag: sections in this group are deduplicated by the linker.
pub const GRP_COMDAT: u32 = 1;
pub const SHT_GNU_HASH: u32 = 0x6fff_fff6;
pub const SHT_GNU_VERSYM: u32 = 0x6fff_ffff;
pub const SHT_GNU_VERNEED: u32 = 0x6fff_fffe;
pub const SHT_GNU_VERDEF: u32 = 0x6fff_fffd;

// ── Section header flags ─────────────────────────────────────────────────────

pub const SHF_WRITE: u64 = 0x1;
pub const SHF_ALLOC: u64 = 0x2;
pub const SHF_EXECINSTR: u64 = 0x4;
pub const SHF_MERGE: u64 = 0x10;
pub const SHF_STRINGS: u64 = 0x20;
pub const SHF_INFO_LINK: u64 = 0x40;
pub const SHF_GROUP: u64 = 0x200;
pub const SHF_TLS: u64 = 0x400;
pub const SHF_EXCLUDE: u64 = 0x8000_0000;

// ── Symbol binding ───────────────────────────────────────────────────────────

pub const STB_LOCAL: u8 = 0;
pub const STB_GLOBAL: u8 = 1;
pub const STB_WEAK: u8 = 2;

// ── Symbol types ─────────────────────────────────────────────────────────────

pub const STT_NOTYPE: u8 = 0;
pub const STT_OBJECT: u8 = 1;
pub const STT_FUNC: u8 = 2;
pub const STT_SECTION: u8 = 3;
pub const STT_FILE: u8 = 4;
pub const STT_COMMON: u8 = 5;
pub const STT_TLS: u8 = 6;
pub const STT_GNU_IFUNC: u8 = 10;

// ── Symbol visibility ────────────────────────────────────────────────────────

pub const STV_DEFAULT: u8 = 0;
pub const STV_INTERNAL: u8 = 1;
pub const STV_HIDDEN: u8 = 2;
pub const STV_PROTECTED: u8 = 3;

// ── Special section indices ──────────────────────────────────────────────────

pub const SHN_UNDEF: u16 = 0;
pub const SHN_ABS: u16 = 0xfff1;
pub const SHN_COMMON: u16 = 0xfff2;

// ── Program header types ─────────────────────────────────────────────────────

pub const PT_NULL: u32 = 0;
pub const PT_LOAD: u32 = 1;
pub const PT_DYNAMIC: u32 = 2;
pub const PT_INTERP: u32 = 3;
pub const PT_NOTE: u32 = 4;
pub const PT_PHDR: u32 = 6;
pub const PT_TLS: u32 = 7;
pub const PT_GNU_EH_FRAME: u32 = 0x6474_e550;
pub const PT_GNU_STACK: u32 = 0x6474_e551;
pub const PT_GNU_RELRO: u32 = 0x6474_e552;

// ── Program header flags ─────────────────────────────────────────────────────

pub const PF_X: u32 = 0x1;
pub const PF_W: u32 = 0x2;
pub const PF_R: u32 = 0x4;

// ── Dynamic section tags ─────────────────────────────────────────────────────

pub const DT_NULL: i64 = 0;
pub const DT_NEEDED: i64 = 1;
pub const DT_PLTGOT: i64 = 3;
pub const DT_HASH: i64 = 4;
pub const DT_STRTAB: i64 = 5;
pub const DT_SYMTAB: i64 = 6;
pub const DT_RELA: i64 = 7;
pub const DT_RELASZ: i64 = 8;
pub const DT_RELAENT: i64 = 9;
pub const DT_STRSZ: i64 = 10;
pub const DT_SYMENT: i64 = 11;
pub const DT_INIT: i64 = 12;
pub const DT_FINI: i64 = 13;
pub const DT_SONAME: i64 = 14;
pub const DT_RPATH: i64 = 15;
pub const DT_REL: i64 = 17;
pub const DT_RELSZ: i64 = 18;
pub const DT_RELENT: i64 = 19;
pub const DT_JMPREL: i64 = 23;
pub const DT_PLTREL: i64 = 20;
pub const DT_PLTRELSZ: i64 = 2;
pub const DT_DEBUG: i64 = 21;
pub const DT_INIT_ARRAY: i64 = 25;
pub const DT_FINI_ARRAY: i64 = 26;
pub const DT_INIT_ARRAYSZ: i64 = 27;
pub const DT_FINI_ARRAYSZ: i64 = 28;
pub const DT_RUNPATH: i64 = 29;
pub const DT_FLAGS: i64 = 30;
pub const DF_BIND_NOW: i64 = 8;
pub const DT_PREINIT_ARRAY: i64 = 32;
pub const DT_PREINIT_ARRAYSZ: i64 = 33;
pub const DT_RELACOUNT: i64 = 0x6fff_fff9;
pub const DT_GNU_HASH: i64 = 0x6fff_fef5;
pub const DT_VERSYM: i64 = 0x6fff_fff0;
pub const DT_VERNEED: i64 = 0x6fff_fffe;
pub const DT_VERNEEDNUM: i64 = 0x6fff_ffff;
pub const DT_FLAGS_1: i64 = 0x6fff_fffb;
pub const DF_1_NOW: i64 = 1;

// ── ELF sizes ────────────────────────────────────────────────────────────────

/// Size of ELF64 header in bytes.
pub const ELF64_EHDR_SIZE: usize = 64;
/// Size of ELF32 header in bytes.
pub const ELF32_EHDR_SIZE: usize = 52;
/// Size of ELF64 section header in bytes.
pub const ELF64_SHDR_SIZE: usize = 64;
/// Size of ELF32 section header in bytes.
pub const ELF32_SHDR_SIZE: usize = 40;
/// Size of ELF64 symbol table entry in bytes.
pub const ELF64_SYM_SIZE: usize = 24;
/// Size of ELF32 symbol table entry in bytes.
pub const ELF32_SYM_SIZE: usize = 16;
/// Size of ELF64 RELA relocation entry in bytes.
pub const ELF64_RELA_SIZE: usize = 24;
/// Size of ELF32 REL relocation entry in bytes.
pub const ELF32_REL_SIZE: usize = 8;
/// Size of ELF32 RELA relocation entry in bytes.
pub const ELF32_RELA_SIZE: usize = 12;
/// Size of ELF64 program header in bytes.
pub const ELF64_PHDR_SIZE: usize = 56;
/// Size of ELF32 program header in bytes.
pub const ELF32_PHDR_SIZE: usize = 32;
