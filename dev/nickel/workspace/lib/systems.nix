# System multiplexing for nix-workspace
#
# Handles the expansion of per-system outputs so users never have to write
# `packages.x86_64-linux.my-tool` — they write `packages.my-tool` and the
# system dimension is managed automatically.
#
# Usage:
#   eachSystem ["x86_64-linux" "aarch64-linux"] (system: { packages.${system} = ...; })
{lib}: {
  # `f` takes a system string and returns system-keyed outputs, e.g.
  # `{ packages.${system}.hello = drv; }`. The results merge recursively.
  eachSystem = systems: f:
    lib.foldl'
    (acc: system: lib.recursiveUpdate acc (f system))
    {}
    systems;

  # From a flat name → config, an output key and a builder:
  #   { ${outputKey}.${system}.${name} = builder system config; }
  # An entry declaring its own `systems` is built only for those.
  perSystemOutput = workspaceSystems: outputKey: builder: configs: let
    buildForSystem = system: let
      relevantConfigs =
        lib.filterAttrs (
          _name: cfg: let
            targetSystems = cfg.systems or workspaceSystems;
          in
            lib.elem system targetSystems
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
    lib.foldl'
    (acc: system: lib.recursiveUpdate acc (buildForSystem system))
    {}
    workspaceSystems;

  # An entry's own `systems`, or the workspace-level list.
  resolveEntrySystems = workspaceSystems: entry:
    entry.systems or workspaceSystems;

  # Validate that all systems in a list are known.
  validSystems = knownSystems: systems:
    lib.all (s: lib.elem s knownSystems) systems;

  # An upper bound to validate against; a workspace picks a subset.
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
