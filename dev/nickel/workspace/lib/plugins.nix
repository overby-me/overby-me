# Plugin loading and merging for nix-workspace
#
# This module handles the Nix-side of the plugin system:
#   1. Resolving plugin names to their definition files
#   2. Loading plugin builder.nix files
#   3. Extracting convention mappings from evaluated plugin configs
#   4. Merging plugin conventions into the discovery system
#   5. Routing packages to plugin-provided builders
#   6. Validating plugin configurations and detecting conflicts
#
# The Nickel-side of plugins (contracts, extensions) is handled in
# eval-nickel.nix via the wrapper generation. This module handles
# everything that happens after Nickel evaluation — the Nix build layer.
#
# Plugin resolution:
#   - Built-in plugins are shipped in nix-workspace's plugins/ directory
#   - Plugin names are mapped to directories: "nix-workspace-rust" → plugins/rust/
#   - Each plugin directory contains:
#       plugin.ncl  — Nickel contract/convention definitions (used by eval-nickel.nix)
#       builder.nix — Nix build functions (used by this module)
#
# Updated for v0.4 — Plugin system milestone.
#
{lib}: let
  # ── Plugin resolution ─────────────────────────────────────────
  #
  # Map plugin names to their directories. Built-in plugins use a
  # naming convention: "nix-workspace-<shortname>" → plugins/<shortname>/
  #
  # Type: Path -> String -> Path
  #
  # Arguments:
  #   pluginsDir — Path to the nix-workspace plugins/ directory
  #   pluginName — Plugin name string (e.g. "nix-workspace-rust")
  #
  # Returns: Path to the plugin directory
  #
  resolvePluginDir = pluginsDir: pluginName: let
    # Strip the "nix-workspace-" prefix to get the short name
    shortName =
      if lib.hasPrefix "nix-workspace-" pluginName
      then lib.removePrefix "nix-workspace-" pluginName
      else pluginName;

    pluginDir = pluginsDir + "/${shortName}";
  in
    if builtins.pathExists pluginDir
    then pluginDir
    else
      throw ''
        nix-workspace: plugin '${pluginName}' not found.
        Looked in: ${toString pluginDir}
        Available built-in plugins: nix-workspace-rust, nix-workspace-go
        Hint: check the plugin name in your workspace.ncl plugins list.
      '';

  # Resolve the path to a plugin's .ncl definition file.
  #
  # Type: Path -> String -> Path
  resolvePluginNcl = pluginsDir: pluginName: let
    dir = resolvePluginDir pluginsDir pluginName;
    nclPath = dir + "/plugin.ncl";
  in
    if builtins.pathExists nclPath
    then nclPath
    else
      throw ''
        nix-workspace: plugin '${pluginName}' has no plugin.ncl definition.
        Expected at: ${toString nclPath}
      '';

  # Resolve the path to a plugin's Nix builder file.
  #
  # Type: Path -> String -> Path | null
  #
  # Returns the path if builder.nix exists, or null if the plugin
  # has no Nix-side builder (Nickel-only plugin).
  resolvePluginBuilder = pluginsDir: pluginName: let
    dir = resolvePluginDir pluginsDir pluginName;
    builderPath = dir + "/builder.nix";
  in
    if builtins.pathExists builderPath
    then builderPath
    else null;

  # ── Plugin loading ────────────────────────────────────────────
  #
  # Load all requested plugins and return their builder functions
  # and convention mappings.
  #
  # Type: Path -> [String] -> AttrSet
  #
  # Arguments:
  #   pluginsDir  — Path to the nix-workspace plugins/ directory
  #   pluginNames — List of plugin name strings from workspace config
  #
  # Returns:
  #   {
  #     builders = { builderName = builderFn; ... };
  #     conventions = { conventionName = { path, output, builder, autoDiscover }; ... };
  #     shellExtras = { pluginName = shellExtrasFn; ... };
  #     pluginConfigs = { pluginName = evaluatedConfig; ... };
  #     pluginNames = [ "nix-workspace-rust" ... ];
  #   }
  #
  loadPlugins = pluginsDir: pluginNames: let
    # Load each plugin's builder.nix (if it exists) and extract its exports
    loadedPlugins =
      map (
        pluginName: let
          builderPath = resolvePluginBuilder pluginsDir pluginName;
          hasBuilder = builderPath != null;
          builderModule =
            if hasBuilder
            then import builderPath {inherit lib;}
            else {};
        in {
          name = pluginName;
          inherit hasBuilder builderModule;
        }
      )
      pluginNames;

    # Collect all builder functions keyed by build-system name.
    #
    # Each plugin builder.nix is expected to export functions named
    # like `buildRustPackage`, `buildGo`, etc. The plugin system
    # registers these under the build-system name from the plugin's
    # meta (or inferred from the plugin short name).
    #
    # The convention is:
    #   plugins/rust/builder.nix exports: { buildRustPackage, meta.buildSystem = "rust" }
    #   plugins/go/builder.nix exports:   { buildGo, meta.buildSystem = "go" }
    #
    # We key the builder by meta.buildSystem (or meta.name).
    allBuilders =
      builtins.foldl' (
        acc: plugin:
          if plugin.hasBuilder
          then let
            bm = plugin.builderModule;
            buildSystem = (bm.meta or {}).buildSystem or (bm.meta or {}).name or null;
          in
            if buildSystem != null
            then acc // {${buildSystem} = bm;}
            else acc
          else acc
      ) {}
      loadedPlugins;

    # Collect shell extras functions from plugins.
    #
    # A plugin builder.nix can export a `shellExtras` function:
    #   shellExtras : Pkgs -> ShellConfig -> [Derivation]
    #
    # These are called when building dev shells to add plugin-specific
    # packages (e.g. Rust toolchain components).
    allShellExtras =
      builtins.foldl' (
        acc: plugin:
          if plugin.hasBuilder && (plugin.builderModule ? shellExtras)
          then acc // {${plugin.name} = plugin.builderModule.shellExtras;}
          else acc
      ) {}
      loadedPlugins;
  in {
    builders = allBuilders;
    shellExtras = allShellExtras;
    inherit pluginNames;
  };

  # ── Convention extraction ─────────────────────────────────────
  #
  # Extract convention directory mappings from evaluated plugin configs
  # (the JSON output of Nickel evaluation).
  #
  # This is called after Nickel evaluation has produced the workspace
  # config including plugin data. The conventions from plugins need to
  # be fed into the discovery system.
  #
  # Type: AttrSet -> AttrSet
  #
  # Arguments:
  #   evaluatedPluginConfigs — { pluginName = { conventions = { ... }; ... }; ... }
  #                            from Nickel evaluation of each plugin.ncl
  #
  # Returns:
  #   { conventionName = { dir, output, autoDiscover }; ... }
  #   suitable for merging into discover.defaultConventions
  #
  extractConventions = evaluatedPluginConfigs:
    builtins.foldl' (
      acc: pluginConfig: let
        conventions = pluginConfig.conventions or {};
      in
        acc
        // (lib.mapAttrs (
            _name: conv: {
              dir = conv.path;
              inherit (conv) output;
              autoDiscover = conv.auto-discover or true;
              builder = conv.builder or "generic";
              fromPlugin = true;
            }
          )
          conventions)
    ) {}
    (builtins.attrValues evaluatedPluginConfigs);

  # ── Builder routing ───────────────────────────────────────────
  #
  # Route a package to the correct builder function based on its
  # build-system field, considering both core and plugin builders.
  #
  # Type: AttrSet -> Pkgs -> Path -> String -> AttrSet -> Derivation
  #
  # Arguments:
  #   pluginBuilders — { buildSystemName = builderModule; ... } from loadPlugins
  #   pkgs           — The nixpkgs package set
  #   workspaceRoot  — Path to the workspace root
  #   name           — Package name
  #   cfg            — Evaluated package config
  #
  # Returns: A derivation
  #
  routeBuilder = pluginBuilders: coreBuilders: pkgs: workspaceRoot: name: cfg: let
    buildSystem = cfg.build-system or "generic";

    # Check plugin builders first, then fall back to core builders.
    # Plugin builders take priority to allow overriding core behavior.
    builderFn =
      if builtins.hasAttr buildSystem pluginBuilders
      then let
        pluginModule = pluginBuilders.${buildSystem};
        # Convention: the main build function is named build<BuildSystem>
        # e.g. buildRustPackage, buildGo
        fnName =
          if buildSystem == "rust"
          then "buildRustPackage"
          else "build${lib.toUpper (builtins.substring 0 1 buildSystem)}${builtins.substring 1 (builtins.stringLength buildSystem - 1) buildSystem}";
      in
        pluginModule.${fnName}
        or (throw "nix-workspace: plugin builder for '${buildSystem}' does not export '${fnName}'")
      else if builtins.hasAttr buildSystem coreBuilders
      then coreBuilders.${buildSystem}
      else throw "nix-workspace: unknown build-system '${buildSystem}' for package '${name}'. No plugin or core builder registered for this build system.";
  in
    builderFn pkgs workspaceRoot name cfg;

  # ── Shell extras application ──────────────────────────────────
  #
  # Collect extra packages from all loaded plugins for a given shell config.
  #
  # Type: AttrSet -> Pkgs -> AttrSet -> [Derivation]
  #
  # Arguments:
  #   pluginShellExtras — { pluginName = shellExtrasFn; ... } from loadPlugins
  #   pkgs              — The nixpkgs package set
  #   shellConfig       — The evaluated shell config
  #
  # Returns: A flat list of extra packages to add to the shell
  #
  collectShellExtras = pluginShellExtras: pkgs: shellConfig:
    lib.concatLists (
      lib.mapAttrsToList (
        _pluginName: extrasFn:
          extrasFn pkgs shellConfig
      )
      pluginShellExtras
    );

  # ── Convention discovery ──────────────────────────────────────
  #
  # Discover .ncl files from plugin convention directories.
  #
  # This extends the core discovery to also scan directories registered
  # by plugins. Items discovered from plugin conventions inherit the
  # plugin's builder setting.
  #
  # Type: (Path -> String -> AttrSet) -> Path -> AttrSet -> AttrSet
  #
  # Arguments:
  #   discoverNclFiles    — The core discovery function from discover.nix
  #   workspaceRoot       — Path to the workspace root
  #   pluginConventions   — Convention mappings extracted from plugins
  #
  # Returns:
  #   { conventionName = { name = { path, builder }; ... }; ... }
  #
  discoverPluginConventions = discoverNclFiles: workspaceRoot: pluginConventions:
    lib.mapAttrs (
      _convName: conv: let
        discovered = discoverNclFiles workspaceRoot conv.dir;
      in
        lib.mapAttrs (
          _name: path: {
            inherit path;
            inherit (conv) builder;
          }
        )
        discovered
    )
    (lib.filterAttrs (_: conv: conv.autoDiscover) pluginConventions);

  # ── Plugin validation ─────────────────────────────────────────
  #
  # Validate that loaded plugins don't conflict with each other.
  #
  # Type: [String] -> [AttrSet]
  #
  # Returns: List of diagnostic records for any conflicts found.
  #
  validatePlugins = pluginNames: let
    # Check for duplicate plugin names
    uniqueNames = lib.unique pluginNames;
    hasDuplicates = builtins.length uniqueNames != builtins.length pluginNames;

    duplicateDiagnostics =
      if hasDuplicates
      then let
        counts =
          builtins.foldl' (
            acc: name:
              acc // {${name} = (acc.${name} or 0) + 1;}
          ) {}
          pluginNames;
        duplicates =
          lib.filterAttrs (_: count: count > 1) counts;
      in
        lib.mapAttrsToList (
          name: count: {
            code = "NW400";
            severity = "error";
            message = "Plugin '${name}' is listed ${toString count} times in the plugins list.";
            hint = "Remove duplicate plugin entries from workspace.ncl.";
          }
        )
        duplicates
      else [];
  in
    duplicateDiagnostics;

  # ── Plugin-aware builder defaults ─────────────────────────────
  #
  # Apply builder defaults from plugins to package configs.
  #
  # When a package is routed to a plugin builder, the plugin's default
  # configuration values are merged in with lower priority (the user's
  # explicit values always win).
  #
  # Type: AttrSet -> AttrSet -> AttrSet
  #
  # Arguments:
  #   pluginBuilders — { buildSystem = { meta.defaults = { ... }; ... }; ... }
  #   packageConfig  — The package config from Nickel evaluation
  #
  # Returns: The package config with plugin defaults applied.
  #
  applyBuilderDefaults = pluginBuilders: packageConfig: let
    buildSystem = packageConfig.build-system or "generic";
    pluginModule = pluginBuilders.${buildSystem} or null;
    defaults =
      if pluginModule != null
      then (pluginModule.meta or {}).defaults or {}
      else {};
  in
    defaults // packageConfig;
in {
  inherit
    resolvePluginDir
    resolvePluginNcl
    resolvePluginBuilder
    loadPlugins
    extractConventions
    routeBuilder
    collectShellExtras
    discoverPluginConventions
    validatePlugins
    applyBuilderDefaults
    ;
}
