# x86-64 Built-in Linker -- Design Document

## Overview

This module is a native x86-64 ELF linker that combines relocatable object
files (`.o`) and static archives (`.a`) into either a dynamically-linked ELF
executable or a shared library (`.so`). It resolves symbols across object
files, generates PLT/GOT entries for dynamic function calls, applies
relocations, and produces a ready-to-run binary.

The linker is invoked as an alternative to calling the system `ld` or `gcc`
for the final link step. It handles the typical output of C compilation:
multiple object files, CRT startup objects (`crt1.o`, `crti.o`, `crtn.o`),
static archives (`libc_nonshared.a`), and shared library dependencies
(`libc.so.6`, `libm.so.6`). It also supports producing shared libraries
with full PLT/GOT, RELRO, GLOB_DAT, rpath/runpath, and SONAME support.

### Entry Points

**Executable linking:**

```rust
pub fn link_builtin(
    object_files:       &[&str],
    output_path:        &str,
    user_args:          &[String],
    lib_paths:          &[&str],
    needed_libs:        &[&str],
    crt_objects_before: &[&str],
    crt_objects_after:  &[&str],
) -> Result<(), String>
```

**Shared library linking:**

```rust
pub fn link_shared(
    object_files: &[&str],
    output_path:  &str,
    user_args:    &[String],
    lib_paths:    &[&str],
    needed_libs:  &[&str],
) -> Result<(), String>
```

## Architecture / Pipeline

```text
    CRT .o files     User .o files     -l libraries     Shared .so files
         |                |                 |                  |
         v                v                 v                  v
    +----------------------------------------------------------------+
    |              Input Loading (via linker_common)                  |
    |  - Parse ELF .o  (linker_common::parse_elf64_object)           |
    |  - Parse .a archives  (linker_common::load_archive_elf64)      |
    |  - Parse .so dynamic symbols  (parse_shared_library_symbols)   |
    |  - Parse linker scripts  (parse_linker_script_entries)         |
    +----------------------------------------------------------------+
                              |
                    Objects + Global Symbol Table
                              |
                              v
    +----------------------------------------------------------------+
    |              Symbol Resolution (via linker_common)              |
    |  - register_symbols_elf64: collect defined/undefined globals   |
    |  - Archive selective loading: pull in members that satisfy     |
    |    undefined references (iterated to fixed point)              |
    |  - resolve_dynamic_symbols: match against system .so files     |
    +----------------------------------------------------------------+
                              |
                              v
    +----------------------------------------------------------------+
    |              Section Merging (via linker_common)                |
    |  - Map input sections to output sections by name               |
    |  - Compute per-input-section offsets within output sections     |
    |  - Sort output sections: RO -> Exec -> RW -> BSS               |
    +----------------------------------------------------------------+
                              |
                              v
    +----------------------------------------------------------------+
    |              PLT / GOT Construction                            |
    |  - Scan all relocations for dynamic symbol references          |
    |  - Create PLT entries for dynamic function calls               |
    |  - Create GOT entries for GOTPCREL/GOTTPOFF/data symbols       |
    |  - Set up copy relocations for dynamic data objects            |
    +----------------------------------------------------------------+
                              |
                              v
    +----------------------------------------------------------------+
    |              Address Layout                                    |
    |  - Assign virtual addresses to all segments and sections       |
    |  - Compute PLT/GOT/dynamic section addresses                   |
    |  - Update global symbol values to final virtual addresses      |
    +----------------------------------------------------------------+
                              |
                              v
    +----------------------------------------------------------------+
    |              Emission                                          |
    |  - Write ELF header + program headers                          |
    |  - Write .interp, .gnu.hash, .dynsym, .dynstr                  |
    |  - Write .rela.dyn, .rela.plt                                  |
    |  - Write merged section data                                   |
    |  - Generate PLT stub code                                      |
    |  - Write .dynamic, .got, .got.plt                              |
    |  - Apply all relocations to the output buffer                  |
    |  - Write file + set executable permission                      |
    +----------------------------------------------------------------+
                              |
                              v
                ELF Executable or Shared Library
```

## File Inventory

| File | Lines | Role |
|------|-------|------|
| `mod.rs` | ~28 | Module declarations and public re-exports (`link_builtin`, `link_shared`) |
| `types.rs` | ~89 | `GlobalSymbol` struct with `GlobalSymbolOps` impl, arch constants (`BASE_ADDR`, `INTERP`), `x86_should_replace_extra` |
| `elf.rs` | ~81 | x86-64 relocation constants (`R_X86_64_*`); type aliases mapping `linker_common` types to local names (`ElfObject`, `Symbol`, etc.); thin wrapper functions delegating to `linker_common` for ELF64 parsing, shared library symbols, and SONAME extraction; re-exports shared ELF constants from `crate::backend::elf` |
| `input.rs` | ~78 | File loading dispatch: `load_file` (objects, archives, shared libs, linker scripts) |
| `plt_got.rs` | ~141 | PLT/GOT entry construction from relocation scanning, IFUNC symbol collection |
| `link.rs` | ~386 | Orchestration: `link_builtin` and `link_shared` entry points, dynamic symbol resolution, library group loading, `--gc-sections`, `--export-dynamic`, `--defsym` |
| `emit_exec.rs` | ~1,239 | Executable emission (both static and dynamic): PLT/GOT/.dynamic, layout, relocation application, `resolve_sym` |
| `emit_shared.rs` | ~1,137 | Shared library (`.so`) emission: PIC layout, `R_X86_64_RELATIVE`/`JUMP_SLOT`/`GLOB_DAT`, RELRO, rpath/runpath |

## Key Data Structures

### `ElfObject` (linker_common, aliased in elf.rs)

