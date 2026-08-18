# AArch64 Built-in Linker -- Design Document

## Overview

The built-in AArch64 linker links ELF64 relocatable object files (`.o`) and
static archives (`.a`) into ELF64 executables for AArch64 Linux, supporting
both static and dynamic linking. It can also produce shared libraries
(`ET_DYN` / `.so` files) via `link_shared()`. It replaces the external `ld`
dependency when the `gcc_linker` Cargo feature is not enabled (the default), making the
compiler fully self-hosting.

The linker implements the complete linking pipeline: ELF object parsing,
archive member extraction, symbol resolution, section merging, virtual address
layout, GOT/PLT construction, TLS handling, IFUNC support, relocation
application, dynamic section emission, and final ELF output.

The implementation spans roughly 4,000 lines of Rust across ten modules, plus
shared infrastructure in `linker_common` (`GlobalSymbolOps` trait,
`OutputSection` / `InputSection` types, section merging, symbol registration,
common symbol allocation, archive loading, library resolution, section name
mapping, `.eh_frame` processing, and dynamic symbol resolution).

```text
             AArch64 Built-in Linker
  ============================================================

  .o files    .a archives    -l libraries    .so shared libs
       \           |           /                /
        v          v          v                v
  +------------------------------------------+
  |           elf.rs  (~75 lines)            |
  |   Type aliases + thin wrappers to        |
  |   linker_common; AArch64 reloc consts    |
  +------------------------------------------+
               |
               v
  +------------------------------------------+
  |         input.rs  (~90 lines)            |
  |   File loading: load_file, resolve_lib   |
  +------------------------------------------+
               |
               v
  +------------------------------------------+
  |   types.rs (~93)  |  plt_got.rs (~130)   |
  |   GlobalSymbol,   |  PLT/GOT list        |
  |   arch constants  |  construction        |
  +------------------------------------------+
               |
               v
  +------------------------------------------+
  |          link.rs  (~411 lines)           |
  |   Orchestrator: link_builtin,            |
  |   link_shared entry points               |
  +------------------------------------------+
          /          |          \
         v           v           v
  +-----------+ +-----------+ +-----------+
  | emit_     | | emit_     | | emit_     |
  | dynamic   | | static    | | shared    |
  | (~869)    | | (~645)    | | (~1098)   |
  +-----------+ +-----------+ +-----------+
               |
               v
  +------------------------------------------+
  |          reloc.rs  (~540 lines)          |
  |   Relocation Application: 40+ reloc     |
  |   types, TLS relaxation, GOT refs       |
  +------------------------------------------+
               |
               v
        ELF64 executable on disk
```

---

## Public Entry Points

The linker has two public entry points:

```rust
// mod.rs -- static and dynamic executable linking
pub fn link_builtin(
    object_files: &[&str],          // Paths to .o files from the compiler
    output_path: &str,               // Output executable path
    user_args: &[String],            // Additional flags: -L, -l, -Wl,...
    lib_paths: &[&str],              // Library search paths (from common.rs)
    needed_libs: &[&str],            // Default libraries to link (e.g., "gcc", "c")
    crt_objects_before: &[&str],     // CRT objects before user code (crt1.o, crti.o, ...)
    crt_objects_after: &[&str],      // CRT objects after user code (crtend.o, crtn.o)
    is_static: bool,                 // Static vs dynamic linking
) -> Result<(), String>

// mod.rs -- shared library (.so) output
pub fn link_shared(
    object_files: &[&str],
    output_path: &str,
    user_args: &[String],
    lib_paths: &[&str],
) -> Result<(), String>
```

CRT object discovery, library path resolution, and the `-nostdlib`/`-static`
flags are handled by `common.rs`'s `resolve_builtin_link_setup()` before
calling into the linker. The linker receives pre-resolved paths and loads
them in order.

---

## Stage 1: ELF Parsing (`elf.rs` / `linker_common`)

### Purpose

Read and decode ELF64 relocatable object files, static archives, and minimal
linker scripts. The actual parsing logic lives in the shared `linker_common`
module; `elf.rs` provides AArch64-specific relocation constants and re-exports
shared types under local names via type aliases.

