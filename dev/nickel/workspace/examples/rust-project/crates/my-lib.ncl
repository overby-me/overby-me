# my-lib — A Rust library crate discovered via the crates/ convention
#
# This file is auto-discovered by the nix-workspace-rust plugin because
# it lives in the crates/ convention directory. Items in crates/ are
# automatically routed to the "rust" builder.
#
# The crates/ convention is registered by the nix-workspace-rust plugin:
#   crates/*.ncl → packages.<system>.<name> (built with rust)
#
# So this file produces: packages.<system>.my-lib
{
  description = "A Rust library crate",
  src = "./crates/my-lib",
  version = "0.1.0",
  edition = "2021",
  features = ["serde"],
}
