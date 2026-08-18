# Monorepo root workspace configuration
#
# This is the root workspace for a multi-subworkspace monorepo.
# It defines shared packages and a default development shell.
#
# Subworkspaces (lib-a/ and app-b/) are automatically discovered
# because they contain their own workspace.ncl files. Their outputs
# are namespaced with their directory names:
#
#   Root:
#     packages/shared-lib.ncl  → packages.<system>.shared-lib
#     shells/                   → devShells.<system>.default (auto)
#
#   lib-a/:
#     packages/default.ncl     → packages.<system>.lib-a
#
#   app-b/:
#     packages/default.ncl     → packages.<system>.app-b
#     packages/cli.ncl         → packages.<system>.app-b-cli
{
  name = "my-monorepo",
  description = "A monorepo demonstrating nix-workspace subworkspaces",

  systems = ["x86_64-linux", "aarch64-linux"],

  # Shared package defined in the root workspace
  packages = {
    shared-lib = {
      description = "Shared library used by all subworkspaces",
      build-system = "generic",
    },
  },

  # Default development shell for the whole monorepo
  shells = {
    default = {
      packages = ["git", "nickel"],
      shell-hook = m%"
        echo "Welcome to the monorepo workspace"
        echo "  nix build .#shared-lib   — Build the shared library"
        echo "  nix build .#lib-a        — Build lib-a (subworkspace)"
        echo "  nix build .#app-b        — Build app-b (subworkspace)"
        echo "  nix build .#app-b-cli    — Build app-b CLI tool"
      "%,
    },
  },
}
