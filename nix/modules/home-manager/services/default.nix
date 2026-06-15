{
  pkgs,
  lib,
  ...
}: {
  # All of these home-manager services rely on systemd user units, which are
  # Linux-only. On Darwin (launchd) they are simply not imported.
  imports = lib.optionals pkgs.stdenv.isLinux [
    ./ssh-agent.nix
    ./espanso.nix
  ];
}
