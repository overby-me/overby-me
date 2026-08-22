# Workspace module: pure-eval Buck2 builds. Lowers each Buck2 action to its
# own Nix derivation, with no import-from-derivation. See PLAN.md.
#
# Exposes, via perSystemLib:
#   buildBuck2Project  pkgs -> { src; target|targets; ... } -> derivation
#   buck2Lib           the pure phases (buckconfig, labels, loader, analysis)
#                      and the skylark interpreter, for tests and advanced use.
{
  perSystemLib.buildBuck2Project = pkgs: import ./build/buildBuck2Project.nix {inherit pkgs;};

  perSystemLib.buck2Lib = _pkgs: {
    skylark = import ../skylark/api.nix;
    buckconfig = import ./lib/buckconfig.nix;
    labels = import ./lib/labels.nix;
    loader = import ./lib/loader.nix;
    analysis = import ./lib/analysis.nix;
    cmd_args = import ./lib/cmd_args.nix;
    globals = import ./lib/globals.nix;
    actions = import ./lib/actions.nix;
  };
}
