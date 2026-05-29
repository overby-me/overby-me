{
  pkgs,
  lib,
  ...
}: {
  home.packages = with pkgs.pkgsUnstable;
    [
      #bitwarden
      fragments
      evince
      #bitwarden-desktop
      mpv
      dconf-editor
      rclone
      gnome-network-displays
      gnome-system-monitor
      file-roller
      wireplumber
      gnome-disk-utility
      #firefoxpwa
      cheese
      pavucontrol
      kooha
      rustdesk-flutter
    ]
    ++ lib.optionals pkgs.stdenv.hostPlatform.isx86_64 (with pkgs.pkgsUnstable; [
      slack
      onlyoffice-desktopeditors
    ]);
}
