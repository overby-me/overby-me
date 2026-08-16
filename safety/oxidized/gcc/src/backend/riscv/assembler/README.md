# RISC-V 64-bit Assembler -- Design Document

## Overview

This module implements a complete, self-contained assembler for the RISC-V 64-bit
ISA (RV64GC). It translates textual assembly source -- as emitted by the compiler's
code-generation backend -- into relocatable ELF object files (`.o`). The assembler
is designed to be invoked in-process (no fork/exec of an external tool), which
removes a hard dependency on a host `as` binary and dramatically improves
compilation latency on cross-compilation setups.

The assembler supports the full RV64I base integer ISA, the M (multiply/divide),
A (atomics), F (single-precision float), D (double-precision float),
C (compressed 16-bit), Zbb (basic bit manipulation), and V (vector) standard extensions,
plus Zvksh/Zvksed vector crypto. It handles all standard assembler directives,
pseudo-instructions, relocation modifiers, numeric local labels, macro expansion,
and conditional assembly.

### Capabilities at a glance

- Full RV64IMAFDCV + Zbb + Zvksh/Zvksed instruction encoding
- 40+ pseudo-instructions (li, la, call, tail, mv, not, negw, seqz, ...)
- All standard assembler directives (.text, .data, .globl, .align, .byte, ...)
- Preprocessor: `.macro/.endm`, `.rept/.irp/.endr`, `.if/.else/.endif`
- Relocation modifier parsing (%pcrel_hi, %pcrel_lo, %hi, %lo, %tprel_*, %got_pcrel_hi, ...)
- RV64C compression pass (currently disabled; linker relaxation preferred)
- Numeric local labels (1:, 1b, 1f) with forward/backward reference resolution
- Correct ELF object file emission with .symtab, .strtab, .rela.* sections

## Architecture / Pipeline

```text
                          Assembly source (text)
                                  |
                                  v
                     +----------------------------+
                     | Preprocessor               |
                     |   (asm_preprocess.rs,       |
                     |    shared across backends)  |
                     |  - Strip C-style comments   |
                     |  - Expand .macro/.endm      |
                     |  - Expand .rept/.irp/.endr  |
                     +----------------------------+
                                  |
                                  v
                     +----------------------------+
                     |     Parser (parser.rs)      |
                     |  - Tokenize lines           |
                     |  - Parse operands           |
                     |  - Recognize directives     |
                     |  - Evaluate .if/.else/.endif|
                     +----------------------------+
                                  |
                         Vec<AsmStatement>
                                  |
                                  v
                     +----------------------------+
                     |   Encoder (encoder/)        |
                     |  - Map mnemonic to ISA      |
                     |  - Encode R/I/S/B/U/J       |
                     |  - Expand pseudos            |
                     |  - Emit relocations          |
                     +----------------------------+
                                  |
                       EncodeResult + Relocation
                                  |
                                  v
                     +----------------------------+
                     | ELF Writer (elf_writer.rs)  |
                     |  - Section management       |
                     |  - Label/symbol tracking     |
                     |  - Directive execution       |
                     |  - Branch reloc resolution   |
                     |  - ELF64 serialization       |
                     +----------------------------+
                                  |
                                  v
                         Relocatable ELF .o file
```

Note: A compression pass (`compress.rs`) exists and can rewrite eligible 32-bit
instructions to 16-bit RVC equivalents, but it is currently disabled because
the linker handles relaxation via `R_RISCV_RELAX` and running our own
compression would conflict with the linker's relaxation pass.

## File Inventory

