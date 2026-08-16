# A minimal package definition for the hello example.
#
# This file is auto-discovered from the packages/ convention directory
# and becomes `packages.<system>.hello` in the flake outputs.
{
  description = "A simple hello world package",
  build-system = "generic",
  src = "./src",
}
