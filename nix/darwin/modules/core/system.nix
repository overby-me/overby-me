{
  darwinStateVersion,
  hostname,
  ...
}: {
  networking.hostName = hostname;
  networking.computerName = hostname;

  system = {
    # nix-darwin's own state version (separate from home-manager's).
    stateVersion = darwinStateVersion;

    # Sensible macOS defaults. These map to `defaults write` settings and are
    # applied on `darwin-rebuild switch`.
    defaults = {
      NSGlobalDomain = {
        AppleInterfaceStyle = "Dark";
        ApplePressAndHoldEnabled = false;
        InitialKeyRepeat = 15;
        KeyRepeat = 2;
        NSAutomaticCapitalizationEnabled = false;
        NSAutomaticSpellingCorrectionEnabled = false;
        # Natural ("content tracks finger") scrolling for the trackpad.
        "com.apple.swipescrolldirection" = true;
      };

      dock = {
        autohide = true;
        mru-spaces = false;
        show-recents = false;
        tilesize = 48;
      };

      finder = {
        AppleShowAllExtensions = true;
        FXPreferredViewStyle = "Nlsv";
        ShowPathbar = true;
        ShowStatusBar = true;
      };

      # Require password immediately after sleep/screensaver.
      screencapture.location = "~/Pictures/screenshots";

      # Let the Fn (Globe) key fall through to the hidutil mapping below (needs restart).
      hitoolbox.AppleFnUsageType = "Do Nothing";
    };

    # Get as close to a Linux/PC feel as macOS's 1:1 hidutil remapping allows:
    # map the bottom-left Fn (Globe) key to Left Command so the pinky position
    # used for Ctrl+C on Linux triggers the real macOS copy/paste (Cmd) chord.
    # (hidutil cannot translate Ctrl-combos into Cmd-combos, so this is the
    # closest native approximation.) enableKeyMapping is required for any
    # system.keyboard remapping to be applied.
    keyboard = {
      enableKeyMapping = true;
      userKeyMapping = [
        {
          # Fn (Globe, 0xFF00000003) -> Left Command (0x7000000E3)
          HIDKeyboardModifierMappingSrc = 1095216660483;
          HIDKeyboardModifierMappingDst = 30064771299;
        }
      ];
    };
  };

  # Enable Touch ID for sudo.
  security.pam.services.sudo_local.touchIdAuth = true;
}
