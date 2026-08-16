# diffutils — diff, cmp, sdiff, diff3
#
# GNU diffutils provides file comparison utilities used extensively in
# configure scripts, patch workflows, and the Nix build sandbox.
#
# oxidized-diffutils provides individual diff, cmp, sdiff, and diff3 binaries
# via argv[0] detection from a single binary with symlinks.
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
  replacement = pkgs.oxidized-diffutils;
  status = status.available;
  source = source.repo;
  phase = 2;
  description = "File comparison utilities (diff, cmp, sdiff, diff3)";
  notes = "Using oxidized-diffutils from safety/oxidized/diffutils — Myers diff algorithm, normal/unified/context output";
}
