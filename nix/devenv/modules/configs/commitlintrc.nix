# Generate commitlintrc.yml with scope options dynamically derived from
# the Nix flake source tree.
# Discovers top-level files (e.g. "flake.nix"), top-level dirs (e.g. "nix",
# ".tangled"), and their immediate subdirectories (e.g. "lib").
{
  pkgs,
  lib,
  src,
}: let
  entries = lib.readDir src;
  files = lib.attrNames (lib.filterAttrs (_: type: type == "regular" || type == "symlink") entries);
  dirs = lib.attrNames (lib.filterAttrs (_: type: type == "directory") entries);

  # For each top-level dir, discover immediate subdirectories as "parent/child" scopes.
  subScopes = dir: let
    dirPath = src + "/${dir}";
    subEntries = lib.readDir dirPath;
    subDirs = lib.filter (name: !lib.hasPrefix "." name) (
      lib.attrNames (lib.filterAttrs (_: type: type == "directory") subEntries)
    );
  in
    map (sub: "${dir}/${sub}") subDirs;

  topLevel = dirs;
  nested = lib.concatMap subScopes dirs;
  allScopes = lib.sort (a: b: a < b) (topLevel ++ nested ++ files);
  scopeOptionsYaml = lib.concatMapStringsSep "\n" (s: "      - ${s}") allScopes;
  commitlintrcContent = lib.replaceStrings ["@SCOPE_OPTIONS@"] [scopeOptionsYaml] (
    lib.readFile ./commitlintrc.yml
  );
in
  pkgs.writeText "commitlintrc.yml" commitlintrcContent
