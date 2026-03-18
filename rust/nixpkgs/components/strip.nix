# strip → rust-strip
#
# GNU strip removes symbols and debug info from ELF binaries.
# It is used by stdenv's fixup phase (stripDirs) to reduce closure
# size. rust-strip uses the object crate for ELF section manipulation.
{
  pkgs,
  mkComponent,
  status,
  source,
  ...
}:
mkComponent {
  name = "strip";
  original = pkgs.binutils-unwrapped;
  replacement = pkgs.rust-strip;
  status = status.available;
  source = source.repo;
  phase = 5;
  description = "ELF symbol stripping tool (from binutils)";
  notes = "Using rust-strip from rust/strip — uses object crate for ELF rewriting";
}
