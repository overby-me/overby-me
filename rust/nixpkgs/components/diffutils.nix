# diffutils — diff, cmp, sdiff, diff3
#
# GNU diffutils provides file comparison utilities used extensively in
# configure scripts, patch workflows, and the Nix build sandbox.
#
# uutils-diffutils (https://github.com/uutils/diffutils) exists but
# currently only provides a single `diffutils` binary, NOT the individual
# diff/cmp/sdiff/diff3 commands that stdenv expects. It cannot serve as
# a drop-in replacement until it provides these individual commands.
{
  pkgs,
  mkComponent,
  status,
  source,
  ...
}:
mkComponent {
  name = "diffutils";
  original = pkgs.diffutils;
  replacement = pkgs.uutils-diffutils;
  status = status.inProgress;
  source = source.nixpkgs;
  phase = 2;
  description = "File comparison utilities (diff, cmp, sdiff, diff3)";
  notes = "uutils-diffutils only provides a single binary — not yet a drop-in for stdenv";
}
