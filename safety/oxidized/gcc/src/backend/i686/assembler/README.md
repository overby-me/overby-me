# i686 Built-in Assembler -- Design Document

## Overview

The i686 built-in assembler translates AT&T-syntax assembly text into 32-bit ELF
relocatable object files (`.o`). It replaces the external GNU assembler (`as`)
when the compiler is configured to use its own toolchain, giving the compiler a
self-contained build path for the `i686-linux-gnu` target.

The assembler is structured as a three-stage pipeline:

```text
  AT&T assembly text
        |
        v
  +------------------+
  |   Parser          |  Reused from the x86-64 backend
  |   (x86/parser.rs) |  Produces Vec<AsmItem>
  +------------------+
        |
        v
  +------------------+
  |   Encoder         |  i686-specific: no REX, 32-bit default operand size
  |   encoder/        |  Produces machine-code bytes + Relocation entries
  +------------------+
        |
        v
  +---------------------+
  |   ELF Writer         |  elf_writer.rs (i686 adapter) +
  |   + ElfWriterCore    |  elf_writer_common.rs (shared logic)
  |                      |  Produces ELFCLASS32 / EM_386 / Elf32_Rel .o files
  +---------------------+
        |
        v
   .o file on disk
```

The entry point is the `assemble()` function in `mod.rs`, which wires the three
stages together: parse, build, write.

## Relationship to the x86-64 Assembler

The i686 and x86-64 backends share two major components:

1. **AT&T syntax parser** (`crate::backend::x86::assembler::parser`). The parsed
   representation (`AsmItem`, `Instruction`, `Operand`, etc.) is
   architecture-neutral -- the parser does not make assumptions about register
   width or operand-size defaults.

2. **ELF writer core** (`crate::backend::elf_writer_common::ElfWriterCore`).
   Section management, symbol tables, jump relaxation, numeric label resolution,
   and internal relocation resolution are all generic over an `X86Arch` trait.
   The i686 adapter (`elf_writer.rs`) plugs in i686-specific constants and
   the instruction encoder; the shared core handles everything else.

Everything else is i686-specific:

| Concern                 | x86-64                        | i686                          |
|-------------------------|-------------------------------|-------------------------------|
| Default operand size    | 64-bit (for GP instrs)        | 32-bit                        |
| REX prefix              | Required for r8-r15, 64-bit   | Not used                      |
| Register file           | 16 GP + 16 XMM               | 8 GP + 8 XMM                 |
| Addressing modes        | RIP-relative (`%rip`)         | Absolute displacement only    |
| ELF class               | ELFCLASS64, Elf64_Sym (24 B)  | ELFCLASS32, Elf32_Sym (16 B)  |
| Relocation format       | RELA (Elf64_Rela, 24 B)       | REL (Elf32_Rel, 8 B)          |
| Relocation types        | R_X86_64_*                    | R_386_*                       |
| ELF machine             | EM_X86_64 (62)                | EM_386 (3)                    |
| `inc`/`dec` encoding    | ModR/M form (0xFF /0, /1)     | Compact form (0x40+r, 0x48+r) |
| Mnemonic `q` suffix     | 64-bit operations             | Mapped to 32-bit gracefully   |

## Key Data Structures

### Parser types (from `x86::assembler::parser`)

| Type                | Role                                                    |
|---------------------|---------------------------------------------------------|
| `AsmItem`           | One parsed assembly line: directive, label, instruction |
| `Instruction`       | Mnemonic + optional prefix + operand list               |
| `Operand`           | Register, Immediate, Memory, Label, or Indirect         |
| `MemoryOperand`     | `disp(%base, %index, scale)` with optional segment      |
| `Displacement`      | Integer, symbol, symbol+addend, or `sym@MODIFIER`       |
| `SectionDirective`  | `.section name,"flags",@type`                           |
| `DataValue`         | Integer, symbol, `sym+offset`, or `sym1-sym2`           |
| `SizeExpr`          | Constant, `.-sym`, or `end-start` for `.size` directive |
| `SymbolKind`        | Function, Object, TlsObject, NoType                    |

### Encoder types (`encoder/`)

| Type                   | Role                                                       |
|------------------------|------------------------------------------------------------|
| `InstructionEncoder`   | Stateful encoder; accumulates bytes and relocations         |
| `Relocation`           | Offset + symbol + R_386 type + addend + optional diff_symbol|

The encoder's `bytes: Vec<u8>` collects the raw machine code for one
instruction. The `offset: u64` field tracks the current position within the
section so that relocation offsets are computed correctly.

