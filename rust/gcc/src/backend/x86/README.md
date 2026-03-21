# x86-64 Backend

The x86-64 backend targets AMD64 with the System V AMD64 ABI. It covers the
full pipeline from IR to ELF executable or shared library: code generation
(instruction selection, register allocation, peephole optimization), a builtin
assembler (AT&T syntax parser, multi-format instruction encoder, ELF object
writer), and a builtin linker (PLT/GOT, TLS, copy relocations, shared library
output).

## Directory Structure

```text
x86/
  codegen/            Code generation and peephole optimizer  (~6,300 lines)
    peephole/         Multi-phase post-codegen assembly optimizer  (~6,200 lines)
  assembler/          Builtin x86-64 assembler  (~8,600 lines)
  linker/             Builtin x86-64 linker  (~3,200 lines)
```

## Sub-Module Documentation

| Module | README |
|--------|--------|
| Code generation | [`codegen/README.md`](codegen/README.md) |
| Peephole optimizer | [`codegen/peephole/README.md`](codegen/peephole/README.md) |
| Assembler | [`assembler/README.md`](assembler/README.md) |
| Linker | [`linker/README.md`](linker/README.md) |

## Pipeline Overview

```text
IR Function
    |
    v
Code generation (codegen/)     -- IR -> AT&T assembly text
    |
    v
Peephole optimizer (peephole/) -- assembly text optimization
    |
    v
Assembler (assembler/)         -- AT&T text -> ELF .o
    |
    v
Linker (linker/)               -- ELF .o files -> executable or .so
```

## Key Characteristics

- **ABI**: System V AMD64 -- 6 GP argument registers, 8 XMM argument
  registers, per-eightbyte struct classification
- **Accumulator model**: Most values flow through `%rax`; a linear-scan
  register allocator assigns hot values to callee-saved registers
  (`rbx`, `r12`-`r15`) and caller-saved registers (`r8`-`r11`, `rdi`, `rsi`)
- **F128 (long double)**: Native x87 80-bit extended precision via
  `fldt`/`fstpt`
- **Peephole optimizer**: 15 passes in 7 phases (local pattern matching,
  global store forwarding / copy propagation / dead code elimination,
  loop trampoline coalescing, tail call optimization, never-read store
  elimination, callee-save elimination, frame compaction)
- **Assembler**: Full AT&T syntax with REX, VEX, and EVEX prefix formats;
  covers SSE through SSE4.1, AVX/AVX2, initial AVX-512, BMI2, AES-NI,
  PCLMULQDQ, CRC32, and x87 FPU
- **Linker**: Produces dynamically-linked executables and shared libraries
  (.so); PLT/GOT with lazy binding, TLS (IE-to-LE relaxation), GOT-to-LEA
  relaxation, copy relocations, GNU symbol versioning
