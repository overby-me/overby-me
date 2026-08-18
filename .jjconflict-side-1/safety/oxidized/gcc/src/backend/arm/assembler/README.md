# AArch64 Built-in Assembler -- Design Document

## Overview

The built-in AArch64 assembler is a self-contained subsystem that translates
GNU-style assembly text (`.s` files), as emitted by the compiler's AArch64
codegen, into ELF64 relocatable object files (`.o`). Its purpose is to
eliminate the external dependency on `aarch64-linux-gnu-gcc` for assembling,
making the compiler fully self-hosting on AArch64 Linux targets.

The assembler is active by default (when the `gcc_assembler` Cargo feature is
not enabled). It accepts the same textual assembly that GCC's gas would consume
and produces ABI-compatible `.o` files that any standard AArch64 ELF linker (or
the companion built-in linker) can link.

The implementation spans roughly 9,600 lines of Rust across four files and is
organized as a clean three-stage pipeline. Shared ELF infrastructure (section
management, symbol tables, ELF serialization) lives in `ElfWriterBase` in
`elf.rs`; the files here contain only AArch64-specific logic.

```text
                    AArch64 Built-in Assembler
  ================================================================

  .s assembly text
       |
       v
  +------------------+
  |   parser.rs       |   Stage 1: Preprocess + Parse
  |   (~2,600 lines)  |   Macros, .rept, .if -> AsmStatement[]
  +------------------+
       |
       | Vec<AsmStatement>
       v
  +------------------+
  |   elf_writer.rs   |   Stage 2: Process + Encode + Emit
  |   (~580 lines)    |   AArch64-specific: branch resolution,
  |                    |   sym diffs (uses ElfWriterBase)
  +------------------+
       |         ^
       |         |  encode_instruction()
       v         |
  +------------------+
  |   encoder/        |   Instruction Encoding Library
  |   (~6,190 lines)  |   Mnemonic + Operands -> u32 words
  +------------------+
       |
       v
  ELF64 .o file on disk
```

The single public entry point is:

```rust
// mod.rs (~220 lines)
pub fn assemble(asm_text: &str, output_path: &str) -> Result<(), String>
```

It calls `parse_asm()`, expands literal pools (`ldr Xn, =symbol` → `ldr Xn, .Llpool_N`

+ `.quad` pool entries), resolves GNU numeric labels (`1f`/`1b`), creates an
`ElfWriter`, feeds it the parsed statements, and writes the final `.o` file.

---

## Stage 1: Parser (`parser.rs`)

### Purpose

Convert raw assembly text into a structured, typed intermediate
representation -- a `Vec<AsmStatement>`. Every subsequent stage works on this
IR; no raw text parsing happens after this point.

### Key Data Structures

| Type | Role |
|------|------|
| `AsmStatement` | Top-level IR node: `Label`, `Directive`, `Instruction`, or `Empty`. |
| `AsmDirective` | Fully-typed directive variant (28 kinds, from `.section` to `.cfi_*`). |
| `Operand` | Operand of an instruction (20 variants covering every AArch64 addressing mode). |
| `SectionDirective` | Parsed `.section name, "flags", @type` triple. |
| `DataValue` | Data that can be an integer, a symbol, a symbol+offset, or a symbol difference. |
| `SizeExpr` | The expression in `.size sym, expr` -- either a constant or `.- sym`. |
| `SymbolKind` | From `.type`: `Function`, `Object`, `TlsObject`, `NoType`. |

### Operand Variants

The `Operand` enum models every AArch64 operand shape the codegen can emit:

```text
Reg("x0")                          -- general / FP / SIMD register
Imm(42)                            -- immediate value (#42)
Symbol("printf")                   -- bare symbol reference
SymbolOffset("arr", 16)            -- symbol + constant
Mem { base, offset }               -- [base, #offset]
MemPreIndex { base, offset }       -- [base, #offset]!
MemPostIndex { base, offset }      -- [base], #offset
MemRegOffset { base, index, .. }   -- [base, Xm, extend #shift]
Modifier { kind, symbol }          -- :lo12:symbol, :got_lo12:symbol
ModifierOffset { kind, sym, off }  -- :lo12:symbol+offset
Shift { kind, amount }             -- lsl #N
Extend { kind, amount }            -- sxtw #N
Cond("eq")                         -- condition code
Barrier("ish")                     -- barrier option
Label(".LBB0_4")                   -- branch target
Expr("complex_expr")               -- raw expression fallback
RegArrangement { reg, arr }        -- v0.16b (NEON arrangement)
RegLane { reg, elem_size, index }  -- v0.d[1] (NEON lane)
RegList(Vec<Operand>)              -- {v0.16b, v1.16b}
RegListIndexed { regs, index }     -- {v0.s, v1.s}[0] (NEON single-element)
```

### Parsing Algorithm

```text
parse_asm(text)
  0. Strip C-style /* ... */ block comments
  1. expand_macros() -- collect .macro/.endm definitions, expand invocations
     - Supports default parameters, varargs (\()), .purgem
     - Nested macro definitions tracked with depth counter
     - Recursive expansion with 64-level depth limit
     - \@ counter substitution for unique label generation
  2. expand_rept_blocks() -- flatten .rept/.endr and .irp/.irpc/.endr
  3. resolve_set_constants() -- substitute .set/.equ symbol values
  4. resolve_register_aliases() -- process .req/.unreq register aliases
  5. Conditional assembly (evaluated during line processing):
     - .if/.elseif/.elsif/.else/.endif
     - .ifdef/.ifndef (symbol existence)
     - .ifc/.ifnc (string comparison)
     - .ifb/.ifnb (blank argument test)
     - .ifeq/.ifne (numeric comparison)
     - Supports ==, !=, >, >=, <, <= comparisons
     - Arithmetic expressions via shared asm_expr evaluator
  6. ldr =symbol pseudo-instruction → LdrLiteralPool (expanded to literal pool later)
  for each line:
    a. Trim whitespace, strip comments (// and @ style)
    b. Split on ';' (GAS multi-statement separator)
    c. For each sub-statement:
       - Try to match "name:" -> Label(name)
       - Try to match "." prefix -> parse_directive()
       - Otherwise -> parse_instruction()
         - parse_operands() splits on ',' respecting [] and {} nesting
         - parse_single_operand() handles all operand shapes
         - Post-pass: merge [base], #offset into MemPostIndex
```

### Supported Directives

| Category | Directives |
|----------|-----------|
| Sections | `.section`, `.text`, `.data`, `.bss`, `.rodata`, `.pushsection`, `.popsection`/`.previous` |
| Symbols | `.globl`/`.global`, `.weak`, `.hidden`, `.protected`, `.internal`, `.type`, `.size`, `.local`, `.comm`, `.set`/`.equ` |
| Alignment | `.align`, `.p2align` (power-of-2), `.balign` (byte count) |
| Data emission | `.byte`, `.short`/`.hword`/`.2byte`, `.long`/`.4byte`/`.word`, `.quad`/`.8byte`/`.xword`, `.zero`/`.space`, `.ascii`, `.asciz`/`.string`, `.float`/`.single`, `.double`, `.inst` |
| Macros | `.macro`/`.endm` (with default params, varargs), `.purgem`, `.req`/`.unreq` (register aliases) |
| Repetition | `.rept`/`.endr`, `.irp`/`.endr`, `.irpc`/`.endr` |
| Conditionals | `.if`/`.elseif`/`.elsif`/`.else`/`.endif`, `.ifdef`/`.ifndef`, `.ifc`/`.ifnc`, `.ifb`/`.ifnb`, `.ifeq`/`.ifne` |
| Includes | `.incbin` (binary file inclusion) |
| CFI | `.cfi_startproc`, `.cfi_endproc`, `.cfi_def_cfa_offset`, `.cfi_offset`, and 12 more (all passed through as no-ops) |
| Literal pool | `.ltorg`/`.pool` (flushes pending literal pool entries from `ldr Xn, =symbol`) |
| Ignored | `.file`, `.loc`, `.ident`, `.addrsig`, `.addrsig_sym`, `.build_attributes`, `.eabi_attribute`, `.arch`, `.arch_extension`, `.cpu` |