The parsed representation of one relocatable object file. Defined as
`Elf64Object` in `linker_common.rs`; aliased as `ElfObject` in `elf.rs`:

```rust
struct Elf64Object {
    sections:     Vec<Elf64Section>,   // parsed section headers
    symbols:      Vec<Elf64Symbol>,    // parsed symbol table
    section_data: Vec<Vec<u8>>,        // raw bytes for each section
    relocations:  Vec<Vec<Elf64Rela>>, // relocations indexed by target section
    source_name:  String,              // file path for diagnostics
}
```

### `Elf64Section` (linker_common, aliased as `SectionHeader`)

```rust
struct Elf64Section {
    name_idx:  u32,
    name:      String,      // resolved from .shstrtab
    sh_type:   u32,         // SHT_PROGBITS, SHT_NOBITS, SHT_RELA, ...
    flags:     u64,         // SHF_ALLOC | SHF_WRITE | SHF_EXECINSTR | ...
    addr:      u64,
    offset:    u64,
    size:      u64,
    link:      u32,
    info:      u32,
    addralign: u64,
    entsize:   u64,
}
```

### `Elf64Symbol` (linker_common, aliased as `Symbol`)

```rust
struct Elf64Symbol {
    name_idx: u32,
    name:     String,   // resolved from .strtab
    info:     u8,       // (binding << 4) | type
    other:    u8,       // visibility in low 2 bits
    shndx:    u16,      // section index, SHN_UNDEF, SHN_ABS, SHN_COMMON
    value:    u64,
    size:     u64,
}
```

Helper methods: `binding()`, `sym_type()`, `visibility()`, `is_undefined()`,
`is_global()`, `is_weak()`, `is_local()`.

### `Elf64Rela` (linker_common, aliased as `Rela`)

```rust
struct Elf64Rela {
    offset:    u64,    // offset within the section
    sym_idx:   u32,    // index into the object's symbol table
    rela_type: u32,    // R_X86_64_* constant
    addend:    i64,
}
```

### `GlobalSymbol` (types.rs)

The linker's unified view of a resolved global symbol. Implements the
`GlobalSymbolOps` trait from `linker_common`:

```rust
struct GlobalSymbol {
    value:         u64,            // virtual address (after layout)
    size:          u64,
    info:          u8,             // original ELF st_info
    defined_in:    Option<usize>,  // object index, or None for undefined
    from_lib:      Option<String>, // SONAME if from a shared library
    plt_idx:       Option<usize>,  // index into PLT, if any
    got_idx:       Option<usize>,  // index into GOT entries, if any
    section_idx:   u16,            // section index in defining object
    is_dynamic:    bool,           // true if resolved from a .so
    copy_reloc:    bool,           // true if needs R_X86_64_COPY
    lib_sym_value: u64,            // original value from shared library symbol
    version:       Option<String>, // GLIBC version string (e.g. "GLIBC_2.3")
}
```

### `OutputSection` (linker_common)

Represents one merged output section in the final executable:

```rust
struct OutputSection {
    name:        String,
    sh_type:     u32,
    flags:       u64,
    alignment:   u64,
    inputs:      Vec<InputSection>,  // contributing input sections
    data:        Vec<u8>,            // merged section data
    addr:        u64,                // assigned virtual address
    file_offset: u64,                // file offset in output
    mem_size:    u64,                // total size in memory
}
```

### `InputSection` (linker_common)

Tracks where one input section is placed within an output section:

```rust
struct InputSection {
    object_idx:    usize,   // which ElfObject
    section_idx:   usize,   // which section within that object
    output_offset: u64,     // byte offset within the output section
    size:          u64,
}
```

### `DynStrTab` (linker_common)

A simple dynamic string table builder with deduplication, used for `.dynstr`.
Defined in the shared `linker_common` module and re-used by x86 and other backends:

```rust
struct DynStrTab {
    data:    Vec<u8>,                  // NUL-terminated string pool
    offsets: HashMap<String, usize>,   // name -> offset
}
```

## Processing Algorithm

### Phase 1: Input Loading

Files are loaded in a specific order to ensure correct symbol resolution
precedence:

```text
1. CRT objects before (crt1.o, crti.o)
2. User object files (compiler output)
3. Extra object/archive files from user args (-Wl,...)
4. CRT objects after (crtn.o)
5. All needed libraries in a group loop (-lc, -lm, -lgcc, user -l flags)
   Iterates until no new objects are pulled in (handles circular dependencies)
```

The `load_file()` function dispatches based on file format:

| Magic | Format | Handler |
|-------|--------|---------|
| `\x7fELF` + `ET_REL` | Relocatable object | `parse_object()` -> register symbols |
| `\x7fELF` + `ET_DYN` | Shared library | `load_shared_library()` -> extract dynamic symbols |
| `!<arch>\n` | Static archive | `load_archive_elf64()` -> selective member extraction |
| `!<thin>\n` | Thin archive | `load_thin_archive_elf64()` -> selective member extraction |
| Text | Linker script | `parse_linker_script_entries()` -> follow `GROUP(...)` / `INPUT(...)` references |

**Archive loading** (`load_archive_elf64`):

Archives are loaded selectively. The algorithm:

1. Parse all archive members (via `parse_archive_members` in `crate::backend::elf`).
2. Parse each ELF member into an `Elf64Object`, skipping non-ELF members and
   members with mismatched `e_machine`.
3. Iterate: for each unpulled member, check if it defines a symbol that is
   currently undefined in `globals`.
4. If yes, pull the member: register its symbols and add it to `objects`.
5. Repeat until no more members are pulled (fixed-point iteration).

This handles transitive dependencies between archive members.

**Shared library loading** (`load_shared_library_elf64`):

1. Parse the `.dynsym` section to extract exported symbol names (with version
   information from `.gnu.version` / `.gnu.verdef`).
