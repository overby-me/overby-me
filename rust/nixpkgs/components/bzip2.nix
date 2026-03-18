# bzip2 → rust-bzip2
#
# bzip2 is used by stdenv to decompress .bz2 source tarballs.
# rust-bzip2 wraps the Rust bzip2 crate with full CLI compatibility.
# Provides bzip2, bunzip2, and bzcat via argv[0] detection.
{
  pkgs,
  mkComponent,
  status,
  source,
  ...
}:
mkComponent {
  name = "bzip2";
  original = pkgs.bzip2;
  replacement = pkgs.rust-bzip2;
  status = status.available;
  source = source.repo;
  phase = 3;
  description = "bzip2 compression/decompression";
  notes = "Using rust-bzip2 from rust/bzip2 — wraps Rust bzip2 crate";
}
