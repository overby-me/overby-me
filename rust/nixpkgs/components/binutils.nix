# Binutils: binutils → rust-binutils
#
# rust-binutils provides GNU binutils-compatible tools: ar, ranlib, nm,
# objdump, readelf, objcopy, strings, size, addr2line, c++filt, strip.
# It is a multicall binary dispatching based on argv[0].
{
  pkgs,
  mkComponent,
  status,
  source,
  ...
}:
mkComponent {
  name = "binutils";
  original = pkgs.binutils-unwrapped;
  replacement = pkgs.rust-binutils;
  status = status.available;
  source = source.repo;
  phase = 5;
  description = "Binary utilities (ar, ranlib, nm, objdump, readelf, etc.)";
  notes = "Rust rewrite at rust/binutils";
}
