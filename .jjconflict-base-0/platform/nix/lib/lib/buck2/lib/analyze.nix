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

  parsedConfig = buckconfig.parse (builtins.readFile (src + "/.buckconfig"));
  inherit (parsedConfig) cells;
  # A project may keep machine-local values (tool paths) in .buckconfig.local, which
  # buck2 layers on top of .buckconfig; read_root_config has to see those too.
  localConfigPath = src + "/.buckconfig.local";
  localSections =
    if builtins.pathExists localConfigPath
    then (buckconfig.parse (builtins.readFile localConfigPath)).sections
    else {};
  configSections = builtins.foldl' (acc: name:
    acc // {${name} = (acc.${name} or {}) // localSections.${name};})
  parsedConfig.sections (builtins.attrNames localSections);
  loader = mkLoader {
    inherit skylark cells system;
    sections = configSections;
    root = src;
  };
  analysis = mkAnalysis {inherit skylark loader;};
  node = analysis.analyzeTarget "" target;
in
  serialize.plainGraph {
    actions = analysis.collectActions node;
    defaultOutput = analysis.defaultOutputForNode node;
  }