### Key Data Structures

ELF64 types are defined in `linker_common` and re-exported via type aliases:

| Type | Alias | Role |
|------|-------|------|
| `Elf64Object` | `ElfObject` | A fully parsed object file: sections, symbols, raw section data, relocations indexed by section. |
| `Elf64Section` | `SectionHeader` | Parsed `Elf64_Shdr`: name, type, flags, offset, size, link, info, alignment, entsize. |
| `Elf64Symbol` | `Symbol` | Parsed `Elf64_Sym`: name, info (binding + type), other (visibility), shndx, value, size. |
| `Elf64Rela` | `Rela` | Parsed `Elf64_Rela`: offset, sym_idx, rela_type, addend. |

### Object Parsing (`parse_object`)

Delegates to `linker_common::parse_elf64_object(data, source_name, EM_AARCH64)`.

### Archive and Linker Script Parsing

Archive parsing (`parse_archive_members`, `parse_thin_archive_members`) and
linker script parsing (`parse_linker_script_entries`) are provided by the
shared `crate::backend::elf` module.

---

## Stage 2: Orchestration (`link.rs` + `input.rs` + `plt_got.rs`)

### Purpose

These modules form the linker driver. `link.rs` coordinates the pipeline:
file loading (delegated to `input.rs`), symbol resolution, section merging,
address layout, PLT/GOT construction (delegated to `plt_got.rs`), and
dispatching to the appropriate emission module.

### Key Data Structures

| Type | Role |
|------|------|
| `OutputSection` | Shared type from `linker_common`: merged output section with name, type, flags, alignment, list of `InputSection` references, merged data buffer, assigned virtual address and file offset, memory size. |
| `InputSection` | Shared type from `linker_common`: reference to one input section with object index, section index, output offset within the merged section, size. |
| `GlobalSymbol` | ARM-specific resolved global symbol: implements `linker_common::GlobalSymbolOps` trait. Contains final value (address), size, info byte, defining object index, section index, plus dynamic linking fields (`from_lib`, `plt_idx`, `got_idx`, `is_dynamic`, `copy_reloc`, `lib_sym_value`). |

### Constants

```text
BASE_ADDR  = 0x400000     -- Base virtual address for the executable
PAGE_SIZE  = 0x10000      -- 64 KB (AArch64 linker page alignment)
INTERP     = "/lib/ld-linux-aarch64.so.1"  -- dynamic linker path
```

### Linking Algorithm -- Step by Step

```text
link_builtin(object_files, output_path, user_args, lib_paths,
             needed_libs, crt_before, crt_after, is_static):

  1. ARGUMENT PARSING
     Parse user_args for -L (extra library paths), -l (libraries),
     -Wl,--defsym, -Wl,--export-dynamic, -rdynamic, etc.

  2. FILE LOADING
     a. Load CRT objects (before): pre-resolved by common.rs
     b. Load user object files from object_files[]
     c. Load objects/archives/libraries from user_args (-l flags)
     d. Load CRT objects (after): pre-resolved by common.rs
     e. Group-load default libraries from needed_libs[]
        (iterate until no new symbols resolved -- handles circular deps)
     f. For dynamic linking: resolve remaining undefs against
        system .so files (libc.so.6, libm.so.6, libgcc_s.so.1)

  3. SYMBOL RESOLUTION (linker_common::register_symbols_elf64, per object)
     - Skip FILE, SECTION, and local symbols
     - Defined symbols: insert or replace if existing is
       undefined, dynamic, or weak-vs-global
     - COMMON symbols: insert if not already defined
     - Undefined symbols: insert placeholder if not present

  4. DEFSYM APPLICATION
     Apply --defsym=ALIAS=TARGET definitions (symbol aliasing).

  5. GARBAGE COLLECTION (if --gc-sections)
     BFS reachability from entry points (_start, main, __libc_csu_init,
     __libc_csu_fini) and init/fini arrays; unreachable sections are
     excluded from the link.

  6. UNRESOLVED SYMBOL CHECK
     Error on undefined non-weak symbols, excluding linker-defined
     names recognized by linker_common::is_linker_defined_symbol().

  7. SECTION MERGING (linker_common::merge_sections_elf64)
     Delegates to shared implementation that:
     a. Maps input section names to output names via map_section_name()
     b. For each allocatable input section, appends to matching output section
     c. Calculates output offsets within each merged section
     d. Sorts output sections: RO -> Exec -> RW(progbits) -> RW(nobits)
     e. Builds section_map: (obj_idx, sec_idx) -> (out_idx, offset)

  8. COMMON SYMBOL ALLOCATION (linker_common::allocate_common_symbols_elf64)
     Allocate SHN_COMMON symbols into .bss with proper alignment.

  9. EMIT
     If dynamic symbols present and !is_static:
       create_plt_got() then emit_dynamic_executable()
     Otherwise:
       emit_executable() (static linking)
```

