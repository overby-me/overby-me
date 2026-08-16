# nix-workspace-rust — Built-in Rust/Cargo plugin
#
# Provides Rust-specific contracts, conventions, and builder metadata
# for building Rust packages with nix-workspace.
#
# When loaded via `plugins = ["nix-workspace-rust"]` in workspace.ncl,
# this plugin:
#
#   1. Adds a `RustPackage` contract for Rust-specific configuration
#   2. Registers a `crates/` convention directory that auto-discovers
#      Rust crate definitions and routes them to the "rust" builder
#   3. Extends `PackageConfig` with optional Rust-specific fields
#      (edition, features, cargo-lock, workspace-member)
#   4. Provides builder metadata with sensible defaults
#
# Example usage in workspace.ncl:
#
#   {
#     name = "my-rust-project",
#     plugins = ["nix-workspace-rust"],
#
#     packages = {
#       my-tool = {
#         build-system = "rust",
#         edition = '2024,
#         features = ["cli", "json"],
#       },
#     },
#   }
#
# Or with the crates/ convention directory:
#
#   my-project/
#   ├── workspace.ncl
#   └── crates/
#       ├── my-lib.ncl        → packages.<system>.my-lib  (built with rust)
#       └── my-cli.ncl        → packages.<system>.my-cli  (built with rust)
#
{
  name = "nix-workspace-rust",
  description = "Rust/Cargo build support for nix-workspace",
  version = "0.4.0",

  # ── Contracts ─────────────────────────────────────────────────
  #
  # New contract types introduced by the Rust plugin.

  contracts = {
    # RustPackage is a specialized contract for Rust packages.
    # It can be used as a standalone contract for crate .ncl files
    # that want stricter Rust-specific validation.
    RustPackage = {
      edition
        | std.contract.from_validator (fun value =>
          if !(std.is_string value) then
            'Error { message = "expected a string, got %{std.typeof value}" }
          else if std.array.elem value ["2015", "2018", "2021", "2024"] then
            'Ok
          else
            'Error {
              message = "invalid Rust edition \"%{value}\"",
              notes = ["Valid editions: \"2015\", \"2018\", \"2021\", \"2024\""],
            }
        )
        | doc m%"
            Rust edition for this crate.
            Corresponds to the `edition` field in Cargo.toml.
          "%
        | default
        = "2021",

      features
        | Array String
        | doc m%"
            Cargo features to enable when building this crate.
            Corresponds to `--features feat1,feat2` on the command line.
          "%
        | default
        = [],

      default-features
        | Bool
        | doc m%"
            Whether to enable default Cargo features.
            Set to false and use `features` to select specific features.
          "%
        | default
        = true,

      cargo-lock
        | String
        | doc m%"
            Path to Cargo.lock relative to the package source root.
            Required for reproducible builds with buildRustPackage.
          "%
        | default
        = "Cargo.lock",

      workspace-member
        | String
        | doc m%"
            If this crate is part of a Cargo workspace, the member
            directory name (used with `buildAndTestSubdir`).
            Leave unset for standalone crates.
          "%
        | optional,

      cargo-build-flags
        | Array String
        | doc m%"
            Extra flags passed to `cargo build`.
            For example: ["--release", "--target", "wasm32-unknown-unknown"]
          "%
        | default
        = [],

      cargo-test-flags
        | Array String
        | doc m%"
            Extra flags passed to `cargo test`.
            For example: ["--workspace", "--exclude", "integration-tests"]
          "%
        | default
        = [],

      use-nightly
        | Bool
        | doc m%"
            Whether to use the nightly Rust toolchain.
            When true, the builder will use pkgs.rust-bin.nightly (if available
            via an overlay) or fall back to pkgs.rustc-nightly.
          "%
        | default
        = false,
    },
  },

  # ── Conventions ───────────────────────────────────────────────
  #
  # Custom directory conventions for Rust projects.
  #
  # The `crates/` directory mirrors Cargo workspace conventions.
  # Each .ncl file in crates/ is discovered and built as a Rust package.

  conventions = {
    crates = {
      path = "crates",
      output = "packages",
      builder = "rust",
    },
  },

  # ── Builders ──────────────────────────────────────────────────
  #
  # Builder metadata for the "rust" builder.
  # The actual Nix build function is in plugins/rust/builder.nix;
  # this record provides defaults and documentation.

  builders = {
    rust = {
      name = "rust",
      description = "Build Rust packages via rustPlatform.buildRustPackage",
      defaults = {
        build-system = "rust",
      },
    },
  },

  # ── Contract extensions ───────────────────────────────────────
  #
  # Extend the core PackageConfig with optional Rust-specific fields.
  # These fields are only meaningful when build-system = "rust", but
  # they are always accepted (as optional) to avoid validation errors
  # when users declare them.

  extend = {
    PackageConfig = {
      edition
        | std.contract.from_validator (fun value =>
          if !(std.is_string value) then
            'Error { message = "expected a string, got %{std.typeof value}" }
          else if std.array.elem value ["2015", "2018", "2021", "2024"] then
            'Ok
          else
            'Error {
              message = "invalid Rust edition \"%{value}\"",
              notes = ["Valid editions: \"2015\", \"2018\", \"2021\", \"2024\""],
            }
        )
        | doc "Rust edition (only used when build-system = \"rust\")"
        | optional,

      features
        | Array String
        | doc "Cargo features to enable (only used when build-system = \"rust\")"
        | optional,

      default-features
        | Bool
        | doc "Enable default Cargo features (only used when build-system = \"rust\")"
        | optional,

      cargo-lock
        | String
        | doc "Path to Cargo.lock relative to the source root"
        | optional,

      workspace-member
        | String
        | doc "Cargo workspace member directory (for multi-crate workspaces)"
        | optional,

      cargo-build-flags
        | Array String
        | doc "Extra flags passed to cargo build"
        | optional,

      cargo-test-flags
        | Array String
        | doc "Extra flags passed to cargo test"
        | optional,

      use-nightly
        | Bool
        | doc "Use the nightly Rust toolchain"
        | optional,
    },

    ShellConfig = {
      rust-toolchain
        | std.contract.from_validator (fun value =>
          if !(std.is_string value) then
            'Error { message = "expected a string, got %{std.typeof value}" }
          else if std.array.elem value ["stable", "nightly", "minimal"] then
            'Ok
          else
            'Error {
              message = "invalid rust-toolchain \"%{value}\"",
              notes = ["Valid toolchains: \"stable\", \"nightly\", \"minimal\""],
            }
        )
        | doc m%"
            Rust toolchain to include in the development shell.
            Only used when the nix-workspace-rust plugin is loaded.
            - "stable"  — Full stable toolchain (cargo, rustc, rustfmt, clippy)
            - "nightly" — Nightly toolchain
            - "minimal" — Only cargo and rustc
          "%
        | optional,
    },
  },
}
