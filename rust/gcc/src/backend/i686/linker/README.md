# i686 Built-in Linker -- Design Document

## Overview

The i686 built-in linker reads ELF32 relocatable object files (`.o`) and static
archives (`.a`, including thin archives), resolves symbols against system shared
libraries, applies i386 relocations, and emits a dynamically-linked (or static)
ELF32 executable or shared library (`-shared`). It replaces the external GNU
linker (`ld`) for the `i686-linux-gnu` target.

The linker is invoked by the compiler driver, which first discovers CRT objects
(`crt1.o`, `crti.o`, `crtn.o`), GCC library directories, and system library
paths using a shared architecture-configuration module (`DirectLdArchConfig`).
The linker itself focuses purely on ELF linking logic.

## Architecture

```text
  +-----------------------+     +-----------------------+
  | CRT objects           |     | User .o files         |
  | (crt1.o, crti.o, ...) |     | (from compilation)    |
  +-----------+-----------+     +-----------+-----------+
              |                             |
              +-------------+---------------+
                            |
                            v
                +------------------------+
                |   Input Parsing         |
                |   parse_elf32()         |  Validate ELFCLASS32, EM_386
                |   parse_archive()       |  Extract .a members
                +------------------------+
                            |
                            v
                +------------------------+
                |   Archive Resolution    |  Demand-driven extraction:
                |   resolve_archive_      |  pull in members that satisfy
                |   members()             |  undefined symbols
                +------------------------+
                            |
                            v
                +------------------------+
                |   Section Merging       |  Group .text.*, .rodata.*, etc.
                |   output_section_name() |  into canonical output sections
                |                         |  (with COMDAT deduplication)
                +------------------------+
                            |
                            v
                +------------------------+
                |   Symbol Resolution     |  Two-pass: definitions, then
                |   global_symbols        |  undefined -> dynamic lookup
                +------------------------+
                            |
                            v
                +------------------------+
                |   PLT/GOT Construction  |  Build PLT stubs, GOT entries,
                |                         |  .rel.plt, .rel.dyn
                +------------------------+
                            |
                            v
                +------------------------+
                |   Layout                |  Assign virtual addresses and
                |   LOAD segments         |  file offsets; page alignment
                +------------------------+
                            |
                            v
                +------------------------+
                |   Relocation Application|  Apply R_386_32, R_386_PC32,
                |                         |  R_386_PLT32, R_386_GOT*,
                |                         |  TLS relocations, etc.
                +------------------------+
                            |
                            v
                +------------------------+
                |   ELF Emission          |  Write headers, segments,
                |                         |  synthetic sections, data
                +------------------------+
                            |
                            v
                      ELF32 executable
```

## Differences from the x86-64 Linker

| Aspect                    | x86-64                                | i686                                  |
|---------------------------|---------------------------------------|---------------------------------------|
| ELF class                 | ELFCLASS64                            | ELFCLASS32                            |
| Machine type              | EM_X86_64 (62)                        | EM_386 (3)                            |
| ELF header size           | 64 bytes                              | 52 bytes                              |
| Program header size       | 56 bytes                              | 32 bytes                              |
| Section header size       | 64 bytes                              | 40 bytes                              |
| Symbol entry size         | 24 bytes (Elf64_Sym)                  | 16 bytes (Elf32_Sym)                  |
| Relocation format         | RELA (Elf64_Rela, 24 bytes)           | REL (Elf32_Rel, 8 bytes)             |
| Relocation `r_info`       | `(sym << 32) \| type`                 | `(sym << 8) \| type`                  |
| Implicit addend           | Stored in `r_addend` field            | Read from section data at reloc site  |
| Base address              | `0x400000`                            | `0x08048000`                          |
| Dynamic linker            | `/lib64/ld-linux-x86-64.so.2`         | `/lib/ld-linux.so.2`                  |
| Relocation types          | `R_X86_64_*`                          | `R_386_*`                             |
| Address size              | 8 bytes                               | 4 bytes                               |
| GOT entry size            | 8 bytes                               | 4 bytes                               |
| Dynamic tag size          | 16 bytes (d_tag + d_val)              | 8 bytes (d_tag + d_val)              |
| PLT addressing            | RIP-relative (`jmp *GOT(%rip)`)       | Absolute (`jmp *[abs32]`)             |
| PLT entry size            | 16 bytes                              | 16 bytes                              |
| DT_PLTREL value           | DT_RELA (7)                           | DT_REL (17)                           |
| `.rel.plt` entry type     | R_X86_64_JUMP_SLOT (Rela, 24 B)       | R_386_JMP_SLOT (Rel, 8 B)            |
| `.rel.dyn` entry type     | R_X86_64_GLOB_DAT (Rela)              | R_386_GLOB_DAT (Rel)                 |
| Copy relocation           | R_X86_64_COPY                         | R_386_COPY                            |
| PC-relative addressing    | RIP-relative (native)                 | Requires GOT-based PIC tricks         |