| File            | Lines  | Role                                                    |
|-----------------|--------|---------------------------------------------------------|
| `mod.rs`        | ~100   | Public `assemble_with_args()` entry point; orchestrates parser → ELF writer pipeline, handles `-mabi=` flag for float ABI selection and `-march=` for RV32/RV64 and RVC detection |
| `parser.rs`     | ~1060  | Line tokenizer and operand parser; splits assembly text into `AsmStatement` records, evaluates `.if/.else/.endif` conditionals |
| `encoder/`      | ~2670  | Instruction encoder (split into focused submodules, see below) |
| `compress.rs`   | ~850   | Post-encoding RV64C compression pass; rewrites eligible 32-bit instructions to 16-bit compressed equivalents (currently disabled) |
| `elf_writer.rs` | ~1410  | ELF object file builder; composes with `ElfWriterBase` (from shared `elf` module) for section/symbol management, adds RISC-V-specific pcrel_hi/lo pairing, branch resolution, numeric label handling, and ELF serialization |

### Encoder Submodules (`encoder/`)

The instruction encoder is organized as a directory of focused submodules:

| File | Lines | Role |
|------|-------|------|
| `mod.rs` | ~926 | `EncodeResult`/`RelocType`/`Relocation` types, register encoding (`encode_reg`), format encoders (`encode_r/i/s/b/u/j`), opcode constants, `encode_instruction()` dispatch |
| `base.rs` | ~320 | RV64I base integer instructions (R/I/S/B/U/J-type) plus Zbb bit-manipulation extension |
| `atomics.rs` | ~106 | A-extension: LR/SC (load-reserved/store-conditional), AMO (atomic memory operations) |
| `system.rs` | ~115 | System instructions: ECALL, EBREAK, FENCE, FENCE.I, CSR read/write/set/clear |
| `float.rs` | ~191 | F/D floating-point extensions: arithmetic, comparisons, conversions, FMA, sign injection, classify, load/store |
| `pseudo.rs` | ~608 | Pseudo-instruction expansion: LI (large immediates), LA/LLA, CALL/TAIL, branch aliases, CSR shorthands, FP move/abs/neg, relocation modifier parsing |
| `compressed.rs` | ~196 | RVC compressed 16-bit instructions and `.insn` raw instruction directive |
| `vector.rs` | ~210 | RVV vector extension and Zvksh/Zvksed vector crypto instructions |

## Key Data Structures

### `ElfWriter` (elf_writer.rs)

The ELF writer composes with `ElfWriterBase` (from the shared `elf` module) for
common infrastructure and adds RISC-V-specific logic.

```text
ElfWriter {
    pub base:              ElfWriterBase,               // Shared: sections, labels, symbols, directives
    pending_branch_relocs: Vec<PendingReloc>,           // unresolved local branches
    pcrel_hi_counter:      u32,                         // counter for synthetic .Lpcrel_hi labels
    numeric_labels:        HashMap<String, Vec<(String, u64)>>,  // "1" -> [(sec, off), ...]
    deferred_exprs:        Vec<DeferredExpr>,            // forward label data expressions (.word sym - .)
    elf_flags:             u32,                          // ELF e_flags (default: RVC | FLOAT_ABI_DOUBLE)
    elf_class:             u8,                           // ELFCLASS64 (default) or ELFCLASS32 for RV32
    no_relax:              bool,                         // suppress R_RISCV_RELAX (via .option norelax)
}
```

The `ElfWriterBase` (defined in `elf/writer_base.rs`) holds all shared state:
`current_section`, `sections` (as `HashMap<String, ObjSection>`),
`section_order`, `labels`, `global_symbols`, `weak_symbols`, `symbol_types`,
`symbol_sizes`, `symbol_visibility`, `aliases`, and section push/pop stacks.
It provides shared methods for section management, directive processing, data
emission, and ELF serialization. It is also used by the ARM assembler.

### `ObjSection` (elf/object_writer.rs)

Represents a single ELF section being built (shared across ARM and RISC-V).

```text
ObjSection {
    name:           String,           // section name
    data:           Vec<u8>,          // accumulated bytes
    sh_type:        u32,              // SHT_PROGBITS, SHT_NOBITS, ...
    sh_flags:       u64,              // SHF_ALLOC | SHF_WRITE | SHF_EXECINSTR
    sh_addralign:   u64,              // required alignment
    relocs:         Vec<ObjReloc>,    // pending relocations for this section
}
```