### Memory Layout (Static Executable)

The static linker produces a two-segment layout. Note: `emit_executable()`
places executable sections first, then read-only data, regardless of the
earlier section sort order.

```text
  Virtual Address Space
  ====================================================================

  0x400000 +========================+  ----+
           |  ELF Header (64 B)     |      |
           |  Program Headers       |      |
           +------------------------+      |
           |  .text                  |      |  LOAD segment 1
           |  (executable code)      |      |  RX (Read + Execute)
           +------------------------+      |
           |  .rodata               |      |
           |  (read-only data)       |      |
           +------------------------+      |
           |  .gcc_except_table     |      |
           |  .eh_frame             |      |
           +------------------------+      |
           |  .eh_frame_hdr         |      |
           +------------------------+      |
           |  [IPLT stubs]          |      |  (in RX padding gap)
           +========================+  ----+
           |  (page alignment gap)  |  <- 64 KB aligned
           +========================+  ----+
           |  .tdata                |      |
           |  (TLS initialized)     |      |
           +------------------------+      |
           |  .init_array           |      |
           |  .fini_array           |      |
           +------------------------+      |  LOAD segment 2
           |  .data.rel.ro          |      |  RW (Read + Write)
           |  .data                 |      |
           +------------------------+      |
           |  .got                  |      |  (built by linker)
           +------------------------+      |
           |  [IPLT GOT slots]     |      |
           |  [.rela.iplt entries]  |      |
           +========================+  ----+
           |  .bss                  |  (no file space, only memsize)
           |  .tbss                 |
           +========================+

  Program Headers (up to 5):
    LOAD  RX: file offset 0, vaddr BASE_ADDR, filesz=rx_filesz
    LOAD  RW: file offset rw_page_offset, vaddr=rw_page_addr
    TLS:  .tdata + .tbss (if present)
    GNU_STACK: RW, no exec
    GNU_EH_FRAME: .eh_frame_hdr (if .eh_frame present)
```

### GOT (Global Offset Table) Construction

The linker builds a GOT for two purposes:

1. **Regular GOT entries** (`R_AARCH64_ADR_GOT_PAGE` / `R_AARCH64_LD64_GOT_LO12_NC`):
   8-byte slots containing the absolute address of the target symbol.

2. **TLS IE GOT entries** (`R_AARCH64_TLSIE_ADR_GOTTPREL_PAGE21` /
   `R_AARCH64_TLSIE_LD64_GOTTPREL_LO12_NC`): 8-byte slots containing the
   TP-relative offset of the TLS variable (computed as
   `sym_addr - tls_base + 16` per AArch64 Variant 1 TLS).

The `collect_got_symbols()` function in `reloc.rs` scans all relocations
to determine which symbols need GOT entries, and what kind (`Regular` or
`TlsIE`). GOT entries are allocated in the RW segment, 8-byte aligned.

### IFUNC / IPLT Support

The linker handles `STT_GNU_IFUNC` symbols (indirect functions whose runtime
address is determined by a resolver function):

1. **Identify IFUNC symbols** in the global symbol table.
2. **Allocate IPLT GOT slots** (one 8-byte slot per IFUNC) in the RW segment.
3. **Generate `.rela.iplt` entries** with `R_AARCH64_IRELATIVE` relocations
   pointing to the resolver function.