## Key Data Structures

### Input structures

| Type            | Role                                                          |
|-----------------|---------------------------------------------------------------|
| `InputObject`   | Parsed `.o` file: sections + symbols + filename               |
| `InputSection`  | One section from an input file: data, flags, relocations      |
| `InputSymbol`   | One symbol: name, value, size, binding, type, section index   |
| `Elf32Sym`      | Raw ELF32 symbol entry (16 bytes)                             |
| `Elf32Shdr`     | Raw ELF32 section header (40 bytes)                           |

### Linker-internal structures

| Type            | Role                                                          |
|-----------------|---------------------------------------------------------------|
| `LinkerSymbol`  | Resolved symbol: address, binding, PLT/GOT indices, dynamic  |
|                 | library info, copy relocation flag, version string,           |
|                 | `uses_textrel` flag for weak dynamic data symbols             |
| `OutputSection` | Merged output section: data, vaddr, file offset              |
| `SectionMap`    | Maps `(obj_idx, sec_idx)` to `(out_sec_idx, offset)`         |
| `DynSymInfo`    | Symbol exported by a shared library: type, version, binding  |
| `DynStrTab`     | Builder for `.dynstr` (dynamic string table)                  |

### Output layout tracking

Virtual addresses and file offsets are tracked as `u32` variables during the
layout phase. Each segment and synthetic section has corresponding `*_offset`,
`*_vaddr`, and `*_size` variables.

## Processing Algorithm

### Step 1: Input Parsing

**Object files** are parsed by `parse_elf32()`:

1. Validate ELF magic, ELFCLASS32, ELFDATA2LSB, ET_REL, EM_386.
2. Read section headers and section name string table.
3. Find `.symtab` and its linked `.strtab`.
4. Parse all symbols into `InputSymbol` structs.
5. For each section, find associated `.rel.*` sections and parse their
   `Elf32_Rel` entries. The implicit addend is read from the section data at
   the relocation offset (i386 REL convention).

**Archives** are parsed by `parse_archive()` (regular) or
`parse_thin_archive_i686()` (thin):

1. Validate `!<arch>\n` magic (or `!<thin>\n` for thin archives).
2. For regular archives: extract each member whose content starts with ELF magic.
3. For thin archives: read member paths and load external `.o` files.
4. Each extracted member is then parsed by `parse_elf32()`.

Archive members are placed into an archive pool for demand-driven extraction
(see Step 2).

**Shared libraries** are scanned by `read_dynsyms_with_search()`:

1. Validate ELF magic, ELFCLASS32, ET_DYN. If the file is a GNU linker script
   (GROUP/INPUT directives), resolve the referenced `.so` files recursively.
2. Find `.dynsym`, `.gnu.version`, and `.gnu.verdef` sections.
3. Parse version definitions (verdef) to build a version-index-to-name map.
4. Enumerate all defined, global/weak symbols from `.dynsym`.
5. For each symbol, look up its version from `.gnu.version` and map it through
   the verdef table.
6. Return a list of `DynSymInfo` entries with name, type, size, and version.

