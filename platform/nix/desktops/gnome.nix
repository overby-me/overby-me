{pkgs, ...}: let
  gnomeExtensions = with pkgs.gnomeExtensions; [
    pop-shell
    legacy-gtk3-theme-scheme-auto-switcher
    #pop-launcher-super-key
    quake-terminal
    caffeine
    bluetooth-quick-connect
    light-style
    #current-screen-only-for-alternate-tab
    #system76-scheduler
    pkgs.breezy-desktop
  ];
in {
  # ── NixOS ───────────────────────────────────────────────────────────────
  services = {
    desktopManager.gnome.enable = true;
    gnome = {
      gnome-browser-connector.enable = true;
    };
  };
  environment.gnome.excludePackages = with pkgs; [epiphany];
  # Unmanaged gnome-extensions deps
  #environment.sessionVariables = with pkgs; {
  #  GI_TYPELIB_PATH = map (pkg: "${pkg}/lib/girepository-1.0") [vte pango harfbuzz gtk3 gdk-pixbuf at-spi2-core];
  #};

  # ── Home-Manager ──────────────────────────────────────────────────────────
  home-manager.sharedModules = [
    {
      home.packages = gnomeExtensions;
      dconf.settings = {
        "org/gnome/shell" = {
          enabled-extensions = map (extension: extension.extensionUuid) gnomeExtensions;
          disabled-extensions = [];
          favorite-apps = [
            "dev.zed.Zed.desktop"
            "org.gnome.Nautilus.desktop"
            "org.gnome.Geary.desktop"
            "org.gnome.Calendar.desktop"
            "com.spotify.Client.desktop"
          ];
        };
        "org/gnome/desktop/peripherals/mouse" = {
          natural-scroll = true;
        };
        "org/gnome/desktop/peripherals/touchpad" = {
          natural-scroll = true;
          tap-to-click = true;
        };
        "org/gnome/desktop/calendar" = {
          show-weekdate = true;
        };
        "org/gnome/desktop/interface" = {
          clock-show-date = true;
          clock-show-weekday = true;
          enable-hot-corners = false;
        };
        "org/gnome/settings-daemon.plugins/color" = {
          night-light-enabled = true;
        };
        "org/gnome/Console" = {
          shell = ["zellij-cwd"];
        };
        "org/gnome/mutter/wayland/keybindings" = {
          restore-shortcuts = [];
        };
        "org/gnome/shell/keybindings" = {
          open-application-menu = [];
          toggle-message-tray = ["<Super>v"];
          toggle-overview = [];
        };
        "org/gnome/desktop/wm/keybindings" = {
          # Not pop-shell
          switch-windows = ["<Alt>Tab"];
          switch-windows-backward = ["<Shift><Alt>Tab"];

          minimize = ["<Super>comma"];
          switch-to-workspace-left = [];
          switch-to-workspace-right = [];
          maximize = [];
          unmaximize = [];
          move-to-monitor-up = [];
          move-to-monitor-down = [];
          move-to-monitor-left = [];
          move-to-workspace-down = [];
          move-to-workspace-up = [];
          move-to-monitor-right = [];
          switch-to-workspace-down = ["<Primary><Super>Down" "<Primary><Super>j"];
          switch-to-workspace-up = ["<Primary><Super>Up" "<Primary><Super>k"];
          toggle-maximized = ["<Super>m"];
          close = ["<Super>q" "<Alt>F4"];
        };
        "org/gnome/mutter/keybindings" = {
          toggle-tiled-left = [];
          toggle-tiled-right = [];
        };
        "org/gnome/settings-daemon/plugins/media-keys" = {
          screensaver = ["<Super>Escape"];
          home = ["<Super>f"];
          email = ["<Super>e"];
          www = ["<Super>b"];
          terminal = ["<Super>t"];
          rotate-video-lock-static = [];
        };
        "org/gnome/desktop/wm/preferences" = {
          titlebar-uses-system-font = "Fira Sans Book 10";
        };
        "org/gnome/desktop/interface" = {
          interface-font-name = "Fira Sans Book 10";
          document-font-name = "Roboto Slab Regular 11";
          monospace-font-name = "Fira Mono Regular 11";
        };
      };
    }
  ];
}
