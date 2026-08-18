# gawk → oxidized-awk
#
# GNU Awk is used in stdenv by configure scripts, makefiles, and
# various build system hooks. oxidized-awk provides a POSIX awk
# implementation with GNU extensions (gensub, etc.) using a proper
# lexer/parser/interpreter architecture.
{
  pkgs,
  mkComponent,
  status,
  source,
  ...
}:
mkComponent {
  name = "awk";
  original = pkgs.gawk;
  replacement = pkgs.oxidized-awk;
  status = status.available;
  source = source.repo;
  phase = 2;
  description = "Pattern scanning and text processing language";
  notes = "Using oxidized-awk from safety/oxidized/awk — POSIX awk with GNU extensions";
}