### Step 2: Archive Resolution

Archive members are extracted on demand using an iterative algorithm:

1. Scan all already-linked objects to collect defined and undefined symbols.
2. Search the archive pool for members that define any currently-undefined symbol.
3. When a member is extracted, add its definitions and new undefined references.
4. Repeat until no more members can be extracted (fixed-point iteration).

This avoids linking unused code from archives while correctly handling
transitive dependencies between archive members. The `--defsym` alias targets
are also considered as undefined symbols to ensure their definitions are pulled
from archives.

### Step 3: Section Merging

Input sections are merged into canonical output sections by name. COMDAT
groups (`SHT_GROUP` with `GRP_COMDAT` flag) are deduplicated: only the first
instance of each group signature is kept, and duplicate members are skipped.

```text
  Input section name           ->  Output section name
  ──────────────────────────       ────────────────────
  .text, .text.*                   .text
  .init                            .init  (kept separate)
  .fini                            .fini  (kept separate)
  .rodata, .rodata.*               .rodata
  .data, .data.*, .tm_clone_table  .data
  .bss, .bss.*, SHT_NOBITS        .bss
  .tdata, .tdata.*                 .tdata  (TLS initialized)
  .tbss, .tbss.*                   .tbss   (TLS zero-init)
  .init_array, .init_array.*       .init_array
  .fini_array, .fini_array.*       .fini_array
  .eh_frame                        .eh_frame
  .note.*  (SHT_NOTE)              .note
```

Non-allocatable sections (`SHT_NULL`, `SHT_SYMTAB`, `SHT_STRTAB`, `SHT_REL`,
`SHT_RELA`, `SHT_GROUP`, `.note.GNU-stack`, `.comment`) are skipped.

Each input section's data is appended to the output section with alignment
padding. The `SectionMap` records the mapping from `(object_index,
input_section_index)` to `(output_section_index, offset_within_output)`.

### Step 4: Symbol Resolution

**First pass -- collect definitions:**

For each defined symbol in each input object (skipping FILE, SECTION, and
UNDEF symbols), compute its output-section index and offset. If a name
collision occurs:

- Global beats weak.
- Defined beats undefined.
- Otherwise, keep the first definition.

**Second pass -- resolve undefined references:**

For each undefined symbol, check:

1. Already defined in `global_symbols`? Skip.
2. Available in `dynlib_syms` (from shared library scanning)?
   - Function symbols (including IFUNC): mark `needs_plt = true`, `needs_got = true`.
   - Data symbols: mark `needs_copy = true` (copy relocation).
3. Weak undefined? Insert with address 0 (allowed to remain unresolved).
4. Truly undefined? Will produce an error later, unless it is a linker-
   defined symbol.

**Section symbol resolution:**

Section symbols (STT_SECTION) are mapped to synthetic names
`__section_{obj}_{idx}` for relocation resolution.

**PLT/GOT marking:**

A third scan over all relocations marks which symbols need PLT or GOT entries
based on relocation types:

- `R_386_PLT32` on a dynamic symbol -> `needs_plt = true`, `needs_got = true`.
- `R_386_GOT32` or `R_386_GOT32X` -> `needs_got = true`.
- `R_386_TLS_GOTIE` or `R_386_TLS_IE` -> `needs_got = true`.

**Weak dynamic data handling:**

After PLT/GOT lists are built, weak dynamic data symbols (non-function,
`STB_WEAK`) are converted from copy relocations to text relocations
(`DT_TEXTREL`). This avoids issues with copy relocations for weak symbols
that may not exist in all library versions.

**COMMON symbol allocation:**

Tentative definitions (`SHN_COMMON`) -- global variables declared without an
initializer -- are allocated space in `.bss` with proper alignment. This
happens after symbol resolution so that a real definition from another object
file takes precedence.

**Undefined symbol check:**

