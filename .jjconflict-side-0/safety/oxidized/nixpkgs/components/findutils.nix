# findutils — find, xargs, locate, updatedb
#
# GNU findutils provides `find` and `xargs`, both heavily used in
# stdenv's setup.sh and in configure scripts.
#
# uutils-findutils (https://github.com/uutils/findutils) is a Rust
# reimplementation aiming to be a full drop-in replacement. It runs
# the GNU testsuite and is packaged in nixpkgs.
{
  pkgs,
  mkComponent,
  status,
  source,
  ...
}:
mkComponent {
  name = "findutils";
  original = pkgs.findutils;
  replacement = pkgs.uutils-findutils;
  status = status.available;
  source = source.nixpkgs;
  phase = 2;
  description = "File search and command execution (find, xargs)";
  notes = "Using uutils-findutils — GNU-compatible Rust rewrite from uutils project";
}
