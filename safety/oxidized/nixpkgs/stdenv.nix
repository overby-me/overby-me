# stdenv.nix — Rust stdenv assembler
#
# Constructs a modified stdenv where C-based tools in the initialPath
# are replaced with Rust equivalents. Components are wired in from
# the component registry (components/*.nix) — each component can be
# backed by an existing Rust rewrite (e.g. uutils), a repo-root
# subproject (e.g. ../make), or left as the original C tool.
#
# Usage from an overlay:
#   let mkRustStdenv = import ./stdenv.nix; in
#   mkRustStdenv { inherit pkgs; components = import ./components pkgs; }
#
{
  pkgs,
  components,
}: let
  inherit (pkgs) lib;

  # Filter to only components that have a non-null replacement
  available = lib.filterAttrs (_: c: c.replacement != null) components;

  # Build the replacement map: original → replacement
  # Used for stdenv.initialPath overrides and closure-wide substitution
  replacementMap =
    lib.mapAttrs (_: c: {
      inherit (c) original replacement;
    })
    available;

  # Replace a single package in a list by store path identity
  replaceInList = list:
    map (
      pkg: let
        match =
          lib.findFirst
          (c: c.original == pkg)
          null
          (lib.attrValues replacementMap);
      in
        if match != null
        then match.replacement
        else pkg
    )
    list;

  # The original stdenv we're modifying
  baseStdenv = pkgs.stdenv;

  # Shell replacement: use Rust shell if available, otherwise keep bash
  shellPkg =
    if components ? shell && components.shell.replacement != null
    then components.shell.replacement
    else baseStdenv.shell;

  shellBin =
    if components ? shell && components.shell.replacement != null
    then "${components.shell.replacement}/bin/${components.shell.mainProgram or "bash"}"
    else baseStdenv.shell;

  # Construct the new initialPath by replacing known C tools with Rust equivalents.
  #
  # stdenv.initialPath contains: bash, coreutils, findutils, diffutils,
  # gnused, gnugrep, gawk, gnutar, gzip, bzip2, xz
  #
  # We walk the list and substitute any package that has a registered replacement.
  rustInitialPath = replaceInList baseStdenv.initialPath;

  # Override stdenv with Rust components.
  #
  # We use stdenv.override which reconstructs the stdenv with our
  # modified initialPath and shell. This is the same mechanism nixpkgs
  # uses internally for cross-compilation stdenvs and other variants.
  rustStdenv = baseStdenv.override {
    initialPath = rustInitialPath;
    shell = shellBin;
  };
in {
  # The assembled Rust stdenv
  stdenv = rustStdenv;

  # Individual replacement entries (for use with system.replaceDependencies
  # in NixOS configurations, mirroring the rust-nixos pattern)
  replacements =
    lib.mapAttrsToList (_: c: {
      inherit (c) original;
      # Name-match the original so closure rewriting works
      replacement = c.replacement.overrideAttrs or c.replacement (_: {
        name = c.original.name or (lib.getName c.original + "-" + lib.getVersion c.original);
      });
    })
    replacementMap;

  # Metadata for tooling and reporting
  status =
    lib.mapAttrs (_: c: {
      inherit (c) status source;
      originalName = c.original.pname or c.original.name or "unknown";
      replacementName =
        if c.replacement != null
        then c.replacement.pname or c.replacement.name or "unknown"
        else null;
    })
    components;

  # Export the shell package for use in mkDerivation overrides
  inherit shellPkg shellBin;

  # Export the full initialPath for inspection
  initialPath = rustInitialPath;
}
