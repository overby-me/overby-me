{
  pkgs,
  lib,
  ...
}: {
  home.packages = with pkgs.pkgsUnstable;
    [
      # Cross-platform GUI/CLI apps (Linux and Darwin).
      mpv
      rclone
      signal-desktop
      slack
    ]
    # GNOME/PipeWire/Wayland desktop apps that only build/apply on Linux.
    ++ lib.optionals pkgs.stdenv.isLinux [
      #bitwarden
      fragments
      evince
      #bitwarden-desktop
      dconf-editor
      gnome-network-displays
      gnome-system-monitor
      file-roller
      wireplumber
      gnome-disk-utility
      #firefoxpwa
      snapshot
      pavucontrol
      kooha
      rustdesk-flutter
    ]
    # Linux x86_64-only (not available on Darwin or aarch64).
    ++ lib.optionals (pkgs.stdenv.isLinux && pkgs.stdenv.hostPlatform.isx86_64) [
      onlyoffice-desktopeditors
    ];
}
