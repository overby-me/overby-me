{
  lib,
  pkgs,
  ...
}:
# Spotlight and Launch Services refuse to index symlinked .app bundles, so
# home-manager's default `linkApps` leaves GUI apps invisible to Spotlight,
# Launchpad and the Dock. `copyApps` materialises real bundles instead, which
# they do index; it needs App Management permission on first activation, or
# Full Disk Access over SSH.
#
# Removable once home.stateVersion reaches 25.11, where copyApps is the default.
lib.mkIf pkgs.stdenv.isDarwin {
  targets.darwin = {
    linkApps.enable = false;
    copyApps.enable = true;
  };
}
