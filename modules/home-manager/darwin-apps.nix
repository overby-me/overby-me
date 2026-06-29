{
  lib,
  pkgs,
  ...
}:
# Make home-manager GUI apps discoverable by macOS Spotlight, Launchpad and the
# Dock.
#
# By default (with home.stateVersion < 25.11) home-manager links GUI apps into
# ~/Applications/Home Manager Apps as symlinks pointing into the Nix store
# (targets.darwin.linkApps). macOS Spotlight / Launch Services refuse to index
# symlinked .app bundles, so apps like Zed never show up in Spotlight, Launchpad
# or the Dock.
#
# copyApps instead materializes real .app bundle copies (via
# rsync --copy-unsafe-links), which Spotlight does index. It requires the App
# Management permission on first activation (or Full Disk Access over SSH).
#
# Once home.stateVersion >= 25.11, copyApps becomes the default and this module
# can be removed.
lib.mkIf pkgs.stdenv.isDarwin {
  targets.darwin = {
    linkApps.enable = false;
    copyApps.enable = true;
  };
}
