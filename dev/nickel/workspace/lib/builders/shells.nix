# Shell builder for nix-workspace
#
# Transforms ShellConfig records (from Nickel evaluation) into
# mkShell derivations that become devShells flake outputs.
#
# Updated for v0.4 to support plugin shell extras. When plugins are
# loaded, they can contribute extra packages to dev shells (e.g. the
# Rust plugin adds cargo/rustc/clippy when `rust-toolchain` is set).
#
# Input shape (from evaluated workspace.ncl):
#   {
#     packages = ["cargo" "rustc" "rust-analyzer"];
#     env = { RUST_LOG = "debug"; };
#     shell-hook = "echo 'welcome'";
#     tools = { rust-analyzer = ""; };
#     inputs-from = ["my-tool"];
#     systems = null;  # optional override
#   }
#
{lib}: let
  # Build a single devShell derivation from a ShellConfig.
  #
  # Type: Pkgs -> String -> AttrSet -> AttrSet -> [Derivation] -> Derivation
  #
  # Arguments:
  #   pkgs              — The nixpkgs package set for the target system
  #   name              — The shell name (e.g. "default")
  #   shellConfig       — The evaluated ShellConfig from Nickel
  #   workspacePackages — Built packages from this workspace (for inputs-from)
  #   pluginExtras      — Extra packages from loaded plugins (default [])
  #
  buildShell = pkgs: name: shellConfig: workspacePackages: pluginExtras: let
    # Resolve package names to actual nixpkgs derivations.
    # Names are dot-path attribute lookups into pkgs (e.g. "cargo" → pkgs.cargo).
    resolvePackage = attrName: let
      parts = lib.splitString "." attrName;
    in
      lib.attrByPath parts
      (throw "nix-workspace: shell '${name}' references unknown package '${attrName}' — not found in nixpkgs")
      pkgs;

    shellPackages =
      map resolvePackage (shellConfig.packages or []);

    # Resolve tools: { name = version; } pairs.
    # For now, version is ignored (always latest from nixpkgs).
    # This provides a forward-compatible interface for version pinning.
    toolPackages = lib.mapAttrsToList (
      toolName: _version:
        resolvePackage toolName
    ) (shellConfig.tools or {});

    # Resolve inputs-from: include build inputs from named workspace packages.
    inputsFromPackages = map (
      pkgName:
        workspacePackages.${pkgName}
          or (throw "nix-workspace: shell '${name}' has inputs-from '${pkgName}' but no such package exists in the workspace")
    ) (shellConfig.inputs-from or []);

    # Environment variables from the config
    envVars = shellConfig.env or {};

    # Shell hook script
    shellHook = shellConfig.shell-hook or "";
  in
    pkgs.mkShell (
      {
        inherit shellHook;

        name = "nix-workspace-${name}";

        packages = shellPackages ++ toolPackages ++ pluginExtras;

        inputsFrom = inputsFromPackages;
      }
      // envVars
    );

  # Build all shells for a given system.
  #
  # Type: Pkgs -> AttrSet -> AttrSet -> [Derivation] -> AttrSet
  #
  # Arguments:
  #   pkgs              — The nixpkgs package set for the target system
  #   shellConfigs      — { name = ShellConfig; ... } from workspace evaluation
  #   workspacePackages — { name = derivation; ... } built packages for inputs-from
  #   pluginExtras      — Extra packages from loaded plugins (default [])
  #
  # Returns:
  #   { name = derivation; ... } suitable for devShells.${system}
  #
  buildShells = pkgs: shellConfigs: workspacePackages: pluginExtras:
    lib.mapAttrs (
      name: cfg:
        buildShell pkgs name cfg workspacePackages pluginExtras
    )
    shellConfigs;
in {
  inherit buildShell buildShells;
}
