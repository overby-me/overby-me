# i686 Backend -- 32-bit x86

The i686 backend targets 32-bit x86 (IA-32) with the System V i386 ABI
(cdecl). It covers the full pipeline from IR to ELF executable: code
generation (instruction selection, register allocation, peephole
optimization), a builtin assembler (reuses the x86-64 AT&T parser with a
32-bit encoder, producing ELFCLASS32 objects), and a builtin linker
(32-bit ELF, `.rel` relocations, dynamic or static linking).

## Directory Structure

```text
i686/
  codegen/            Code generation and peephole optimizer
  assembler/          Builtin i686 assembler (shared x86 parser, 32-bit encoder)
  linker/             Builtin i686 linker (32-bit ELF, R_386 relocations)
```

## Sub-Module Documentation

| Module | README |
|--------|--------|
| Code generation | [`codegen/README.md`](codegen/README.md) |
| Assembler | [`assembler/README.md`](assembler/README.md) |
| Linker | [`linker/README.md`](linker/README.md) |

## Key Characteristics

- **ABI**: cdecl (all arguments on stack), with `-mregparm=N` and fastcall
  support
- **ILP32 type model**: Pointers are 4 bytes, `long` is 4 bytes,
  `long long` requires `eax:edx` register pairs
- **Accumulator model**: Values flow through `%eax`; only 3 callee-saved
  registers (`ebx`, `esi`, `edi`) plus `ebp` with `-fomit-frame-pointer`
- **64-bit splitting**: 64-bit arithmetic splits across `eax:edx` pairs
  with carry/borrow propagation; 64-bit values stored in 8-byte stack slots
- **F128 (long double)**: Native x87 80-bit extended precision in 12-byte
  stack slots
- **PIC mode**: GOT-relative addressing via `%ebx` as GOT base register
- **Assembler**: Reuses x86-64 AT&T parser, 32-bit instruction encoding
- **Linker**: Dynamic and static linking with PLT/GOT, 32-bit ELF,
  `.rel` (not `.rela`)
- **`-m16` / `.code16gcc`**: Supports 16-bit real-mode code generation for
  Linux kernel early boot