### `AsmStatement` (parser.rs)

The parser's output. Each source line produces one `AsmStatement`.

```text
AsmStatement::Label(String)                           // label definition: "name:"
AsmStatement::Directive(Directive)                    // typed assembler directive
AsmStatement::Instruction { mnemonic, operands, raw_operands }  // RISC-V instruction
AsmStatement::Empty                                   // blank line / comment-only
```

The `Directive` enum has ~30 variants covering all recognized directives
(section switches, data emission, symbol attributes, alignment, etc.).

### `Operand` (parser.rs)

A tagged union covering every operand form the parser recognizes.

```text
Operand::Reg(String)                                  // register: "x5", "a0", "sp", "fa3"
Operand::Imm(i64)                                     // immediate: 42, -7, 0xff
Operand::Symbol(String)                               // symbol reference: "printf", ".LC0"
Operand::SymbolOffset(String, i64)                    // symbol + addend: "sym+8"
Operand::Mem { base, offset }                         // memory: 8(sp), 0(a0)
Operand::MemSymbol { base, symbol, modifier }         // memory with reloc: %lo(sym)(a0)
Operand::Label(String)                                // branch target label
Operand::FenceArg(String)                             // fence operand: "iorw"
Operand::Csr(String)                                  // CSR register name or number
Operand::RoundingMode(String)                         // rne, rtz, rdn, rup, rmm, dyn
```

### `EncodeResult` (encoder/mod.rs)

The encoder returns one of several result variants depending on the instruction
class.

```text
EncodeResult::Word(u32)                               // single 32-bit instruction
EncodeResult::Half(u16)                               // 16-bit compressed instruction
EncodeResult::Words(Vec<u32>)                         // multi-word sequence (e.g., li with large imm)
EncodeResult::WordWithReloc { word, reloc }           // instruction + relocation
EncodeResult::WordsWithRelocs(Vec<(u32, Option<Reloc>)>)  // multi-word + relocs (e.g., call = auipc+jalr)
EncodeResult::Skip                                    // pseudo handled elsewhere (no output)
```

### `Relocation` / `RelocType` (encoder/mod.rs)

```text
Relocation {
    reloc_type: RelocType,     // semantic relocation kind
    symbol:     String,        // target symbol name
    addend:     i64,           // constant addend
}

RelocType::Branch          -> R_RISCV_BRANCH       (B-type, 12-bit PC-rel)
RelocType::Jal             -> R_RISCV_JAL          (J-type, 20-bit PC-rel)
RelocType::CallPlt         -> R_RISCV_CALL_PLT     (AUIPC+JALR pair)
RelocType::PcrelHi20       -> R_RISCV_PCREL_HI20
RelocType::PcrelLo12I      -> R_RISCV_PCREL_LO12_I
RelocType::PcrelLo12S      -> R_RISCV_PCREL_LO12_S
RelocType::Hi20            -> R_RISCV_HI20
RelocType::Lo12I           -> R_RISCV_LO12_I
RelocType::Lo12S           -> R_RISCV_LO12_S
RelocType::TprelHi20       -> R_RISCV_TPREL_HI20
RelocType::TprelLo12I      -> R_RISCV_TPREL_LO12_I
RelocType::TprelLo12S      -> R_RISCV_TPREL_LO12_S
RelocType::TprelAdd        -> R_RISCV_TPREL_ADD
RelocType::GotHi20         -> R_RISCV_GOT_HI20
RelocType::TlsGdHi20       -> R_RISCV_TLS_GD_HI20
RelocType::TlsGotHi20      -> R_RISCV_TLS_GOT_HI20
RelocType::Abs32           -> R_RISCV_32
RelocType::Abs64           -> R_RISCV_64
RelocType::Add16           -> R_RISCV_ADD16   (16-bit symbol difference, e.g. .2byte)
RelocType::Sub16           -> R_RISCV_SUB16
RelocType::Add32           -> R_RISCV_ADD32   (32-bit symbol difference, e.g. .4byte)
RelocType::Sub32           -> R_RISCV_SUB32
RelocType::Add64           -> R_RISCV_ADD64   (64-bit symbol difference, e.g. .8byte)
RelocType::Sub64           -> R_RISCV_SUB64
```

