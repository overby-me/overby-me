# Plugin contracts for nix-workspace
#
# Defines the PluginConfig contract — the shape of a plugin definition.
# Plugins extend nix-workspace with new contracts, conventions, builders,
# and contract extensions.
#
# A plugin is a Nickel record shipped as a .ncl file, either built-in
# (in nix-workspace's plugins/ directory) or user-provided. Plugins are
# referenced by name in workspace.ncl:
#
#   { plugins = ["nix-workspace-rust", "nix-workspace-go"] }
#
# At evaluation time, each plugin's .ncl is imported and validated
# against PluginConfig. Its contracts, conventions, and extensions are
# merged into the workspace's validation and discovery pipeline.
#
# Plugin anatomy:
#
#   {
#     name = "nix-workspace-rust",
#     description = "Rust/Cargo build support",
#
#     contracts = {
#       RustPackage = {
#         edition | [| '2021, '2024 |] | default = '2024,
#         features | Array String | default = [],
#       },
#     },
#
#     conventions = {
#       crates = {
#         path = "crates",
#         output = "packages",
#         builder = "rust",
#       },
#     },
#
#     extend = {
#       PackageConfig = {
#         edition | [| '2021, '2024 |] | optional,
#         features | Array String | optional,
#         cargo-lock | String | optional,
#       },
#     },
#   }
let { Name, NonEmptyString, .. } = import "./common.ncl" in

{
  # A convention directory mapping added by a plugin.
  #
  # Maps a named convention to a directory path, output type, and builder.
  # For example, a Rust plugin might add:
  #   { path = "crates", output = "packages", builder = "rust" }
  #
  # This tells discovery to scan `crates/` for .ncl files and route
  # them to the "packages" output using the "rust" builder.
  PluginConvention = {
    path
      | NonEmptyString
      | doc m%"
          Directory path (relative to workspace root) to scan for .ncl files.
          For example, "crates" scans the crates/ directory.
        "%,

    output
      | NonEmptyString
      | doc m%"
          The flake output category these discovered items map to.
          Must be one of the standard output types: "packages", "devShells",
          "nixosModules", "homeModules", "overlays", "nixosConfigurations",
          "templates", "checks", "lib".

          Most plugin conventions map to "packages".
        "%,

    builder
      | NonEmptyString
      | doc m%"
          The builder name to use for items discovered in this directory.
          This must match a builder registered by the plugin or a built-in
          builder (e.g. "generic", "rust", "go").
        "%,

    auto-discover
      | Bool
      | doc "Whether to auto-discover .ncl files in this directory"
      | default
      = true,
  },

  # A builder definition provided by a plugin.
  #
  # On the Nickel side, this is a metadata record describing the builder.
  # The actual build logic is implemented in Nix (in the plugin's builder.nix).
  # This record tells the Nix layer which builder to invoke and what
  # defaults to apply.
  PluginBuilder = {
    name
      | Name
      | doc "Builder name, used to route packages to this builder",

    description
      | String
      | doc "Human-readable description of what this builder does"
      | optional,

    # Default configuration values applied to packages using this builder.
    # These are merged (with lower priority) into the package config before
    # the Nix builder processes it.
    defaults
      | { _ : Dyn }
      | doc m%"
          Default configuration values for packages using this builder.
          These are merged with lower priority into the package config,
          so user-specified values always win.
        "%
      | default
      = {},
  },

  # The top-level plugin definition contract.
  #
  # Every plugin .ncl file must conform to this shape. The Nickel evaluation
  # wrapper validates each plugin against PluginConfig before merging.
  PluginConfig = {
    name
      | Name
      | doc m%"
          Plugin name, typically prefixed with "nix-workspace-".
          Must be unique across all loaded plugins.
        "%,

    description
      | String
      | doc "Human-readable description of what this plugin provides"
      | optional,

    version
      | String
      | doc m%"
          Plugin version string. Informational only — not used for
          dependency resolution (yet).
        "%
      | optional,

    # ── Contracts ─────────────────────────────────────────────────
    #
    # New contract types introduced by this plugin. These become
    # available for use in workspace.ncl and convention .ncl files
    # when the plugin is loaded.
    #
    # Example:
    #   contracts = {
    #     RustPackage = {
    #       edition | [| '2021, '2024 |] | default = '2024,
    #       features | Array String | default = [],
    #     },
    #   }
    contracts
      | { _ : { _ : Dyn } }
      | doc m%"
          New contract types introduced by this plugin.
          Keys are contract names (e.g. "RustPackage"); values are
          Nickel record contracts. These are available for use in
          workspace configs when the plugin is loaded.
        "%
      | default
      = {},

    # ── Conventions ───────────────────────────────────────────────
    #
    # Custom directory conventions added by this plugin. Each convention
    # maps a directory name to an output type and builder.
    #
    # Example:
    #   conventions = {
    #     crates = { path = "crates", output = "packages", builder = "rust" },
    #   }
    conventions
      | { _ : PluginConvention }
      | doc m%"
          Custom directory conventions added by this plugin.
          Keys are convention names; values describe the directory path,
          output type, and builder to use for discovered items.
        "%
      | default
      = {},

    # ── Builders ──────────────────────────────────────────────────
    #
    # Builder definitions provided by this plugin. The actual Nix build
    # logic lives in the plugin's builder.nix; this Nickel-side record
    # provides metadata and default configuration.
    #
    # Example:
    #   builders = {
    #     rust = {
    #       name = "rust",
    #       description = "Build Rust packages via rustPlatform.buildRustPackage",
    #       defaults = { build-system = "rust" },
    #     },
    #   }
    builders
      | { _ : PluginBuilder }
      | doc m%"
          Builder definitions provided by this plugin.
          Keys are builder names; values are PluginBuilder records
          describing the builder and its defaults.
        "%
      | default
      = {},

    # ── Contract extensions ───────────────────────────────────────
    #
    # Fields to merge into existing core contracts. This allows plugins
    # to add new optional fields to PackageConfig, ShellConfig, etc.
    # without replacing the entire contract.
    #
    # Extension fields should always be optional or have defaults,
    # since they won't be present in workspaces that don't use the plugin.
    #
    # Example:
    #   extend = {
    #     PackageConfig = {
    #       edition | [| '2021, '2024 |] | optional,
    #       features | Array String | optional,
    #       cargo-lock | String | optional,
    #     },
    #   }
    extend
      | { _ : { _ : Dyn } }
      | doc m%"
          Extensions to merge into existing core contracts.
          Keys are contract names (e.g. "PackageConfig", "ShellConfig");
          values are Nickel records whose fields are merged into the
          named contract. Extension fields should be optional or have
          defaults.
        "%
      | default
      = {},
  },
}
