{pkgs, ...}: {
  # ── NixOS ───────────────────────────────────────────────────────────────
  programs.niri.enable = true;

  environment.systemPackages = with pkgs; [
    # Launcher
    anyrun

    # Status bar
    ironbar

    # Notifications
    rustyfications

    # Screen locker
    cthulock

    # Wallpaper
    wpaperd

    # Screenshot & annotation
    wayshot
    satty

    # Clipboard
    wl-clipboard-rs
    wl-clip-persist

    # OSD for volume/brightness
    swayosd
  ];

  # Use greetd + tuigreet as the display manager for niri
  services.greetd = {
    enable = true;
    settings = {
      default_session = {
        command = "${pkgs.greetd.tuigreet}/bin/tuigreet --time --remember --cmd niri-session";
        user = "greeter";
      };
    };
  };

  security.pam.services.cthulock = {};
  services.udev.packages = [pkgs.swayosd];

  xdg.portal = {
    enable = true;
    extraPortals = with pkgs; [
      xdg-desktop-portal-gnome
      xdg-desktop-portal-gtk
    ];
    config = {
      niri = {
        default = ["gnome" "gtk"];
        "org.freedesktop.impl.portal.Secret" = "gnome-keyring";
      };
    };
  };

  # ── Home-Manager ──────────────────────────────────────────────────────────
  home-manager.sharedModules = [
    {
      services.swayosd.enable = true;
    }
  ];
}