### Design Decisions (Parser)

- **Eager parsing**: Directives are fully parsed at parse time (not deferred).
  The `.section` flags string is decomposed into `SectionDirective`; `.type`
  maps to a `SymbolKind` enum; `.align` values are converted from power-of-2
  to byte counts immediately.

- **Comment stripping guards**: Both `//` and `@` are handled, but `@` is only
  treated as a comment character when it does not prefix known GAS type tags
  (`@object`, `@function`, `@progbits`, `@nobits`, `@tls_object`, `@note`).

- **Raw operand preservation**: Each `Instruction` stores both the parsed
  `Vec<Operand>` and the raw operand text string, allowing the encoder to fall
  back to text-level heuristics for unusual operand patterns.

---

## Stage 2: Instruction Encoder (`encoder/`)

### Purpose

Given a mnemonic string and a `Vec<Operand>`, produce the 4-byte (32-bit)
little-endian machine code word. AArch64 has a fixed 32-bit instruction
width, which makes encoding straightforward compared to variable-length ISAs.

### Key Data Structures

| Type | Role |
|------|------|
| `EncodeResult` | Outcome of encoding one instruction. |
| `RelocType` | AArch64 ELF relocation types (21 variants). |
| `Relocation` | A relocation request: type + symbol + addend. |

The `EncodeResult` enum has four variants:

```text
Word(u32)                        -- single fully-resolved instruction
WordWithReloc { word, reloc }    -- instruction needing a linker relocation
Words(Vec<u32>)                  -- multi-word sequence (e.g., movz+movk)
Skip                             -- pseudo-instruction; no code emitted
```

### Supported Instruction Categories

The encoder handles a comprehensive set of AArch64 instructions.
The dispatch table in `encode_instruction()` maps ~400 base mnemonics
(~440 including condition code variants):