### ELF writer types

The i686 `elf_writer.rs` is a thin adapter (see `I686Arch`) that plugs into the
shared `ElfWriterCore<A: X86Arch>` from `elf_writer_common.rs`. The shared
core defines the types that drive ELF emission:

| Type (in `elf_writer_common`) | Role                                                |
|-------------------------------|-----------------------------------------------------|
| `ElfWriterCore<A>`            | Top-level builder: sections, symbols, label positions|
| `Section`                     | In-progress section: name, type, flags, data, relocs|
| `ElfRelocation`               | Section-local relocation (offset, symbol, type, addend)|
| `SymbolInfo`                  | Binding, type, visibility, section, value, size      |
| `JumpInfo`                    | Tracks a jump for short-form relaxation              |

The i686 adapter (`elf_writer.rs`) defines:

| Type                | Role                                                         |
|---------------------|--------------------------------------------------------------|
| `I686Arch`          | Implements `X86Arch`: encoder dispatch, ELF constants, REL format|
| `ElfWriter`         | Type alias for `ElfWriterCore<I686Arch>`                     |

String tables (`StringTable`) live in `backend::elf` and are used during final
serialization.

## Processing Algorithm

### Stage 1: Parsing

`parse_asm(text)` iterates over lines and produces a `Vec<AsmItem>`. Each line
is classified as one of:

1. **Empty / comment** -- stripped away.
2. **Label** -- `name:` at the start of a line.
3. **Directive** -- lines starting with `.` (`.section`, `.globl`, `.align`,
   `.byte`, `.long`, `.asciz`, `.type`, `.size`, `.comm`, `.set`, CFI, etc.).
4. **Prefixed instruction** -- `lock`, `rep`, `repz`, `repnz` followed by a
   mnemonic.
5. **Instruction** -- mnemonic + comma-separated operands in AT&T order
   (source, destination).

Lines containing `;` are split into multiple items (GAS multi-statement syntax).
Comments starting with `#` are stripped. String literals are respected during
both comment stripping and semicolon splitting.

### Numeric Label Resolution (pre-pass)

Before encoding begins, the ELF writer core runs a numeric label resolution
pre-pass (`resolve_numeric_labels`). GNU assembler numeric labels (`1:`, `2:`,
etc.) can be defined multiple times; forward references (`1f`) refer to the next
definition, and backward references (`1b`) refer to the most recent.

The pre-pass renames each numeric label definition to a unique internal name
(`.Lnum_N_K`) and updates all instruction operands and data directives (`Byte`,
`Long`, `Quad`, `SkipExpr`) that reference them. This converts inherently
ambiguous references into unique `.L`-prefixed labels that the rest of the
pipeline handles normally.

As a defense-in-depth measure, the ELF writer also tracks numeric label
positions at runtime for fallback resolution during jump relaxation and
relocation processing.

### Stage 2: Instruction Encoding

The `InstructionEncoder` converts each `Instruction` into machine-code bytes.
The main dispatch function `encode_mnemonic()` is a large `match` that covers:

- **Data movement**: `mov`, `movsx`/`movzx`, `lea`, `push`, `pop`, `xchg`
- **ALU**: `add`, `sub`, `and`, `or`, `xor`, `cmp`, `test` (8 ALU group ops)
- **Multiply/divide**: `imul` (1/2/3 operand), `mul`, `div`, `idiv`
- **Unary**: `neg`, `not`, `inc`, `dec`
- **Shifts**: `shl`/`shr`/`sar`/`rol`/`ror`/`rcl`/`rcr`, `shld`/`shrd`
- **Bit operations**: `bt`/`bts`/`btr`/`btc`, `bsf`/`bsr`, `lzcnt`/`tzcnt`/`popcnt`
- **Sign extension**: `cdq`, `cwde`, `cbw`, `cwd`
- **Conditional**: `setcc`, `cmovcc`
- **Control flow**: `jmp`, `jcc`, `jecxz`, `loop`, `call`, `ret`
- **Atomics**: `cmpxchg`, `xadd`, `cmpxchg8b`
- **String ops**: `movsb`/`movsl`, `stosb`/`stosl`, `cmpsb`/`cmpsl`, etc.
- **I/O**: `inb`/`outb`, `insb`/`outsb`, etc.
- **SSE/SSE2**: Scalar and packed float/integer ops, shuffles, conversions,
  comparisons, non-temporal stores
