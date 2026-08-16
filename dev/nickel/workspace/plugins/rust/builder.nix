# nix-workspace-rust — Nix-side builder
#
# Enhanced Rust builder that handles plugin-specific configuration fields
# from the RustPackage contract and PackageConfig extensions.
#
# This builder is registered by the plugin system and invoked when
# a package has build-system = "rust". It extends the core buildRust
# with support for:
#   - Rust edition selection
#   - Cargo feature flags
#   - Default feature toggling
#   - Cargo workspace member builds
#   - Custom cargo build/test flags
#   - Nightly toolchain support
#
# The builder receives the full package config (after Nickel validation
# and plugin extension merging) and produces a derivation.
{lib}: let
  # Build a Rust package with full plugin support.
  #
  # Type: Pkgs -> Path -> String -> AttrSet -> Derivation
  #
  # Arguments:
  #   pkgs          — The nixpkgs package set for the target system
  #   workspaceRoot — Path to the workspace (or subworkspace) root
  #   name          — Package name
  #   cfg           — Evaluated package config (with plugin extensions)
  #
  buildRustPackage = pkgs: workspaceRoot: name: cfg: let
    src =
      if cfg ? src
      then workspaceRoot + "/${cfg.src}"
      else workspaceRoot;

    cargoLockPath = src + "/${cfg.cargo-lock or "Cargo.lock"}";

    # ── Feature flags ─────────────────────────────────────────────
    features = cfg.features or [];
    defaultFeatures = cfg.default-features or true;

    cargoBuildFlags =
      (cfg.cargo-build-flags or [])
      ++ (lib.optionals (features != []) [
        "--features"
        (builtins.concatStringsSep "," features)
      ])
      ++ (lib.optionals (!defaultFeatures) [
        "--no-default-features"
      ]);

    cargoTestFlags =
      (cfg.cargo-test-flags or [])
      ++ (lib.optionals (features != []) [
        "--features"
        (builtins.concatStringsSep "," features)
      ])
      ++ (lib.optionals (!defaultFeatures) [
        "--no-default-features"
      ]);

    # ── Workspace member support ──────────────────────────────────
    hasWorkspaceMember = cfg ? workspace-member;

    # ── Nightly toolchain ─────────────────────────────────────────
    #
    # If use-nightly is set, try to use a nightly Rust toolchain.
    # This supports rust-overlay style (pkgs.rust-bin.nightly) and
    # falls back to the default rustPlatform if nightly is not available.
    useNightly = cfg.use-nightly or false;

    # Select the rust platform — nightly if requested and available,
    # otherwise the default stable platform from nixpkgs.
    rustPlatform =
      if useNightly && (pkgs ? rust-bin)
      then
        # rust-overlay provides pkgs.rust-bin
        let
          nightlyToolchain = pkgs.rust-bin.nightly.latest.default;
        in
          pkgs.makeRustPlatform {
            cargo = nightlyToolchain;
            rustc = nightlyToolchain;
          }
      else pkgs.rustPlatform;

    # ── Resolve nixpkgs packages ──────────────────────────────────
    resolvePkgList = names:
      map (
        n:
          pkgs.${n}
          or (throw "nix-workspace-rust: package '${n}' not found in nixpkgs (required by '${name}')")
      )
      names;
  in
    rustPlatform.buildRustPackage (
      {
        pname = name;
        version = cfg.version or "0.0.0";

        inherit src;
        cargoLock.lockFile = cargoLockPath;

        inherit cargoBuildFlags cargoTestFlags;

        buildInputs = resolvePkgList (cfg.build-inputs or []);
        nativeBuildInputs = resolvePkgList (cfg.native-build-inputs or []);
      }
      // (lib.optionalAttrs hasWorkspaceMember {
        buildAndTestSubdir = cfg.workspace-member;
      })
      // (lib.optionalAttrs (cfg ? description) {
        meta.description = cfg.description;
      })
      // (lib.optionalAttrs (cfg ? env) cfg.env)
      // (cfg.override or {})
    );

  # Build a Rust development shell with plugin-specific extras.
  #
  # When the nix-workspace-rust plugin is loaded and a shell config
  # has a `rust-toolchain` field, this function adds the appropriate
  # Rust toolchain packages to the shell.
  #
  # Type: Pkgs -> AttrSet -> [Derivation]
  #
  # Arguments:
  #   pkgs        — The nixpkgs package set
  #   shellConfig — The evaluated shell config (with plugin extensions)
  #
  # Returns: A list of extra packages to add to the shell
  #
  shellExtras = pkgs: shellConfig: let
    toolchain = shellConfig.rust-toolchain or null;
  in
    if toolchain == "stable"
    then [
      pkgs.cargo
      pkgs.rustc
      pkgs.rustfmt
      pkgs.clippy
    ]
    else if toolchain == "nightly"
    then
      if pkgs ? rust-bin
      then [pkgs.rust-bin.nightly.latest.default]
      else [
        pkgs.cargo
        pkgs.rustc
        pkgs.rustfmt
        pkgs.clippy
      ]
    else if toolchain == "minimal"
    then [
      pkgs.cargo
      pkgs.rustc
    ]
    else [];
in {
  inherit buildRustPackage shellExtras;

  # Metadata for the plugin system to discover this builder.
  meta = {
    name = "rust";
    buildSystem = "rust";
    description = "Build Rust packages via rustPlatform.buildRustPackage with plugin enhancements";
  };
}
