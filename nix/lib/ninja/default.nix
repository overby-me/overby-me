# Flakelight module: pure-eval Ninja builds. Lowers each Ninja edge to its own
# Nix derivation. A sibling to nix/lib/buck2 (per-action Buck2 builds) and
# nix/lib/cargo (per-crate builds); the graph is extracted once with
# `rust-ninja -t graph-json` (one IFD), then lowered with builtins only.
# See PLAN.md.
#
# Exposes, via perSystemLib:
#   buildNinjaProject  pkgs -> { src; target|targets; ... } -> derivation
#   ninjaLib           the lowering phase, for tests and advanced use.
{
  perSystemLib.buildNinjaProject = pkgs: import ./build/buildNinjaProject.nix {inherit pkgs;};

  perSystemLib.ninjaLib = _pkgs: {
    lower = import ./build/lower.nix;
  };
}
