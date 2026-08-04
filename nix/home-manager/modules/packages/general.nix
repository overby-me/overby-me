{
  pkgs,
  lib,
  ...
}: {
  home.packages = with pkgs.pkgsUnstable;
    [
      # Cross-platform GUI/CLI apps (Linux and Darwin).
      mpv
      signal-desktop
    ]
    # Slack ships prebuilt binaries for x86_64-linux and both Darwin arches
    # only, so it throws "Unsupported system" on aarch64-linux (armitas,
    # phone) rather than merely being unavailable.
    ++ lib.optionals (pkgs.stdenv.isDarwin || pkgs.stdenv.hostPlatform.isx86_64) [
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
