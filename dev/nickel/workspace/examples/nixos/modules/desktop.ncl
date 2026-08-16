# Desktop module configuration
#
# This Nickel config provides metadata and validation for the desktop
# NixOS module. The actual NixOS module implementation lives in
# desktop.nix alongside this file.
#
# When nix-workspace discovers modules/desktop.ncl, it:
#   1. Validates this config against the ModuleConfig contract
#   2. Looks for a matching modules/desktop.nix for the implementation
#   3. Produces a nixosModules.desktop flake output
{
  description = "Desktop environment with basic GUI applications",

  # This module has no dependencies on other workspace modules
  imports = [],

  # The NixOS option namespace this module uses
  options-namespace = "services.xserver",
}