- **SSE3/SSSE3/SSE4**: Horizontal ops, blends, rounds, dot products, `ptest`
- **AES-NI**: `aesenc`, `aesdec`, `aeskeygenassist`, `pclmulqdq`
- **x87 FPU**: Load/store, arithmetic, transcendentals, control word, `fcomip`
- **System**: `int`, `cpuid`, `rdtsc`, `syscall`, `sysenter`, `hlt`, `mfence`,
  `rdmsr`/`wrmsr`, `bswap`, `ud2`, `endbr32`
- **Prefixes**: `lock`, `rep`/`repe`/`repnz` (both as prefixes and standalone)

#### ModR/M and SIB encoding

`encode_modrm_mem()` is the central memory-operand encoder. It handles:

- **No base, no index**: `mod=00, rm=5` with disp32 (absolute addressing).
- **Base only**: Direct ModR/M when base is not ESP/EBP-special.
- **Base + index + scale**: SIB byte. ESP (reg 4) as base always triggers SIB.
- **Symbol displacements**: Always use `mod=10` (disp32) and emit a relocation
  pointing at the displacement bytes.

Displacement sizes are chosen automatically:

```text
  disp == 0 && base != EBP  ->  mod=00  (no displacement bytes)
  -128 <= disp <= 127        ->  mod=01  (disp8)
  otherwise                  ->  mod=10  (disp32)
  symbol reference           ->  mod=10  (disp32 + relocation)
```

Relocations are deferred until after the ModR/M and SIB bytes are emitted so
that the relocation offset points to the displacement field, not the ModR/M
byte. This is essential for the REL format where the addend is embedded inline.

#### Key i686-specific encoding decisions

- **No REX prefix**. The register file is limited to 8 GP registers (0-7) and
  8 XMM registers (0-7). The 3-bit `reg_num()` function maps register names
  directly without any extension bit.

- **32-bit default operand size**. Unsuffixed instructions default to 32-bit.
  The `l` suffix is standard; `q` suffix is mapped to 32-bit with a graceful
  fallback.

- **Compact `inc`/`dec`**. Unlike x86-64 (where 0x40-0x4F are REX prefixes),
  i686 uses the single-byte `0x40+r` (inc) and `0x48+r` (dec) encodings for
  32-bit registers.

- **Absolute addressing for calls/jumps**. All branch instructions (`call`,
  `jmp`, `jcc`) emit `R_386_PLT32` relocations for label targets, matching
  modern GCC/binutils behavior. The `@PLT` suffix is stripped from symbol names
  but does not affect the relocation type (always PLT32).

- **Operand-size prefix** (`0x66`) is emitted for 16-bit operations. Segment
  override prefixes (`0x64` for `%fs`, `0x65` for `%gs`) are emitted for
  segment-prefixed memory operands.

#### TLS and GOT relocation mapping

The encoder maps AT&T `@MODIFIER` syntax to i386 relocation types:

| AT&T modifier    | Relocation constant  | Usage                              |
|------------------|----------------------|------------------------------------|
| `@NTPOFF`        | `R_386_TLS_LE_32`    | Negative TP offset (Local Exec)    |
| `@TPOFF`         | `R_386_32S`          | TP offset (Local Exec)             |
| `@TLSGD`         | `R_386_TLS_GD`       | General Dynamic TLS                |
| `@TLSLDM`        | `R_386_TLS_LDM`      | Local Dynamic TLS                  |
| `@DTPOFF`        | `R_386_TLS_LDO_32`   | DTP-relative offset                |
| `@GOT`           | `R_386_GOT32`        | GOT entry                          |
| `@GOTOFF`        | `R_386_GOTOFF`       | Offset from GOT base               |
| `@PLT`           | `R_386_PLT32`        | PLT-relative call                  |
| `@GOTPC`         | `R_386_GOTPC`        | PC-relative to GOT base            |
| `@GOTNTPOFF`     | `R_386_TLS_IE`       | IE model via GOT                   |
| `@INDNTPOFF`     | `R_386_TLS_IE`       | IE model via GOT (alias)           |

### Stage 3: ELF Object File Emission

The `ElfWriterCore` (parameterized with `I686Arch`) processes all `AsmItem`s in
order, building up sections, symbols, and relocations, then serializes them into
an ELF32 relocatable object.

#### Item processing