| Category | Mnemonics |
|----------|-----------|
| **Data Processing** | `mov`, `movz`, `movk`, `movn`, `add`, `adds`, `sub`, `subs`, `and`, `orr`, `eor`, `ands`, `orn`, `eon`, `bics`, `mul`, `madd`, `msub`, `smull`, `umull`, `smaddl`, `umaddl`, `mneg`, `udiv`, `sdiv`, `umulh`, `smulh`, `neg`, `negs`, `mvn`, `adc`, `adcs`, `sbc`, `sbcs` |
| **Shifts** | `lsl`, `lsr`, `asr`, `ror` |
| **Bit fields** | `ubfm`, `sbfm`, `ubfx`, `sbfx`, `ubfiz`, `sbfiz`, `bfm`, `bfi`, `bfxil`, `extr` |
| **Extensions** | `sxtw`, `sxth`, `sxtb`, `uxtw`, `uxth`, `uxtb` |
| **Compare** | `cmp`, `cmn`, `tst`, `ccmp`, `fccmp` |
| **Conditional select** | `csel`, `csinc`, `csinv`, `csneg`, `cset`, `csetm`, `fcsel` |
| **Branches** | `b`, `bl`, `br`, `blr`, `ret`, `cbz`, `cbnz`, `tbz`, `tbnz`, `b.eq`/`beq`, ... (all 16 conditions) |
| **Loads/Stores** | `ldr`, `str`, `ldrb`, `strb`, `ldrh`, `strh`, `ldrsw`, `ldrsb`, `ldrsh`, `ldur`, `stur`, `ldp`, `stp`, `ldnp`, `stnp`, `ldxr`, `stxr`, `ldxrb`, `stxrb`, `ldxrh`, `stxrh`, `ldaxr`, `stlxr`, `ldaxrb`, `stlxrb`, `ldaxrh`, `stlxrh`, `ldar`, `stlr`, `ldarb`, `stlrb`, `ldarh`, `stlrh` |
| **Address** | `adrp`, `adr` |
| **Floating point** | `fmov`, `fadd`, `fsub`, `fmul`, `fdiv`, `fmax`, `fmin`, `fmaxnm`, `fminnm`, `fneg`, `fabs`, `fsqrt`, `fcmp`, `fmadd`, `fmsub`, `fnmadd`, `fnmsub`, `frintn`/`p`/`m`/`z`/`a`/`x`/`i`, `fcvtzs`, `fcvtzu`, `fcvtas`/`au`/`ns`/`nu`/`ms`/`mu`/`ps`/`pu`, `ucvtf`, `scvtf`, `fcvt` |
| **NEON three-same** | `add`, `sub`, `mul`, `and`, `orr`, `eor`, `orn`, `bic`, `bif`, `bit`, `bsl`, `cmeq`, `cmge`, `cmgt`, `cmhi`, `cmhs`, `cmtst`, `sqadd`, `uqadd`, `sqsub`, `uqsub`, `shadd`, `uhadd`, `shsub`, `uhsub`, `srhadd`, `urhadd`, `smax`, `umax`, `smin`, `umin`, `sabd`, `uabd`, `saba`, `uaba`, `sshl`, `ushl`, `sqshl`, `uqshl`, `srshl`, `urshl`, `sqrshl`, `uqrshl`, `addp`, `uminp`, `umaxp`, `sminp`, `smaxp`, `pmul` |
| **NEON two-misc** | `cnt`, `not`/`mvn`, `rev16`, `rev32`, `rev64`, `cls`, `clz`, `neg`, `abs`, `sqabs`, `sqneg`, `xtn`/`xtn2`, `uqxtn`/`uqxtn2`, `sqxtn`/`sqxtn2`, `sqxtun`/`sqxtun2`, `fcvtn`/`fcvtl`, `shll`/`shll2` |
| **NEON float vector** | `fadd`, `fsub`, `fmul`, `fdiv`, `fmax`, `fmin`, `fmaxnm`, `fminnm`, `fneg`, `fabs`, `fsqrt`, `frintn`/`p`/`m`/`z`/`a`/`x`/`i` (vector forms), `frecpe`, `frsqrte`, `frecps`, `frsqrts`, `faddp`, `fmaxp`, `fminp`, `fmaxnmp`, `fminnmp` |
| **NEON compare-zero** | `cmgt`/`cmge`/`cmeq`/`cmle`/`cmlt` `#0`, `fcmeq`/`fcmgt`/`fcmle`/`fcmlt` `#0.0` |
| **NEON shifts** | `sshr`, `ushr`, `srshr`, `urshr`, `ssra`, `usra`, `srsra`, `ursra`, `sri`, `sli`, `shl`, `sqshl`, `uqshl`, `sqshlu` |
| **NEON narrow** | `shrn`/`shrn2`, `rshrn`/`rshrn2`, `sqshrn`/`uqshrn`/`sqrshrn`/`uqrshrn` (+ `2` variants), `sqshrun`/`sqrshrun` (+ `2` variants) |
| **NEON widen/long** | `sshll`/`ushll`/`sxtl`/`uxtl` (+ `2` variants), `smull`/`umull`/`smlal`/`umlal`/`smlsl`/`umlsl`/`saddw`/`uaddw`/`ssubw`/`usubw`/`addhn`/`subhn`/`pmull` (+ `2` variants) |
| **NEON by-element** | `mul`, `mla`, `mls`, `fmul`, `fmla`, `fmls`, `smull`, `umull`, `smlal`, `umlal`, `sqdmulh`, `sqrdmulh` (with lane index) |
| **NEON reduce** | `addv`, `saddlv`, `umaxv`, `uminv`, `smaxv`, `sminv`, `fmaxv`, `fminv`, `fmaxnmv`, `fminnmv` |
| **NEON permute** | `zip1`, `zip2`, `uzp1`, `uzp2`, `trn1`, `trn2`, `ext`, `tbl`, `tbx` |
| **NEON insert/move** | `ins` (element/GPR), `umov`, `smov`, `dup` (element/GPR), `movi`, `mvni` |
| **NEON load/store** | `ld1`/`st1` (1-4 regs), `ld2`/`st2`, `ld3`/`st3`, `ld4`/`st4` (with post-index), `ld1r`/`ld2r`/`ld3r`/`ld4r` (with post-index) |
| **NEON convert** | `fcvtzs`, `fcvtzu`, `scvtf`, `ucvtf`, `fcvtns`, `fcvtms`, `fcvtas`, `fcvtps` (vector forms) |
| **NEON scalar** | `addp` (scalar), `add`/`sub` (d-regs), `sqabs`/`sqneg` (scalar), `sqshrn` (scalar) |
| **NEON crypto** | `aese`, `aesd`, `aesmc`, `aesimc`, `sha1h`, `sha1c`, `sha1m`, `sha1p`, `sha1su0`, `sha1su1`, `sha256h`, `sha256h2`, `sha256su0`, `sha256su1`, `eor3` |
| **System** | `nop`, `yield`, `wfe`, `wfi`, `sev`, `sevl`, `clrex`, `hint`, `bti`, `dc`, `ic`, `tlbi`, `dmb`, `dsb`, `isb`, `mrs`, `msr`, `svc`, `brk` |
| **Bit manipulation** | `clz`, `cls`, `rbit`, `rev`, `rev16`, `rev32` |
| **CRC32** | `crc32b`, `crc32h`, `crc32w`, `crc32x`, `crc32cb`, `crc32ch`, `crc32cw`, `crc32cx` |
| **LSE Atomics** | `cas`/`swp`/`ldadd`/`ldclr`/`ldeor`/`ldset` (with acquire/release/byte/halfword variants), `stadd`/`stclr`/`steor`/`stset` store aliases (with release/byte/halfword variants) |
| **Prefetch** | `prfm` |

