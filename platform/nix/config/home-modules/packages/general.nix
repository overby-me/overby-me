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
    # Every Linux host, both arches: upstream's CI ships an amd64 and an arm64
    # deb, so the tablet gets the office suite too.  It costs the phone's
    # flashed rootfs about 1.4 GB, which make-ext4-fs sizes to fit.
    ++ lib.optionals pkgs.stdenv.isLinux [
      euro-office
    ];
}
