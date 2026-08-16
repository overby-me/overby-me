# System multiplexing for nix-workspace
#
# Handles the expansion of per-system outputs so users never have to write
# `packages.x86_64-linux.my-tool` — they write `packages.my-tool` and the
# system dimension is managed automatically.
#
# Usage:
#   eachSystem ["x86_64-linux" "aarch64-linux"] (system: { packages.${system} = ...; })
{lib}: {
  # Apply a function to each system and merge the results.
  #
  # Type: [String] -> (String -> AttrSet) -> AttrSet
  #
  # The function `f` receives a system string and should return an attribute set
  # with system-keyed outputs (e.g. { packages.${system}.hello = drv; }).
  # Results from all systems are recursively merged.
  eachSystem = systems: f:
    builtins.foldl'
    (acc: system: lib.recursiveUpdate acc (f system))
    {}
    systems;

  # Build a per-system output attribute set from a flat config.
  #
  # Type: [String] -> String -> (String -> AttrSet -> a) -> AttrSet -> AttrSet
  #
  # Given a list of systems, an output key (e.g. "packages"), a builder function,
  # and a flat config (name → config), produces:
  #   { ${outputKey}.${system}.${name} = builder system config; }
  #
  # Entries that declare their own `systems` list are only built for those systems.
  perSystemOutput = workspaceSystems: outputKey: builder: configs: let
    buildForSystem = system: let
      # Filter configs to those that should be built for this system
      relevantConfigs =
        lib.filterAttrs (
          _name: cfg: let
            targetSystems = cfg.systems or workspaceSystems;
          in
            builtins.elem system targetSystems
        )
        configs;

      built =
        lib.mapAttrs (
          name: cfg:
            builder system name cfg
        )
        relevantConfigs;
    in {${outputKey}.${system} = built;};
  in
    builtins.foldl'
    (acc: system: lib.recursiveUpdate acc (buildForSystem system))
    {}
    workspaceSystems;

  # Resolve the effective systems list for a single output entry.
  #
  # Type: [String] -> AttrSet -> [String]
  #
  # If the entry has a `systems` field, use that; otherwise fall back
  # to the workspace-level systems list.
  resolveEntrySystems = workspaceSystems: entry:
    entry.systems or workspaceSystems;

  # Validate that all systems in a list are known.
  #
  # Type: [String] -> [String] -> Bool
  validSystems = knownSystems: systems:
    builtins.all (s: builtins.elem s knownSystems) systems;

  # The set of all systems Nix generally supports.
  # Used as a reference / upper bound; workspaces pick a subset.
  allSystems = [
    "x86_64-linux"
    "aarch64-linux"
    "x86_64-darwin"
    "aarch64-darwin"
  ];

  # Default systems when none are specified in workspace.ncl.
  defaultSystems = [
    "x86_64-linux"
    "aarch64-linux"
  ];
}
