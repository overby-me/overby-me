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
  };
  services = {
    desktopManager.cosmic.enable = true;
    displayManager.cosmic-greeter.enable = true;
    system76-scheduler.enable = true;
  };
  systemd.user.services.cosmic-ext-quake-terminal = {
    description = "COSMIC Quake Terminal Daemon";
    wantedBy = ["graphical-session.target"];
    partOf = ["graphical-session.target"];
    after = ["graphical-session.target"];
    serviceConfig = {
      ExecStart = "${pkgs.cosmic-ext-quake-terminal}/bin/cosmic-ext-quake-terminal";
      Restart = "on-failure";
      RestartSec = 3;
    };
  };

  # Fix Zed open urls: https://github.com/NixOS/nixpkgs/issues/189851#issuecomment-1759954096
  systemd.user.extraConfig = ''
    DefaultEnvironment="PATH=/run/wrappers/bin:/etc/profiles/per-user/%u/bin:/nix/var/nix/profiles/default/bin:/run/current-system/sw/bin"
  '';

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