```text
  for each AsmItem:
    Section(dir)       -> switch to / create section
    Global(name)       -> mark symbol as STB_GLOBAL (pending)
    Weak(name)         -> mark symbol as STB_WEAK (pending)
    Hidden(name)       -> mark symbol visibility STV_HIDDEN (pending)
    Label(name)        -> record label position; create/update SymbolInfo
    Align(n)           -> pad with NOP (text) or 0x00 (data) to alignment
    Byte/Short/Long/   -> append data bytes; emit R_386_32 relocs for symbols
      Quad/Zero/Asciz
    Comm(n,s,a)        -> create COMMON symbol (SHN_COMMON)
    Set(alias,target)  -> record symbol alias
    Instruction(instr) -> encode via InstructionEncoder; copy bytes & relocs
    SymbolType/Size    -> deferred; applied after encoding
    Cfi/File/Loc/Empty -> ignored (debug info not emitted by built-in assembler)
```

#### Jump relaxation

After all items are processed, the ELF writer runs a **jump relaxation pass**.
Jumps are initially encoded in their long form:

- Unconditional `jmp`: `E9 rel32` (5 bytes)
- Conditional `jcc`: `0F 8x rel32` (6 bytes)

The relaxation algorithm iterates until convergence:

```text
  loop:
    for each jump in section:
      if jump is not yet relaxed AND target is in same section:
        compute displacement assuming short encoding (2 bytes)
        if displacement fits in [-128, 127]:
          mark jump for relaxation
    if no new relaxations: break
    for each newly relaxed jump (processed back-to-front):
      rewrite opcode to short form:
        jmp  -> EB disp8  (2 bytes)
        jcc  -> 7x disp8  (2 bytes)
      remove excess bytes from section data
      adjust all label positions after this jump
      adjust all relocation offsets after this jump
      remove the now-unnecessary PC32 relocation for this jump
      adjust offsets of other tracked jumps
```

After relaxation, short jump displacements are patched with the final
`disp8` values.

#### Internal relocation resolution

Same-section, PC-relative relocations to **local** symbols (STB_LOCAL or `.L*`
labels) are resolved inline before serialization. The resolved value is written
directly into the section data, and the relocation entry is removed. This
avoids emitting relocations that the linker would just resolve to the same
object anyway.

Global and weak symbols always keep their relocations so the linker can handle
symbol interposition and PLT redirection.

#### Symbol table construction

The ELF symbol table is built in the standard order:

1. **Null symbol** (index 0)
2. **Section symbols** (STT_SECTION, STB_LOCAL) -- one per section
3. **Local defined symbols** (STB_LOCAL, excluding `.L*` labels)
4. **Global and weak symbols** -- the `sh_info` field of `.symtab` records the
   index of the first global symbol
5. **Alias symbols** (from `.set` directives) -- cloned from their targets
6. **Undefined external symbols** -- created on demand when a relocation
   references a symbol not defined in this object

Size expressions (`.-symbol`, `end-start`) are resolved after jump relaxation
so that function sizes account for any shortened jumps.

#### ELF32 file layout

```text
  +-----------------------------+  offset 0
  | ELF32 Header (52 bytes)     |  e_ident[EI_CLASS] = ELFCLASS32
  |   e_machine = EM_386        |  e_ident[EI_DATA]  = ELFDATA2LSB
  |   e_type    = ET_REL        |
  +-----------------------------+
  | Section data                |  .text, .data, .rodata, .bss, ...
  |   (each aligned per         |  (SHT_NOBITS sections occupy no space)
  |    section requirements)    |
  +-----------------------------+
  | .symtab                     |  Elf32_Sym entries (16 bytes each)
  |   (4-byte aligned)          |
  +-----------------------------+
  | .strtab                     |  NUL-terminated symbol name strings
  +-----------------------------+
  | .shstrtab                   |  NUL-terminated section name strings
  +-----------------------------+
  | .rel.text, .rel.data, ...   |  Elf32_Rel entries (8 bytes each)
  |   (4-byte aligned)          |  r_info = (sym << 8) | type
  +-----------------------------+
  | Section Header Table        |  Elf32_Shdr entries (40 bytes each)
  |   (4-byte aligned)          |  Null + data sections + symtab +
  |                             |  strtab + shstrtab + rel sections
  +-----------------------------+
```

The critical difference from the x86-64 ELF writer: this uses **Elf32_Rel**
(8 bytes: `r_offset` + `r_info`) rather than **Elf64_Rela** (24 bytes:
`r_offset` + `r_info` + `r_addend`). In the REL format, the addend is
embedded in the instruction bytes at the relocation site. The ELF writer
patches these implicit addends into the section data during serialization.

The `r_info` field is packed as `(symbol_index << 8) | reloc_type` (32-bit
format), compared to x86-64's `(symbol_index << 32) | reloc_type` (64-bit
format).