After resolution, any symbol that is undefined, not dynamic, not weak, and not
linker-defined triggers an error. Linker-defined symbols are managed by
`linker_common::is_linker_defined_symbol()` and include:
`_GLOBAL_OFFSET_TABLE_`, `__ehdr_start`, `__executable_start`, `_end`,
`_edata`, `_etext`, `__bss_start`, `__dso_handle`, `__rel_iplt_start`, etc.

### Step 5: PLT and GOT Construction

PLT and GOT symbols are sorted alphabetically for deterministic output.

**GOT layout:**

```text
  .got:
    [0]     _DYNAMIC address     (1 reserved entry)
    [1..N]  non-PLT GOT entries  (dynamic imports via R_386_GLOB_DAT,
                                  then local symbols resolved at link time)

  .got.plt:
    [0]  _DYNAMIC address     (GOT.PLT reserved)
    [1]  0 (link_map, filled by ld.so at runtime)
    [2]  0 (resolver, filled by ld.so at runtime)
    [3+i]  PLT[i+1] + 6      (lazy binding: points to pushl in PLT stub)
```

**PLT layout:**

```text
  PLT[0]:  (resolver stub, 16 bytes)
    ff 35 [GOT.PLT+4]      pushl GOT.PLT[1]
    ff 25 [GOT.PLT+8]      jmp  *GOT.PLT[2]
    90 90 90 90             nop  padding

  PLT[i]:  (per-symbol stub, 16 bytes each)
    ff 25 [GOT.PLT[3+i]]   jmp  *GOT.PLT[3+i]
    68 [i*8]                pushl reloc byte-offset into .rel.plt
    e9 [PLT[0] - here]     jmp  PLT[0]
```

On i686, PLT stubs use **absolute addressing** (`jmp *[abs32]`) rather than
RIP-relative addressing. This is the fundamental difference from x86-64 PLT
entries.

**IFUNC support (static linking):**

For static executables, `STT_GNU_IFUNC` symbols are handled through IPLT
(indirect PLT) stubs. Each IFUNC gets an 8-byte IPLT entry (`jmp *[GOT]`

+ nop padding), a dedicated GOT entry initialized to the resolver address,
and an `R_386_IRELATIVE` entry in `.rel.iplt`. The C runtime invokes the
resolver at startup and patches the GOT entry with the resolved address.

### Step 6: Layout

The linker uses a fixed base address of `0x08048000` (standard i386 convention)
and lays out LOAD segments with page alignment:

```text
  Segment 0 (RO):  ELF header + program headers + .interp + .note +
                   .gnu.hash + .dynsym + .dynstr + .gnu.version +
                   .gnu.version_r + .rel.dyn + .rel.plt

  Segment 1 (RX):  .init + .plt + .text + .fini + .iplt

  Segment 2 (RO):  .rodata + .eh_frame + .eh_frame_hdr
                   (omitted if empty)

  Segment 3 (RW):  .init_array + .fini_array + .dynamic + .got +
                   .got.plt + .data + .tdata + .bss (+ copy reloc space)
```

The virtual address of each segment is aligned to `PAGE_SIZE` (0x1000) and
satisfies `vaddr = file_offset (mod PAGE_SIZE)` so that mmap-based loading
works correctly.

**Program headers emitted:**

| Type              | Segment                              | Flags       |
|-------------------|--------------------------------------|-------------|
| `PT_PHDR`         | Program header table itself           | `PF_R`      |
| `PT_INTERP`       | `.interp` (`/lib/ld-linux.so.2`)     | `PF_R`      |
| `PT_LOAD`         | Read-only headers segment             | `PF_R`      |
| `PT_LOAD`         | Text segment (RX)                    | `PF_R|PF_X` |
| `PT_LOAD`         | Read-only data segment               | `PF_R`      |
| `PT_LOAD`         | Read-write data segment              | `PF_R|PF_W` |
| `PT_DYNAMIC`      | `.dynamic` section                   | `PF_R|PF_W` |
| `PT_GNU_STACK`    | Stack permissions (non-executable)   | `PF_R|PF_W` |
| `PT_GNU_EH_FRAME` | `.eh_frame_hdr` for unwinding        | `PF_R`      |
| `PT_TLS`          | TLS template (`.tdata` + `.tbss`)    | `PF_R`      |

