# coreutils → uutils-coreutils
#
# uutils is a cross-platform Rust rewrite of GNU coreutils.
# The `noprefix` variant provides unprefixed binary names (ls, cp, etc.)
# matching the GNU coreutils layout expected by stdenv.
#
# https://github.com/uutils/coreutils
{
  pkgs,
  mkComponent,
  status,
  source,
  ...
}:
mkComponent {
  name = "coreutils";
  original = pkgs.coreutils;
  replacement = pkgs.uutils-coreutils-noprefix;
  status = status.available;
  source = source.nixpkgs;
  phase = 1;
  description = "Core file, text, and shell utilities";
  notes = "Using uutils-coreutils-noprefix — drop-in GNU coreutils replacement";
}
