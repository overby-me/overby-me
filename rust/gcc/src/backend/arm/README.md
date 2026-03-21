# AArch64 (ARM64) Backend

The AArch64 backend targets ARM64 with the AAPCS64 calling convention. It
covers the full pipeline from IR to ELF executable: code generation
(instruction selection, register allocation, peephole optimization), a builtin
assembler (GNU assembly syntax parser, fixed-width encoder, ELF object writer),
and a builtin linker (static and dynamic linking, shared library output,
IFUNC/IPLT, TLS support).

## Directory Structure

```text
arm/
  codegen/            Code generation and peephole optimizer
  assembler/          Builtin AArch64 assembler (parser, encoder, ELF writer)
  linker/             Builtin AArch64 linker (static/dynamic linking, IFUNC/TLS)
```

## Sub-Module Documentation

| Module | README |
|--------|--------|
| Code generation | [`codegen/README.md`](codegen/README.md) |
| Assembler | [`assembler/README.md`](assembler/README.md) |
| Linker | [`linker/README.md`](linker/README.md) |

## Key Characteristics

- **ABI**: AAPCS64 -- 8 GP argument registers, 8 FP/SIMD argument registers,
  F128 in Q registers, I128 in aligned register pairs
- **Accumulator model**: Values flow through `x0`; callee-saved registers
  (`x20`-`x28`) and two caller-saved registers (`x13`, `x14`) reduce stack
  traffic
- **F128 (long double)**: IEEE binary128 via soft-float library calls
  (`__addtf3`, `__multf3`, etc.) through NEON Q registers
- **NEON intrinsics**: SSE-equivalent 128-bit vector operations via NEON
  instructions
- **Atomics**: ARMv8 exclusive monitor (LDXR/STXR) loops
- **Peephole optimizer**: 3-phase pipeline (iterative local, global copy
  propagation + dead store elimination, local cleanup)
- **Assembler**: GNU assembly syntax, fixed 32-bit encoding, macro/conditional
  preprocessor, ~400 base mnemonics
- **Linker**: Static and dynamic linking, shared library (`.so`) output,
  IFUNC/IPLT with IRELATIVE relocations, TLS (LE, IE via GOT, TLSDESC and
  GD relaxation to LE)
