# xz → rust-xz
#
# xz is used by stdenv to decompress .tar.xz source archives, the
# most common archive format in modern nixpkgs. rust-xz wraps the
# xz2 crate with full CLI compatibility.
# Provides xz, unxz, xzcat, lzma, unlzma, lzcat via argv[0] detection.
{
  pkgs,
  mkComponent,
  status,
  source,
  ...
}:
mkComponent {
  name = "xz";
  original = pkgs.xz;
  replacement = pkgs.rust-xz;
  status = status.available;
  source = source.repo;
  phase = 3;
  description = "LZMA/XZ compression and decompression";
  notes = "Using rust-xz from rust/xz — wraps xz2 crate, provides xz/unxz/xzcat/lzma/unlzma/lzcat";
}
