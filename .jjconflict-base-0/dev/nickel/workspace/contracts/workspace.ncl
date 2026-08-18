# Top-level workspace configuration contract
# This is the primary contract applied to workspace.ncl files.
#
# Updated for v1.0 to support overlays, checks, and templates.
#
# The `mkWorkspaceConfig` function allows the plugin system to create
# a variant of WorkspaceConfig that uses extended sub-contracts
# (e.g. PackageConfig with plugin-specific fields). This avoids
# duplicating the workspace structure in generated wrapper code.
let { System, Name, .. } = import "./common.ncl" in
let { PackageConfig, .. } = import "./package.ncl" in
let { ShellConfig, .. } = import "./shell.ncl" in
let { MachineConfig, .. } = import "./machine.ncl" in
let { ModuleConfig, HomeConfig, .. } = import "./module.ncl" in
let { OverlayConfig, .. } = import "./overlay.ncl" in
let { CheckConfig, .. } = import "./check.ncl" in
let { TemplateConfig, .. } = import "./template.ncl" in

{
  NixpkgsConfig = {
    allow-unfree
      | Bool
      | doc "Allow unfree packages from nixpkgs"
      | default
      = false,

    overlays
      | Array String
      | doc "Overlay references to apply to nixpkgs"
      | default
      = [],

    config
      | { _ : Dyn }
      | doc "Additional nixpkgs config options"
      | default
      = {},
  },

  ConventionConfig = {
    path
      | String
      | doc "Directory path (relative to workspace root)"
      | optional,

    auto-discover
      | Bool
      | doc "Enable auto-discovery of .ncl files in this directory"
      | default
      = true,
  },

  # Factory function for building workspace config contracts.
  #
  # When plugins are loaded, the Nickel evaluation wrapper calls this
  # function with extended sub-contracts to produce a workspace config
  # contract that validates plugin-specific fields.
  #
  # Without plugins, `WorkspaceConfig` (defined below) is used directly,
  # which is equivalent to `mkWorkspaceConfig PackageConfig ShellConfig`.
  #
  # Usage in generated wrapper code:
  #
  #   let ExtPkg = PackageConfig & plugin_0.extend.PackageConfig in
  #   let ExtShell = ShellConfig & plugin_0.extend.ShellConfig in
  #   let ExtWorkspaceConfig = mkWorkspaceConfig ExtPkg ExtShell in
  #   merged | ExtWorkspaceConfig
  #
  mkWorkspaceConfig = fun pkg_contract shell_contract =>
    {
      name
        | Name
        | doc "Workspace name (used for namespacing in subworkspaces)",

      description
        | String
        | doc "Human-readable description of the workspace"
        | optional,

      systems
        | Array System
        | doc "Target systems for system-multiplexed outputs"
        | default
        = ["x86_64-linux", "aarch64-linux"],

      nixpkgs
        | NixpkgsConfig
        | doc "Nixpkgs configuration"
        | default
        = {},

      packages
        | { _ : pkg_contract }
        | doc "Package definitions (auto-discovered from packages/ or declared here)"
        | default
        = {},

      shells
        | { _ : shell_contract }
        | doc "Development shell definitions (auto-discovered from shells/ or declared here)"
        | default
        = {},

      machines
        | { _ : MachineConfig }
        | doc m%"
        NixOS machine configurations (auto-discovered from machines/ or declared here).
        Each entry produces a nixosConfigurations.<name> flake output.
      "%
        | default
        = {},

      modules
        | { _ : ModuleConfig }
        | doc m%"
        NixOS module definitions (auto-discovered from modules/ or declared here).
        Each entry produces a nixosModules.<name> flake output.
      "%
        | default
        = {},

      home
        | { _ : HomeConfig }
        | doc m%"
        Home-manager module definitions (auto-discovered from home/ or declared here).
        Each entry produces a homeModules.<name> flake output.
      "%
        | default
        = {},

      overlays
        | { _ : OverlayConfig }
        | doc m%"
        Nixpkgs overlay definitions (auto-discovered from overlays/ or declared here).
        Each entry produces an overlays.<name> flake output.
        Overlays are functions (final: prev: { ... }) that extend or override nixpkgs packages.
      "%
        | default
        = {},

      checks
        | { _ : CheckConfig }
        | doc m%"
        CI check definitions (auto-discovered from checks/ or declared here).
        Each entry produces a checks.<system>.<name> flake output.
        Checks are derivations that succeed when the check passes and fail otherwise.
        Run by `nix flake check`.
      "%
        | default
        = {},

      templates
        | { _ : TemplateConfig }
        | doc m%"
        Flake template definitions (auto-discovered from templates/ or declared here).
        Each entry produces a templates.<name> flake output.
        Templates are used by `nix flake init -t` to scaffold new projects.
      "%
        | default
        = {},

      dependencies
        | { _ : DependencyRef }
        | doc m%"
        Cross-subworkspace dependency declarations.
        Keys are local aliases; values are sibling subworkspace names.
        Only meaningful for subworkspaces — ignored for the root workspace.

        Example:
          dependencies = {
            wasm = "mojo-wasm",
          }

        This declares that this subworkspace depends on a sibling
        subworkspace named "mojo-wasm", referenced locally as "wasm".
      "%
        | default
        = {},

      conventions
        | { _ : ConventionConfig }
        | doc m%"
        Override directory conventions for auto-discovery.
        Keys are convention names (e.g. "packages", "shells", "machines", "modules", "home").
      "%
        | default
        = {},

      plugins
        | Array String
        | doc m%"
        Plugins to load for this workspace.
        Each entry is a plugin name (e.g. "nix-workspace-rust", "nix-workspace-go").
        Plugins can add contracts, conventions, builders, and extend existing contracts.

        Built-in plugins:
          - "nix-workspace-rust" — Rust/Cargo build support (crates/ convention)
          - "nix-workspace-go"   — Go module build support (go-modules/ convention)

        Example:
          plugins = ["nix-workspace-rust", "nix-workspace-go"]
      "%
        | default
        = [],
    },

  # The default WorkspaceConfig contract — uses base PackageConfig and ShellConfig.
  # This is what workspace.ncl files are validated against when no plugins are loaded.
  WorkspaceConfig = mkWorkspaceConfig PackageConfig ShellConfig,

  DependencyRef
    | doc m%"
      A reference to a sibling subworkspace by its directory name.
      Must be a valid Name (alphanumeric, hyphens, underscores, starts
      with a letter or underscore).
    "%
    = Name,
}