2. Extract the SONAME from the `.dynamic` section (falls back to the file
   basename at the call site if no SONAME is found).
3. For each dynamic symbol, if it satisfies an undefined reference in
   `globals`, create a `GlobalSymbol` entry with `is_dynamic = true`.
   Alias matching is also performed: if a matched weak `STT_OBJECT` symbol
   shares the same `(value, size)` as another library export, the alias is
   also added to `globals`.
4. Add the SONAME to `needed_sonames` for the `DT_NEEDED` entries.

**Fallback dynamic resolution** (`resolve_dynamic_symbols_elf64`):

After all user-specified inputs are loaded, any remaining undefined symbols
are searched in system libraries (`libc.so.6`, `libm.so.6`, `libgcc_s.so.1`)
at well-known paths. Linker-defined symbols are excluded from this check
since they are provided during the layout phase. The full exclusion list
is maintained in `linker_common::is_linker_defined_symbol()` and includes
`_GLOBAL_OFFSET_TABLE_`, `__bss_start`, `_edata`, `_end`, `__end`,
`__ehdr_start`, `__executable_start`, `_etext`, `etext`, `__dso_handle`,
`_DYNAMIC`, `__data_start`, `data_start`, init/fini/preinit array boundary
symbols, `__rela_iplt_start`/`__rela_iplt_end`, `_IO_stdin_used`,
`_init`/`_fini`, `__tls_get_addr`, and various unwinder/ITM symbols.

### Phase 2: Symbol Resolution

`linker_common::register_symbols_elf64()` processes each object file's symbol table:

1. **Section symbols** and **file symbols** are skipped.
2. **Local symbols** and empty-named symbols are skipped.
3. **Defined non-local symbols** are inserted into `globals`:
   - A new definition replaces an undefined reference.
   - A global definition replaces a weak definition.
   - A dynamic definition can be replaced by a static definition (via
     the `x86_should_replace_extra` callback).
4. **COMMON symbols** are recorded but can be overridden by real definitions.
5. **Undefined symbols** create placeholder entries in `globals` (only if
   not already present).

Resolution priority: `static defined > dynamic defined > weak > undefined`.

### Phase 3: Section Merging

`linker_common::merge_sections_elf64()` combines input sections from all
objects into output sections:

1. **Filtering** -- Only `SHF_ALLOC` sections are included. Non-allocatable
   sections (`.strtab`, `.symtab`, `.rela.*`, group sections) and
   `SHF_EXCLUDE` sections are skipped.
2. **Name mapping** -- Input section names are mapped to canonical output names
   (evaluated in order, first match wins):

   ```text
   .text.*            -> .text
   .data.rel.ro*      -> .data.rel.ro    (checked before .data.*)
   .data.*            -> .data
   .rodata.*          -> .rodata
   .bss.*             -> .bss
   .init_array*       -> .init_array
   .fini_array*       -> .fini_array
   .tbss.*            -> .tbss
   .tdata.*           -> .tdata
   .gcc_except_table* -> .gcc_except_table
   .eh_frame*         -> .eh_frame
   .note.*            -> (unchanged, kept as-is)
   (anything else)    -> (unchanged, kept as-is)
   ```

3. **Offset computation** -- Each input section is assigned an offset within
   its output section, respecting its alignment requirement.
4. **Sorting** -- Output sections are sorted by a `(category, is_nobits)` key:

   ```text
   (0, 0)  Read-only PROGBITS     (e.g., .rodata, .eh_frame)
   (0, 1)  Read-only NOBITS
   (1, 0)  Executable PROGBITS    (e.g., .text)
   (1, 1)  Executable NOBITS
   (2, 0)  Read-write PROGBITS    (e.g., .data, .init_array)
   (2, 1)  Read-write NOBITS      (e.g., .bss)
   ```

5. **Section map** -- A `(object_idx, section_idx) -> (output_idx, offset)`
   map is built for relocation processing.

### Phase 4: COMMON Symbol Allocation

`linker_common::allocate_common_symbols_elf64()` places COMMON symbols (from
`.comm` directives) into the `.bss` section:

1. Collect all globals with `SHN_COMMON`.
2. For each, align and append to `.bss`'s `mem_size` (the symbol's `value`
   field is used as its alignment requirement).
3. Update the symbol's `value` to its offset within `.bss`.

### Phase 5: PLT/GOT Construction

`create_plt_got()` scans all relocations to determine which symbols need PLT
entries, GOT entries, or copy relocations:

| Relocation | Symbol Type | Action |
|------------|------------|--------|
| `R_X86_64_PLT32` / `R_X86_64_PC32` | Dynamic function | Create PLT entry |
| `R_X86_64_PLT32` / `R_X86_64_PC32` | Dynamic object (`STT_OBJECT`) | Create copy relocation |
| `R_X86_64_GOTPCREL` / `R_X86_64_REX_GOTPCRELX` / `R_X86_64_GOTPCRELX` | Any | Create dedicated GOT entry (even if symbol has PLT) |
| `R_X86_64_GOTTPOFF` | Any | Create GOT entry (skipped if symbol has PLT) |
| `R_X86_64_64` | Dynamic function | Create PLT entry (function pointer) |
| Any other | Dynamic symbol (non-object) | Create GOT entry |

Copy relocation aliasing is also handled: symbols at the same library address
share the same BSS copy slot.

**GOT layout:**

```text
.got.plt:
  GOT[0]:          .dynamic address (for ld.so)
  GOT[1]:          link_map pointer (reserved, filled by ld.so)
  GOT[2]:          _dl_runtime_resolve (reserved, filled by ld.so)
  GOT[3..3+N_plt]: PLT GOT entries (one per PLT stub)

.got:
  Entries for GLOB_DAT symbols (non-PLT dynamic data), TLS GOTTPOFF
  entries, locally-resolved GOT entries, and GOTPCREL entries for
  symbols that also have PLT entries.
```