### Relocation Types Emitted

When an instruction references an external symbol (e.g., `bl printf` or
`adrp x0, :got:variable`), the encoder returns `WordWithReloc`. The 21
relocation types cover the full AArch64 static-linking relocation model:

| Relocation | ELF Number | Usage |
|-----------|-----------|-------|
| `Call26` | 283 | `bl` (26-bit PC-relative call) |
| `Jump26` | 282 | `b` (26-bit PC-relative jump) |
| `AdrPrelLo21` | 274 | `adr` (21-bit PC-relative) |
| `AdrpPage21` | 275 | `adrp` (page-relative, bits [32:12]) |
| `AddAbsLo12` | 277 | `add :lo12:sym` (low 12 bits) |
| `Ldst8AbsLo12` | 278 | Load/store byte, low 12 |
| `Ldst16AbsLo12` | 284 | Load/store halfword, low 12 |
| `Ldst32AbsLo12` | 285 | Load/store word, low 12 |
| `Ldst64AbsLo12` | 286 | Load/store doubleword, low 12 |
| `Ldst128AbsLo12` | 299 | Load/store quadword, low 12 |
| `AdrGotPage21` | 311 | `adrp` via GOT |
| `Ld64GotLo12` | 312 | `ldr` from GOT entry |
| `TlsLeAddTprelHi12` | 549 | TLS Local Exec, high 12 |
| `TlsLeAddTprelLo12` | 551 | TLS Local Exec, low 12 (no overflow check) |
| `CondBr19` | 280 | Conditional branch, 19-bit |
| `TstBr14` | 279 | Test-and-branch, 14-bit |
| `Ldr19` | 273 | LDR literal, 19-bit PC-relative |
| `Abs64` | 257 | 64-bit absolute |
| `Abs32` | 258 | 32-bit absolute |
| `Prel32` | 261 | 32-bit PC-relative |
| `Prel64` | 260 | 64-bit PC-relative |

### Encoding Approach

