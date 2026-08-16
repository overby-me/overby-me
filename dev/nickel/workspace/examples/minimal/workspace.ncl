# Minimal nix-workspace example
#
# This workspace defines a single package and a default dev shell.
# It demonstrates the basic workspace.ncl structure that nix-workspace
# evaluates and validates against the WorkspaceConfig contract.
{
  name = "minimal-example",
  description = "A minimal nix-workspace example",

  systems = ["x86_64-linux", "aarch64-linux"],

  packages = {
    hello = {
      description = "A simple hello world package",
      build-system = "generic",
    },
  },

  shells = {
    default = {
      packages = ["hello"],
      env = {
        GREETING = "Hello from nix-workspace!",
      },
      shell-hook = m%"
        echo "$GREETING"
      "%,
    },
  },
}
