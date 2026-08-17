{
  pkgs,
  lib,
  ...
}: let
  # cosmic-ext-quake-terminal is built from source out of platform/nix/config/packages, so no
  # binary cache has it and every aarch64 host compiles it under emulation,
  # where it is one of the slowest things in the closure.  x86_64 only.
  quakeTerminal = pkgs.stdenv.hostPlatform.isx86_64;
in {
  environment = {
    systemPackages = with pkgs;
      [
        #cosmic-ext-applet-emoji-selector
        #cosmic-ext-applet-external-monitor-brightness
        cosmic-ext-applet-caffeine
        cosmic-ext-calculator
        wezterm
        examine
        forecast
        tasks
        cosmic-ext-tweaks
        cosmic-player
        #cosmic-reader
        #stellarshot
      ]
      ++ lib.optional quakeTerminal cosmic-ext-quake-terminal;
    sessionVariables = {
      COSMIC_DATA_CONTROL_ENABLED = 1;
    };
  };
  services = {
    desktopManager.cosmic.enable = true;
    displayManager.cosmic-greeter.enable = true;
    system76-scheduler.enable = true;
  };
  systemd.user.services.cosmic-ext-quake-terminal = lib.mkIf quakeTerminal {
    description = "COSMIC Quake Terminal Daemon";
    wantedBy = ["graphical-session.target"];
    partOf = ["graphical-session.target"];
    after = ["graphical-session.target"];
    path = [pkgs.wezterm];
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

  # ── Home-Manager ──────────────────────────────────────────────────────────
  home-manager.sharedModules = [
    {
      # Open COSMIC Terminal straight into zellij, the same way GNOME Console
      # does through `org/gnome/Console`.`shell`.
      #
      # cosmic-config stores one file per config key and reads
      # `cosmic/<app-id>/v1/<key>` out of the XDG data dirs as the default,
      # with `~/.config/cosmic/<app-id>/v1/<key>` taking precedence. Shipping
      # the profile as a data-dir default rather than a config file leaves the
      # settings UI free to write, instead of pinning a read-only store
      # symlink over the path it needs to save to.
      xdg.dataFile = {
        "cosmic/com.system76.CosmicTerm/v1/profiles".text = ''
          {
              0: (
                  name: "Zellij",
                  command: "zellij-cwd",
                  syntax_theme_dark: "COSMIC Dark",
                  syntax_theme_light: "COSMIC Light",
                  tab_title: "",
                  working_directory: "",
                  drain_on_exit: false,
              ),
          }
        '';
        # Which of the above profiles new terminals launch with.
        "cosmic/com.system76.CosmicTerm/v1/default_profile".text = "Some(0)";
      };
    }
  ];
}
