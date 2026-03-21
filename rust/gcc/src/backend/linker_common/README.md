# linker_common/

Shared linker infrastructure used by all four backend linkers (x86, ARM, RISC-V, i686).

## Why this exists

The x86, ARM, and RISC-V linkers share nearly identical logic for parsing ELF64
objects, resolving symbols, loading archives, and emitting ELF headers. Before
this module, each backend had its own copy. `linker_common` extracts that
duplicated code behind the `GlobalSymbolOps` trait so each backend only
implements architecture-specific pieces (relocations, PLT/GOT, ELF headers).

The i686 linker uses ELF32 (different field widths), so it uses a subset of
this module (mainly `DynSymbol`, `is_linker_defined_symbol`, `DynStrTab`,
hash functions, and argument parsing).

## Module layout

| File | Purpose |
|------|---------|
| `types.rs` | Core ELF64 data types: `Elf64Section`, `Elf64Symbol`, `Elf64Rela`, `Elf64Object`, `DynSymbol` |
| `parse_object.rs` | Parse ELF64 relocatable objects (.o) into `Elf64Object` |
| `parse_shared.rs` | Extract dynamic symbols and SONAME from shared libraries (.so) |
| `symbols.rs` | `GlobalSymbolOps` trait, `InputSection`/`OutputSection`, linker-defined symbol table |
| `merge.rs` | Merge input sections into output sections, allocate COMMON symbols |
| `dynamic.rs` | Match undefined globals against shared library exports, register object symbols |
| `archive.rs` | Load archives (.a, thin archives), iterative symbol resolution |
| `resolve_lib.rs` | Resolve `-l` library names to filesystem paths |
| `args.rs` | Parse `-Wl,` linker flags into structured `LinkerArgs` |
| `check.rs` | Post-link undefined symbol validation |
| `write.rs` | ELF64 section/program header emission helpers |
| `dynstr.rs` | `.dynstr` string table builder with deduplication |
| `hash.rs` | GNU and SysV ELF hash functions |
| `section_map.rs` | Input-to-output section name mapping (`.text.foo` -> `.text`) |
| `eh_frame.rs` | `.eh_frame_hdr` builder for stack unwinding |
| `gc_sections.rs` | `--gc-sections` dead section elimination |

## Key design decisions

- **`GlobalSymbolOps` trait**: Each backend has its own `GlobalSymbol` struct
  (different fields for dynamic linking state). The trait abstracts over these
  so shared functions like `register_symbols_elf64` and `match_shared_library_dynsyms`
  work generically.

- **ELF64 only in shared code**: The i686 backend uses ELF32 with `u32` fields
  instead of `u64`. Rather than making everything generic over word size
  (which adds complexity for little benefit), the shared code is ELF64-only
  and i686 maintains its own ELF32 parser.

- **Linker script support**: `load_shared_library_elf64` handles the case where
  `libc.so` is actually a text file with `GROUP(...)` directives pointing to
  the real `.so` files. This is common on modern Linux distributions.
