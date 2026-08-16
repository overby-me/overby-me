# gnutar → oxidized-tar
#
# GNU tar is used by stdenv to unpack source tarballs and create
# archives during the install phase. oxidized-tar wraps the Rust `tar`
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
  replacement = pkgs.oxidized-tar;
  status = status.available;
  source = source.repo;
  phase = 3;
  description = "Tape archive utility for packing/unpacking source tarballs";
  notes = "Using oxidized-tar from safety/oxidized/tar — wraps the Rust tar crate with GNU-compatible CLI";
}