1. **Register parsing**: `parse_reg_num()` converts textual register names
   (`x0`-`x30`, `w0`-`w30`, `sp`, `xzr`, `wzr`, `lr`, `d0`-`d31`,
   `s0`-`s31`, `q0`-`q31`, `v0`-`v31`) to 5-bit register numbers.

2. **Size inference**: The `sf` bit (bit 31) is set from the register name
   prefix: `x`/`sp`/`xzr` = 64-bit, `w`/`wsp`/`wzr` = 32-bit.

3. **Condition codes**: 16 codes (`eq`, `ne`, `cs`/`hs`, `cc`/`lo`, ...,
   `al`, `nv`) are mapped to their 4-bit encoding.

4. **Wide immediates**: `mov Xd, #large` first tries single-instruction
   encodings (MOVZ for 0..0xFFFF, MOVN for bitwise-NOT in 16-bit range,
   ORR with bitmask immediate for repeating patterns like 0x0101010101010101),
   then falls back to a `movz`+`movk` sequence (up to 4 instructions for a
   64-bit constant), returned as `EncodeResult::Words`.

5. **MOV special cases**: `mov` to/from `sp` encodes as `add Xd, Xn, #0`;
   register-to-register `mov` encodes as `orr Rd, xzr, Rm`. NEON `mov`
   variants (lane insert, lane extract, element-to-element) are detected by
   operand type and encoded as `INS`/`UMOV`/`ORR` as appropriate.

---

## Stage 3: ELF Object Writer (`elf_writer.rs`)

### Purpose

Walk the `Vec<AsmStatement>`, accumulate section data, build the symbol table,
resolve local branches, and serialize everything into a valid ELF64
relocatable object file.

### Key Data Structures

| Type | Role |
|------|------|
| `ElfWriter` | AArch64-specific ELF writer; composes with `ElfWriterBase` for shared infrastructure. |
| `ElfWriterBase` | Shared state machine from `elf.rs` -- section management, symbols, labels, directive processing, ELF serialization. |
| `ObjSection` | A section being built: name, type, flags, data bytes, alignment, relocation list (from `elf.rs`). |
| `ObjReloc` | A relocation entry: offset, type, symbol name, addend (from `elf.rs`). |
| `PendingReloc` | A deferred branch/local relocation (resolved after all labels are known). |
| `PendingSymDiff` | A deferred symbol-difference expression (e.g., `.long .LBB3 - .Ljt_0`). |
| `PendingExpr` | A deferred complex expression (section, offset, expression string, size). |

### ElfWriter State

```rust
pub struct ElfWriter {
    pub base: ElfWriterBase,              // Shared: sections, labels, symbols, directives
    pending_branch_relocs: Vec<PendingReloc>,  // Branch relocs to resolve at asm time
    pending_sym_diffs: Vec<PendingSymDiff>,     // Deferred A-B expressions
    pending_exprs: Vec<PendingExpr>,            // Deferred complex expressions
}
```

The `ElfWriterBase` (defined in `elf.rs`) holds all shared state: `current_section`,
`sections`, `section_order`, `labels`, `global_symbols`, `weak_symbols`,
`symbol_types`, `symbol_sizes`, `symbol_visibility`, and `aliases`. It also
provides shared methods for section management, directive processing, data
emission, and ELF serialization. This file only adds AArch64-specific branch
resolution and symbol difference handling.

### Processing Algorithm

