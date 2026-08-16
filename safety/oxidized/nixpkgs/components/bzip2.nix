# bzip2 → oxidized-bzip2
#
# bzip2 is used by stdenv to decompress .bz2 source tarballs.
# oxidized-bzip2 wraps the Rust bzip2 crate with full CLI compatibility.
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
  replacement = pkgs.oxidized-bzip2;
  status = status.available;
  source = source.repo;
  phase = 3;
  description = "bzip2 compression/decompression";
  notes = "Using oxidized-bzip2 from safety/oxidized/bzip2 — wraps Rust bzip2 crate";
}
