# x86-64 Built-in Assembler -- Design Document

## Overview

This module is a native x86-64 assembler that translates AT&T-syntax assembly
text into ELF relocatable object files (`.o`). It replaces the external
`gcc -c` invocation that would otherwise be needed to assemble
compiler-generated assembly, removing the hard dependency on an installed
toolchain at compile time.

The assembler handles the full subset of AT&T syntax that the compiler's code
generator emits, plus enough additional coverage to assemble hand-written
assembly from projects such as musl libc and the Linux kernel. It is not a
general-purpose GAS replacement -- it intentionally does not implement the full
GNU assembler specification -- but it covers a broad swath of the x86-64 ISA,
including SSE through SSE4.1, SSE3/SSSE3, AVX, AVX2, initial AVX-512 (EVEX),
BMI2, AES-NI, PCLMULQDQ, CRC32, and x87 FPU instructions.

### Entry Point

```rust
pub fn assemble(asm_text: &str, output_path: &str) -> Result<(), String>
```

Defined in `mod.rs`. Called when the built-in assembler is selected
(the default mode). The implementation is three lines:

1. `parse_asm(asm_text)` -- parse into `Vec<AsmItem>`
2. `ElfWriter::new().build(&items)` -- encode, relax, resolve, serialize
3. `std::fs::write(output_path, &elf_bytes)` -- write the object file

## Architecture / Pipeline

```text
                      Assembly Source Text (AT&T syntax)
                                   |
                                   v
                  +------------------------------------+
                  |           parser.rs                 |
                  |  1. strip_c_comments()              |
                  |  2. Split into lines                |
                  |  3. expand_rept_blocks()            |
                  |  4. expand_gas_macros()             |
                  |  5. For each line:                  |
                  |     - Strip # comments              |
                  |     - Split on semicolons           |
                  |     - Parse directives/instructions |
                  +------------------------------------+
                                   |
                            Vec<AsmItem>
                                   |
                                   v
                  +------------------------------------+
                  | ElfWriterCore<X86_64Arch>           |
                  |  (elf_writer_common.rs)             |
                  |  (First Pass -- process_item)       |
                  |  - Numeric label resolution         |
                  |  - Section management               |
                  |  - Symbol table construction        |
                  |  - Label position recording         |
                  |  - Data emission                    |
                  |  - Delegates to encoder.rs          |
                  |    via X86_64Arch trait impl        |
                  +------------------------------------+
                        |                   |
                        v                   v
          +--------------------+   +------------------------+
          |    encoder/        |   | ElfWriterCore (cont.)  |
          | - Suffix inference |   |  (Second Pass)         |
          | - REX/VEX/EVEX    |   |  1. relax_jumps()      |
          | - ModR/M + SIB    |   |  2. resolve_deferred   |
          | - Displacement     |   |     _skips()           |
          | - Immediate        |   |  3. resolve_deferred   |
          | - Relocation       |   |     _byte_diffs()      |
          |   generation       |   |  4. Update symbol vals |
          |                    |   |  5. Resolve .size dirs  |
          +--------------------+   |  6. resolve_internal   |
                                   |     _relocations()     |
                                   |  7. emit_elf() ->      |
                                   |     write_relocatable  |
                                   |     _object()          |
                                   +------------------------+
                                             |
                                     ELF .o bytes
                                             |
                                             v
                                   std::fs::write()
```

### Pass Summary

| Pass | Module | Purpose |
|------|--------|---------|
| 1a   | `parser.rs` | Tokenize and parse all lines into `Vec<AsmItem>` (including `.rept` expansion and GAS macro processing) |
| 1b   | `elf_writer_common.rs` + `encoder/` | Walk items sequentially: switch sections, record labels, encode instructions, emit data, collect relocations |
| 2a   | `elf_writer_common.rs` | Relax long jumps to short form (iterative until convergence) |
| 2b   | `elf_writer_common.rs` | Resolve deferred `.skip` expressions (insert bytes, shift labels/relocations) |
| 2c   | `elf_writer_common.rs` | Resolve deferred byte-sized symbol diffs |
| 2d   | `elf_writer_common.rs` | Update symbol values from `label_positions` (post-relaxation/skip) |
| 2e   | `elf_writer_common.rs` | Resolve `.size` directives |
| 2f   | `elf_writer_common.rs` | Resolve same-section PC-relative relocations for local symbols |
| 2g   | `elf_writer_common.rs` | Resolve `.set` aliases, create undefined/weak symbols, convert to shared format, serialize ELF via `write_relocatable_object()` |

## File Inventory

| File | Lines | Role |
|------|-------|------|
| `mod.rs` | ~30 | Public `assemble()` entry point; module wiring |
| `parser.rs` | ~2200 | AT&T syntax parser with `.rept` expansion, GAS macro processing, expression evaluation; produces `Vec<AsmItem>` |
| `encoder/` | ~6260 | x86-64 instruction encoder (split into focused submodules, see below) |
| `elf_writer.rs` | ~90 | Thin x86-64 adapter: implements `X86Arch` trait for `ElfWriterCore<X86_64Arch>`, wiring architecture constants and instruction encoding |

### Encoder Submodules (`encoder/`)

The instruction encoder is organized as a directory of focused submodules:

| File | Lines | Role |
|------|-------|------|
| `mod.rs` | ~1544 | `InstructionEncoder` struct, `encode()` entry point, `encode_mnemonic()` dispatch match (~800 arms), `Relocation` type, ELF relocation constants |
| `registers.rs` | ~296 | Register number mapping (`reg_num`), register classification (`is_reg64/32/16/8`), REX extension detection, suffix inference (`infer_suffix`), condition code parsing |
| `core.rs` | ~269 | Low-level encoding primitives: REX prefix emission (`rex`, `emit_rex_*`), ModR/M + SIB byte construction, memory operand encoding (`encode_modrm_mem`), relocation helpers |
| `gp_integer.rs` | ~1145 | General-purpose integer instructions: MOV variants, ALU ops (ADD/SUB/AND/OR/XOR/CMP/TEST), shifts, IMUL, PUSH/POP, LEA, XCHG, CMPXCHG, conditional moves/sets, JMP/CALL/RET |
| `sse.rs` | ~653 | SSE/SSE2/SSE3/SSSE3/SSE4.1 instructions: scalar and packed float ops, integer SIMD, shuffles, conversions, non-temporal stores, AES-NI, CRC32 |
| `avx.rs` | ~1456 | VEX/EVEX-encoded instructions: AVX, AVX2, initial AVX-512, BMI2; VEX prefix emission (`emit_vex`), EVEX prefix emission (`emit_evex`) |
| `system.rs` | ~513 | System and privileged instructions: I/O port ops (IN/OUT), control register moves, system table loads (LGDT/LIDT), CLFLUSH, WBINVD, RDMSR/WRMSR, segment register moves |
| `x87_misc.rs` | ~385 | x87 FPU instructions (FLD/FSTP/FADD/etc.), bit test operations (BT/BTS/BTR/BTC), segment register loads, miscellaneous suffixless encodings |

The bulk of the ELF building logic (section management, jump relaxation,
deferred evaluation, ELF serialization) lives in the shared
`backend::elf_writer_common` module (~1700 lines), parameterized by the
`X86Arch` trait. Expression evaluation for deferred `.skip` directives uses
the shared `backend::asm_expr` module (~410 lines). C-comment stripping,
semicolon splitting, and some preprocessing helpers come from
`backend::asm_preprocess`.

## Key Data Structures

### `AsmItem` enum (33 variants) -- parser.rs

The fundamental intermediate representation. Each non-empty line of assembly
maps to one `AsmItem` variant:

```text
AsmItem
  |-- Section(SectionDirective)        .section / .text / .data / .bss / .rodata
  |-- Global(String)                   .globl name
  |-- Weak(String)                     .weak name
  |-- Hidden(String)                   .hidden name
  |-- Protected(String)                .protected name
  |-- Internal(String)                 .internal name
  |-- SymbolType(String, SymbolKind)   .type name, @function/@object/@tls_object/@notype
  |-- Size(String, SizeExpr)           .size name, expr
  |-- Label(String)                    name:
  |-- Align(u32)                       .align N / .p2align N / .balign N
  |-- Byte(Vec<DataValue>)             .byte val, ...
  |-- Short(Vec<DataValue>)            .short/.value/.2byte val, ...
  |-- Long(Vec<DataValue>)             .long/.4byte/.int val, ...
  |-- Quad(Vec<DataValue>)             .quad/.8byte val, ...
  |-- Zero(u32)                        .zero N / .skip N
  |-- SkipExpr(String, u8)             .skip expr, fill  (deferred expression)
  |-- Asciz(Vec<u8>)                   .asciz "str" / .string "str"
  |-- Ascii(Vec<u8>)                   .ascii "str"
  |-- Comm(String, u64, u32)           .comm name, size, align
  |-- Set(String, String)              .set alias, target
  |-- Cfi(CfiDirective)                .cfi_* (parsed but not emitted)
  |-- File(u32, String)                .file N "name"
  |-- Loc(u32, u32, u32)               .loc filenum line col
  |-- Instruction(Instruction)         encoded x86-64 instruction
  |-- OptionDirective(String)          .option (ignored on x86)
  |-- PushSection(SectionDirective)    .pushsection
  |-- PopSection                       .popsection
  |-- Previous                         .previous (swap current/previous)
  |-- Org(String, i64)                 .org expression, fill
  |-- Incbin { path, skip, count }     .incbin "file"[, skip[, count]]
  |-- Symver(String, String)           .symver name, alias@@VERSION
  |-- CodeMode(u8)                     .code16 / .code32 / .code64
  |-- Empty                            blank/comment-only line
```

### Key parser types

- **`SectionDirective`** -- Section name, optional flags/type/extra, optional
  COMDAT group name (from `.section name,"axG",@progbits,group,comdat`).

- **`SizeExpr`** -- `Constant(u64)`, `CurrentMinusSymbol(String)`,
  `SymbolDiff(String, String)`, or `SymbolRef(String)` (for `.set` aliases).

- **`ImmediateValue`** -- `Integer(i64)`, `Symbol(String)`,
  `SymbolPlusOffset(String, i64)` (e.g., `init_top_pgt - 0xffffffff80000000`),
  `SymbolMod(String, String)` (e.g., `symbol@GOTPCREL`), or
  `SymbolDiff(String, String)` (e.g., `$_DYNAMIC-1b`).

- **`Instruction`** -- Prefix (lock/rep/repnz/notrack), mnemonic, operands.

- **`Operand`** -- Register, Immediate, Memory, Label, or Indirect.

- **`MemoryOperand`** -- Optional segment override (fs/gs), displacement,
  base register, index register, scale (1/2/4/8).

### Encoder types -- encoder/

- **`InstructionEncoder`** -- Stateful encoder that accumulates machine code
  bytes and relocations for one instruction at a time.

- **`Relocation`** -- Offset, symbol name, `R_X86_64_*` type, and addend.

### ELF writer types -- elf_writer_common.rs

The shared `ElfWriterCore<A>` struct owns all sections, symbols, labels, and
pending attributes. Key internal types:

