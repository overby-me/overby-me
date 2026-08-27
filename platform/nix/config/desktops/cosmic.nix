{
  lib,
  pkgs,
  ...
}: let
  # cosmic-config replaces a key wholesale rather than merging, so changing one
  # action means shipping the whole map; deriving it from upstream keeps the
  # actions COSMIC adds later, and --replace-fail catches an upstream rename.
  systemActions = pkgs.runCommand "cosmic-system-actions" {} ''
    install -m 644 ${pkgs.cosmic-settings-daemon}/share/cosmic/com.system76.CosmicSettings.Shortcuts/v1/system_actions $out
    substituteInPlace $out --replace-fail '"cosmic-term"' '"wezterm"'
  '';

  # One file per key, under the app id. See the data-dir note below.
  defaults = app:
    lib.mapAttrs' (key: text: {
      name = "cosmic/com.system76.${app}/v1/${key}";
      value = {inherit text;};
    });
in {
  environment = {
    systemPackages = with pkgs; [
      #cosmic-ext-applet-emoji-selector
      #cosmic-ext-applet-external-monitor-brightness
      cosmic-ext-applet-caffeine
      cosmic-ext-calculator
      cosmic-monitor
      wezterm
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
    ({config, ...}: {
      # cosmic-config stores one file per config key and reads
      # `cosmic/<app-id>/v1/<key>` out of the XDG data dirs as the default,
      # with `~/.config/cosmic/<app-id>/v1/<key>` taking precedence. Shipping
      # these as data-dir defaults rather than config files leaves the settings
      # UI free to write, instead of pinning a read-only store symlink over the
      # path it needs to save to. The corollary is that a key already written to
      # `~/.config` shadows what is declared here until that file is deleted:
      # this seeds a fresh machine, it does not reassert itself on a live one.
      xdg.dataFile =
        {
          # Super+T, and every other Terminal system action, open WezTerm.
          "cosmic/com.system76.CosmicSettings.Shortcuts/v1/system_actions".source = systemActions;
        }
        # Open COSMIC Terminal straight into zellij, the same way GNOME Console
        # does through `org/gnome/Console`.`shell`.
        // defaults "CosmicTerm" {
          profiles = ''
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
          default_profile = "Some(0)";
        }
        // defaults "CosmicComp" {
          autotile = "true";
          autotile_behavior = "PerWorkspace";
          xkb_config = ''
            (
                rules: "",
                model: "pc104",
                layout: "us",
                variant: "altgr-intl",
                options: Some("terminate:ctrl_alt_bksp"),
                repeat_delay: 600,
                repeat_rate: 25,
            )
          '';
          input_default = ''
            (
                state: Enabled,
                scroll_config: Some((
                    method: None,
                    natural_scroll: Some(true),
                    scroll_button: None,
                    scroll_factor: None,
                )),
            )
          '';
          input_touchpad = ''
            (
                state: Enabled,
                click_method: Some(Clickfinger),
                scroll_config: Some((
                    method: Some(TwoFinger),
                    natural_scroll: Some(true),
                    scroll_button: None,
                    scroll_factor: None,
                )),
                tap_config: Some((
                    enabled: true,
                    button_map: Some(LeftRightMiddle),
                    drag: true,
                    drag_lock: false,
                )),
            )
          '';
        }
        # The machine never puts itself to sleep.
        // defaults "CosmicIdle" {
          screen_off_time = "None";
          suspend_on_ac_time = "None";
          suspend_on_battery_time = "None";
        }
        // defaults "CosmicAppletTime" {
          military_time = "true";
          first_day_of_week = "0";
        }
        // defaults "CosmicFiles" {show_details = "false";}
        # `source` points at the image home.nix installs from nix-wallpaper;
        # without this key cosmic never picks that file up.
        // defaults "CosmicBackground" {
          same-on-all = "true";
          all = ''
            (
                output: "all",
                source: Path("${config.home.homeDirectory}/.local/share/wallpapers/current.png"),
                filter_by_theme: true,
                rotation_frequency: 300,
                filter_method: Lanczos,
                scaling_mode: Zoom,
                sampling_method: Alphanumeric,
            )
          '';
        }
        # Only the panel runs; the dock's own config is left undeclared because
        # nothing outside this list starts it.
        // defaults "CosmicPanel" {
          entries = ''
            [
                "Panel",
            ]
          '';
        }
        // defaults "CosmicPanel.Panel" {
          name = ''"Panel"'';
          anchor = "Right";
          anchor_gap = "false";
          autohide = "None";
          autohover_delay_ms = "Some(500)";
          background = "ThemeDefault";
          border_radius = "0";
          exclusive_zone = "true";
          expand_to_edges = "true";
          keyboard_interactivity = "OnDemand";
          layer = "Top";
          margin = "0";
          opacity = "1.0";
          output = "All";
          padding = "0";
          padding_overlap = "0.5";
          size = "XS";
          size_center = "None";
          size_wings = "None";
          spacing = "0";
          plugins_wings = "Some(([], []))";
          plugins_center = ''
            Some([
                "com.system76.CosmicAppletTime",
                "com.system76.CosmicAppletStatusArea",
                "com.system76.CosmicAppletTiling",
                "com.system76.CosmicAppletAudio",
                "com.system76.CosmicAppletNetwork",
                "com.system76.CosmicAppletBattery",
                "com.system76.CosmicAppletBluetooth",
                "com.system76.CosmicAppletNotifications",
                "com.system76.CosmicAppletPower",
            ])
          '';
        };
    })
  ];
}