**GOTPCREL + PLT coexistence:** A symbol can have both a PLT entry (for
function calls) and a separate `.got` entry (for address-of via GOTPCREL).
The PLT's `.got.plt` slot uses `R_X86_64_JUMP_SLOT` with lazy binding
(initially pointing to PLT+6), which is unsuitable for address-of. The
dedicated `.got` entry is statically filled with the PLT entry address (the
canonical function address in a non-PIE executable), avoiding a GLOB_DAT
dynamic relocation. This ensures that `movq sym@GOTPCREL(%rip)` and
`.quad sym` both resolve to the same canonical PLT address.

### Phase 6: Address Layout

The linker uses a fixed base address (`0x400000`) and a `0x1000` page size.
The executable is laid out as four PT_LOAD segments plus metadata:

```text
+-----------------------------------------------------------------------+
| Segment 1: Read-only (PT_LOAD, PF_R)                                 |
|   ELF header + program headers                                       |
|   .interp  (dynamic linker path)                                     |
|   .gnu.hash                                                          |
|   .dynsym                                                            |
|   .dynstr                                                            |
|   .gnu.version   (symbol version indices, if versioning needed)      |
|   .gnu.version_r (version requirements, if versioning needed)        |
|   .rela.dyn                                                          |
|   .rela.plt                                                          |
+-----------------------------------------------------------------------+
| Segment 2: Executable (PT_LOAD, PF_R | PF_X)         [page-aligned]  |
|   .text  (merged code sections)                                       |
|   .plt   (PLT stubs)                                                  |
+-----------------------------------------------------------------------+
| Segment 3: Read-only data (PT_LOAD, PF_R)            [page-aligned]  |
|   .rodata  (merged read-only data sections)                           |
+-----------------------------------------------------------------------+
| Segment 4: Read-write (PT_LOAD, PF_R | PF_W)         [page-aligned]  |
|   .init_array                                                         |
|   .fini_array                                                         |
|   .dynamic                                                            |
|   .got                                                                |
|   .got.plt                                                            |
|   .data  (merged writable data sections)                              |
|   .tdata (TLS initialized data)                                       |
|   .tbss  (TLS zero-initialized data, NOBITS)                          |
|   .bss   (zero-initialized data, NOBITS)                              |
|   [copy-relocated symbols]                                            |
+-----------------------------------------------------------------------+
```

Additional program headers:

- `PT_PHDR` -- Points to the program header table itself.
- `PT_INTERP` -- Points to the `.interp` section.
- `PT_DYNAMIC` -- Points to the `.dynamic` section.
- `PT_GNU_STACK` -- Declares a non-executable stack (`PF_R | PF_W`, align 16).
- `PT_TLS` -- Present only if TLS sections exist; describes the TLS template.

Note: Executables do **not** emit `PT_GNU_RELRO` (only shared libraries do).

**TLS layout** (x86-64 variant II):

On x86-64, `%fs:0` points past the end of the TLS block. Thread-local
variables are accessed at negative offsets from `%fs:0`:

```text
TPOFF(sym) = (sym_addr - tls_segment_addr) - tls_mem_size
```

This value is stored in GOT entries for `R_X86_64_GOTTPOFF` and written
inline for `R_X86_64_TPOFF32`.

### Phase 7: PLT Stub Generation

Each PLT entry is 16 bytes. The stub layout:

**PLT[0] (resolver stub, 16 bytes):**

```asm
ff 35 XX XX XX XX    pushq  GOT[1](%rip)     # push link_map
ff 25 XX XX XX XX    jmpq   *GOT[2](%rip)    # jump to resolver
90 90 90 90          nop; nop; nop; nop       # padding
```

**PLT[N] (function stub, 16 bytes each):**

```asm
ff 25 XX XX XX XX    jmpq   *GOT[3+N](%rip)  # indirect jump via GOT
68 NN 00 00 00       pushq  $N               # push relocation index
e9 XX XX XX XX       jmpq   PLT[0]           # jump to resolver stub
```

On first call, `GOT[3+N]` points to `PLT[N]+6` (the `pushq`), so control
falls through to the resolver. After lazy binding, `GOT[3+N]` is patched
to the actual function address.

### Phase 8: Relocation Application

The linker iterates over every relocation in every input section and applies
it to the output buffer. The general formula depends on the relocation type:

| Relocation Type | Formula | Notes |
|----------------|---------|-------|
| `R_X86_64_64` | `S + A` | 64-bit absolute; uses PLT address for dynamic functions |
| `R_X86_64_PC32` | `S + A - P` | 32-bit PC-relative; PLT-routed for dynamic symbols |
| `R_X86_64_PLT32` | `S + A - P` | Same as PC32 but always routes through PLT if available |
| `R_X86_64_32` | `S + A` | 32-bit unsigned absolute |
| `R_X86_64_32S` | `S + A` | 32-bit signed absolute |
| `R_X86_64_GOTPCREL` | `G + A - P` | 32-bit PC-relative to GOT entry |
| `R_X86_64_GOTPCRELX` | `G + A - P` or relaxed | May relax `mov` to `lea` for locally-defined symbols |
| `R_X86_64_REX_GOTPCRELX` | `G + A - P` or relaxed | Same, with REX prefix |
| `R_X86_64_GOTTPOFF` | `G + A - P` or IE-to-LE | GOT-relative TLS; relaxes to `mov $tpoff` when possible |
| `R_X86_64_TPOFF32` | `S - TLS_addr - TLS_memsz + A` | Direct TLS offset |
| `R_X86_64_PC64` | `S + A - P` | 64-bit PC-relative |
| `R_X86_64_NONE` | (ignored) | |

