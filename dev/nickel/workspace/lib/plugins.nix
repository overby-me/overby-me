# The Nix side of the plugin system: everything after Nickel evaluation, where
# eval-nickel.nix handles the Nickel side (contracts and extensions) through its
# wrapper generation. Plugins resolve to a directory by name convention,
# "nix-workspace-rust" → plugins/rust/, holding a plugin.ncl that eval-nickel.nix
# reads and a builder.nix that this module does.
{lib}: let
  # ── Plugin resolution ─────────────────────────────────────────
  resolvePluginDir = pluginsDir: pluginName: let
    shortName =
      if lib.hasPrefix "nix-workspace-" pluginName
      then lib.removePrefix "nix-workspace-" pluginName
      else pluginName;

    pluginDir = pluginsDir + "/${shortName}";
  in
    if lib.pathExists pluginDir
    then pluginDir
    else
      throw ''
        nix-workspace: plugin '${pluginName}' not found.
        Looked in: ${toString pluginDir}
        Available built-in plugins: nix-workspace-rust, nix-workspace-go
        Hint: check the plugin name in your workspace.ncl plugins list.
      '';

  resolvePluginNcl = pluginsDir: pluginName: let
    dir = resolvePluginDir pluginsDir pluginName;
    nclPath = dir + "/plugin.ncl";
  in
    if lib.pathExists nclPath
    then nclPath
    else
      throw ''
        nix-workspace: plugin '${pluginName}' has no plugin.ncl definition.
        Expected at: ${toString nclPath}
      '';

  # Null for a Nickel-only plugin, which has no Nix-side builder.
  resolvePluginBuilder = pluginsDir: pluginName: let
    dir = resolvePluginDir pluginsDir pluginName;
    builderPath = dir + "/builder.nix";
  in
    if lib.pathExists builderPath
    then builderPath
    else null;

  # ── Plugin loading ────────────────────────────────────────────
  #
  #   {
  #     builders = { builderName = builderFn; ... };
  #     conventions = { conventionName = { path, output, builder, autoDiscover }; ... };
  #     shellExtras = { pluginName = shellExtrasFn; ... };
  #     pluginConfigs = { pluginName = evaluatedConfig; ... };
  #     pluginNames = [ "nix-workspace-rust" ... ];
  #   }
  loadPlugins = pluginsDir: pluginNames: let
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

    # Keyed by meta.buildSystem, falling back to meta.name. A builder.nix
    # exports its build function plus that meta:
    #   plugins/rust/builder.nix: { buildRustPackage, meta.buildSystem = "rust" }
    #   plugins/go/builder.nix:   { buildGo, meta.buildSystem = "go" }
    allBuilders =
      lib.foldl' (
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

    # A builder.nix may export `shellExtras : Pkgs -> ShellConfig -> [Derivation]`,
    # called while building a dev shell to add the plugin's own packages, a Rust
    # toolchain's components say.
    allShellExtras =
      lib.foldl' (
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
  # The plugins' convention directories, read off the evaluated plugin configs
  # once Nickel has produced them, as
  #   { conventionName = { dir, output, autoDiscover }; ... }
  # ready to merge into discover.defaultConventions.
  extractConventions = evaluatedPluginConfigs:
    lib.foldl' (
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
    (lib.attrValues evaluatedPluginConfigs);

  # ── Builder routing ───────────────────────────────────────────
  routeBuilder = pluginBuilders: coreBuilders: pkgs: workspaceRoot: name: cfg: let
    buildSystem = cfg.build-system or "generic";

    # Plugins first, so one can override core behaviour.
    builderFn =
      if lib.hasAttr buildSystem pluginBuilders
      then let
        pluginModule = pluginBuilders.${buildSystem};
        # The main build function is named build<BuildSystem>: buildRustPackage,
        # buildGo.
        fnName =
          if buildSystem == "rust"
          then "buildRustPackage"
          else "build${lib.toUpper (lib.substring 0 1 buildSystem)}${lib.substring 1 (lib.stringLength buildSystem - 1) buildSystem}";
      in
        pluginModule.${fnName}
        or (throw "nix-workspace: plugin builder for '${buildSystem}' does not export '${fnName}'")
      else if lib.hasAttr buildSystem coreBuilders
      then coreBuilders.${buildSystem}
      else throw "nix-workspace: unknown build-system '${buildSystem}' for package '${name}'. No plugin or core builder registered for this build system.";
  in
    builderFn pkgs workspaceRoot name cfg;

  # ── Shell extras application ──────────────────────────────────
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
  # Core discovery extended over the directories plugins registered, as
  #   { conventionName = { name = { path, builder }; ... }; ... }
  # where each item inherits its plugin's builder setting.
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
  validatePlugins = pluginNames: let
    uniqueNames = lib.unique pluginNames;
    hasDuplicates = lib.length uniqueNames != lib.length pluginNames;

    duplicateDiagnostics =
      if hasDuplicates
      then let
        counts =
          lib.foldl' (
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
  # Returns: The package config with plugin defaults applied.
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
