# A Rust CLI tool defined in the standard packages/ directory.
#
# When the nix-workspace-rust plugin is loaded, PackageConfig is extended
# with Rust-specific fields (edition, features, cargo-lock, etc.) that
# are validated by Nickel and passed to the enhanced Rust builder.
{
  description = "My CLI tool built with Rust",
  build-system = "rust",
  src = ".",

  # Rust plugin fields (available because plugins = ["nix-workspace-rust"])
  edition = "2021",
  features = ["cli", "json"],
  cargo-lock = "Cargo.lock",

  build-inputs = [],
  native-build-inputs = [],
}
