# ModuleConfig and HomeConfig records into nixosModules.<name> and
# homeModules.<name>.
#
# Unlike packages and machines these are not derivations but module functions,
# `{ config, lib, pkgs, ... }: { ... }`, so the work is resolving each module's
# path and wrapping the discovered .nix file with the extra configuration its
# Nickel config carries. A ModuleConfig looks like
#
#   {
#     description = "Desktop environment module";
#     imports = ["base"];
#     options-namespace = "services.my-service";
#     platforms = ["x86_64-linux"];
#     path = "./modules/desktop.nix";
#     extra-config = {};
#   }
#
# and a HomeConfig the same plus `state-version`.
{lib}: let
  # A `path` is imported and wrapped with extra-config; without one, the module
  # is built from the config fields alone, which is the inline case.
  buildNixosModule = {
    workspaceRoot,
    name,
    moduleConfig,
    allModulePaths ? {},
  }: let
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

    # A discovered .nix file, or the allModulePaths entry for this name.
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

  # buildNixosModule for the home-manager module system.
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

  # The .nix half of a module, alongside the .ncl the Nickel discovery finds:
  #   modules/desktop.ncl  — options, description, imports
  #   modules/desktop.nix  — the NixOS module itself
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