- **`Section`** -- Name, type, flags, data bytes, alignment, relocations,
  jump candidates, alignment markers, and optional COMDAT group.

- **`SymbolInfo`** -- Name, binding, type, visibility, section, value, size,
  and common-symbol fields.

- **`JumpInfo`** -- Offset, length, target label, conditional flag, relaxed
  flag. Used by the relaxation pass.

- **`ElfRelocation`** -- Offset, symbol, type, addend, optional diff_symbol,
  and patch_size (1/2/4/8 bytes).

## Processing Algorithm

### Step 1: Parsing (parser.rs)

`parse_asm()` transforms raw assembly text into `Vec<AsmItem>` through a
multi-stage pipeline:

1. **C-comment stripping** -- `strip_c_comments()` (from `asm_preprocess`)
   removes all `/* ... */` comments globally, preserving newlines so that
   line numbers remain correct.

2. **Line splitting** -- The comment-stripped text is split into lines.

3. **`.rept` expansion** -- `expand_rept_blocks()` finds `.rept N` / `.endr`
   pairs and repeats the enclosed lines N times. Supports nesting and tracks
   `.irp` depth to avoid consuming `.endr` lines that belong to `.irp` blocks.

4. **GAS macro expansion** -- `expand_gas_macros()` (in parser.rs) processes:
   - `.macro name [params]` / `.endm` with positional and default arguments
   - `.irp var, items` iteration loops
   - `.if expr` / `.elseif` / `.else` / `.endif` conditional assembly
   - `.ifc str1, str2` string comparison conditionals
   - `.set symbol, expr` symbol definitions with expression evaluation
   - `.purgem name` macro removal

5. **Per-line processing** -- For each line:
   - Strip `#`-to-end-of-line comments.
   - Split on semicolons (GAS instruction separator, e.g., `rep; nop`).
   - Parse each part independently.

6. **Line classification** -- Each trimmed part is classified as:
   - **Label** -- Ends with `:`, no spaces, not starting with `$` or `%`.
   - **Directive** -- Starts with `.`.
   - **Prefixed instruction** -- Starts with `lock`, `rep`, `repz`, `repnz`,
     `repne`, or `notrack`.
   - **Instruction** -- Everything else.

7. **Directive parsing** -- A large `match` on the directive name handles
   `.section`, `.globl`, `.type`, `.size`, `.align`/`.p2align`/`.balign`,
   `.byte`/`.short`/`.value`/`.2byte`/`.word`/`.hword`/`.long`/`.4byte`/`.int`/`.quad`/`.8byte`,
   `.zero`, `.skip`, `.asciz`/`.string`/`.ascii`, `.comm`, `.set`,
   `.cfi_*`, `.file`, `.loc`, `.pushsection`, `.popsection`, `.previous`,
   `.org`, `.incbin`, and more. Unknown directives (`.ident`, `.addrsig`,
   etc.) are silently ignored and produce `AsmItem::Empty`.

8. **Instruction parsing** -- The mnemonic is split from operands by
   whitespace. Operands are split by commas (respecting parentheses for
   memory operands) and individually parsed:
   - `%name` -> `Register`
   - `$value` -> `Immediate` (integer, symbol, or symbol@modifier)
   - `disp(%base, %index, scale)` -> `Memory`
   - Bare identifier -> `Label`
   - `*operand` -> `Indirect`

9. **Expression evaluation** -- `parse_integer_expr()` implements a full
   recursive-descent expression evaluator supporting `+`, `-`, `*`, `/`,
   `%`, `|`, `^`, `&`, `<<`, `>>`, `~`, and parentheses with proper operator
   precedence. Used for integer expressions in data directives, alignment
   values, and `.skip`/`.rept` arguments.

10. **Integer parsing** -- Decimal, `0x` hex, `0b` binary, and leading-zero
    octal. Handles negative values and large unsigned values via `u64`
    parsing.

11. **String literal parsing** -- Supports standard C escapes (`\n`, `\t`,
    `\r`, `\0`, `\\`, `\"`, `\a`, `\b`, `\f`, `\v`), octal escapes
    (`\NNN`), and hex escapes (`\xNN`).

### Step 2: First Pass -- Item Processing (elf_writer_common.rs)

`ElfWriterCore::build()` first resolves numeric local labels (1:, 2:, etc.)
to unique names via a shared utility, then calls `process_item()` for each
`AsmItem`:

