# Generate commitlintrc.yml with scope options dynamically derived from
# the Nix flake source tree.
# Discovers top-level files (e.g. "flake.nix") and directories nested up to
# `maxDepth` segments deep (e.g. "nix", "platform/nix/nixos", "platform/nix/nixosConfigurations").
{
  pkgs,
  lib,
  src,
}: let
  # Path segments a scope may have. Two was too blunt for a tree where a
  # top-level directory is a whole platform: every NixOS change read as
  # `platform/nix/nixos` whether it touched one host or the module system. Three
  # separates them. Raising this further is cheap in Nix and expensive to read,
  # since the enum below is what a human scrolls when they forget a name.
  maxDepth = 3;

  entries = lib.readDir src;
  files = lib.attrNames (lib.filterAttrs (_: type: type == "regular" || type == "symlink") entries);
  dirs = lib.attrNames (lib.filterAttrs (_: type: type == "directory") entries);

  # Immediate subdirectories of a src-relative path, minus hidden ones.
  # Hidden directories are kept at the top level (`.tangled` is a real scope)
  # but not below it, where they are caches and editor state.
  subDirsOf = dir:
    lib.filter (name: !lib.hasPrefix "." name) (
      lib.attrNames (lib.filterAttrs (_: type: type == "directory") (lib.readDir (src + "/${dir}")))
    );

  # `dir` itself, plus everything under it within `remaining` further levels.
  scopesUnder = remaining: dir:
    [dir]
    ++ lib.optionals (remaining > 0) (
      lib.concatMap (sub: scopesUnder (remaining - 1) "${dir}/${sub}") (subDirsOf dir)
    );

  nested = lib.concatMap (scopesUnder (maxDepth - 1)) dirs;
  allScopes = lib.sort (a: b: a < b) (nested ++ files);
  scopeOptionsYaml = lib.concatMapStringsSep "\n" (s: "      - ${s}") allScopes;
  commitlintrcContent = lib.replaceStrings ["@SCOPE_OPTIONS@"] [scopeOptionsYaml] (
    lib.readFile ./commitlintrc.yml
  );
in
  pkgs.writeText "commitlintrc.yml" commitlintrcContent
