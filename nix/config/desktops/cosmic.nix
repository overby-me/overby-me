{pkgs, ...}: {
  environment = {
    systemPackages = with pkgs; [
      #cosmic-ext-applet-emoji-selector
      #cosmic-ext-applet-external-monitor-brightness
      cosmic-ext-applet-caffeine
      cosmic-ext-calculator
      cosmic-ext-quake-terminal
      examine
      forecast
      tasks
      cosmic-ext-tweaks
      cosmic-player
      #cosmic-reader
      #stellarshot
    ];
    sessionVariables = {
      COSMIC_DATA_CONTROL_ENABLED = 1;
    };
    etc = {
      # Quake terminal: daemon via XDG autostart, hotkey via swhkd
      # (COSMIC's built-in Spawn shortcuts break after Alt-Tab)
      "xdg/autostart/cosmic-ext-quake-terminal.desktop".text = ''
        [Desktop Entry]
        Type=Application
        Name=COSMIC Quake Terminal
        Exec=${pkgs.cosmic-ext-quake-terminal}/bin/cosmic-ext-quake-terminal
        NoDisplay=true
        X-COSMIC-Autostart=true
      '';
      "swhkd/swhkdrc".text = ''
        grave
          ${pkgs.cosmic-ext-quake-terminal}/bin/cosmic-ext-quake-terminal toggle
      '';
    };
  };
  services = {
    desktopManager.cosmic.enable = true;
    displayManager.cosmic-greeter.enable = true;
    system76-scheduler.enable = true;
  };
  systemd = {
    # swhkd runs as root to read input devices
    services.swhkd = {
      description = "Simple Wayland HotKey Daemon";
      after = ["graphical.target"];
      wantedBy = ["graphical.target"];
      environment.PKEXEC_UID = "1000";
      serviceConfig = {
        ExecStart = "${pkgs.swhkd}/bin/swhkd -c /etc/swhkd/swhkdrc";
        Restart = "on-failure";
        RestartSec = 3;
      };
    };
    user = {
      # swhks user server for environment passing
      services.swhks = {
        description = "Simple Wayland HotKey Server";
        wantedBy = ["graphical-session.target"];
        partOf = ["graphical-session.target"];
        serviceConfig = {
          ExecStart = "${pkgs.swhkd}/bin/swhks";
          Restart = "on-failure";
          RestartSec = 3;
        };
      };
      # Fix Zed open urls: https://github.com/NixOS/nixpkgs/issues/189851#issuecomment-1759954096
      extraConfig = ''
        DefaultEnvironment="PATH=/run/wrappers/bin:/etc/profiles/per-user/%u/bin:/nix/var/nix/profiles/default/bin:/run/current-system/sw/bin"
      '';
    };
  };

  # Needed to make Zed login work in Cosmic
  xdg.portal = {
    enable = true;
    config = {
      common = {
        default = "*";
        "org.freedesktop.impl.portal.Secret" = "gnome-keyring";
      };
      gnome = {
        default = "*";
        "org.freedesktop.impl.portal.Secret" = "gnome-keyring";
      };
      gtk = {
        default = "*";
        "org.freedesktop.impl.portal.Secret" = "gnome-keyring";
      };
    };
  };
}