Where:

- `S` = symbol value (virtual address after layout)
- `A` = relocation addend
- `P` = relocation position (virtual address of the bytes being patched)
- `G` = GOT entry address

**Symbol resolution** (`resolve_sym`):

1. **Section symbols** -- Resolved via the section map to the output section
   address plus input section offset.
2. **Named non-local symbols** -- Looked up in the `globals` table. Linker-defined
   symbols (e.g., `_GLOBAL_OFFSET_TABLE_`, `__bss_start`, `_edata`, `_end`,
   `__ehdr_start`, `_etext`, `__dso_handle`, `_DYNAMIC`, `__data_start`,
   init/fini array boundaries) are inserted into `globals` during the layout
   phase via `get_standard_linker_symbols()` (from `crate::backend::elf`) and
   resolved through the normal lookup path.
   - Defined globals: use `value` directly.
   - Dynamic symbols: use PLT address if available.
   - Weak undefined: resolve to 0.
3. **Weak undefined** -- Resolve to 0 (even without checking globals).
4. **Undefined** -- Resolve to 0.
5. **`SHN_ABS` symbols** -- Use `sym.value` directly.
6. **Other local symbols** -- Resolved via section map plus `sym.value`.

**GOT-to-direct relaxation** (GOTPCRELX / REX_GOTPCRELX):

When the target symbol is locally defined (not dynamic) and has no GOT entry,
the linker can relax a GOT-indirect load into a direct LEA:

```text
Before:  mov symbol@GOTPCREL(%rip), %reg   (48 8b XX YY YY YY YY)
After:   lea symbol(%rip), %reg            (48 8d XX YY YY YY YY)
```

The opcode byte at `fp-2` is changed from `0x8b` (mov) to `0x8d` (lea), and
the displacement is rewritten to point directly at the symbol.

**IE-to-LE TLS relaxation** (GOTTPOFF without a GOT entry):

When a TLS symbol is locally defined and no GOT entry was allocated, the
linker relaxes the Initial Exec access pattern to Local Exec:

```text
Before:  movq symbol@GOTTPOFF(%rip), %reg  (48 8b XX YY YY YY YY)
After:   movq $tpoff, %reg                 (48 c7 CX YY YY YY YY)
```

The `mov r/m64, reg` instruction (`0x8b`) is rewritten to `mov $imm32, reg`
(`0xc7`), and the ModR/M byte is adjusted to `0xc0 | reg` to encode the
register in the `/0` extension field.

## Dynamic Linking Support

### .dynamic Section

The `.dynamic` section contains an array of tag-value pairs that the runtime
dynamic linker (`ld-linux-x86-64.so.2`) reads at program startup:

| Tag | Value |
|-----|-------|
| `DT_NEEDED` | One entry per required shared library (SONAME) |
| `DT_STRTAB` | Address of `.dynstr` |
| `DT_SYMTAB` | Address of `.dynsym` |
| `DT_STRSZ` | Size of `.dynstr` |
| `DT_SYMENT` | Size of one `.dynsym` entry (24) |
| `DT_DEBUG` | Reserved for debugger use (executables only) |
| `DT_PLTGOT` | Address of `.got.plt` |
| `DT_PLTRELSZ` | Size of `.rela.plt` |
| `DT_PLTREL` | Relocation type (7 = RELA) |
| `DT_JMPREL` | Address of `.rela.plt` |
| `DT_RELA` | Address of `.rela.dyn` |
| `DT_RELASZ` | Size of `.rela.dyn` |
| `DT_RELAENT` | Size of one RELA entry (24) |
| `DT_GNU_HASH` | Address of `.gnu.hash` |
| `DT_INIT_ARRAY` | Address of `.init_array` (if present) |
| `DT_INIT_ARRAYSZ` | Size of `.init_array` (if present) |
| `DT_FINI_ARRAY` | Address of `.fini_array` (if present) |
| `DT_FINI_ARRAYSZ` | Size of `.fini_array` (if present) |
| `DT_SONAME` | Shared library name (shared libraries only) |
| `DT_RPATH` / `DT_RUNPATH` | Runtime library search path (if `-rpath` specified) |
| `DT_RELACOUNT` | Number of R_X86_64_RELATIVE entries (shared libraries only) |
| `DT_VERSYM` | Address of `.gnu.version` (executables only, if versioned symbols present) |
| `DT_VERNEED` | Address of `.gnu.version_r` (executables only, if versioned symbols present) |
| `DT_VERNEEDNUM` | Number of Verneed entries (executables only, if versioned symbols present) |
| `DT_NULL` | Terminator |

Note: For shared libraries, `DT_PLTGOT`/`DT_PLTRELSZ`/`DT_PLTREL`/`DT_JMPREL`
are only emitted when PLT entries are present. `DT_DEBUG` and symbol versioning
entries are not emitted for shared libraries.

### .rela.dyn Entries

Contains `R_X86_64_GLOB_DAT` entries (type 6) for GOT slots that need to be
filled at load time, plus `R_X86_64_COPY` entries (type 5) for copy-relocated
data objects. For shared libraries, also contains `R_X86_64_RELATIVE` entries
(type 8) for internal absolute address fixups.

### .rela.plt Entries

Contains `R_X86_64_JUMP_SLOT` entries (type 7) for PLT GOT slots. These
enable lazy binding: the dynamic linker patches the GOT entry on first call.

### Copy Relocations

When code references a global data object defined in a shared library (e.g.,
`stdin`, `stderr`, `environ`), the linker:

1. Allocates space in `.bss` for a copy of the object.
2. Emits an `R_X86_64_COPY` relocation in `.rela.dyn`.
3. Updates the symbol's value to point to the BSS copy.
4. At runtime, `ld.so` copies the initial value from the shared library.

