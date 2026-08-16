# Check builder for nix-workspace
#
# Converts validated CheckConfig records into check derivations.
# Checks are discovered from the checks/ convention directory
# or declared explicitly in workspace.ncl.
#
# Each check config maps to a checks.<system>.<name> flake output.
# Checks are derivations that succeed (exit 0) when the check passes
# and fail (non-zero exit) when it doesn't. They are run by `nix flake check`.
#
# Input shape (from evaluated workspace.ncl):
#   {
#     description = "Run unit tests";
#     command = "cargo test --workspace";
#     packages = ["cargo" "rustc"];
#     env = { RUST_LOG = "debug"; };
#     systems = null;
#     timeout = 600;
#     inputs-from = ["my-tool"];
#     extra-config = {};
#   }
#
{lib}: let
  # Resolve a list of package attribute names to actual packages from nixpkgs.
  #
  # Type: Pkgs -> [String] -> [Derivation]
  resolvePkgList = pkgs: names:
    map (
      name:
        pkgs.${name}
        or (throw "nix-workspace: check references unknown package '${name}' — not found in nixpkgs")
    )
    names;

  # Build a single check derivation from a CheckConfig.
  #
  # There are two modes:
  #
  #   1. `command` mode — The config provides a shell command string.
  #      We wrap it in a runCommand derivation with the specified packages
  #      and environment variables.
  #
  #   2. `path` mode — The config provides a path to a .nix file that
  #      evaluates to a derivation. We import it directly, passing pkgs
  #      and the workspace root.
  #
  # Type: Pkgs -> Path -> String -> AttrSet -> AttrSet -> Derivation
  #
  # Arguments:
  #   pkgs              — The nixpkgs package set for the target system
  #   workspaceRoot     — Path to the workspace root directory
  #   name              — Check name (e.g. "test", "lint")
  #   checkConfig       — The evaluated CheckConfig from Nickel
  #   workspacePackages — Built packages for inputs-from resolution
  #
  buildCheck = pkgs: workspaceRoot: name: checkConfig: workspacePackages: let
    hasPath = checkConfig ? path;
    hasCommand = checkConfig ? command && checkConfig.command != "";

    resolvedPath =
      if hasPath
      then
        if lib.hasPrefix "./" checkConfig.path || lib.hasPrefix "../" checkConfig.path
        then workspaceRoot + "/${checkConfig.path}"
        else if lib.hasPrefix "/" checkConfig.path
        then /. + checkConfig.path
        else workspaceRoot + "/${checkConfig.path}"
      else null;

    # Resolve inputs-from packages
    inputsFromPackages = map (
      pkgName:
        workspacePackages.${pkgName}
          or (throw "nix-workspace: check '${name}' has inputs-from '${pkgName}' but no such package exists in the workspace")
    ) (checkConfig.inputs-from or []);

    # Resolve named packages from nixpkgs
    checkPackages = resolvePkgList pkgs (checkConfig.packages or []);

    # Environment variables
    envVars = checkConfig.env or {};

    # Timeout in seconds
    timeout = checkConfig.timeout or 600;

    # Description for the derivation
    description = checkConfig.description or "nix-workspace check: ${name}";
  in
    if resolvedPath != null
    then let
      imported = import resolvedPath;
    in
      if builtins.isFunction imported
      then imported {inherit pkgs workspaceRoot lib;}
      else if lib.isDerivation imported
      then imported
      else
        throw ''
          nix-workspace: check '${name}' at '${toString resolvedPath}' must evaluate to a derivation
          or a function ({ pkgs, workspaceRoot, lib }: derivation).
        ''
    else if hasCommand
    then
      pkgs.runCommand "nix-workspace-check-${name}" (
        {
          nativeBuildInputs = checkPackages ++ inputsFromPackages;
          src = workspaceRoot;
          meta.description = description;
        }
        // envVars
        // (lib.optionalAttrs (timeout != null) {
          # Use Nix's timeout mechanism
          inherit timeout;
        })
        // (checkConfig.extra-config or {})
      ) ''
        # Copy workspace source to a writable directory
        cp -r $src source
        chmod -R u+w source
        cd source

        echo "==> Running check: ${name}"
        ${checkConfig.command}
        echo "==> Check passed: ${name}"
        touch $out
      ''
    else
      # Neither path nor command — produce a trivial passing check.
      # This covers purely declarative check entries used as metadata.
      pkgs.runCommand "nix-workspace-check-${name}" {
        meta.description = description;
      } ''
        echo "Check '${name}' has no command or path — trivially passing."
        touch $out
      '';

  # Build all checks for a given system.
  #
  # Type: Pkgs -> Path -> AttrSet -> AttrSet -> AttrSet
  #
  # Arguments:
  #   pkgs              — The nixpkgs package set for the target system
  #   workspaceRoot     — Path to the workspace root
  #   system            — Current system string
  #   workspaceSystems  — Workspace-level systems list
  #   checkConfigs      — { name = CheckConfig; ... } from workspace evaluation
  #   workspacePackages — { name = derivation; ... } built packages for inputs-from
  #   discoveredPaths   — { name = /path/to/check.nix; ... } from auto-discovery
  #
  # Returns:
  #   { name = derivation; ... } suitable for checks.${system}
  #
  buildAllChecks = {
    pkgs,
    workspaceRoot,
    system,
    workspaceSystems,
    checkConfigs,
    workspacePackages ? {},
    discoveredPaths ? {},
  }: let
    # For discovered checks without explicit config, create minimal configs
    effectiveConfigs =
      (lib.mapAttrs (_name: path: {path = toString path;}) discoveredPaths)
      // checkConfigs;

    # Filter checks to those that should run on this system
    relevantChecks =
      lib.filterAttrs (
        _name: cfg: let
          targetSystems = cfg.systems or workspaceSystems;
        in
          builtins.elem system targetSystems
      )
      effectiveConfigs;
  in
    lib.mapAttrs (
      name: cfg:
        buildCheck pkgs workspaceRoot name cfg workspacePackages
    )
    relevantChecks;
in {
  inherit
    buildCheck
    buildAllChecks
    resolvePkgList
    ;
}
