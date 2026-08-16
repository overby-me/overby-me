# Git submodule example — root workspace configuration
#
# This workspace demonstrates how nix-workspace discovers and integrates
# git submodules (or any external directory checkout) as subworkspaces.
#
# The external-tool/ directory simulates a git submodule containing its
# own workspace.ncl. nix-workspace discovers it automatically and
# namespaces its outputs with the directory name:
#
#   Root:
#     packages.my-tool               — defined here in the root workspace
#
#   external-tool/ (subworkspace):
#     packages.external-tool         — from external-tool/packages/default.ncl
#     packages.external-tool-lib     — from external-tool/packages/lib.ncl
#
# The root flake.nix does not need to list subworkspaces explicitly —
# discovery is automatic and VCS-agnostic.
{
  name = "submodule-example",
  description = "Example workspace with a git submodule as a subworkspace",

  systems = ["x86_64-linux", "aarch64-linux"],

  # A package defined in the root workspace — no namespace prefix
  packages = {
    my-tool = {
      description = "The main tool in this repository",
      build-system = "generic",
    },
  },

  # Default development shell for the whole project
  shells = {
    default = {
      packages = ["git"],
      shell-hook = m%"
        echo "Submodule example workspace"
        echo "  nix build .#my-tool            — Build the root package"
        echo "  nix build .#external-tool      — Build the submodule package"
        echo "  nix build .#external-tool-lib  — Build the submodule library"
      "%,
    },
  },
}