## Processing Algorithm Step by Step

### Step 1: Preprocessing (`asm_preprocess.rs`)

Before parsing, the shared `asm_preprocess` module runs several expansion passes
over the raw assembly text:

1. Strips C-style block comments (`/* ... */`) and line comments.
2. Expands `.macro` / `.endm` definitions and invocations.
3. Expands `.rept` / `.irp` / `.endr` repetition blocks.

These passes are shared across all assembler backends (RISC-V, ARM, x86).

### Step 2: Parsing (`parser.rs`)

The parser processes the preprocessed source line by line. For each line it:

1. Detects a leading label (any identifier followed by `:`). Numeric labels
   such as `1:` are recognized as re-definable local labels.
2. Identifies the mnemonic -- either an instruction name (`add`, `ld`, `beq`)
   or a directive (`.text`, `.globl`, `.quad`).
3. Evaluates `.if` / `.else` / `.endif` conditional assembly blocks, skipping
   lines inside false conditions.
4. Parses the operand list, recognizing:
   - Integer and FP register names (x0-x31, a0-a7, s0-s11, t0-t6, f0-f31,
     fa0-fa7, fs0-fs11, ft0-ft11), including ABI aliases.
   - Immediates in decimal, hex (`0x`), octal (`0`), and binary (`0b`).
   - Memory references in the form `offset(base)`.
   - Relocation modifiers: `%pcrel_hi(sym)`, `%lo(sym)`, `%tprel_add(sym)`, etc.
   - Bare symbol references and label references (including numeric `1b`, `1f`).
   - CSR names, fence arguments, and FP rounding modes.

### Step 3: Directive Execution (`elf_writer.rs`)

The ELF writer iterates over parsed statements and handles directives inline:

| Directive              | Effect                                           |
|------------------------|--------------------------------------------------|
| `.text` / `.data` / `.bss` / `.rodata` / `.section` | Switch or create a section |
| `.globl` / `.global`  | Mark symbol as globally visible                   |
| `.local`              | Mark symbol as local binding                       |
| `.weak`               | Mark symbol as weak binding                        |
| `.hidden` / `.protected` / `.internal` | Set symbol visibility              |
| `.type`               | Set symbol type (function/object/tls)              |
| `.size`               | Set symbol size                                    |
| `.byte` / `.short` / `.half` / `.word` / `.long` / `.quad` / `.8byte` | Emit literal data |
| `.zero` / `.space`    | Emit N zero bytes (with optional fill value)       |
| `.string` / `.asciz` / `.ascii` | Emit string data (with/without NUL)     |
| `.align` / `.balign` / `.p2align` | Pad to alignment boundary              |
| `.equ` / `.set`       | Define a symbol with a constant value              |
| `.comm`               | Reserve common storage                             |
| `.pushsection` / `.popsection` / `.previous` | Push/pop section stack     |
| `.option push/pop/rvc/norvc/norelax` | Control RVC compression and relaxation |
| `.insn`               | Emit a raw instruction encoding                    |
| `.incbin`             | Include binary file contents                       |
| `.attribute` / `.file` / `.ident` | Metadata / ignored                      |
| `.cfi_*`              | Silently consumed (CFI info not emitted)           |
| `.addrsig` / `.addrsig_sym` | Address-significance (silently consumed)    |

### Step 4: Instruction Encoding (`encoder/`)

For each instruction mnemonic, the encoder:

1. Looks up the mnemonic in a master dispatch table. The table covers:
   - **R-type**: add, sub, sll, slt, sltu, xor, srl, sra, or, and, mul, div,
     rem, addw, subw, sllw, srlw, sraw, mulw, divw, remw, plus all
     variants (unsigned, word-width).
   - **I-type**: addi, slti, sltiu, xori, ori, andi, slli, srli, srai,
     addiw, slliw, srliw, sraiw, lb/lh/lw/ld/lbu/lhu/lwu, jalr.
   - **S-type**: sb, sh, sw, sd.
   - **B-type**: beq, bne, blt, bge, bltu, bgeu.
   - **U-type**: lui, auipc.
   - **J-type**: jal.
   - **Atomics (A-extension)**: lr.w/d, sc.w/d, amo{swap,add,and,or,xor,min,max,minu,maxu}.w/d
   - **Floating-point (F/D)**: fadd/fsub/fmul/fdiv/fsqrt, fmin/fmax,
     fcvt (all int/float conversions), fmv.x.w/d, fmv.w.x/d.x,
     fmadd/fmsub/fnmadd/fnmsub, feq/flt/fle, fclass, flw/fld/fsw/fsd,
     fsgnj/fsgnjn/fsgnjx.
   - **System**: ecall, ebreak, fence, fence.i, csrr/csrw/csrs/csrc and
     their immediate variants (csrwi, csrsi, csrci).

2. Expands pseudo-instructions into their real instruction sequences:

   | Pseudo         | Expansion                                            |
   |----------------|------------------------------------------------------|
   | `li rd, imm`   | `lui + addi(w)` or single `addi`, up to 3-instruction sequences for 64-bit constants |
   | `mv rd, rs`    | `add rd, x0, rs` (uses ADD form for RV64C eligibility)|
   | `not rd, rs`   | `xori rd, rs, -1`                                    |
   | `neg rd, rs`   | `sub rd, x0, rs`                                     |
   | `negw rd, rs`  | `subw rd, x0, rs`                                    |
   | `sext.w rd, rs`| `addiw rd, rs, 0`                                    |
   | `seqz rd, rs`  | `sltiu rd, rs, 1`                                    |
   | `snez rd, rs`  | `sltu rd, x0, rs`                                    |
   | `sltz rd, rs`  | `slt rd, rs, x0`                                     |
   | `sgtz rd, rs`  | `slt rd, x0, rs`                                     |
   | `beqz/bnez`    | `beq/bne rs, x0, label`                              |
   | `blez/bgez/...`| Corresponding `bge`/`blt` with x0                    |
   | `bgt/ble/bgtu/bleu` | Swapped-operand `blt`/`bge` variants            |
   | `j label`      | `jal x0, label`                                      |
   | `jr rs`        | `jalr x0, 0(rs)`                                     |
   | `ret`          | `jalr x0, 0(ra)`                                     |
   | `call sym`     | `auipc ra, %pcrel_hi(sym)` + `jalr ra, %pcrel_lo(sym)(ra)` |
   | `tail sym`     | `auipc t1, %pcrel_hi(sym)` + `jalr x0, %pcrel_lo(sym)(t1)` |
   | `la rd, sym`   | `auipc rd, ...` + `addi rd, rd, ...` (pcrel pair)    |
   | `lla rd, sym`  | Same as `la` (non-PIC)                                |
   | `nop`          | `addi x0, x0, 0`                                     |
   | `fmv.s/d`      | `fsgnj.s/d rd, rs, rs`                               |
   | `fabs.s/d`     | `fsgnjx.s/d rd, rs, rs`                              |
   | `fneg.s/d`     | `fsgnjn.s/d rd, rs, rs`                              |
   | `rdcycle/rdtime/rdinstret` | `csrrs rd, csr, x0`                    |
   | `csrr/csrw/csrs/csrc` | Expanded `csrrs`/`csrrw`/`csrrc` forms         |

3. Produces an `EncodeResult` containing the machine code bytes and any
   relocations required for symbol references.

The encoding functions for each instruction format follow the RISC-V ISA
specification exactly:

```text
R-type:  [funct7 | rs2 | rs1 | funct3 |  rd  | opcode]
I-type:  [    imm[11:0]  | rs1 | funct3 |  rd  | opcode]
S-type:  [imm[11:5]| rs2 | rs1 | funct3 | imm[4:0] | opcode]
B-type:  [imm[12|10:5] | rs2 | rs1 | funct3 | imm[4:1|11] | opcode]
U-type:  [          imm[31:12]           |  rd  | opcode]
J-type:  [imm[20|10:1|11|19:12]         |  rd  | opcode]
```

### Step 5: Section Data Accumulation (`elf_writer.rs`)

As instructions are encoded, the ELF writer appends the resulting bytes to the
current section's data buffer. For instructions with relocations:

- **Intra-section branches** (same section, label already defined or to be
  defined): recorded as `PendingReloc` entries for later resolution.
- **External symbol references**: recorded as `ObjReloc` entries in the
  section's relocation list, to be emitted as `.rela.*` sections.

For multi-word expansions (e.g., `call` emitting AUIPC+JALR), the assembler
generates synthetic labels (`.Lpcrel_hiN`) so that `%pcrel_lo` relocations can
reference the AUIPC's PC, as required by the RISC-V ABI.

### Step 6: Local Branch Resolution (`elf_writer.rs` -- `resolve_local_branches`)

Before ELF emission, the assembler resolves all pending intra-section branch
relocations:

1. For each `PendingReloc`, it looks up the target label in the label table.
2. If the target is in the same section, it computes the PC-relative offset
   and patches the instruction word directly in the section data buffer,
   encoding the offset into the appropriate bit fields for the relocation type:
   - **R_RISCV_BRANCH** (B-type): 12-bit signed offset, bit-scattered
   - **R_RISCV_JAL** (J-type): 20-bit signed offset, bit-scattered
   - **R_RISCV_CALL_PLT**: patches both AUIPC (hi20) and JALR (lo12)
   - **R_RISCV_PCREL_HI20**: patches AUIPC upper 20 bits
   - **R_RISCV_PCREL_LO12_I/S**: patches load/store lower 12 bits
3. If the target is in a different section or undefined, the relocation is
   promoted to an external ELF relocation for the linker to resolve.

### Step 7: ELF Object Emission (`elf_writer.rs`)

The final step serializes the assembled state into a conformant ELF64 relocatable
object file. The layout is:

```text
+----------------------------------+  offset 0
|  ELF Header (64 bytes)           |
|  - e_machine = EM_RISCV (243)    |
|  - e_flags = FLOAT_ABI_DOUBLE    |
|             | RVC                 |
+----------------------------------+
|  Section data                    |
|  (.text, .data, .rodata, .bss,   |
|   .sdata, .init_array, etc.)     |
|  (each aligned per sh_addralign) |
+----------------------------------+
|  .rela.text (relocation entries) |
|  .rela.data                      |
|  (24 bytes per entry: ELF64_Rela)|
+----------------------------------+
|  .symtab (symbol table)          |
|  (24 bytes per entry: ELF64_Sym) |
|  Ordering: NULL, section syms,   |
|            local syms, global    |
+----------------------------------+
|  .strtab (symbol string table)   |
+----------------------------------+
|  .shstrtab (section name strings)|
+----------------------------------+
|  Section header table            |
|  (64 bytes per header: Elf64_Shdr|
|  NULL, content sections,         |
|  .rela.*, .symtab, .strtab,      |
|  .shstrtab)                      |
+----------------------------------+
```

The writer performs several bookkeeping tasks:

- **Symbol table construction**: Local labels (`.L*`) are included only if
  they are referenced by a relocation (e.g., synthetic `%pcrel_lo` labels).
  Section symbols are emitted for every content section. The `sh_info` field
  of `.symtab` is set to the index of the first global symbol, per ELF spec.