4. **Generate IPLT PLT stubs** in the RX gap between text and data segments.
   Each stub is 16 bytes:

   ```text
   ADRP  x16, page_of(got_slot)
   LDR   x17, [x16, #lo12(got_slot)]
   BR    x17
   NOP
   ```

5. **Redirect IFUNC symbol addresses** to point to the PLT stub instead of
   the resolver. The symbol type is changed from `STT_GNU_IFUNC` to
   `STT_FUNC`.

### Linker-Defined Symbols

The following symbols are automatically provided (via
`linker_common::get_standard_linker_symbols()`):

| Symbol | Value |
|--------|-------|
| `__dso_handle` | `BASE_ADDR` |
| `_DYNAMIC` | 0 (no dynamic section in static executables) |
| `_GLOBAL_OFFSET_TABLE_` | GOT base address |
| `__init_array_start` / `__init_array_end` | `.init_array` bounds |
| `__fini_array_start` / `__fini_array_end` | `.fini_array` bounds |
| `__preinit_array_start` / `__preinit_array_end` | Same as init_array start |
| `__ehdr_start` | `BASE_ADDR` |
| `__executable_start` | `BASE_ADDR` |
| `_etext` / `etext` | End of text (RX) segment |
| `__data_start` / `data_start` | Start of RW data segment |
| `_init` / `_fini` | Address of `.init` / `.fini` sections |
| `__rela_iplt_start` / `__rela_iplt_end` | IRELATIVE relocation table bounds |
| `__bss_start` / `_edata` | BSS start address |
| `_end` / `__end` | BSS end address |

---

## Stage 3: Relocation Application (`reloc.rs`)

### Purpose

After all sections have been laid out and symbol addresses are known, apply
every relocation from every input object to the output buffer. This module
also handles TLS model relaxation and GOT-indirect references.

### Key Data Structures

| Type | Role |
|------|------|
| `TlsInfo` | TLS segment base address and total size. |
| `GotInfo` | GOT base address and a map of symbol keys to entry indices. |
| `GotEntryKind` | Whether a GOT entry is `Regular` (absolute address) or `TlsIE` (TP offset). |

### Symbol Resolution (`resolve_sym`)

```text
resolve_sym(obj_idx, sym, globals, section_map, output_sections):
  if sym is STT_SECTION:
    return output_sections[mapped_section].addr + section_offset
  if sym is non-local and in globals and defined:
    return global value           // includes linker-defined symbols
  if sym is non-local and weak:
    return 0
  if sym is undefined:
    return 0
  if sym is SHN_ABS:
    return sym.value
  otherwise:
    return mapped section addr + section offset + sym.value
```

### Supported Relocation Types

The linker handles 40+ AArch64 relocation types, organized by category:

#### Absolute Relocations

| Type | ELF # | Formula | Usage |
|------|-------|---------|-------|
| `R_AARCH64_ABS64` | 257 | S + A | 64-bit data pointer |
| `R_AARCH64_ABS32` | 258 | S + A | 32-bit data pointer |
| `R_AARCH64_ABS16` | 259 | S + A | 16-bit data value |

#### PC-Relative Relocations

| Type | ELF # | Formula | Usage |
|------|-------|---------|-------|
| `R_AARCH64_PREL64` | 260 | S + A - P | 64-bit PC-relative |
| `R_AARCH64_PREL32` | 261 | S + A - P | 32-bit PC-relative (jump tables) |
| `R_AARCH64_PREL16` | 262 | S + A - P | 16-bit PC-relative |

#### Page-Relative and Immediate Relocations

