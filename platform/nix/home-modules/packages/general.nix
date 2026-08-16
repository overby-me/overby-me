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
    # Linux x86_64-only.  euro-office would evaluate on aarch64 as well, but
    # it builds CEF and the editor core from source, which is not something to
    # hand an emulated builder or the tablet itself.  The
    # onlyoffice-desktopeditors it replaces was x86_64-only too, so the office
    # suite stays where it already was.
    #
    # `.app` rather than the bare attribute: platform/nix/packages/euro-office/default.nix
    # resolves the top level to the data bundle (fonts, dictionaries,
    # templates) so the flake's package set stays green everywhere, and hangs
    # the real application off passthru.  Installing the bare attribute gets
    # you no editors at all.
    ++ lib.optionals (pkgs.stdenv.isLinux && pkgs.stdenv.hostPlatform.isx86_64) [
      euro-office.app
    ];
}
