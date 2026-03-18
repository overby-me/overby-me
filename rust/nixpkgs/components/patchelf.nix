# patchelf → rust-patchelf
#
# patchelf is a utility for modifying ELF binaries — changing the
# dynamic linker, RPATH, RUNPATH, SONAME, and other ELF headers.
# It is a critical part of Nix's fixup phase. rust-patchelf uses
# the goblin crate for ELF parsing and supports all common operations.
{
  pkgs,
  mkComponent,
  status,
  source,
  ...
}:
mkComponent {
  name = "patchelf";
  original = pkgs.patchelf;
  replacement = pkgs.rust-patchelf;
  status = status.available;
  source = source.repo;
  phase = 5;
  description = "ELF binary patching tool (interpreter, RPATH, SONAME)";
  notes = "Using rust-patchelf from rust/patchelf — uses goblin crate for ELF parsing";
}
