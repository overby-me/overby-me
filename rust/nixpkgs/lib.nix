# rust-nixpkgs/lib.nix
#
# Helper functions for assembling a Rust-based stdenv from individual
# component replacements.  Each "component" is a small attribute set
# describing the original C package it replaces, the Rust replacement
# package (or null when not yet available), and metadata used for
# status reporting.
#
# Design principles:
#   - Each component is declared in components/*.nix as a function
#     taking `pkgs` and returning the component attrset.
#   - Components whose `replacement` is null are silently skipped
#     when assembling the stdenv — this lets us declare the full
#     target map up front and fill in replacements incrementally.
#   - Replacement packages must be flag-compatible drop-ins for the
#     originals.  Wrappers are acceptable when needed for
#     binary name compatibility.
{lib}: let
  # Status values used in component declarations.
  #   available   — a working Rust replacement exists and is wired in
  #   in-progress — a Rust rewrite is underway (in this repo or upstream)
  #   planned     — replacement is on the roadmap but not started
  status = {
    available = "available";
    inProgress = "in-progress";
    planned = "planned";
  };

  # Source values describing where the replacement comes from.
  #   nixpkgs   — already packaged in nixpkgs (e.g. uutils, ripgrep)
  #   repo      — a sibling subproject at the monorepo root (e.g. ../make-rs)
  #   upstream  — an upstream Rust project not yet in nixpkgs
  #   internal  — will be developed inside rust-nixpkgs/crates
  source = {
    nixpkgs = "nixpkgs";
    repo = "repo";
    upstream = "upstream";
    internal = "internal";
  };

  # mkComponent — canonical constructor for a component declaration.
  #
  #   mkComponent {
  #     name        = "coreutils";
  #     original    = pkgs.coreutils;
  #     replacement = pkgs.uutils-coreutils-noprefix;
  #     status      = status.available;
  #     source      = source.nixpkgs;
  #     phase       = 1;
  #     description = "Core file/text/shell utilities";
  #     notes       = "Using uutils — drop-in noprefix variant";
  #   }
  #
  mkComponent = {
    name,
    original,
    replacement ? null,
    status ? "planned",
    source ? "internal",
    phase ? 0,
    description ? "",
    notes ? "",
  }: {
    inherit
      name
      original
      replacement
      status
      source
      phase
      description
      notes
      ;
    isAvailable = replacement != null;
  };

  # loadComponents — import every components/*.nix file and call each
  # with `pkgs`, returning an attrset keyed by component name.
  loadComponents = pkgs: let
    dir = ./components;
    entries = lib.readDir dir;
    nixFiles =
      lib.filterAttrs
      (n: t: t == "regular" && lib.hasSuffix ".nix" n)
      entries;
    load = filename: _: let
      component = import (dir + "/${filename}") {inherit pkgs lib mkComponent status source;};
    in {
      inherit (component) name;
      value = component;
    };
  in
    lib.listToAttrs (lib.mapAttrsToList load nixFiles);

  # availableComponents — filter to only components with a non-null
  # replacement.
  availableComponents = components:
    lib.filterAttrs (_: c: c.isAvailable) components;

  # componentsByPhase — group components by their phase number.
  componentsByPhase = components: let
    phaseNums = lib.unique (lib.mapAttrsToList (_: c: c.phase) components);
    grouped =
      map (
        p: {
          name = toString p;
          value = lib.filterAttrs (_: c: c.phase == p) components;
        }
      )
      phaseNums;
  in
    lib.listToAttrs grouped;

  # mkReplacements — produce a list of { original, replacement } attrsets
  # suitable for `system.replaceDependencies.replacements` or for
  # iterating when building a custom stdenv.  Only includes components
  # whose replacement is non-null.
  mkReplacements = components:
    lib.mapAttrsToList
    (_: c: {
      inherit (c) original;
      inherit (c) replacement;
    })
    (availableComponents components);

  # overrideInitialPath — given a stdenv and a components attrset,
  # produce a new initialPath where every original package that has
  # a Rust replacement is swapped out.
  overrideInitialPath = stdenv: components: let
    available = availableComponents components;
    replacementMap = lib.listToAttrs (
      lib.mapAttrsToList
      (_: c: {
        name = lib.unsafeDiscardStringContext (toString c.original);
        value = c.replacement;
      })
      available
    );
    swapPkg = pkg: let
      key = lib.unsafeDiscardStringContext (toString pkg);
    in
      replacementMap.${key} or pkg;
  in
    map swapPkg stdenv.initialPath;

  # mkRustStdenv — create an overridden stdenv with available Rust
  # replacements swapped into the initial path, and optionally the
  # shell replaced.
  #
  # This is the main entry point for Phase 7 (full oxidized stdenv).
  # Earlier phases use individual component replacements or targeted
  # overlays instead.
  mkRustStdenv = {
    stdenv,
    components,
    replaceShell ? false,
  }: let
    available = availableComponents components;
    newInitialPath = overrideInitialPath stdenv components;
    shellComponent = available.shell or null;
    newShell =
      if replaceShell && shellComponent != null
      then "${shellComponent.replacement}/bin/bash"
      else stdenv.shell;
  in
    stdenv.override {
      initialPath = newInitialPath;
      shell = newShell;
    };

  # statusReport — produce a human-readable markdown table of all
  # components and their replacement status.
  statusReport = components: let
    header = "| Component | Phase | Status | Source | Notes |\n|-----------|-------|--------|--------|-------|\n";
    rows = lib.mapAttrsToList (
      _: c: "| ${c.name} | ${toString c.phase} | ${c.status} | ${c.source} | ${c.notes} |"
    ) (lib.attrsets.mergeAttrsList (lib.mapAttrsToList (_: phase: phase) (componentsByPhase components)));
  in
    header + (lib.concatStringsSep "\n" rows);
in {
  inherit
    status
    source
    mkComponent
    loadComponents
    availableComponents
    componentsByPhase
    mkReplacements
    overrideInitialPath
    mkRustStdenv
    statusReport
    ;
}
