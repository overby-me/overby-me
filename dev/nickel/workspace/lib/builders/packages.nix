# Package builder for nix-workspace
#
# Converts validated package configuration records into Nix derivations.
# Each package config (from workspace.ncl or discovered .ncl files) is
# mapped to a concrete derivation using the appropriate build system.
#
# Supported build systems (v0.1):
#   - "generic"  → stdenv.mkDerivation
#   - "rust"     → rustPlatform.buildRustPackage
#   - "go"       → buildGoModule
#
{lib}: let
  # Resolve a list of package attribute names to actual packages from nixpkgs.
  #
  # Type: Pkgs -> [String] -> [Derivation]
  resolvePkgList = pkgs: names:
    map (
      name:
        pkgs.${name}
        or (throw "nix-workspace: package '${name}' not found in nixpkgs")
    )
    names;

  # Build a generic package using stdenv.mkDerivation.
  #
  # This is the fallback builder for packages that don't specify a
  # build system, or explicitly set build-system = "generic".
  buildGeneric = pkgs: workspaceRoot: name: cfg:
    pkgs.stdenv.mkDerivation (
      {
        pname = name;
        version = cfg.version or "0.0.0";

        src =
          if cfg ? src
          then workspaceRoot + "/${cfg.src}"
          else workspaceRoot;

        buildInputs = resolvePkgList pkgs (cfg.build-inputs or []);
        nativeBuildInputs = resolvePkgList pkgs (cfg.native-build-inputs or []);
      }
      // (lib.optionalAttrs (cfg ? description) {
        meta = {inherit (cfg) description;};
      })
      // (lib.optionalAttrs (cfg ? meta) {
        meta =
          (cfg.meta or {})
          // (lib.optionalAttrs (cfg ? description) {
            inherit (cfg) description;
          });
      })
      // (lib.optionalAttrs (cfg ? env) cfg.env)
      // (cfg.override or {})
    );

  # Build a Rust package using rustPlatform.buildRustPackage.
  #
  # Expects a Cargo.lock to exist at the package source root.
  buildRust = pkgs: workspaceRoot: name: cfg: let
    src =
      if cfg ? src
      then workspaceRoot + "/${cfg.src}"
      else workspaceRoot;
  in
    pkgs.rustPlatform.buildRustPackage (
      {
        pname = name;
        version = cfg.version or "0.0.0";

        inherit src;
        cargoLock.lockFile = src + "/${cfg.cargo-lock or "Cargo.lock"}";

        buildInputs = resolvePkgList pkgs (cfg.build-inputs or []);
        nativeBuildInputs = resolvePkgList pkgs (cfg.native-build-inputs or []);
      }
      // (lib.optionalAttrs (cfg ? description) {
        meta.description = cfg.description;
      })
      // (lib.optionalAttrs (cfg ? env) cfg.env)
      // (cfg.override or {})
    );

  # Build a Go package using buildGoModule.
  #
  # Expects a go.sum to exist at the package source root.
  buildGo = pkgs: workspaceRoot: name: cfg: let
    src =
      if cfg ? src
      then workspaceRoot + "/${cfg.src}"
      else workspaceRoot;
  in
    pkgs.buildGoModule (
      {
        pname = name;
        version = cfg.version or "0.0.0";

        inherit src;

        # Users must provide a vendorHash or use vendoring
        vendorHash = cfg.vendor-hash or null;

        buildInputs = resolvePkgList pkgs (cfg.build-inputs or []);
        nativeBuildInputs = resolvePkgList pkgs (cfg.native-build-inputs or []);
      }
      // (lib.optionalAttrs (cfg ? description) {
        meta.description = cfg.description;
      })
      // (lib.optionalAttrs (cfg ? env) cfg.env)
      // (cfg.override or {})
    );

  # Route a package config to the correct builder based on build-system.
  #
  # Type: Pkgs -> Path -> String -> AttrSet -> Derivation
  buildPackage = pkgs: workspaceRoot: name: cfg: let
    buildSystem = cfg.build-system or "generic";
    builder =
      {
        generic = buildGeneric;
        rust = buildRust;
        go = buildGo;
      }
      .${
        buildSystem
      }
      or (throw "nix-workspace: unknown build-system '${buildSystem}' for package '${name}'");
  in
    builder pkgs workspaceRoot name cfg;

  # Build all packages from a config attrset for a given system.
  #
  # Type: Nixpkgs -> Path -> [String] -> String -> AttrSet -> AttrSet
  #
  # Arguments:
  #   nixpkgs         — The nixpkgs input (for import)
  #   nixpkgsConfig   — Config to apply when importing nixpkgs (e.g. { allowUnfree })
  #   workspaceRoot   — Path to the workspace root directory
  #   workspaceSystems — The workspace-level systems list
  #   system          — The current system string
  #   packageConfigs  — Attrset of { name = packageConfig; ... }
  #
  # Returns: Attrset of { name = derivation; ... } for this system.
  buildAllPackages = {
    nixpkgs,
    nixpkgsConfig ? {},
    workspaceRoot,
    workspaceSystems,
    system,
    packageConfigs,
  }: let
    pkgs = import nixpkgs {
      inherit system;
      config = nixpkgsConfig;
    };

    # Filter packages that should be built for this system
    relevantPackages =
      lib.filterAttrs (
        _name: cfg: let
          targetSystems = cfg.systems or workspaceSystems;
        in
          builtins.elem system targetSystems
      )
      packageConfigs;
  in
    lib.mapAttrs (
      name: cfg:
        buildPackage pkgs workspaceRoot name cfg
    )
    relevantPackages;
in {
  inherit
    buildPackage
    buildAllPackages
    buildGeneric
    buildRust
    buildGo
    resolvePkgList
    ;
}
