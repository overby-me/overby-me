# rust-gcc

A GCC-compatible C compiler written in Rust, targeting x86-64, i686, ARM64, and RISC-V.

Based on [Claude's C Compiler (ccc)](https://github.com/anthropics/claudes-c-compiler) by Anthropic.

## Binaries

| Binary | Description |
|--------|-------------|
| `gcc` | Default (auto-detects target) |
| `cc` | Symlink to `gcc` |
| `gcc-x86` | x86-64 target |
| `gcc-i686` | i686 (32-bit x86) target |
| `gcc-arm` | ARM64/AArch64 target |
| `gcc-riscv` | RISC-V target |

## Usage in rust-nixpkgs

This package is intended as a drop-in GCC replacement for the Rust stdenv in nixpkgs. It includes a built-in assembler and linker, removing the dependency on GNU binutils for many packages.

## License

CC0 1.0 Universal (Public Domain) — see [LICENSE](LICENSE).
