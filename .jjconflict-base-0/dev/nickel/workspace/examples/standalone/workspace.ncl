# Standalone workspace example — no flake.nix required
#
# This workspace demonstrates nix-workspace's standalone mode.
# Instead of writing a flake.nix, you use the CLI directly:
#
#   nix-workspace check              # Validate configuration
#   nix-workspace info               # Show workspace structure
#   nix-workspace build greeter      # Build (generates flake on-the-fly)
#   nix-workspace shell              # Enter dev shell (generates flake on-the-fly)
#
# The CLI generates a temporary flake.nix behind the scenes, so you
# get all the benefits of Nix flakes without writing any Nix code.
#
# To generate a permanent flake.nix later:
#   nix-workspace init --name standalone-example
#
# See: https://tangled.org/@overby.me/overby.me/tree/main/nix-workspace
{
  name = "standalone-example",
  description = "A workspace that works without a flake.nix",

  systems = ["x86_64-linux", "aarch64-linux"],

  packages = {
    greeter = {
      description = "A friendly greeter",
      build-system = "generic",
    },
  },

  shells = {
    default = {
      packages = ["hello"],
      shell-hook = m%"
        echo "Welcome to the standalone workspace!"
        echo "This workspace has no flake.nix — the CLI generates one on-the-fly."
      "%,
    },
  },
}
