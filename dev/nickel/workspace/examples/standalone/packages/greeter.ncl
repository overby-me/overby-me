# Package: greeter
#
# A simple greeter package for the standalone workspace example.
# This file is auto-discovered from the packages/ convention directory.
#
# Build with:
#   nix-workspace build greeter
#
# Or if you have a flake.nix:
#   nix build .#greeter
{
  description = "A friendly greeter",
  build-system = "generic",
}
