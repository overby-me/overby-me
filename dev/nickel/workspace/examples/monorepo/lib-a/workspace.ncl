# lib-a subworkspace configuration
#
# This is a subworkspace within the monorepo. It is automatically
# discovered by nix-workspace because this directory contains a
# workspace.ncl file.
#
# Outputs are namespaced with the directory name "lib-a":
#   packages/default.ncl → packages.<system>.lib-a
#
# This subworkspace has no dependencies on other subworkspaces.
{
  name = "lib-a",
  description = "A shared library subworkspace",

  systems = ["x86_64-linux", "aarch64-linux"],

  packages = {
    default = {
      description = "lib-a core library",
      build-system = "generic",
    },
  },
}
