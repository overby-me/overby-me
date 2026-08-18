# OverlayConfig records into overlays.<name>, the `final: prev: { ... }`
# functions that extend or override nixpkgs. An OverlayConfig looks like
#   {
#     description = "Custom packages overlay";
#     path = "./overlays/custom.nix";
#     priority = 100;
#     packages = ["my-tool" "my-lib"];
#     extra-config = {};
#   }
{lib}: let
  # A `path` is imported directly and must evaluate to `final: prev: { ... }`.
  # Without one, the overlay comes from the extra-config escape hatch, or is a
  # no-op.
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
      if lib.isFunction imported
      then imported
      else
        throw ''
          nix-workspace: overlay '${name}' at '${toString resolvedPath}' does not evaluate to a function.
          Overlays must be functions of the form: final: prev: { ... }
        ''
    else _final: _prev: {};

  # Build all overlays from the workspace config.
  #
  # Returns:
  #   { name = overlayFn; ... } suitable for the overlays flake output
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
          if cfg ? path && lib.isPath cfg.path
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
  # Returns: List of (name, config) pairs sorted by priority (ascending),
  #          then alphabetically by name for equal priorities.
  sortByPriority = overlayConfigs: let
    entries =
      lib.mapAttrsToList (name: cfg: {
        inherit name cfg;
        priority = cfg.priority or 100;
      })
      overlayConfigs;
  in
    lib.sort (
      a: b:
        if a.priority != b.priority
        then a.priority < b.priority
        else a.name < b.name
    )
    entries;

  # Compose all overlays into a single overlay function, respecting priority order.
  #
  # Returns: A single composed overlay function
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
