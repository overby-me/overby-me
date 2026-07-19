# Pure entry point: a Buck2 project + target -> a JSON-safe action graph.
# builtins only (skylark + buck2 lib, no pkgs), so it can run under
# `nix-instantiate --eval --json` inside a derivation for the opt-in IFD path
# (see build/buildBuck2Project.nix). The same function serves the pure-eval
# path directly.
#
#   import ./analyze.nix { src = <project path>; target = "//pkg:name"; system = "x86_64-linux"; }
#     -> { actions = [ ... ]; defaultOutput = { ... } | null; }
{
  src,
  target,
  system,
}: let
  skylark = import ../../skylark/api.nix;
  buckconfig = import ./buckconfig.nix;
  mkLoader = import ./loader.nix;
  mkAnalysis = import ./analysis.nix;
  serialize = import ./serialize.nix;

  inherit ((buckconfig.parse (builtins.readFile (src + "/.buckconfig")))) cells;
  loader = mkLoader {
    inherit skylark cells system;
    root = src;
  };
  analysis = mkAnalysis {inherit skylark loader;};
  node = analysis.analyzeTarget "" target;
in
  serialize.plainGraph {
    actions = analysis.collectActions node;
    defaultOutput = analysis.defaultOutputForNode node;
  }