- **Section** -- Creates or switches to the named section with appropriate
  type/flags. Well-known names get default flags (see "Well-Known Section
  Defaults" below). Supports COMDAT groups.

- **PushSection / PopSection** -- `section_stack` saves and restores
  `current_section`.

- **Global/Weak/Hidden/Protected/Internal** -- Recorded as pending attributes.
  Applied when a label bearing that name is defined.

- **SymbolType** -- Recorded in `pending_types`.

- **Size** -- For `.-symbol` expressions, a synthetic end-label is created at
  the current section position so the correct size can be computed after jump
  relaxation.

- **Label** -- Records `(section_index, byte_offset)` in `label_positions`.
  Creates or updates a `SymbolInfo` entry, applying any pending attributes.

- **Align** -- Pads the current section to the requested alignment. Code
  sections are padded with NOP (`0x90`); data sections with zero bytes.
  Records an alignment marker for post-relaxation recalculation.

- **Data directives** (Byte, Short, Long, Quad) -- Each `DataValue` variant
  is handled: integers written in little-endian, symbols generate relocations,
  symbol diffs may be deferred for byte/short sizes.

- **Zero** -- Extends the section data with N zero bytes.

- **Org** -- Advances the current position to the specified offset.

- **SkipExpr** -- Deferred to be evaluated after all labels are known.

- **Asciz / Ascii** -- Appended directly to the current section.

- **Comm** -- Creates a COMMON symbol.

- **Set** -- Records a symbol alias.

- **Incbin** -- Reads an external binary file and appends its contents.

- **Instruction** -- Delegates to `X86_64Arch::encode_instruction()`, which
  creates an `InstructionEncoder` and encodes the instruction. Jump
  instructions targeting labels are detected and recorded for relaxation.

- **Ignored items** -- `Cfi`, `File`, `Loc`, `OptionDirective`, and `Empty`
  are silently skipped.

### Step 3: Instruction Encoding (encoder/)

The encoder translates one `Instruction` into machine code bytes. Its main
dispatch is a large `match` on the mnemonic string (800+ arms) in
`encoder/mod.rs`, with encoding logic organized into focused submodules
by instruction category.

**Suffix inference** -- The `infer_suffix()` function infers AT&T size
suffixes from register operands for unsuffixed mnemonics. This enables
hand-written assembly that omits the size suffix when it can be derived from
context. Only mnemonics in a whitelist are candidates:

```text
mov, add, sub, and, or, xor, cmp, test, push, pop, lea,
shl, shr, sar, rol, ror, inc, dec, neg, not,
imul, mul, div, idiv, adc, sbb,
xchg, cmpxchg, xadd, bswap, bsf, bsr
```

**Encoding formats:**

1. **Legacy encoding** -- Optional prefix bytes, optional REX, opcode (1-3
   bytes), ModR/M, SIB, displacement (0/1/4 bytes), immediate (0/1/2/4/8
   bytes).
2. **VEX encoding** -- 2-byte or 3-byte VEX prefix for AVX/AVX2/BMI2.
3. **EVEX encoding** -- 4-byte EVEX prefix for initial AVX-512 support.

**Prefix emission:**

- `lock` (0xF0), `rep`/`repe`/`repz` (0xF3), `repne`/`repnz` (0xF2),
  `notrack` (0x3E).
- Operand-size override `0x66` for 16-bit operations.
- Segment overrides `0x64` (fs), `0x65` (gs).

**REX prefix** -- Computed from operand width (W), register extension bits
(R, X, B), and special 8-bit register requirements (spl, bpl, sil, dil).

**RIP-relative addressing:**

When the base register is `%rip`, the encoder emits `mod=00, rm=101` in
ModR/M and a 32-bit displacement. Symbol references generate `R_X86_64_PC32`
relocations with an initial addend of `-4` (to account for the 4-byte
displacement field). For instructions that encode immediate bytes *after*
the displacement (e.g. `addl $1, sym(%rip)` has an imm8 trailing byte),
`adjust_rip_reloc_addend()` subtracts the trailing byte count from the
addend (e.g. `-5` for imm8, `-6` for imm16, `-8` for imm32). This is
necessary because RIP-relative displacements are resolved relative to the
end of the *entire* instruction, not just the displacement field. Modifiers
like `@GOTPCREL` and `@GOTTPOFF` produce the corresponding relocation types.

**Relocation generation:**

Relocations are generated for:

- RIP-relative symbol references (PC32, PLT32, GOTPCREL, GOTTPOFF)
- Absolute symbol references in immediates and displacements (32, 32S, 64)
- Call/jump targets (PLT32)
- Data symbol references in `.long`/`.quad` directives (32, 64)
- Thread-local storage references (TPOFF32, GOTTPOFF)
- Internal-only 8-bit PC-relative for `jrcxz`/`loop` (R_X86_64_PC8_INTERNAL)

### Step 4: Jump Relaxation (elf_writer_common.rs)

After the first pass, `relax_jumps()` attempts to shrink long-form jumps to
short form:

```text
Long JMP:  E9 rel32   (5 bytes) --> Short JMP:  EB rel8  (2 bytes)  saves 3
Long Jcc:  0F 8x rel32 (6 bytes) --> Short Jcc: 7x rel8  (2 bytes)  saves 4
```

The algorithm is iterative because shrinking one jump can bring another
jump's target into short range:

1. Build a map of label positions within the current section.
2. For each un-relaxed jump, compute the displacement as if the jump were
   already in short form (end of instruction = offset + 2).
3. If the displacement fits in a signed byte [-128, +127], mark for
   relaxation.
4. Process relaxations **back-to-front** (so byte offsets remain valid):
   - Rewrite the opcode bytes in place (Jcc: `0x70+cc`; JMP: `0xEB`).
   - Remove the extra bytes via `data.drain()`.
   - Adjust all `label_positions`, `numeric_label_positions`, relocation
     offsets, other jump offsets, `deferred_skips`, and `deferred_byte_diffs`
     that fall after the shrunk instruction.
   - Remove the relocation entry for this jump (displacement is now inline).
5. Repeat from step 1 until no more relaxation occurs (fixed-point
   convergence).
6. After convergence, compute and patch the final short displacements.

### Step 5: Deferred Expression Resolution (elf_writer_common.rs)

Two categories of deferred work are resolved after jump relaxation:

**Deferred `.skip` expressions** -- `resolve_deferred_skips()` evaluates
complex `.skip` expressions that reference labels. The expression evaluator
(from `backend::asm_expr`) supports arithmetic, comparison, bitwise, and
unary operators with proper operator precedence. Symbol references are
resolved from `label_positions`.

Skips are processed in reverse order within each section so that earlier
insertions do not invalidate the offsets of later ones. After each insertion,
all subsequent label positions, numeric label positions, relocation offsets,
jump offsets, and deferred byte diffs in the same section are adjusted.

**Deferred byte-sized symbol diffs** -- `resolve_deferred_byte_diffs()`
resolves 1-byte and 2-byte `symbol_a - symbol_b` expressions. Both symbols
must be in the same section; cross-section diffs are an error.

### Step 6: Post-Relaxation Updates (elf_writer_common.rs)

1. **Symbol value update** -- All symbol values are refreshed from
   `label_positions` (which were adjusted during relaxation and skip
   resolution).
2. **Size resolution** -- `.size` directives are resolved:
   - `Constant(v)` -- Used directly.
   - `CurrentMinusSymbol(start)` -- `section_data_len - start_offset`.
   - `SymbolDiff(end, start)` -- `end_offset - start_offset`.
   - `SymbolRef(name)` -- Resolved through `.set` aliases.

### Step 7: Internal Relocation Resolution (elf_writer_common.rs)

`resolve_internal_relocations()` resolves relocations that can be computed
without the linker:

- **`R_X86_64_PC8_INTERNAL`** -- Internal-only 8-bit PC-relative relocations
  for `jrcxz`/`loop` instructions. Same-section targets are patched inline;
  these are never emitted to the ELF file.

- **`R_X86_64_PC32`** -- Same-section, local-symbol targets: the ELF formula
  `S + A - P` is applied and the result is patched into the section data.

- **`R_X86_64_PLT32`** -- Same-section, local-symbol targets: resolved
  identically to PC32.

- **`R_X86_64_32`** -- Absolute references to local symbols: `S + A` patched
  directly.

- **Symbol-difference relocations** -- Where both symbols in `.long a - b`
  are in the same section, the difference `offset(a) - offset(b)` is computed
  and patched as a constant.

Global and weak symbols are **never** resolved at this stage -- their
relocations are kept for the linker to handle symbol interposition and PLT
redirection correctly.

### Step 8: ELF Emission (elf_writer_common.rs)

`emit_elf()` resolves `.set` aliases into proper symbols, creates undefined
symbols for external references found in relocations, then converts the
internal data structures into the shared `ObjSection`/`ObjSymbol`/`ObjReloc`
format and delegates to `write_relocatable_object()` in `backend/elf.rs`.

**Internal label conversion:** Relocations referencing `.L*` labels are
converted to reference the parent section's section symbol, with the label's
offset baked into the addend. This matches GAS behavior and keeps the symbol
table small.

**ELF file layout:**

```text
+---------------------------------------------------+
| ELF Header (64 bytes)                             |
+---------------------------------------------------+
| Section 1 data (aligned)                          |
| Section 2 data (aligned)                          |
| ...                                               |
+---------------------------------------------------+
| .rela.text (if .text has unresolved relocations)  |
| .rela.data (if .data has unresolved relocations)  |
| ...                                               |
+---------------------------------------------------+
| .symtab (8-byte aligned)                          |
|   - Null symbol                                   |
|   - Section symbols (STT_SECTION, one per section)|
|   - Local defined symbols                         |
|   - Global and weak symbols                       |
|   - Alias symbols (.set)                          |
|   - Undefined external symbols                    |
+---------------------------------------------------+
| .strtab (symbol name strings, NUL-terminated)     |
+---------------------------------------------------+
| .shstrtab (section name strings, NUL-terminated)  |
+---------------------------------------------------+
| Section Header Table (8-byte aligned)             |
|   [0] NULL                                        |
|   [1..N] data sections                            |
|   [N+1..] .rela.* sections                        |
|   [M] .symtab                                     |
|   [M+1] .strtab                                   |
|   [M+2] .shstrtab                                 |
+---------------------------------------------------+
```

**Symbol table ordering** (required by ELF spec):

1. Null symbol (index 0)
2. Section symbols (`STT_SECTION`, one per data section, `STB_LOCAL`)
3. Local non-internal symbols
4. ---- `sh_info` boundary (first global index) ----
5. Global and weak symbols
6. Alias symbols from `.set` directives
7. Undefined external symbols (auto-created from unresolved relocations)

**ELF configuration:**

- Class: ELFCLASS64
- Machine: EM_X86_64
- Relocation format: RELA (with explicit addends)
- Flags: 0

## Instruction Encoding -- Supported Families

The encoder covers the following instruction categories (800+ match arms):

| # | Category | Instructions |
|---|----------|-------------|
| 1 | **Data movement** | mov (b/w/l/q), movabs, movsx (bl/bq/wl/wq/slq), movzx (bl/bq/wl/wq), lea (l/q), push, pop, xchg, cmpxchg (b/w/l/q), xadd (b/w/l/q), cmpxchg8b, cmpxchg16b |
| 2 | **Arithmetic** | add, sub, adc, sbb, and, or, xor, cmp, test (b/w/l/q), neg, not, inc, dec, imul (1/2/3-operand), mul, div, idiv |
| 3 | **Shifts/Rotates** | shl, shr, sar, rol, ror, rcl, rcr (b/w/l/q), shld, shrd (l/q) |
| 4 | **Sign extension** | cltq, cqto/cqo, cltd/cdq, cbw, cwd |
| 5 | **Byte swap** | bswap (l/q) |
| 6 | **Bit manipulation** | lzcnt, tzcnt, popcnt, bsf, bsr, bt, bts, btr, btc (l/q/w) |
| 7 | **Conditional set** | setcc (all 20+ conditions) |
| 8 | **Conditional move** | cmovcc (w/l/q, all conditions, plus suffix-less forms) |
| 9 | **Control flow** | jmp/jmpq, jcc (all conditions), call/callq, ret/retq, jrcxz, loop |
| 10 | **SSE/SSE2 scalar float** | movss, movsd, addss/sd, subss/sd, mulss/sd, divss/sd, sqrtss/sd, ucomisd, ucomiss, xorpd/ps, andpd/ps, minss/sd, maxss/sd |
| 11 | **SSE packed float** | addpd/ps, subpd/ps, mulpd/ps, divpd/ps, orpd/ps, andnpd/ps, minpd/ps, maxpd/ps |
| 12 | **SSE data movement** | movaps, movdqa, movdqu, movlpd, movhpd, movapd, movups, movupd, movhlps, movlhps, movd, movq, movmskpd, movmskps |
| 13 | **SSE2 integer SIMD** | paddb/w/d/q, psubb/w/d, pmulhw, pmaddwd, pcmpgtw/b, packssdw, packuswb, punpckl/h (bw/wd/dq/qdq), pmovmskb, pcmpeqb/d/w, pand, pandn, por, pxor, psubusb/w, paddusb/w, paddsb/w, pmuludq, pmullw, pmulld, pminub, pmaxub, pminsd, pmaxsd, pavgb/w, psadbw |
| 14 | **SSE shifts** | psllw/d/q, psrlw/d/q, psraw/d, pslldq, psrldq |
| 15 | **SSE shuffles** | pshufd, pshuflw, pshufhw, shufps, shufpd, palignr, pshufb |
| 16 | **SSE insert/extract** | pinsrb/w/d/q, pextrb/w/d/q |
| 17 | **SSE3/SSSE3** | haddpd/ps, hsubpd/ps, addsubpd/ps, movddup, movshdup, movsldup, pabsb/w/d, phaddw/d, phsubw/d, pmulhrsw |
| 18 | **SSE4.1** | blendvpd/ps, pblendvb, roundsd/ss/pd/ps, pblendw, blendpd/ps, dpps, dppd, ptest, pminsb, pminuw, pmaxsb, pmaxuw, pminud, pmaxud, pminsw, pmaxsw, phminposuw, packusdw, packsswb, pmovzxbw/bd/bq/wd/wq/dq, pmovsxbw/bd/bq/wd/wq/dq |
| 19 | **SSE unpacks** | unpcklpd/ps, unpckhpd/ps |
| 20 | **SSE non-temporal** | movnti, movntdq, movntdqa, movntpd, movntps |
| 21 | **SSE MXCSR** | ldmxcsr, stmxcsr |
| 22 | **SSE conversions** | cvtsd2ss, cvtss2sd, cvtsi2sdq/ssq/sdl/ssl, cvttsd2siq/ssiq/sd2sil/ss2sil, cvtsd2siq/sd2si/ss2siq/ss2si, cvtps2dq, cvtdq2ps, cvttps2dq |
| 23 | **AES-NI** | aesenc, aesenclast, aesdec, aesdeclast, aesimc, aeskeygenassist |
| 24 | **PCLMULQDQ** | pclmulqdq |
| 25 | **CRC32** | crc32 (b/w/l/q) |
| 26 | **AVX data movement** | vmovdqa, vmovdqu, vmovaps, vmovapd, vmovups, vmovupd, vmovd, vmovq, vmovss, vmovsd, vbroadcastss, vbroadcastsd, vmovddup, vmovshdup, vmovsldup |
| 27 | **AVX float** | vaddpd/ps, vsubpd/ps, vmulpd/ps, vdivpd/ps, vxorpd/ps, vandpd/ps, vandnpd/ps, vorpd/ps, vminpd/ps, vmaxpd/ps |
| 28 | **AVX scalar float** | vaddss/sd, vsubss/sd, vmulss/sd, vdivss/sd, vcmpps/pd/ss/sd (including pseudo-ops like vcmpnleps) |
| 29 | **AVX integer** | vpaddb/w/d/q, vpsubb/w/d/q, vpmullw, vpmulld, vpmuludq, vpcmpeqb/w/d/q, vpcmpgtq, vpand, vpandn, vpor, vpxor, vpunpckl/h (bw/wd/dq/qdq), vpslldq, vpsrldq, vpabsb |
| 30 | **AVX shifts** | vpsllw/d/q, vpsrlw/d/q, vpsraw/d |
| 31 | **AVX shuffles** | vpshufd, vpshufb, vpalignr, vpermilps/pd |
| 32 | **AVX2** | vpbroadcastb/w/d/q, vperm2i128, vperm2f128, vpermd, vpermq, vpblendd, vblendvps/pd, vpblendvb, vinserti128, vextracti128, vextractf128, vbroadcasti128 |
| 33 | **AVX AES-NI/PCLMUL** | vaesenc, vaesenclast, vaesdec, vaesdeclast, vpclmulqdq |
| 34 | **AVX misc** | vpmovmskb, vpextrq, vpinsrq, vptest, vzeroupper, vzeroall, vcvtps2dq, vcvtdq2ps, vcvttps2dq |
| 35 | **AVX-512 (EVEX, initial)** | vpxord, vpxorq, vpandd, vpandq, vpord, vporq |
| 36 | **BMI2** | shrx, shlx, sarx, rorx, bzhi, pext, pdep, mulx, andn, bextr (l/q plus suffix-less forms) |
| 37 | **x87 FPU** | fld, fstp, fldl/s/t, fstpl/s/t, fsts, fild, fistp, fisttp, fist, fld1, fldl2e, fldlg2, fldln2, fldz, fldpi, fldl2t, faddp, fsubp, fsubrp, fmulp, fdivp, fdivrp, fchs, fadd, fmul, fsub, fdiv, faddl/s, fmull/s, fsubl/s, fdivl/s, fabs, fsqrt, frndint, f2xm1, fscale, fpatan, fprem, fprem1, fyl2x, fyl2xp1, fptan, fsin, fcos, fxtract, fcomip, fucomip, fxch, fninit, fwait/wait, fnstcw/fstcw, fldcw, fnclex, fnstenv, fldenv, fnstsw |
| 38 | **String ops** | movsb/w/l/d/q, stosb/w/l/d/q, lodsb/w/d/q, scasb/w/d/q, cmpsb/w/d/q |
| 39 | **I/O string ops** | insb/w/d, outsb/w/d |
| 40 | **Port I/O** | inb/w/l, outb/w/l |
| 41 | **Standalone prefixes** | lock, rep/repe/repz, repne/repnz |
| 42 | **System** | syscall, sysenter, cpuid, rdtsc, rdtscp, wbinvd, invd, int, int3, sldt, str |
| 43 | **CET** | endbr64, rdsspq, rdsspd |
| 44 | **Flags** | cld, std, clc, stc, cmc, cli, sti, sahf, lahf, pushf/pushfq, popf/popfq |
| 45 | **Misc** | nop, hlt, leave, ud2, pause, mfence, lfence, sfence, clflush |
| 46 | **Segment/Control regs** | mov to/from segment registers (es/cs/ss/ds/fs/gs), mov to/from control registers (cr0/cr2/cr3/cr4/cr8) |
| 47 | **MMX** | emms, paddb (MMX form) |
| 48 | **VMX** | vmcall, vmmcall, vmlaunch, vmresume, vmxoff, vmfunc |

## Supported Relocation Types

| Constant | Value | Usage |
|----------|-------|-------|
| `R_X86_64_NONE` | 0 | No relocation |
| `R_X86_64_64` | 1 | Absolute 64-bit address (`.quad symbol`) |
| `R_X86_64_PC32` | 2 | PC-relative 32-bit (RIP-relative addressing, `.long a - b`) |
| `R_X86_64_GOT32` | 3 | 32-bit GOT offset |
| `R_X86_64_PLT32` | 4 | PC-relative via PLT (`call`/`jmp` targets) |
| `R_X86_64_GOTPCREL` | 9 | GOT entry, PC-relative (`symbol@GOTPCREL(%rip)`) |
| `R_X86_64_32` | 10 | Absolute 32-bit unsigned (`.long symbol`) |
| `R_X86_64_32S` | 11 | Absolute 32-bit signed (symbol displacement in non-RIP memory) |
| `R_X86_64_TPOFF64` | 18 | 64-bit TLS offset from thread pointer |
| `R_X86_64_GOTTPOFF` | 22 | TLS GOT entry, PC-relative (`symbol@GOTTPOFF(%rip)`) |
| `R_X86_64_TPOFF32` | 23 | 32-bit TLS offset from thread pointer (`symbol@TPOFF`) |
| `R_X86_64_PC8_INTERNAL` | 0x8000_0001 | **Internal-only**: 8-bit PC-relative for `jrcxz`/`loop` (never emitted to ELF) |

## Numeric Labels

Labels like `1:`, `2:`, etc. can be defined multiple times. Forward
references (`1f`) resolve to the next definition after the reference point;
backward references (`1b`) resolve to the most recent definition before the
reference point.

Numeric labels are resolved by a pre-pass (`resolve_numeric_labels()`) that
converts them to unique names before the main processing begins. The
`resolve_numeric_label()` method in the ELF writer provides fallback
resolution for labels encountered in deferred expressions.

## Well-Known Section Defaults

When switching to a section by its well-known name (without explicit flags),
the following defaults are applied:

| Section Name | Type | Flags |
|-------------|------|-------|
| `.text` | PROGBITS | ALLOC, EXECINSTR |
| `.data` | PROGBITS | ALLOC, WRITE |
| `.bss` | NOBITS | ALLOC, WRITE |
| `.rodata` | PROGBITS | ALLOC |
| `.tdata` | PROGBITS | ALLOC, WRITE, TLS |
| `.tbss` | NOBITS | ALLOC, WRITE, TLS |
| `.init` / `.fini` | PROGBITS | ALLOC, EXECINSTR |
| `.init_array` | INIT_ARRAY | ALLOC, WRITE |
| `.fini_array` | FINI_ARRAY | ALLOC, WRITE |
| `.text.*` | PROGBITS | ALLOC, EXECINSTR |
| `.data.*` | PROGBITS | ALLOC, WRITE |
| `.bss.*` | NOBITS | ALLOC, WRITE |
| `.rodata.*` | PROGBITS | ALLOC |
| `.note.*` | NOTE | (no flags) |

Sections created with explicit `.section name, "flags", @type` use the
provided flags/type instead of these defaults.

## Supported Directives

| Directive | Syntax | Purpose |
|-----------|--------|---------|
| `.section` | `.section name, "flags", @type` | Switch to named section with explicit attributes |
| `.text` | `.text` | Switch to `.text` section (shorthand) |
| `.data` | `.data` | Switch to `.data` section (shorthand) |
| `.bss` | `.bss` | Switch to `.bss` section (shorthand) |
| `.rodata` | `.rodata` | Switch to `.rodata` section (shorthand) |
| `.pushsection` | `.pushsection name, "flags", @type` | Push current section, switch to named section |
| `.popsection` | `.popsection` | Pop section stack (from `.pushsection`) |
| `.previous` | `.previous` | Swap current and previous sections (toggle between last two) |
| `.globl` / `.global` | `.globl name` | Mark symbol as global binding |
| `.weak` | `.weak name` | Mark symbol as weak binding |
| `.hidden` | `.hidden name` | Set symbol visibility to STV_HIDDEN |
| `.protected` | `.protected name` | Set symbol visibility to STV_PROTECTED |
| `.internal` | `.internal name` | Set symbol visibility to STV_INTERNAL |
| `.type` | `.type name, @function\|@object\|@tls_object\|@notype` | Set symbol type |
| `.size` | `.size name, expr` | Set symbol size (supports `.-name`, constants, `sym_a - sym_b`) |
| `.align` | `.align N` | Align to N-byte boundary (byte count) |
| `.p2align` | `.p2align N` | Align to 2^N-byte boundary (power of 2) |
| `.balign` | `.balign N` | Align to N-byte boundary (byte count) |
| `.byte` | `.byte val, ...` | Emit 1-byte values |
| `.short` / `.value` / `.2byte` / `.word` / `.hword` | `.short val, ...` | Emit 2-byte values |
| `.long` / `.4byte` / `.int` | `.long val, ...` | Emit 4-byte values |
| `.quad` / `.8byte` | `.quad val, ...` | Emit 8-byte values |
| `.zero` | `.zero N` | Emit N zero bytes |
| `.skip` | `.skip N, fill` | Skip N bytes (simple) or deferred expression |
| `.org` | `.org sym, fill` | Advance to position within section |
| `.asciz` / `.string` | `.asciz "str"` | Emit NUL-terminated string |
| `.ascii` | `.ascii "str"` | Emit string without NUL terminator |
| `.comm` | `.comm name, size, align` | Define common (BSS) symbol |
| `.set` | `.set alias, target` | Define symbol alias |
| `.symver` | `.symver name, alias@@VER` | Define symbol version alias (creates unversioned alias) |
| `.incbin` | `.incbin "file"[, skip[, count]]` | Include binary file contents |
| `.rept` / `.endr` | `.rept N` ... `.endr` | Repeat block of lines N times |
| `.macro` / `.endm` | `.macro name [params]` ... `.endm` | Define a GAS macro |
| `.irp` / `.endr` | `.irp var, items` ... `.endr` | Iterate over items |
| `.if` / `.elseif` / `.else` / `.endif` | `.if expr` | Conditional assembly |
| `.ifc` | `.ifc str1, str2` | String comparison conditional |
| `.purgem` | `.purgem name` | Remove a macro definition |
| `.cfi_*` | `.cfi_startproc`, etc. | CFI directives (parsed, not emitted) |
| `.file` | `.file N "name"` | Debug file directive (parsed, ignored) |
| `.loc` | `.loc filenum line col` | Debug location (parsed, ignored) |
| `.option` | `.option ...` | RISC-V directive (ignored on x86) |
| `.code16` / `.code32` / `.code64` | `.code16` | Set code generation mode (16/32/64-bit) |

## Key Design Decisions and Trade-offs

### 1. Subset, Not Full GAS

The assembler implements what the compiler's codegen actually produces, plus
what hand-written assembly in musl, the Linux kernel, and similar projects
requires. GAS macro support (`.macro`/`.endm`, `.if`/`.endif`, `.irp`,
`.ifc`, `.set`, `.rept`) is handled directly in `parser.rs` via the
`expand_gas_macros()` function, with some preprocessing helpers from the
shared `asm_preprocess` module. Unknown directives are silently ignored with
`AsmItem::Empty`, which provides forward compatibility as the codegen evolves.

### 2. Shared Architecture via Generics

The ELF building logic is shared across all architectures (x86-64, i686, ARM,
RISC-V) via `ElfWriterCore<A: X86Arch>`. Each architecture implements the
`X86Arch` trait to provide instruction encoding, relocation type constants,
and ELF machine/class values. The x86-64 adapter (`elf_writer.rs`) is ~90
lines.

### 3. Suffix Inference for Hand-Written Assembly

Hand-written assembly often omits the AT&T size suffix when it can be inferred
from register operands. The `infer_suffix()` function handles this for a
curated whitelist of mnemonics -- this prevents incorrect inference on
mnemonics where the trailing letter is part of the name (e.g., `movsd`).

### 4. Always-Long Encoding with Post-Hoc Relaxation

Instructions are initially encoded in their longest form (e.g., `jmp` always
uses `E9 rel32`). The relaxation pass then shrinks eligible jumps to short
form. This two-phase approach avoids the complexity of an iterative
encode-measure-re-encode loop during the first pass.

### 5. Deferred Expression Evaluation

Complex `.skip` expressions (such as those used in the Linux kernel
alternatives framework) cannot be evaluated during the first pass because
they reference labels that may not yet be defined, and whose positions may
shift during jump relaxation. These are deferred and evaluated after both
relaxation and skip insertion are complete.

### 6. Lazy Symbol Attribute Application

`.globl`, `.type`, `.hidden`, etc. are collected as "pending" attributes and
only applied when a label definition (`name:`) is encountered. This handles
the common GAS pattern where attributes appear before the label.

### 7. Internal Labels via Section Symbols

Labels starting with `.L` are treated as section-local. In the ELF output,
they are not emitted as named symbols. Instead, relocations referencing them
use the parent section's section symbol, with the addend adjusted to include
the label's offset. This matches GAS behavior.

### 8. CFI Directives Parsed but Not Emitted

The assembler parses `.cfi_*` directives but does not generate `.eh_frame`
section data. The built-in linker does not require unwind information for
basic compilation.

### 9. Immediate-Size Optimization

ALU instructions with small immediates automatically use the sign-extended
imm8 form (`0x83` instead of `0x81`), and `movq` with 32-bit-range
immediates uses `C7` instead of `movabs`.

### 10. Three Encoding Formats

The encoder supports legacy (REX), VEX (2-byte and 3-byte for AVX/BMI2), and
EVEX (4-byte for initial AVX-512) prefix formats. The 2-byte VEX form is
preferred when possible (map=1, W=0, no X/B extension bits).
