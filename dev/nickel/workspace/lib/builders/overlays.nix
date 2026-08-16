# Overlay builder for nix-workspace
#
# Converts validated OverlayConfig records into flake overlay outputs.
# Overlays are discovered from the overlays/ convention directory
# or declared explicitly in workspace.ncl.
#
# Each overlay config maps to an overlays.<name> flake output.
# Overlays are Nix functions (final: prev: { ... }) that extend or
# override packages in nixpkgs.
#
# Input shape (from evaluated workspace.ncl):
#   {
#     description = "Custom packages overlay";
#     path = "./overlays/custom.nix";
#     priority = 100;
#     packages = ["my-tool" "my-lib"];
#     extra-config = {};
#   }
#
{lib}: let
  # Build a single overlay from an OverlayConfig.
  #
  # If the config has a `path`, we import that .nix file directly.
  # The .nix file must evaluate to a function `final: prev: { ... }`.
  #
  # If no path is provided, the overlay is constructed from the
  # extra-config escape hatch (or is a no-op).
  #
  # Type: Path -> String -> AttrSet -> (AttrSet -> AttrSet -> AttrSet)
  #
  # Arguments:
  #   workspaceRoot  — Path to the workspace root directory
  #   name           — Overlay name (e.g. "custom-packages")
  #   overlayConfig  — The evaluated OverlayConfig from Nickel
  #
  # Returns: A nixpkgs overlay function (final: prev: { ... })
  #
  buildOverlay = workspaceRoot: name: overlayConfig: let
    hasPath = overlayConfig ? path;

    resolvedPath =
      if hasPath
      then
        if lib.hasPrefix "./" overlayConfig.path || lib.hasPrefix "../" overlayConfig.path
        then workspaceRoot + "/${overlayConfig.path}"
        else if lib.hasPrefix "/" overlayConfig.path
        then /. + overlayConfig.path
        else workspaceRoot + "/${overlayConfig.path}"
      else null;
  in
    if resolvedPath != null
    then let
      imported = import resolvedPath;
    in
      if builtins.isFunction imported
      then imported
      else
        throw ''
          nix-workspace: overlay '${name}' at '${toString resolvedPath}' does not evaluate to a function.
          Overlays must be functions of the form: final: prev: { ... }
        ''
    else
      # No path — produce a no-op overlay. This covers the case where
      # an overlay is declared purely in workspace.ncl as metadata.
      _final: _prev: {};

  # Build all overlays from the workspace config.
  #
  # Type: AttrSet -> AttrSet
  #
  # Arguments:
  #   workspaceRoot   — Path to the workspace root
  #   overlayConfigs  — { name = OverlayConfig; ... } from workspace evaluation
  #   discoveredPaths — { name = /path/to/overlay.nix; ... } from auto-discovery
  #
  # Returns:
  #   { name = overlayFn; ... } suitable for the overlays flake output
  #
  buildAllOverlays = {
    workspaceRoot,
    overlayConfigs,
    discoveredPaths ? {},
  }: let
    # For discovered overlays without explicit config, create minimal configs
    effectiveConfigs =
      (lib.mapAttrs (_: path: {inherit path;}) discoveredPaths)
      // overlayConfigs;

    # Resolve discovered paths into overlay configs that have a path field
    # (discovered paths are already absolute, so we convert them to strings
    # that the builder can handle)
    resolvedConfigs =
      lib.mapAttrs (
        _name: cfg:
          if cfg ? path && builtins.isPath cfg.path
          then cfg // {path = toString cfg.path;}
          else cfg
      )
      effectiveConfigs;
  in
    lib.mapAttrs (
      name: cfg:
        buildOverlay workspaceRoot name cfg
    )
    resolvedConfigs;

  # Sort overlays by priority for application order.
  #
  # Type: AttrSet -> [(String, AttrSet)]
  #
  # Arguments:
  #   overlayConfigs — { name = OverlayConfig; ... }
  #
  # Returns: List of (name, config) pairs sorted by priority (ascending),
  #          then alphabetically by name for equal priorities.
  #
  sortByPriority = overlayConfigs: let
    entries =
      lib.mapAttrsToList (name: cfg: {
        inherit name cfg;
        priority = cfg.priority or 100;
      })
      overlayConfigs;
  in
    builtins.sort (
      a: b:
        if a.priority != b.priority
        then a.priority < b.priority
        else a.name < b.name
    )
    entries;

  # Compose all overlays into a single overlay function, respecting priority order.
  #
  # Type: Path -> AttrSet -> (AttrSet -> AttrSet -> AttrSet)
  #
  # Arguments:
  #   workspaceRoot  — Path to the workspace root
  #   overlayConfigs — { name = OverlayConfig; ... }
  #
  # Returns: A single composed overlay function
  #
  composeOverlays = workspaceRoot: overlayConfigs: let
    sorted = sortByPriority overlayConfigs;
    overlayFns =
      map (
        entry: buildOverlay workspaceRoot entry.name entry.cfg
      )
      sorted;
  in
    lib.composeManyExtensions overlayFns;
in {
  inherit
    buildOverlay
    buildAllOverlays
    sortByPriority
    composeOverlays
    ;
}