| Type | ELF # | Formula | Usage |
|------|-------|---------|-------|
| `R_AARCH64_ADR_PREL_PG_HI21` | 275 | Page(S+A) - Page(P) | ADRP instruction |
| `R_AARCH64_ADR_PREL_LO21` | 274 | S + A - P | ADR instruction |
| `R_AARCH64_ADD_ABS_LO12_NC` | 277 | (S+A) & 0xFFF | ADD :lo12: |
| `R_AARCH64_LDST8_ABS_LO12_NC` | 278 | (S+A) & 0xFFF | Byte load/store |
| `R_AARCH64_LDST16_ABS_LO12_NC` | 284 | (S+A) & 0xFFF >> 1 | Halfword load/store |
| `R_AARCH64_LDST32_ABS_LO12_NC` | 285 | (S+A) & 0xFFF >> 2 | Word load/store |
| `R_AARCH64_LDST64_ABS_LO12_NC` | 286 | (S+A) & 0xFFF >> 3 | Doubleword load/store |
| `R_AARCH64_LDST128_ABS_LO12_NC` | 299 | (S+A) & 0xFFF >> 4 | Quadword load/store |

#### Branch Relocations

| Type | ELF # | Formula | Usage |
|------|-------|---------|-------|
| `R_AARCH64_CALL26` | 283 | (S+A-P) >> 2 | BL instruction (26-bit) |
| `R_AARCH64_JUMP26` | 282 | (S+A-P) >> 2 | B instruction (26-bit) |
| `R_AARCH64_CONDBR19` | 280 | (S+A-P) >> 2 | Conditional branch (19-bit) |
| `R_AARCH64_TSTBR14` | 279 | (S+A-P) >> 2 | Test-and-branch (14-bit) |

Special: when a `CALL26`/`JUMP26` target resolves to address 0 (undefined
weak symbol), the instruction is replaced with `NOP` (0xd503201f).

#### MOVW Relocations

| Type | ELF # | Formula | Usage |
|------|-------|---------|-------|
| `R_AARCH64_MOVW_UABS_G0[_NC]` | 263/264 | (S+A) & 0xFFFF | MOVZ/MOVK bits [15:0] |
| `R_AARCH64_MOVW_UABS_G1_NC` | 265 | (S+A) >> 16 & 0xFFFF | MOVK bits [31:16] |
| `R_AARCH64_MOVW_UABS_G2_NC` | 266 | (S+A) >> 32 & 0xFFFF | MOVK bits [47:32] |
| `R_AARCH64_MOVW_UABS_G3` | 267 | (S+A) >> 48 & 0xFFFF | MOVK bits [63:48] |

#### GOT Relocations

| Type | ELF # | Description |
|------|-------|-------------|
| `R_AARCH64_ADR_GOT_PAGE` | 311 | ADRP to page containing GOT entry |
| `R_AARCH64_LD64_GOT_LO12_NC` | 312 | LDR from GOT entry (low 12 bits) |

In static linking, the GOT is a real data structure in the RW segment
populated at link time (not lazily at runtime).

#### TLS Local Exec (LE) Relocations

Used when the TLS variable is in the executable itself (most common in
static linking). The TP (Thread Pointer) offset is computed as:

```text
tp_offset = (sym_addr - tls_start_addr) + 16    // AArch64 Variant 1
```

| Type | ELF # | Description |
|------|-------|-------------|
| `R_AARCH64_TLSLE_ADD_TPREL_HI12` | 549 | ADD, high 12 bits of TP offset |
| `R_AARCH64_TLSLE_ADD_TPREL_LO12[_NC]` | 550/551 | ADD, low 12 bits of TP offset |
| `R_AARCH64_TLSLE_MOVW_TPREL_G0[_NC]` | 544/545 | MOVZ/MOVK, bits [15:0] |
| `R_AARCH64_TLSLE_MOVW_TPREL_G1[_NC]` | 546/547 | MOVK, bits [31:16] |
| `R_AARCH64_TLSLE_MOVW_TPREL_G2` | 548 | MOVK, bits [47:32] |

#### TLS Initial Exec (IE) via GOT

Instead of relaxing ADRP+LDR to MOVZ+MOVK (which can break if different
registers are used), the linker uses real GOT entries pre-populated with
TP offsets:

| Type | ELF # | Description |
|------|-------|-------------|
| `R_AARCH64_TLSIE_ADR_GOTTPREL_PAGE21` | 541 | ADRP to GOT page holding TP offset |
| `R_AARCH64_TLSIE_LD64_GOTTPREL_LO12_NC` | 542 | LDR from GOT entry |

