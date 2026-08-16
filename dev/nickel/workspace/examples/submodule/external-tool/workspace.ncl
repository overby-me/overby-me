# External tool — subworkspace configuration
#
# This simulates a git submodule that has been added to the parent
# repository. In a real project, this directory would be created by:
#
#   git submodule add <remote-url> external-tool
#
# Because this directory contains a workspace.ncl, nix-workspace
# automatically discovers it as a subworkspace. Its outputs are
# namespaced with the directory name "external-tool":
#
#   packages/default.ncl → packages.<system>.external-tool
#   packages/lib.ncl     → packages.<system>.external-tool-lib
#
# This workspace is completely self-contained — it could be built
# independently if it had its own flake.nix. When used as a
# subworkspace, its outputs are merged into the parent's flake outputs
# with automatic namespacing.
{
  name = "external-tool",
  description = "An external tool included as a git submodule",

  systems = ["x86_64-linux", "aarch64-linux"],

  packages = {
    default = {
      description = "External tool main binary",
      build-system = "generic",
    },
    lib = {
      description = "External tool shared library",
      build-system = "generic",
    },
  },

  shells = {
    default = {
      packages = ["gcc", "gnumake"],
      shell-hook = m%"
        echo "external-tool development shell"
      "%,
    },
  },
}