This is detected when a `R_X86_64_PC32`/`R_X86_64_PLT32` relocation targets
a dynamic symbol with `STT_OBJECT` type.

### Symbol Versioning (Executables Only)

For executables with versioned dynamic symbols, the linker emits `.gnu.version`
(SHT_GNU_VERSYM) and `.gnu.version_r` (SHT_GNU_VERNEED) sections. Each
`.dynsym` entry has a corresponding 16-bit version index in `.gnu.version`:

- 0 (`VER_NDX_LOCAL`) for the null symbol
- 1 (`VER_NDX_GLOBAL`) for unversioned symbols
- 2+ for specific version strings (e.g., `GLIBC_2.17`)

Version requirement entries in `.gnu.version_r` use `sysv_hash()` for the
`vna_hash` field. Shared libraries do not emit versioning sections.

## Shared Library Output (`link_shared` / `emit_shared_library`)

The linker can produce ELF shared libraries (`ET_DYN`) via `link_shared`.
This is used when the compiler is invoked with `-shared` (e.g., for building
PostgreSQL extension modules like `plpgsql.so` and `libpq.so`).

### Shared Library Layout

```text
+-----------------------------------------------------------------------+
| Segment 1: Read-only (PT_LOAD, PF_R)                                 |
|   ELF header + program headers                                       |
|   .gnu.hash                                                          |
|   .dynsym                                                            |
|   .dynstr                                                            |
|   .rela.dyn  (R_X86_64_RELATIVE + R_X86_64_GLOB_DAT)                |
|   .rela.plt  (R_X86_64_JUMP_SLOT)                                   |
+-----------------------------------------------------------------------+
| Segment 2: Executable (PT_LOAD, PF_R | PF_X)         [page-aligned]  |
|   .text  (merged code sections)                                       |
|   .plt   (PLT stubs for external function calls)                      |
+-----------------------------------------------------------------------+
| Segment 3: Read-only data (PT_LOAD, PF_R)            [page-aligned]  |
|   .rodata  (merged read-only data sections)                           |
+-----------------------------------------------------------------------+
| Segment 4: Read-write (PT_LOAD, PF_R | PF_W)         [page-aligned]  |
|   RELRO region:                                                       |
|     .data.rel.ro  (relocated read-only data)                          |
|     .init_array / .fini_array                                         |
|     .dynamic                                                          |
|     .got  (RELATIVE + GLOB_DAT entries)                               |
|   --- PT_GNU_RELRO boundary (page-aligned) ---                        |
|   .got.plt  (writable for lazy PLT binding)                           |
|   .data / .bss                                                        |
+-----------------------------------------------------------------------+
```

Shared libraries use a base address of `0x0` (position-independent) and
do not emit `.interp` or `.gnu.version`/`.gnu.version_r` sections.

### Key Shared Library Features

- **PLT/GOT for external symbols**: Shared libraries can call functions from
  other shared libraries (e.g., libc) through PLT stubs with lazy binding.
  R_X86_64_JUMP_SLOT relocations in `.rela.plt` enable runtime resolution.

- **GLOB_DAT relocations**: GOT entries for external data symbols (accessed
  via GOTPCREL) are filled at load time using R_X86_64_GLOB_DAT entries in
  `.rela.dyn`, separate from R_X86_64_RELATIVE entries.

- **PT_GNU_RELRO**: The `.got`, `.dynamic`, `.init_array`, `.fini_array`, and
  `.data.rel.ro` sections are placed in the RELRO region, which the dynamic
  linker marks read-only after relocations are applied. The `.got.plt` is
  deliberately placed *after* the RELRO boundary so it remains writable for
  lazy PLT binding. PT_GNU_RELRO is conditional on having RELRO-eligible
  sections.

- **SONAME**: Set via `-Wl,-soname,<name>`. Emitted as a DT_SONAME entry in
  the `.dynamic` section.

- **Rpath/Runpath**: Set via `-Wl,-rpath,<path>`. `--enable-new-dtags` uses
  DT_RUNPATH (searched after LD_LIBRARY_PATH); `--disable-new-dtags` uses
  DT_RPATH (searched before).

- **.gnu.hash layout**: Undefined symbols (imports) are placed before the
  `symoffset` boundary in `.dynsym`; defined (exported) symbols are placed
  after and included in the hash table. The bloom filter scales with the
  number of hashed symbols (1 word for <= 32 symbols, then
  `((n+31)/32).next_power_of_two()` words).

### Shared Library Symbol Extraction (PT_DYNAMIC Fallback)

When parsing input shared libraries that lack section headers (e.g., our own
emitted `.so` files), `parse_shared_library_symbols` in `linker_common.rs`
falls back to using `PT_DYNAMIC` program headers. It walks the dynamic
entries to find `DT_SYMTAB`, `DT_STRTAB`, `DT_STRSZ`, and
`DT_GNU_HASH` (for symbol count), then translates virtual addresses to file
offsets using `PT_LOAD` segments. The same fallback is used by `parse_soname`.

## Supported Relocation Types

These are the x86-64 relocation type constants defined in `elf.rs`. Types
marked with `*` are used only for output dynamic relocations, not for
processing input relocations.