`PT_INTERP` and `PT_DYNAMIC` are omitted in static linking mode.
`PT_TLS` is only emitted when TLS sections are present.
`PT_GNU_RELRO` is intentionally omitted to avoid conflicts with lazy PLT
binding when `.got` and `.got.plt` share the same page.

**Copy relocations:**

Data symbols imported from shared libraries (e.g., `stdin`, `environ`) are
handled via **R_386_COPY** relocations. Space is allocated at the end of BSS
for each copy-relocated symbol, and the dynamic linker copies the symbol's
value from the shared library into this space at load time.

### Step 7: Symbol Address Resolution

After layout, final virtual addresses are assigned:

- **Defined symbols**: `output_section.addr + section_offset`
- **Dynamic function symbols with PLT**: `plt_vaddr + header + index * 16`
- **Dynamic data symbols with copy reloc**: BSS copy address
- **IFUNC symbols (static)**: overridden to IPLT entry address
- **Linker-defined symbols**: Computed from segment boundaries
  (`_etext`, `__bss_start`, `_end`, `_GLOBAL_OFFSET_TABLE_`, etc.)

### Step 8: Relocation Application

For each input section's relocations, the linker computes and patches the
output section data:

| Relocation type     | Formula             | Description                        |
|---------------------|---------------------|------------------------------------|
| `R_386_NONE`        | (skip)              | No-op                              |
| `R_386_32`          | `S + A`             | Absolute 32-bit                    |
| `R_386_PC32`        | `S + A - P`         | PC-relative 32-bit                 |
| `R_386_PLT32`       | `S + A - P`         | PLT-relative (same formula)        |
| `R_386_GOTPC`       | `GOT + A - P`       | PC-relative offset to GOT base     |
| `R_386_GOTOFF`      | `S + A - GOT`       | Offset from GOT base               |
| `R_386_GOT32`       | `G + A - GOT`       | GOT entry offset from GOT base     |
| `R_386_GOT32X`      | `G + A - GOT`       | Relaxable GOT entry reference       |
| `R_386_TLS_TPOFF`   | `S - TP_base`       | Negative TP offset (initial-exec)  |
| `R_386_TLS_LE`      | `S - TP_base`       | Local-exec TP offset               |
| `R_386_TLS_LE_32`   | `TP_base - S`       | Negated TP offset                  |
| `R_386_TLS_TPOFF32` | `TP_base - S`       | Negated TP offset (variant)        |
| `R_386_TLS_IE`      | GOT entry / tpoff   | Initial-exec via GOT               |
| `R_386_TLS_GOTIE`   | GOT offset / tpoff  | Initial-exec GOT-relative          |
| `R_386_TLS_GD`      | tpoff               | General-dynamic (relaxed to LE)    |
| `R_386_TLS_DTPMOD32`| 1                   | Module ID (always 1 for exe)       |
| `R_386_TLS_DTPOFF32`| `S - TLS_base`      | DTP offset within TLS block        |

Where:

- `S` = symbol address (PLT address for dynamic function symbols)
- `A` = addend (read from section data, i386 REL convention)
- `P` = relocation site address (patch_addr)
- `GOT` = GOT base address (start of `.got.plt` when PLT exists, else `.got`)
- `G` = GOT entry address for the symbol
- `TP_base` = TLS segment address + TLS memory size (thread pointer base)

The GOT base address follows the i386 convention: `_GLOBAL_OFFSET_TABLE_`
points to `.got.plt` when PLT entries exist, otherwise to `.got`.

**GOT32X relaxation:** When `R_386_GOT32X` references a locally-defined symbol
that doesn't need a GOT entry, the linker rewrites `mov` instructions to `lea`
(opcode `0x8b` -> `0x8d`) and computes the offset directly, avoiding the GOT
indirection.

### Step 9: Dynamic Section and Version Tables

**`.dynamic` section** entries (each 8 bytes: `d_tag` + `d_val`):

