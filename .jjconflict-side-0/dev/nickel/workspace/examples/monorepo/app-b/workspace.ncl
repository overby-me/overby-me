# app-b subworkspace configuration
#
# This is a subworkspace within the monorepo. It is automatically
# discovered by nix-workspace because this directory contains a
# workspace.ncl file.
#
# Outputs are namespaced with the directory name "app-b":
#   packages/default.ncl → packages.<system>.app-b
#   packages/cli.ncl     → packages.<system>.app-b-cli
#
# This subworkspace declares a dependency on the sibling subworkspace
# "lib-a", which makes lib-a's outputs available for reference.
{
  name = "app-b",
  description = "An application that depends on lib-a",

  systems = ["x86_64-linux", "aarch64-linux"],

  packages = {
    default = {
      description = "app-b main application",
      build-system = "generic",
    },
    cli = {
      description = "app-b command-line interface",
      build-system = "generic",
    },
  },

  shells = {
    default = {
      packages = ["git"],
      shell-hook = m%"
        echo "app-b development shell"
        echo "  nix build .#app-b      — Build the application"
        echo "  nix build .#app-b-cli  — Build the CLI tool"
      "%,
    },
  },

  # Cross-subworkspace dependency declaration.
  # "core" is a local alias; "lib-a" is the sibling subworkspace directory name.
  dependencies = {
    core = "lib-a",
  },
}