## Key Design Decisions and Trade-offs

1. **Parser reuse**. Sharing the AT&T parser between i686 and x86-64 eliminates
   duplicated parsing logic. The parser is architecture-neutral by design:
   register names, mnemonic suffixes, and memory operand syntax are identical in
   AT&T notation. The cost is that the parser accepts some x86-64-only
   constructs (like `%rax`) that the encoder will reject.

2. **REL vs. RELA**. i386 ELF conventionally uses REL relocations. This
   requires the assembler to embed addends in the instruction stream, and the
   ELF writer to patch them during serialization. The x86-64 backend uses RELA
   (explicit addends in the relocation entry), which is simpler to implement.
   The REL approach here adds complexity but produces standard-conforming i386
   objects that work with any ELF linker.

3. **Shared ELF writer infrastructure**. The `ElfWriterCore` is generic over an
   `X86Arch` trait, so the i686 and x86-64 backends share all section/symbol
   management, jump relaxation, and ELF serialization logic. The i686 adapter
   (`elf_writer.rs`) only needs to provide architecture-specific constants and
   wire up the instruction encoder.

4. **Eager long encoding + relaxation**. Instructions are initially encoded in
   their longest form. A post-encoding relaxation pass shortens jumps that can
   reach their targets with 8-bit displacements. This avoids the complexity of
   multi-pass encoding (where shortening one jump might allow others to shorten)
   while still producing reasonably compact code.

5. **Inline resolution of local relocations**. Same-section PC-relative
   relocations to local symbols are resolved by the assembler, not deferred to
   the linker. This reduces the number of relocations in the output and avoids
   unnecessary linker work. Only global/weak symbols retain relocations.

6. **No `.eh_frame` / DWARF generation**. The assembler ignores CFI directives
   and debug metadata. This simplifies the implementation at the cost of no
   stack unwinding or debug info in the output. The linker can still link
   objects that contain `.eh_frame` from other sources (e.g., CRT objects).

7. **Compact `inc`/`dec` encoding**. The i686 backend uses the single-byte
   `0x40+r` / `0x48+r` forms for 32-bit `inc`/`dec`, which are unavailable on
   x86-64 (where those bytes are REX prefixes). This produces smaller code.

## File Inventory

| File                        | Lines  | Role                                            |
|-----------------------------|--------|-------------------------------------------------|
| `mod.rs`                    | ~30    | Module root; `assemble()` entry point           |
| `encoder/`                  | ~3520  | i686 instruction encoder (split into focused submodules, see below) |
| `elf_writer.rs`             | ~170   | `I686Arch` adapter for `ElfWriterCore`           |
| *(shared with x86-64)*      |        |                                                 |
| `x86/assembler/parser.rs`   | ~2180  | AT&T syntax parser; data types; directives       |
| `elf_writer_common.rs`      | ~1700  | Section/symbol/jump relax/ELF32 serialization    |

### Encoder Submodules (`encoder/`)

The instruction encoder is organized as a directory of focused submodules:

| File | Lines | Role |
|------|-------|------|
| `mod.rs` | ~770 | `InstructionEncoder` struct, `encode()` entry point, `encode_mnemonic()` dispatch match, `Relocation` type, relocation constants, `split_label_offset()` helper |
| `registers.rs` | ~138 | Register number mapping (`reg_num`), segment register mapping (`seg_reg_num`), XMM detection, suffix inference, x87 ST parsing, condition code parsing |
| `core.rs` | ~180 | Low-level encoding primitives: ModR/M + SIB byte construction, segment prefix emission, memory operand encoding (`encode_modrm_mem`), relocation helpers |
| `gp_integer.rs` | ~1390 | General-purpose integer instructions: MOV, ALU ops, shifts, IMUL, PUSH/POP, LEA, XCHG, CMPXCHG, conditional moves/sets, JMP/CALL/RET, string ops |
| `sse.rs` | ~385 | SSE/SSE2/SSE3/SSSE3/SSE4.1 and MMX instructions: scalar/packed float, integer SIMD, shuffles, conversions, AES-NI |
| `x87.rs` | ~279 | x87 FPU instructions: FLD/FSTP, arithmetic (FADD/FSUB/FMUL/FDIV), transcendentals, control word, FCOMIP |
| `system.rs` | ~379 | System and privileged instructions: INT, CPUID, RDTSC, SYSENTER, HLT, MFENCE, RDMSR/WRMSR, BSWAP, I/O, segment/control register moves |