#### TLS Descriptor (TLSDESC) Relaxation to LE

For static linking, TLSDESC sequences are relaxed to direct TP-offset
computation:

| Type | ELF # | Relaxation |
|------|-------|------------|
| `R_AARCH64_TLSDESC_ADR_PAGE21` | 562 | ADRP -> MOVZ Xd, #tprel_g1, LSL #16 |
| `R_AARCH64_TLSDESC_LD64_LO12` | 563 | LDR -> MOVK Xd, #tprel_lo |
| `R_AARCH64_TLSDESC_ADD_LO12` | 564 | ADD -> NOP |
| `R_AARCH64_TLSDESC_CALL` | 569 | BLR -> NOP |

#### TLS General Dynamic (GD) Relaxation to LE

| Type | ELF # | Relaxation |
|------|-------|------------|
| `R_AARCH64_TLSGD_ADR_PAGE21` | 513 | ADRP -> MOVZ Xd, #tprel_g1, LSL #16 |
| `R_AARCH64_TLSGD_ADD_LO12_NC` | 514 | ADD -> MOVK Xd, #tprel_lo |

### Instruction Patching Helpers

The relocation module includes helpers that patch individual instruction
fields without disturbing other bits:

| Helper | Field Modified |
|--------|---------------|
| `encode_adrp()` | immhi[23:5] and immlo[30:29] of ADRP |
| `encode_adr()` | immhi[23:5] and immlo[30:29] of ADR |
| `encode_add_imm12()` | imm12[21:10] of ADD immediate |
| `encode_ldst_imm12()` | imm12[21:10] of LDR/STR, scaled by access size |
| `encode_movw()` | imm16[20:5] of MOVZ/MOVK |

---

## Archive and Library Handling

### File Loading Dispatch (`load_file`)

The linker dispatches file loading based on format detection:

1. **Archives** (`!<arch>\n` magic): parse members, selectively extract
2. **Thin archives** (`!<thin>\n` magic): members are external files
3. **Linker scripts** (non-ELF text): parse `GROUP`/`INPUT` directives,
   recursively load referenced files and `-l` libraries
4. **Shared libraries** (`ET_DYN`): load dynamic symbols (skipped if static)
5. **Relocatable objects** (`ET_REL`): parse and register symbols

### Archive Loading Strategy

Archives use **selective extraction with iterative resolution**, matching
the behavior of traditional `ld --start-group`. Archive and thin archive
loading is delegated to `linker_common::load_archive_elf64()` and
`linker_common::load_thin_archive_elf64()`, which implement the shared
algorithm: parse members, filter by `e_machine`, iterate until stable
extracting members that resolve currently-undefined symbols.

### Default Library Group Loading

The caller (common.rs) provides `needed_libs` (e.g., `["gcc", "gcc_eh", "c"]`).
The linker resolves these to archive paths and loads them in a group-loading
loop. This handles circular dependencies between these libraries:

```text
repeat:
  prev_count = objects.len()
  for each resolved library archive:
    load_file(archive)  // only extracts members that resolve undefs
  if objects.len() == prev_count:
    break   // stable -- no new members pulled in
```

### Library Resolution (`resolve_lib`)

Libraries specified with `-l` are searched via `linker_common::resolve_lib()`
across all library paths (user `-L` paths first, then system paths provided
by common.rs). In static mode, `.a` is preferred; in dynamic mode, `.so` is
preferred. The special `-l:filename` syntax searches for an exact filename.

---

## Design Decisions and Trade-offs

### 1. Static and Dynamic Linking

The linker supports both static executables (`ET_EXEC`, the default with
`-static`) and dynamically-linked executables with PLT/GOT, `.dynamic`
section, `DT_*` tags, `.interp`, `.gnu.hash`, and copy relocations. It
also produces shared libraries (`ET_DYN`) with `R_AARCH64_RELATIVE`,
`R_AARCH64_JUMP_SLOT`, and `R_AARCH64_GLOB_DAT` relocations, enabling
full `dlopen()` support (e.g., PostgreSQL extension modules).

