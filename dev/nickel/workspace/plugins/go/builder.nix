# Enhanced Go builder for the nix-workspace-go plugin
#
# Extends the base Go builder with plugin-specific configuration fields:
#   - go-version selection (go_1_21, go_1_22, go_1_23, go_1_24)
#   - sub-packages for multi-binary builds
#   - build tags and ldflags
#   - CGO toggle
#   - vendor/proxy configuration
#   - Test flags and do-check toggle
#
# This builder is registered by the Go plugin and invoked when a package
# has build-system = "go" and the nix-workspace-go plugin is loaded.
# It falls back gracefully to the base Go builder behavior when
# plugin-specific fields are absent.
#
{lib}: let
  # Resolve the Go toolchain package based on the go-version field.
  #
  # Type: Pkgs -> String|Null -> Derivation
  #
  # The version string comes from the Nickel enum after JSON export,
  # so it arrives as a plain string like "1_23".
  resolveGoPackage = pkgs: goVersion: let
    versionAttr =
      if goVersion == null
      then "go"
      else "go_${goVersion}";
  in
    pkgs.${versionAttr}
    or (builtins.throw "nix-workspace-go: Go version '${versionAttr}' not found in nixpkgs. Available: go, go_1_21, go_1_22, go_1_23, go_1_24.");

  # Resolve a list of package attribute names to actual packages from nixpkgs.
  #
  # Type: Pkgs -> [String] -> [Derivation]
  resolvePkgList = pkgs: names:
    map (
      name:
        pkgs.${name}
        or (builtins.throw "nix-workspace-go: package '${name}' not found in nixpkgs")
    )
    names;

  # Build a Go package with full plugin support.
  #
  # Type: Pkgs -> Path -> String -> AttrSet -> Derivation
  #
  # Arguments:
  #   pkgs          — The nixpkgs package set for the target system
  #   workspaceRoot — Path to the workspace (or subworkspace) root
  #   name          — Package name
  #   cfg           — The evaluated package config (from Nickel JSON)
  #
  # This builder understands all fields from the GoPackage contract
  # in addition to the base PackageConfig fields.
  buildGo = pkgs: workspaceRoot: name: cfg: let
    src =
      if cfg ? src
      then workspaceRoot + "/${cfg.src}"
      else workspaceRoot;

    # Go version selection
    goVersion = cfg.go-version or null;
    goPackage = resolveGoPackage pkgs goVersion;

    # Sub-packages: if specified, build only those paths
    subPackages = cfg.sub-packages or [];

    # Build tags
    tags = cfg.tags or [];
    tagsFlag =
      if tags != []
      then ["-tags" (builtins.concatStringsSep "," tags)]
      else [];

    # Linker flags
    ldflags = cfg.ldflags or [];
    ldflagsStr =
      if ldflags != []
      then builtins.concatStringsSep " " ldflags
      else null;

    # CGO
    cgoEnabled = cfg.cgo-enabled or false;
    CGO_ENABLED =
      if cgoEnabled
      then "1"
      else "0";

    # Vendor hash
    vendorHash = cfg.vendor-hash or null;

    # Proxy vendor
    proxyVendor = cfg.proxy-vendor or true;

    # Test configuration
    doCheck = cfg.do-check or true;
    checkFlags = cfg.check-flags or [];
  in
    pkgs.buildGoModule (
      {
        pname = name;
        version = cfg.version or "0.0.0";

        inherit src;
        inherit vendorHash proxyVendor;

        # Use the selected Go toolchain
        go = goPackage;

        inherit CGO_ENABLED;

        buildInputs = resolvePkgList pkgs (cfg.build-inputs or []);
        nativeBuildInputs = resolvePkgList pkgs (cfg.native-build-inputs or []);

        inherit doCheck;
      }
      // (lib.optionalAttrs (subPackages != []) {
        inherit subPackages;
      })
      // (lib.optionalAttrs (tagsFlag != []) {
        buildFlags = tagsFlag;
      })
      // (lib.optionalAttrs (ldflagsStr != null) {
        ldflags = [ldflagsStr];
      })
      // (lib.optionalAttrs (checkFlags != []) {
        inherit checkFlags;
      })
      // (lib.optionalAttrs (cfg ? description) {
        meta.description = cfg.description;
      })
      // (lib.optionalAttrs (cfg ? env) cfg.env)
      // (cfg.override or {})
    );

  # Build all Go packages from a config attrset for a given system.
  #
  # This mirrors the interface of the base package builder's buildAllPackages
  # but routes everything through the enhanced Go builder.
  #
  # Type: AttrSet -> AttrSet
  buildAllGoPackages = {
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
        buildGo pkgs workspaceRoot name cfg
    )
    relevantPackages;
in {
  inherit
    buildGo
    buildAllGoPackages
    resolveGoPackage
    ;

  # Metadata for the plugin system to discover this builder.
  meta = {
    name = "go";
    buildSystem = "go";
    description = "Build Go modules via buildGoModule with plugin enhancements";
  };
}
