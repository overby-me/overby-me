# Generic, reusable system-manager modules for a non-NixOS Linux host.
#
# This bundle is host- and distro-agnostic: it carries the pieces that are the
# same everywhere (kanata keyboard remapping, base system packages, and the
# single-deploy home-manager activation *pattern*), but deliberately leaves the
# host-specific wiring to the consumer.
#
# In particular it does NOT set `nix.enable`, `nixpkgs.hostPlatform`, or pull in
# any downstream/company standard module — those belong to a concrete host config
# (see e.g. this flake's `systemConfigs.ubuntas`, or a downstream flake that
# composes these modules with its own standard system module).
#
# `home-manager.nix` here is inert until the consumer sets
# `home-manager.standalone.configuration`, so including this bundle never forces
# a particular user environment.
{
  imports = [
    ./packages.nix
    ./home-manager.nix
    ./kanata.nix
  ];
}
