# Shared library package — defined in the root workspace
#
# This package is available to all subworkspaces and appears in
# the flake outputs as packages.<system>.shared-lib (no namespace
# prefix since it belongs to the root workspace).
{
  description = "Shared library used across the monorepo",
  build-system = "generic",
  src = "./src",
}
