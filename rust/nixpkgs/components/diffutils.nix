# diffutils — diff, cmp, sdiff, diff3
#
# GNU diffutils provides file comparison utilities used extensively in
# configure scripts, patch workflows, and the Nix build sandbox.
#
# uutils-diffutils (https://github.com/uutils/diffutils) is a Rust
# drop-in replacement providing diff, cmp, diff3, and sdiff.
# It is packaged in nixpkgs.
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
  status = status.available;
  source = source.nixpkgs;
  phase = 2;
  description = "File comparison utilities (diff, cmp, sdiff, diff3)";
  notes = "Using uutils-diffutils — GNU-compatible Rust rewrite from uutils project";
}
