# gnutar → rust-tar
#
# GNU tar is used by stdenv to unpack source tarballs and create
# archives during the install phase. rust-tar wraps the Rust `tar`
# crate with a GNU-compatible CLI supporting all common flags.
{
  pkgs,
  mkComponent,
  status,
  source,
  ...
}:
mkComponent {
  name = "tar";
  original = pkgs.gnutar;
  replacement = pkgs.rust-tar;
  status = status.available;
  source = source.repo;
  phase = 3;
  description = "Tape archive utility for packing/unpacking source tarballs";
  notes = "Using rust-tar from rust/tar — wraps the Rust tar crate with GNU-compatible CLI";
}