```text
process_statements(statements):
  for each statement:
    Label(name)        -> record (section, offset) in labels map
    Directive(dir)     -> process_directive():
                           Section   -> ensure_section(), update current_section
                           PushSection -> push current_section onto section_stack, switch
                           PopSection  -> pop section_stack, restore current_section
                           Global    -> mark in global_symbols
                           Weak      -> mark in weak_symbols
                           Hidden/Protected/Internal -> mark visibility
                           SymbolType -> record in symbol_types
                           Size      -> compute and record in symbol_sizes
                           Align/Balign -> pad current section to alignment
                           Byte/Short/Long/Quad -> emit data bytes
                             (Long/Quad with symbols emit relocations)
                           Zero      -> emit fill bytes
                           Asciz/Ascii -> emit string bytes
                           Comm      -> create COMMON symbol
                           Cfi/Ignored -> no-op
    Instruction(m,ops) -> process_instruction():
                           call encode_instruction(m, ops)
                           Word       -> emit 4 bytes
                           WordWithReloc:
                             local (.L*) or branch reloc -> store in pending_branch_relocs
                             other external -> add_reloc() to section
                           Words      -> emit all 4-byte words
                           Skip       -> no-op

  resolve_sym_diffs():   resolve all A-B expressions
    same-section     -> patch data in place
    cross-section    -> emit R_AARCH64_PREL32 or PREL64 relocation (based on data size)

  resolve_local_branches():   resolve all deferred branch targets
    same-section     -> compute PC-relative offset, patch instruction word
      JUMP26/CALL26  -> encode imm26 field
      CONDBR19       -> encode imm19 field
      TSTBR14        -> encode imm14 field
    cross-section    -> emit relocation with section symbol + addend
    undefined symbol -> emit external relocation (symbol name + addend)
```

### ELF File Layout

```text
  +========================+  offset 0
  |  ELF64 Header (64 B)  |  e_machine=EM_AARCH64 (183)
  |                        |  e_type=ET_REL (1)
  +========================+
  |  Section Data          |  .text, .data, .rodata, .bss, etc.
  |  (aligned per section) |  Each section padded to its sh_addralign
  +========================+
  |  .rela sections        |  One per content section with relocations
  |  (8-byte aligned)      |  Each entry: 24 bytes (Elf64_Rela)
  +========================+
  |  .symtab               |  Symbol table (24 bytes per Elf64_Sym)
  |  (8-byte aligned)      |  Order: NULL, section syms, local, global
  +========================+
  |  .strtab               |  Symbol name strings
  +========================+
  |  .shstrtab             |  Section name strings
  +========================+
  |  Section Header Table  |  One Elf64_Shdr per section (64 bytes each)
  |  (8-byte aligned)      |  Order: NULL, content, rela, symtab,
  |                        |         strtab, shstrtab
  +========================+
```

### Symbol Table Construction

`build_symbol_table()` runs just before ELF serialization:

1. **Section symbols**: One `STT_SECTION` / `STB_LOCAL` symbol per content
   section.

2. **Defined symbols**: Every label recorded in `self.labels`, excluding
   `.L*` / `.l*` local labels. Binding is determined from `global_symbols`,
   `weak_symbols`, or defaults to local. Type and size come from
   `symbol_types` and `symbol_sizes`.

3. **Undefined symbols**: Every symbol referenced in relocations that has no
   definition. These get `STB_GLOBAL` binding (or `STB_WEAK` if declared
   `.weak`).

4. **COMMON symbols**: Created by `.comm` directives with `SHN_COMMON` section
   index.

### Local Label and Data Relocation Resolution

Two resolution passes run after all statements are processed:

- **`resolve_local_data_relocs()`**: Rewrites relocations that reference `.L*`
  labels (which will not appear in the symbol table) to instead reference the
  section symbol plus the label's offset as addend. This matches the behavior
  of GCC's assembler.

- **`resolve_sym_diffs()`**: Handles `.long .LA - .LB` and `.quad sym+off - .`
  style expressions. Same-section differences are computed and patched directly.
  Cross-section and external-symbol differences produce `R_AARCH64_PREL32`
  relocations for 4-byte data or `R_AARCH64_PREL64` for 8-byte data. Composite
  symbol names like `sym+offset` are decomposed into a base symbol and numeric addend.

---

## Design Decisions and Trade-offs

### 1. Single-pass parsing, two-pass encoding

Parsing is single-pass and purely syntactic. The ELF writer makes two
logical passes: a forward pass to collect all labels and emit code/data, then
backward resolution passes to fix up local branches and symbol differences.
This avoids the complexity of a full two-pass assembler while handling forward
references correctly.

### 2. Intra-section branch resolution at assembly time