### 2. Two-Segment Layout

The output uses exactly two `PT_LOAD` segments (RX and RW) plus optional
TLS, GNU_STACK, and GNU_EH_FRAME segments. This is the minimal viable
layout. The 64 KB page alignment (`PAGE_SIZE = 0x10000`) accommodates
AArch64 systems with either 4 KB or 64 KB page sizes.

### 3. Real GOT for All GOT-Based Relocations

Rather than relaxing `ADRP+LDR` GOT sequences to `ADRP+ADD` (which would
save memory but requires verifying instruction sequences), the linker
maintains a real GOT in the RW segment. GOT entries are populated at link
time with final addresses. This is conservative but correct -- the `LDR`
instruction genuinely loads from memory, and converting it to `ADD` would
require instruction replacement.

### 4. TLS IE via GOT (Not MOVZ/MOVK Relaxation)

TLS Initial Exec relocations use real GOT entries containing pre-computed
TP offsets, rather than relaxing to `MOVZ+MOVK` instruction sequences. The
relaxation approach was found to be fragile because the ADRP and LDR
instructions may use different registers, and the relaxed MOVZ+MOVK must
target the same register as the original LDR destination.

### 5. TLSDESC and TLSGD Relaxation to LE

For static linking, both TLSDESC and General Dynamic TLS access patterns are
relaxed to Local Exec. The TLSDESC 4-instruction sequence
(ADRP + LDR + ADD + BLR) is replaced with (MOVZ + MOVK + NOP + NOP).
This is correct because in a static executable, all TLS variables are in the
executable's own TLS block.

### 6. IFUNC Handling via IPLT

GNU IFUNC symbols (where the symbol resolves to a "resolver" function that
returns the actual implementation address at runtime) are handled by
generating IPLT stubs and IRELATIVE relocations. The glibc startup code
processes these relocations to fill the GOT slots with the actual function
addresses returned by the resolvers.

### 7. No Section Headers in Output

The output executable contains no section header table (`e_shnum = 0`).
This is valid per the ELF specification (section headers are optional for
executables) and reduces output size. Tools like `objdump -d` still work
by following program headers.

### 8. Diagnostic Support

Setting `LINKER_DEBUG=1` enables verbose tracing of object loading, symbol
resolution, section layout, GOT allocation, and final addresses.
`LINKER_DEBUG_LAYOUT=1` adds section-by-section layout details, and
`LINKER_DEBUG_TLS=1` traces TLS relocation processing.

---

## File Inventory

| File | Lines | Purpose |
|------|-------|---------|
| `mod.rs` | ~40 | Module declarations and public re-exports (`link_builtin`, `link_shared`) |
| `types.rs` | ~93 | `GlobalSymbol` struct with `GlobalSymbolOps` impl, arch constants (`BASE_ADDR`, `PAGE_SIZE`, `INTERP`), `arm_should_replace_extra` |
| `elf.rs` | ~75 | AArch64 relocation constants (26 types); type aliases delegating to `linker_common` for ELF64 parsing |
| `input.rs` | ~91 | File loading dispatch: `load_file`, `resolve_lib`, `resolve_lib_prefer_shared` |
| `plt_got.rs` | ~130 | PLT/GOT entry list construction from relocation scanning |
| `link.rs` | ~411 | Orchestration: `link_builtin` and `link_shared` entry points, dynamic symbol resolution, library group loading |
| `emit_dynamic.rs` | ~869 | Dynamic executable emission: PLT/GOT/.dynamic section, address layout, copy relocations |
| `emit_shared.rs` | ~1,098 | Shared library (`.so`) emission: PIC layout, `R_AARCH64_RELATIVE`/`JUMP_SLOT`/`GLOB_DAT`, RELRO |
| `emit_static.rs` | ~645 | Static executable emission: IPLT/IRELATIVE, two-segment layout |
| `reloc.rs` | ~540 | Relocation application (40+ types), TLS relaxation, GOT/TLS-IE references, instruction field patching helpers |
| **Total** | **~4,000** | (plus shared infrastructure in `linker_common`) |
