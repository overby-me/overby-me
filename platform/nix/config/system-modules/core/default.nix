# Host- and distro-agnostic system-manager modules for a non-NixOS Linux host.
#
# Deliberately does not set `nix.enable`, `nixpkgs.hostPlatform` or any standard
# module: those belong to a concrete host config. home-manager.nix stays inert
# until the consumer sets `home-manager.standalone.configuration`, so taking
# this bundle never forces a user environment.
{
  imports = [
    ./packages.nix
    ./home-manager.nix
    ./kanata.nix
  ];
}
