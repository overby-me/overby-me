# Rust project example — demonstrating the nix-workspace-rust plugin
#
# This workspace uses the nix-workspace-rust plugin to get:
#   - Extended PackageConfig with Rust-specific fields (edition, features, cargo-lock)
#   - The `crates/` convention directory for auto-discovering Rust crates
#   - Enhanced Rust builder with feature flags, workspace member support, etc.
#
# Directory structure:
#   rust-project/
#   ├── workspace.ncl          # This file
#   ├── packages/
#   │   └── my-tool.ncl        # A Rust package using standard packages/ convention
#   └── crates/
#       └── my-lib.ncl         # A Rust crate using the plugin's crates/ convention
#
# The crates/ directory is registered by the nix-workspace-rust plugin.
# Items discovered there are automatically built with build-system = "rust".
{
  name = "rust-project-example",
  description = "A Rust project demonstrating nix-workspace plugin support",

  systems = ["x86_64-linux", "aarch64-linux"],

  # Load the Rust plugin — this enables:
  #   - crates/ convention directory (auto-discovered, built with rust builder)
  #   - Extended PackageConfig with edition, features, cargo-lock, etc.
  #   - rust-toolchain field on ShellConfig for automatic toolchain setup
  plugins = ["nix-workspace-rust"],

  # A Rust package declared directly in workspace.ncl using plugin-extended fields
  packages = {
    my-tool = {
      description = "A CLI tool written in Rust",
      build-system = "rust",
      src = ".",

      # These fields are available because the nix-workspace-rust plugin
      # extends PackageConfig with Rust-specific contracts:
      edition = "2021",
      features = ["cli", "json"],
      cargo-lock = "Cargo.lock",
    },
  },

  # Development shell with Rust toolchain support from the plugin
  shells = {
    default = {
      packages = ["rust-analyzer"],
      env = {
        RUST_LOG = "debug",
        RUST_BACKTRACE = "1",
      },
      shell-hook = m%"
        echo "Rust project development shell"
        echo "  nix build .#my-tool   — Build the CLI tool"
        echo "  nix build .#my-lib    — Build the library (from crates/)"
        echo "  cargo build           — Build with Cargo directly"
      "%,
    },
  },
}