All branch-type relocations (B, BL, B.cond, CBZ/CBNZ, TBZ/TBNZ) are deferred
and resolved after all labels are known. Same-section branches -- whether to
`.L*` local labels or to named labels like `__primary_switch` -- are resolved
by the assembler itself, producing fully-linked instruction words. Only
cross-section or truly external symbol references generate relocations in the
`.o` file. This matches GAS behavior and avoids relying on the linker to
resolve intra-section PC-relative branches.

### 3. No DWARF emission

CFI directives (`.cfi_startproc`, `.cfi_offset`, etc.) are parsed and
silently ignored. The assembler does not emit `.eh_frame` or `.debug_*`
sections. This is acceptable for the compiler's use case but means
stack unwinding and debugger support rely on the external assembler path.

### 4. Deterministic output

Section order is tracked via `section_order: Vec<String>` rather than relying
on `HashMap` iteration order. This ensures identical input always produces
bit-identical output.

### 5. Fixed instruction width simplifies encoding

AArch64's uniform 32-bit instruction encoding means every instruction is
exactly 4 bytes. There is no need for instruction-length calculation or
relaxation passes (unlike x86). The only multi-word output is the
`movz`+`movk` sequence for wide immediates, returned as `EncodeResult::Words`.

---

## File Inventory

| File | Lines | Purpose |
|------|-------|---------|
| `mod.rs` | ~220 | Public API: `assemble()` entry point, GNU numeric label resolution (`1f`/`1b`) |
| `parser.rs` | ~2,600 | Preprocessor (macros, .rept, .irp, conditionals, aliases) and parser: text -> `Vec<AsmStatement>` |
| `encoder/` | ~6,190 | Instruction encoder (split into focused submodules, see below) |
| `elf_writer.rs` | ~580 | ELF object file writer: composes with `ElfWriterBase` (from `elf.rs`), adds AArch64-specific branch resolution and symbol difference handling |
| **Total** | **~9,600** | |

### Encoder Submodules (`encoder/`)

The instruction encoder is organized as a directory of focused submodules:

| File | Lines | Role |
|------|-------|------|
| `mod.rs` | ~973 | `EncodeResult`/`RelocType`/`Relocation` types, register parsing helpers (`parse_reg_num`, `encode_cond`, `get_reg`, `get_imm`, `sf_bit`), `encode_instruction()` dispatch |
| `data_processing.rs` | ~1,067 | Data processing: MOV/MOVZ/MOVK/MOVN, ADD/SUB, logical (AND/ORR/EOR), multiply/divide, carry ops, shifts, extensions, ORN/EON/BICS/BIC |
| `compare_branch.rs` | ~322 | Compare (CMP/CMN/TST), conditional select (CSEL/CSINC/CSINV/CSNEG), branches (B/BL/BR/BLR/RET/CBZ/CBNZ/TBZ/TBNZ), conditional aliases |
| `load_store.rs` | ~884 | Load/store: LDR/STR variants (byte/half/word/double), exclusive/acquire/release, ADRP/ADR, prefetch, LSE atomic operations |
| `fp_scalar.rs` | ~271 | Scalar floating-point: FADD/FSUB/FMUL/FDIV/FSQRT, FCMP, FMA, rounding, conversions (FCVT/SCVTF/UCVTF) |
| `neon.rs` | ~1,854 | NEON/SIMD: three-same, two-misc, float vector, compare-zero, shifts, narrow/widen, by-element, reduce, permute, insert/move, load/store (LD1-LD4/ST1-ST4), crypto (AES/SHA) |
| `system.rs` | ~577 | System instructions: NOP, WFE, WFI, SEV, BTI, DMB, DSB, ISB, DC, IC, TLBI, MRS, MSR, SVC, BRK, CLREX, HINT |
| `bitfield.rs` | ~246 | Bit manipulation: UBFM/SBFM/UBFX/SBFX/UBFIZ/SBFIZ/BFM/BFI, CLZ/CLS, RBIT/REV/REV16/REV32, CRC32 |