- `DT_NEEDED` for each shared library (libc.so.6, libm.so.6, etc.)
- `DT_GNU_HASH`, `DT_STRTAB`, `DT_SYMTAB`, `DT_STRSZ`, `DT_SYMENT`
- `DT_INIT` / `DT_FINI` (if `.init` / `.fini` sections exist)
- `DT_INIT_ARRAY` / `DT_INIT_ARRAYSZ` / `DT_FINI_ARRAY` / `DT_FINI_ARRAYSZ`
- `DT_DEBUG`
- `DT_PLTGOT`, `DT_PLTRELSZ`, `DT_PLTREL` (= 17, DT_REL), `DT_JMPREL`
- `DT_REL`, `DT_RELSZ`, `DT_RELENT` (= 8)
- `DT_VERNEED`, `DT_VERNEEDNUM`, `DT_VERSYM` (if versions are present)
- `DT_TEXTREL` (if weak dynamic data symbols require text relocations)
- `DT_NULL`

**`.gnu.hash` section** uses the GNU hash algorithm, consistent with the x86-64
and RISC-V linkers. Uses 32-bit bloom filter words (ELF32 word size). Copy-reloc
and textrel symbols (defined in this executable) are placed in the hashed
portion so the dynamic linker can find them for symbol interposition.

**Symbol versioning:**

The linker emits `.gnu.version` (versym) and `.gnu.version_r` (verneed)
sections when dynamic symbols have GLIBC version annotations (e.g.,
`GLIBC_2.0`, `GLIBC_2.17`). Version information is extracted from the shared
library's `.gnu.verdef` section during `read_dynsyms_with_search()`.

Symbols without version annotations use `VER_NDX_GLOBAL` (index 1) in the
versym table, which means "any version" to the dynamic linker.

### Step 10: Output Emission

The final ELF32 executable is written as a flat byte array:

1. ELF32 header (52 bytes) with `ET_EXEC`, `EM_386`, entry point
2. Program headers (32 bytes each)
3. Segment data in file-offset order:
   - Read-only: `.interp`, `.note`, `.gnu.hash`, `.dynsym`, `.dynstr`,
     `.gnu.version`, `.gnu.version_r`, `.rel.dyn`, `.rel.plt`
   - Text: `.init`, `.plt`, `.text`, `.fini`, `.iplt`
   - Read-only data: `.rodata`, `.eh_frame`, `.eh_frame_hdr`
   - Read-write data: `.init_array`, `.fini_array`, `.dynamic`, `.got`,
     `.got.plt`, `.data`, `.tdata`
   - BSS (`.bss` + `.tbss` + copy reloc space) occupies virtual address space
     but no file space
4. File permissions set to 0755

Section headers (`e_shoff`, `e_shnum`) are set to 0 -- the executable does
not include a section header table. This is valid for execution (only program
headers matter) and reduces file size.

## Key Design Decisions and Trade-offs

1. **REL relocation format**. i386 ELF uses `Elf32_Rel` (no explicit addend
   field). The linker reads implicit addends from the section data at the
   relocation offset. This matches the convention established by the assembler
   and is compatible with all i386 object files produced by GCC and LLVM.

2. **GNU hash table**. The linker emits a `.gnu.hash` section using the GNU
   hash algorithm with 32-bit bloom filter words (matching ELF32 word size).
   This is consistent with the x86-64 and RISC-V linkers. The dynsym table
   is ordered with unhashed (undefined import) symbols first, followed by
   hashed (defined copy-reloc and textrel) symbols sorted by bucket.

3. **No section headers in output**. The executable omits section headers
   entirely. The kernel and dynamic linker only need program headers to load
   and execute the file. This simplifies output emission and produces slightly
   smaller executables. Debuggers and tools like `objdump` lose some
   information, but `readelf -l` (program headers) still works.

4. **Fixed base address (0x08048000)**. The linker produces `ET_EXEC`
   executables with a fixed base address, not PIE (`ET_DYN`). This simplifies
   relocation computation (no need for relative addressing throughout) and
   matches the traditional i386 Linux executable layout. The trade-off is no
   ASLR for the main executable.