- **Relocation entries**: Each `ObjReloc` is serialized as an `Elf64_Rela`
  entry (offset, r_info = symbol_index << 32 | type, addend). A companion
  `R_RISCV_RELAX` relocation is emitted alongside `PCREL_HI20`, `CALL_PLT`,
  `TPREL_*`, and `GOT_HI20` relocations to allow the linker to perform
  relaxation optimizations (unless suppressed by `.option norelax`).
  Additionally, `R_RISCV_ALIGN` relocations are emitted at `.align`,
  `.balign`, and `.p2align` directives when relaxation is enabled and the
  current section is executable (`SHF_EXECINSTR`). These mark NOP padding
  regions so the linker can re-pad after relaxation shrinks preceding
  instructions, maintaining correct alignment. Data sections use static
  zero-padding for alignment without relocations.

- **ELF flags**: Default is `EF_RISCV_FLOAT_ABI_DOUBLE | EF_RISCV_RVC` (0x05).
  The float ABI can be overridden via the `-mabi=` flag passed to
  `assemble_with_args()` (lp64/lp64f/lp64d/lp64q).

## Key Design Decisions and Trade-offs

### 1. Post-encoding compression vs. direct compressed emission

The assembler first encodes all instructions as 32-bit words. A separate
compression pass (`compress.rs`) can then scan for eligible instructions and
rewrite them as 16-bit RVC equivalents. This two-phase approach is simpler than
trying to emit compressed instructions inline during encoding, because:

- The compressor can examine each fully-formed 32-bit encoding and make a
  binary yes/no decision. The encoder does not need to be aware of RVC
  constraints at all.
- Relocation offset adjustment is localized to a single pass rather than
  being spread throughout the encoder.
- The approach is trivially correct: removing the compression pass produces
  a valid (if larger) object file.

**Current status:** The compression pass is disabled. The linker handles
relaxation via `R_RISCV_RELAX` hints, and running assembler-side compression
would change code layout in ways the linker's relaxation pass doesn't expect.
The compressor code is retained for potential future use.

### 2. Eager local branch resolution

Branches to labels within the same section are resolved immediately in the
assembler (before ELF emission), rather than being emitted as relocations for
the linker. This reduces the number of relocations the linker must process and
produces smaller object files. The linker only sees cross-section and
cross-module symbol references.

### 3. Synthetic labels for PCREL_LO12

The RISC-V ABI requires that `%pcrel_lo` relocations reference the *AUIPC
instruction's address*, not the symbol directly. The assembler generates
synthetic labels (`.Lpcrel_hiN`) at each AUIPC site and makes the corresponding
LO12 relocation reference that label. The `build_symbol_table` pass in the
ELF writer ensures these synthetic labels appear in `.symtab` whenever they
are referenced by a `.rela.*` entry.

### 4. Numeric local labels

Numeric labels (`1:`, `2:`, etc.) can be redefined multiple times. Forward
references (`1f`) resolve to the *next* definition; backward references (`1b`)
resolve to the *most recent* definition. During preprocessing, all numeric label
references are rewritten to unique synthetic names (`.Lnum_N_I`) so that
the rest of the pipeline can treat them as ordinary labels.

### 5. In-process execution

The assembler runs entirely in-process, sharing the compiler's address space.
There is no serialization to text and back, no fork/exec of a system assembler.
This means:

- No dependency on a host RISC-V cross-assembler being installed.
- Faster compilation: no process spawning overhead.
- The compiler controls the exact assembly dialect and can rely on features
  without worrying about toolchain version skew.

### 6. Shared infrastructure

The `ElfWriterBase` (in the `elf` module) and `asm_preprocess` module are shared between
the RISC-V and ARM assembler backends. This avoids duplicating section management,
symbol table construction, ELF serialization, macro expansion, and repetition
block handling. Each backend composes with the shared base and adds
architecture-specific instruction encoding, branch resolution, and relocation
logic.

### 7. No linker relaxation in the assembler

The assembler emits `R_RISCV_RELAX` hints alongside eligible relocations but
does not perform any relaxation itself. Relaxation (e.g., converting a
`lui+addi` pair to a single `addi` when the symbol is close to GP) is
intentionally left to the linker, which has full address layout information.
The assembler's job is to produce conservative, correct encodings.