| Constant | Value | Description |
|----------|-------|-------------|
| `R_X86_64_NONE` | 0 | No relocation |
| `R_X86_64_64` | 1 | 64-bit absolute |
| `R_X86_64_PC32` | 2 | 32-bit PC-relative |
| `R_X86_64_GOT32` | 3 | 32-bit GOT offset (defined but not currently handled) |
| `R_X86_64_PLT32` | 4 | 32-bit PLT-relative |
| `R_X86_64_COPY`* | 5 | Copy relocation for dynamic data objects (output only) |
| `R_X86_64_GLOB_DAT`* | 6 | GOT slot filled at load time (output only) |
| `R_X86_64_JUMP_SLOT`* | 7 | PLT GOT slot for lazy binding (output only) |
| `R_X86_64_RELATIVE`* | 8 | Base-relative fixup for shared libraries (output only) |
| `R_X86_64_GOTPCREL` | 9 | 32-bit PC-relative GOT |
| `R_X86_64_32` | 10 | 32-bit absolute unsigned |
| `R_X86_64_32S` | 11 | 32-bit absolute signed |
| `R_X86_64_GOTTPOFF` | 22 | TLS IE, PC-relative GOT |
| `R_X86_64_TPOFF32` | 23 | TLS LE, direct offset |
| `R_X86_64_PC64` | 24 | 64-bit PC-relative |
| `R_X86_64_IRELATIVE`* | 37 | IFUNC resolver (output only, static linking) |
| `R_X86_64_GOTPCRELX` | 41 | Relaxable GOTPCREL |
| `R_X86_64_REX_GOTPCRELX` | 42 | Relaxable GOTPCREL with REX |

## ELF Parsing Details (linker_common / elf.rs)

The ELF types (`Elf64Object`, `Elf64Section`, `Elf64Symbol`, `Elf64Rela`)
and core parsing functions live in `linker_common.rs`, shared across all
64-bit backends. The x86 `elf.rs` provides type aliases and thin wrappers
that supply `EM_X86_64` as the expected machine type.

### Object File Parsing (`parse_elf64_object`)

1. Validates ELF magic, class (ELFCLASS64), endianness (ELFDATA2LSB),
   type (`ET_REL`), and machine (parameterized, `EM_X86_64` for x86).
2. Parses section headers from `e_shoff`.
3. Resolves section names from the `.shstrtab` section.
4. Reads section data into per-section byte vectors.
5. Finds `SHT_SYMTAB` and parses all `Elf64_Sym` entries (24 bytes each).
   Resolves symbol names from the associated `.strtab`.
6. Finds all `SHT_RELA` sections and parses `Elf64_Rela` entries (24 bytes each).
   Indexes relocations by their target section (`sh_info`).

### Archive Parsing (`parse_archive_members`)

Defined in `crate::backend::elf` (not `linker_common`). Parses the
`!<arch>\n` format:

1. Each member has a 60-byte header with name, size, and `` `\n `` magic.
2. Special members: `/` (symbol table), `//` (extended name table).
3. Long names use `/offset` syntax into the extended name table.
4. Members are aligned to 2-byte boundaries.

Thin archives (`!<thin>\n`) are also supported via `parse_thin_archive_members`.

### Shared Library Symbol Extraction (`parse_shared_library_symbols`)

1. Validate as `ET_DYN` ELF file.
2. Find the `SHT_DYNSYM` section, `SHT_GNU_VERSYM`, and `SHT_GNU_VERDEF`
   sections (if present).
3. Build a version name map from `.gnu.verdef` entries.
4. Parse each `Elf64_Sym` entry, resolving names from the linked string table.
5. Include only defined symbols (`shndx != SHN_UNDEF`).
6. Filter out non-default versioned symbols using `.gnu.version`: if the hidden
   bit (`0x8000`) is set and the version index is >= 2, the symbol is a non-default
   version (`symbol@VERSION`, not `symbol@@VERSION`) and is skipped. This matches
   GNU ld behavior and prevents linking against deprecated/hidden symbols like
   `sysctl@GLIBC_2.2.5`.
7. Skip the null symbol at index 0.
8. Attach version information (`DynSymbol.version`, `DynSymbol.is_default_ver`)
   from the verdef mapping.

When section headers are unavailable, falls back to `PT_DYNAMIC` program headers,
reading `DT_SYMTAB`, `DT_STRTAB`, `DT_STRSZ`, `DT_GNU_HASH`, and `DT_VERSYM`
to perform the same filtering (without version name resolution).

### SONAME Extraction (`parse_soname`)

1. Find the `SHT_DYNAMIC` section (or `PT_DYNAMIC` program header as fallback).
2. Scan for a `DT_SONAME` entry.
3. Resolve the name from the section's linked string table.
4. Returns `None` if no SONAME is found; the caller is responsible for
   falling back to the file's basename.

### Linker Script Parsing (`parse_linker_script_entries`)