5. **Lazy PLT binding**. GOT.PLT entries initially point to the `pushl`
   instruction in each PLT stub (the "lazy" target). On first call, the
   dynamic linker resolves the symbol and patches the GOT.PLT entry to point
   directly to the resolved function. This is the standard lazy binding
   mechanism for i386.

6. **Copy relocations for data symbols**. When a program references a data
   symbol from a shared library (e.g., `errno`, `stdin`), the linker allocates
   space in BSS and emits `R_386_COPY`. The dynamic linker copies the symbol's
   initial value from the shared library. This avoids indirection through GOT
   for data accesses. Weak dynamic data symbols use text relocations
   (`DT_TEXTREL`) instead, to avoid issues with symbols that may be absent.

7. **Demand-driven archive extraction**. Archive members are extracted only
   when they define a currently-undefined symbol, iterating until a fixed point
   is reached. This avoids linking unused code from large archives while
   correctly resolving transitive dependencies between members.

8. **PT_GNU_RELRO omitted**. The `PT_GNU_RELRO` segment is intentionally not
   emitted. On i386, `.got` and `.got.plt` can share the same virtual page.
   Marking part of that page as read-only after relocation would prevent lazy
   PLT binding from working. A proper implementation would require separating
   `.got` and `.got.plt` onto different pages.

9. **Library search strategy**. The linker searches for shared libraries in the
   caller-provided library paths plus user-specified `-L` paths. It scans
   directory entries for versioned filenames (`libfoo.so.6`, `libfoo.so.6.0.1`)
   and follows symlinks via `canonicalize()`. If no `.so` is found, it falls
   back to a static archive (`libfoo.a`).

10. **TLS relaxation**. All TLS access models (GD, IE, LE) are relaxed to
    direct TP-offset computation for statically-linked TLS. The linker does
    not emit TLS descriptors or `R_386_TLS_*` dynamic relocations; all TLS
    offsets are resolved at link time.

## File Inventory

| File         | Lines | Role                                                        |
|--------------|-------|-------------------------------------------------------------|
| `mod.rs`     | ~50   | Module declarations, `DynStrTab` u32 wrapper, public re-exports |
| `types.rs`   | ~332  | ELF32-specific constants (relocation types, dynamic tags,   |
|              |       | section flags), struct definitions (`InputObject`,          |
|              |       | `LinkerSymbol`, `OutputSection`, etc.), helper functions    |
| `parse.rs`   | ~195  | ELF32 object file parsing (`parse_elf32`), regular and      |
|              |       | thin archive extraction                                     |
| `dynsym.rs`  | ~310  | Dynamic symbol reading from ELF32 shared libraries,         |
|              |       | GNU version info parsing, linker script resolution          |
| `reloc.rs`   | ~302  | i386 relocation application: all R_386_* types including    |
|              |       | GOT32X relaxation and TLS relocations                       |
| `gnu_hash.rs`| ~76   | GNU hash table (.gnu.hash) builder for ELF32 with 32-bit   |
|              |       | bloom filter words                                          |
| `input.rs`   | ~362  | Phases 1-4: argument parsing, file collection, library      |
|              |       | loading, shared lib scanning, archive resolution            |
| `sections.rs`| ~131  | Phase 5: section merging, COMDAT deduplication,             |
|              |       | output section type/flag assignment                         |
| `symbols.rs` | ~356  | Phases 6-9: symbol resolution, COMMON allocation,           |
|              |       | PLT/GOT marking, undefined check, PLT/GOT list building    |
| `link.rs`    | ~216  | Orchestration: `link_builtin` and `link_shared` entry points|
| `emit.rs`    | ~1,248| Phase 10: executable layout, address assignment,            |
|              |       | relocation application, ELF32 emission                      |
| `shared.rs`  | ~1,058| Shared library (.so) emission: PIC layout, dynamic          |
|              |       | relocations, symbol export, NEEDED discovery                |
