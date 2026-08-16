# Module builder for nix-workspace
#
# Converts validated ModuleConfig and HomeConfig records into flake outputs.
#
# NixOS modules   → nixosModules.<name>
# Home modules    → homeModules.<name>
#
# Unlike packages and machines, modules are not derivations — they are
# NixOS/home-manager module functions ({ config, lib, pkgs, ... }: { ... }).
# The builder's job is to:
#   1. Resolve module paths from the workspace
#   2. Wrap discovered .nix files with any extra configuration from the Nickel config
#   3. Produce properly-shaped flake output attributes
#
# Input shape for ModuleConfig (from evaluated workspace.ncl):
#   {
#     description = "Desktop environment module";
#     imports = ["base"];
#     options-namespace = "services.my-service";
#     platforms = ["x86_64-linux"];
#     path = "./modules/desktop.nix";
#     extra-config = {};
#   }
#
# Input shape for HomeConfig:
#   {
#     description = "Shell configuration";
#     imports = ["base"];
#     options-namespace = "programs.my-tool";
#     platforms = ["x86_64-linux"];
#     path = "./home/shell.nix";
#     state-version = "25.05";
#     extra-config = {};
#   }
#
{lib}: let
  # Build a single NixOS module from a ModuleConfig.
  #
  # If the config has a `path`, we import that .nix file and wrap it
  # with any extra-config. If no path is set (e.g. inline module in
  # workspace.ncl), we construct a module from the config fields.
  #
  # Type: Path -> AttrSet -> AttrSet -> NixOS Module
  #
  # Arguments:
  #   workspaceRoot    — Path to the workspace root directory
  #   name             — Module name (e.g. "desktop")
  #   moduleConfig     — The evaluated ModuleConfig from Nickel
  #   allModulePaths   — { name = /path/to/module.nix; ... } for resolving imports
  #
  buildNixosModule = {
    workspaceRoot,
    name,
    moduleConfig,
    allModulePaths ? {},
  }: let
    # Resolve a module reference to an importable path
    resolveImportRef = ref:
      if builtins.hasAttr ref allModulePaths
      then allModulePaths.${ref}
      else if lib.hasPrefix "./" ref || lib.hasPrefix "../" ref
      then workspaceRoot + "/${ref}"
      else if lib.hasPrefix "/" ref
      then /. + ref
      else
        throw ''
          nix-workspace: NixOS module '${name}' imports '${ref}' which was not found.
          Available workspace modules: ${builtins.concatStringsSep ", " (builtins.attrNames allModulePaths)}
          Hint: import references can be a workspace module name, a relative path (./path), or an absolute path.
        '';

    resolvedImports =
      map resolveImportRef (moduleConfig.imports or []);

    extraConfig = moduleConfig.extra-config or {};

    hasPath = moduleConfig ? path;

    # The primary module source — either from a discovered .nix file or
    # from the allModulePaths mapping using the module name.
    modulePath =
      if hasPath
      then
        if lib.hasPrefix "./" moduleConfig.path || lib.hasPrefix "../" moduleConfig.path
        then workspaceRoot + "/${moduleConfig.path}"
        else if lib.hasPrefix "/" moduleConfig.path
        then /. + moduleConfig.path
        else workspaceRoot + "/${moduleConfig.path}"
      else if builtins.hasAttr name allModulePaths
      then allModulePaths.${name}
      else null;
  in
    # If we have a concrete .nix file, produce a module that imports it
    # plus any resolved imports and extra-config.
    if modulePath != null
    then
      {lib, ...}: {
        imports =
          [modulePath]
          ++ resolvedImports;

        config = lib.mkIf true extraConfig;
      }
    else
      # No file path — produce a module from just imports + extra-config.
      # This covers the case where a module is declared purely in workspace.ncl.
      {lib, ...}: {
        imports = resolvedImports;

        config = lib.mkIf true extraConfig;
      };

  # Build a single home-manager module from a HomeConfig.
  #
  # Similar to buildNixosModule but for the home-manager module system.
  #
  # Type: Path -> AttrSet -> AttrSet -> Home-Manager Module
  buildHomeModule = {
    workspaceRoot,
    name,
    homeConfig,
    allHomePaths ? {},
  }: let
    resolveImportRef = ref:
      if builtins.hasAttr ref allHomePaths
      then allHomePaths.${ref}
      else if lib.hasPrefix "./" ref || lib.hasPrefix "../" ref
      then workspaceRoot + "/${ref}"
      else if lib.hasPrefix "/" ref
      then /. + ref
      else
        throw ''
          nix-workspace: home-manager module '${name}' imports '${ref}' which was not found.
          Available home modules: ${builtins.concatStringsSep ", " (builtins.attrNames allHomePaths)}
          Hint: import references can be a home module name, a relative path (./path), or an absolute path.
        '';

    resolvedImports =
      map resolveImportRef (homeConfig.imports or []);

    extraConfig = homeConfig.extra-config or {};

    hasPath = homeConfig ? path;

    modulePath =
      if hasPath
      then
        if lib.hasPrefix "./" homeConfig.path || lib.hasPrefix "../" homeConfig.path
        then workspaceRoot + "/${homeConfig.path}"
        else if lib.hasPrefix "/" homeConfig.path
        then /. + homeConfig.path
        else workspaceRoot + "/${homeConfig.path}"
      else if builtins.hasAttr name allHomePaths
      then allHomePaths.${name}
      else null;
  in
    if modulePath != null
    then
      {lib, ...}: {
        imports =
          [modulePath]
          ++ resolvedImports;

        config = lib.mkIf true extraConfig;
      }
    else
      {lib, ...}: {
        imports = resolvedImports;

        config = lib.mkIf true extraConfig;
      };

  # Build all NixOS modules from the workspace config.
  #
  # Type: AttrSet -> AttrSet
  #
  # Arguments:
  #   workspaceRoot  — Path to the workspace root
  #   moduleConfigs  — { name = ModuleConfig; ... } from workspace evaluation
  #   discoveredPaths — { name = /path/to/module.nix; ... } from auto-discovery
  #
  # Returns:
  #   { name = nixosModule; ... } suitable for nixosModules flake output
  #
  buildAllNixosModules = {
    workspaceRoot,
    moduleConfigs,
    discoveredPaths ? {},
  }:
    lib.mapAttrs (
      name: moduleConfig:
        buildNixosModule {
          inherit workspaceRoot name moduleConfig;
          allModulePaths = discoveredPaths;
        }
    )
    moduleConfigs;

  # Build all home-manager modules from the workspace config.
  #
  # Type: AttrSet -> AttrSet
  #
  # Arguments:
  #   workspaceRoot  — Path to the workspace root
  #   homeConfigs    — { name = HomeConfig; ... } from workspace evaluation
  #   discoveredPaths — { name = /path/to/module.nix; ... } from auto-discovery
  #
  # Returns:
  #   { name = homeModule; ... } suitable for homeModules flake output
  #
  buildAllHomeModules = {
    workspaceRoot,
    homeConfigs,
    discoveredPaths ? {},
  }:
    lib.mapAttrs (
      name: homeConfig:
        buildHomeModule {
          inherit workspaceRoot name homeConfig;
          allHomePaths = discoveredPaths;
        }
    )
    homeConfigs;

  # Discover .nix files (not .ncl) in a convention directory.
  #
  # This complements the Nickel discovery — modules may also have
  # plain .nix implementation files alongside their .ncl configuration.
  # For example:
  #   modules/desktop.ncl  — Nickel config (options, description, imports)
  #   modules/desktop.nix  — Actual NixOS module implementation
  #
  # Type: Path -> String -> AttrSet
  #
  # Returns: { name = /path/to/name.nix; ... }
  #
  discoverNixFiles = workspaceRoot: relativeDir: let
    dirPath = workspaceRoot + "/${relativeDir}";
  in
    if builtins.pathExists dirPath
    then let
      entries = builtins.readDir dirPath;
      nixEntries =
        lib.filterAttrs (
          name: type:
            type == "regular" && lib.hasSuffix ".nix" name
        )
        entries;
    in
      lib.mapAttrs' (
        name: _: {
          name = lib.removeSuffix ".nix" name;
          value = dirPath + "/${name}";
        }
      )
      nixEntries
    else {};
in {
  inherit
    buildNixosModule
    buildHomeModule
    buildAllNixosModules
    buildAllHomeModules
    discoverNixFiles
    ;
}
