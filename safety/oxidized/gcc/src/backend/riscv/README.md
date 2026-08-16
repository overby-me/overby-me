# RISC-V 64-bit Backend

The RISC-V backend targets RV64GC (RV64IMAFDC) with the LP64D calling
convention. It covers the full pipeline from IR to ELF executable: code
generation (instruction selection, register allocation, peephole optimization),
a builtin assembler (RV assembly syntax parser, instruction encoder, ELF object
writer), and a builtin linker (static and dynamic linking, shared library
output, TLS support).

## Directory Structure

```text
riscv/
  codegen/            Code generation and peephole optimizer
  assembler/          Builtin RV64 assembler (parser, encoder, ELF writer)
  linker/             Builtin RV64 linker (static/dynamic linking, TLS)
```

## Sub-Module Documentation

| Module | README |
|--------|--------|
| Code generation | [`codegen/README.md`](codegen/README.md) |
| Assembler | [`assembler/README.md`](assembler/README.md) |
| Linker | [`linker/README.md`](linker/README.md) |

## Key Characteristics

- **ABI**: LP64D -- 8 GP argument registers, 8 FP argument registers,
  hardware float struct classification, I128/F128 in GP register pairs
- **Accumulator model**: Values flow through `t0`; up to 11 callee-saved
  registers (`s1`, `s2`-`s11`) available for allocation
- **F128 (long double)**: IEEE binary128 via soft-float library calls through
  GP register pairs (`a0:a1`)
- **Software SIMD**: SSE-equivalent 128-bit vector operations emulated with
  scalar RISC-V instructions
- **Software builtins**: CLZ, CTZ, BSWAP, POPCOUNT implemented in software
  (no Zbb extension dependency)
- **Atomics**: AMO instructions for word/doubleword; LR/SC with bit masking
  for sub-word atomics
- **Assembler**: RV assembly syntax, all RV64IMAFDC instructions, macro and
  conditional preprocessor
- **Linker**: Static and dynamic linking, shared library (`.so`) output,
  PLT/GOT generation, TLS (LE, IE, GD→LE relaxation for static binaries)
