# diffutils — diff, cmp, sdiff, diff3
#
# GNU diffutils provides file comparison utilities used extensively in
# configure scripts, patch workflows, and the Nix build sandbox.
#
# rust-diffutils provides individual diff, cmp, sdiff, and diff3 binaries
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
  replacement = pkgs.rust-diffutils;
  status = status.available;
  source = source.repo;
  phase = 2;
  description = "File comparison utilities (diff, cmp, sdiff, diff3)";
  notes = "Using rust-diffutils from rust/diffutils — Myers diff algorithm, normal/unified/context output";
}
