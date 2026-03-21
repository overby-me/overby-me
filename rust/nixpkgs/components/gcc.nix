# GCC: gcc-wrapper → rust-gcc
#
# rust-gcc is a GCC-compatible C compiler written in Rust (based on
# Anthropic's ccc). It includes a built-in assembler, linker, and
# preprocessor, targeting x86-64, i686, ARM64, and RISC-V.
#
# Note: This component is NOT in initialPath — it replaces stdenv.cc
# which is part of defaultNativeBuildInputs. The overlay handles the
# wrapping via wrapCCWith.
{
  pkgs,
  mkComponent,
  status,
  source,
  ...
}:
mkComponent {
  name = "gcc";
  original = pkgs.stdenv.cc;
  replacement = pkgs.rust-gcc;
  status = status.available;
  source = source.repo;
  phase = 1;
  description = "GCC-compatible C compiler";
  notes = "Rust rewrite at rust/gcc (based on Anthropic's ccc, CC0)";
}