Defined in `crate::backend::elf`. Handles the common case where a `.so` file
is actually a text linker script (e.g., glibc's `libc.so`):

```text
/* GNU ld script */
GROUP ( /lib/x86_64-linux-gnu/libc.so.6 /usr/lib/x86_64-linux-gnu/libc_nonshared.a )
```

The parser:

1. Finds `GROUP ( ... )` or `INPUT ( ... )`.
2. Extracts file paths and `-l` library references, skipping `AS_NEEDED()` blocks.
3. Returns a list of `LinkerScriptEntry` values (either `Path` or `Lib`).

## Key Design Decisions and Trade-offs

### 1. Non-PIE Executable Output / PIC Shared Library Output

For executables, the linker produces a non-position-independent executable
(`ET_EXEC`) with a fixed base address of `0x400000`. For shared libraries,
it produces a position-independent shared object (`ET_DYN`) with a base
address of `0x0` and `R_X86_64_RELATIVE` relocations for internal absolute
addresses. PIE executable support is not yet implemented.

### 2. Fixed Four-Segment Layout

For executables, four `PT_LOAD` segments are used (RO metadata, executable
code, read-only data, read-write data) plus `PT_TLS` when TLS is present.
Executables use 8 program headers (9 with TLS): `PT_PHDR`, `PT_INTERP`,
4x `PT_LOAD`, `PT_DYNAMIC`, `PT_GNU_STACK`, and optionally `PT_TLS`.

For shared libraries, four `PT_LOAD` segments are also used, plus
`PT_GNU_RELRO` (conditional) to protect `.got`, `.dynamic`, and
`.data.rel.ro` after load-time relocations are applied. Shared libraries
use 7-9 program headers: `PT_PHDR`, 4x `PT_LOAD`, `PT_DYNAMIC`,
`PT_GNU_STACK`, optionally `PT_GNU_RELRO`, and optionally `PT_TLS`.

### 3. Lazy PLT Binding

The PLT uses the standard lazy binding model: GOT entries initially point back
into the PLT stub, and the dynamic linker patches them on first use. This is
the default behavior and does not require `DT_BIND_NOW` or `DT_FLAGS`.

### 4. .gnu.hash with Copy-Reloc Symbol Support

For executables, the `.gnu.hash` section covers copy-reloc symbols (e.g.
`optind`, `stderr`, `stdout`). Non-hashed symbols (PLT imports, GLOB_DAT
imports) are placed first in `.dynsym`, followed by hashed copy-reloc
symbols. The executable hash table uses a single 64-bit bloom word with
shift=6.

For shared libraries, all defined (exported) symbols are hashed, with
undefined symbols (imports) placed before the `symoffset` boundary. The
bloom filter scales: 1 word for <= 32 symbols, otherwise
`((n+31)/32).next_power_of_two()` words.

Both use the standard GNU hash function (DJB hash starting at 5381),
`next_power_of_two` buckets, and the chain-with-stop-bit format.
This is required for symbol interposition to work: the dynamic linker must
be able to find copy-reloc symbols in the executable via `.gnu.hash` so
that shared library references resolve to the executable's BSS copy.

### 5. Archive Selective Loading (Group Resolution)

Archives are loaded using the traditional Unix semantics: only members that
define symbols satisfying currently-undefined references are pulled in. The
iteration continues until a fixed point is reached, handling chains of
dependencies between archive members.

All libraries (both default and user-specified) are loaded in a group loop,
equivalent to `ld`'s `--start-group`/`--end-group`. This handles circular
dependencies between archives (e.g., `libc.a` needing `__letf2` from
`libgcc.a` on architectures with software floating-point). The outer loop
re-scans all archives until no new objects are pulled in.

### 6. Copy Relocations for Dynamic Data Objects

When code takes the address of a global variable defined in a shared library,
the linker creates a copy of the variable in `.bss` and emits an
`R_X86_64_COPY` relocation. This is the standard approach for non-PIE
executables and matches what `ld` does.

### 7. GOT-to-LEA Relaxation

For `R_X86_64_GOTPCRELX` and `R_X86_64_REX_GOTPCRELX` relocations targeting
locally-defined symbols, the linker relaxes `mov GOT(%rip), %reg` to
`lea symbol(%rip), %reg`. This eliminates a memory indirection and is a
significant optimization for accessing global variables.

### 8. IE-to-LE TLS Relaxation

When a TLS variable is locally defined and accessed via the Initial Exec model
(`R_X86_64_GOTTPOFF`), the linker can relax the GOT-indirect access to a
direct `mov $tpoff, %reg` instruction. This eliminates both the GOT entry and
the memory load.

### 9. System Library Fallback

The `resolve_dynamic_symbols_elf64` function (in `linker_common`) searches
library paths provided by the caller (typically `/lib/x86_64-linux-gnu/` and
`/usr/lib/x86_64-linux-gnu/`) for a default set of system libraries. This
is pragmatic but not portable. A production linker would use the `ldconfig`
cache or search paths from `/etc/ld.so.conf`.

### 10. Linker Option Support

The linker handles several GNU `ld`-compatible options parsed from `-Wl,...` args:

- **`--gc-sections`**: Dead-code elimination. After symbol resolution,
  `gc_collect_sections_elf64()` identifies sections unreachable from entry
  points. Dead sections are excluded from merging, and undefined symbols
  referenced only from dead code are pruned from `globals`.
- **`--export-dynamic`**: Passed through to the emitter so all defined
  globals appear in `.dynsym` (needed for `dlsym()` lookups).
- **`--defsym=ALIAS=TARGET`**: Creates symbol aliases by cloning the
  target's `GlobalSymbol` entry under the alias name.
- **`--whole-archive` / `--no-whole-archive`**: Positional flags that
  force all members of subsequent archives to be loaded (rather than
  selective loading). Tracked per-item in `link_shared`'s ordered item
  list.

### 11. Static Linking with IFUNC Support

When linking with `-static`, the linker collects symbols with `STT_GNU_IFUNC`
type and emits `R_X86_64_IRELATIVE` relocations in a `.rela.iplt` section.
The CRT startup code calls the IFUNC resolver functions and patches the
GOT entries before `main()` runs. The linker-defined symbols
`__rela_iplt_start` and `__rela_iplt_end` bracket this relocation array.

### 12. Section Headers in Output

The output executable includes a section header table appended after the main
file data. This enables tools like `strip`, `readelf -S`, and `objdump -d`
to work correctly. The section header table describes all loadable sections
(.interp, .gnu.hash, .dynsym, .dynstr, .gnu.version, .gnu.version_r,
.rela.dyn, .rela.plt, .plt, merged text/rodata/data sections, TLS sections,
.init_array, .fini_array, .dynamic, .got, .got.plt, .bss) plus a `.shstrtab`
string table. The `e_shoff`, `e_shnum`, and `e_shstrndx` fields are patched
back into the ELF header.

### 13. Flat Output Buffer

The entire output file is allocated as a single `Vec<u8>` of the computed
file size, initialized to zero. All writes are done via helper functions
(`w16`, `w32`, `w64`, `write_bytes`) that write at absolute offsets. This
makes the layout explicit and avoids the complexity of streaming writes, at the
cost of holding the entire output in memory.
