# Package configuration records into derivations, by build system:
#   "generic" → stdenv.mkDerivation
#   "rust"    → rustPlatform.buildRustPackage
#   "go"      → buildGoModule
{lib}: let
  # Resolve a list of package attribute names to actual packages from nixpkgs.
  resolvePkgList = pkgs: names:
    map (
      name:
        pkgs.${name}
        or (throw "nix-workspace: package '${name}' not found in nixpkgs")
    )
    names;

  # The fallback, for a package naming no build system.
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

  # Expects a Cargo.lock at the package source root.
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

  # Expects a go.sum at the package source root.
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
