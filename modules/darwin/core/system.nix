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
    };

    # Swap the left Control and Fn (Globe) keys via hidutil. enableKeyMapping is
    # required for any system.keyboard remapping to be applied.
    keyboard = {
      enableKeyMapping = true;
      swapLeftCtrlAndFn = true;
    };
  };

  # Enable Touch ID for sudo.
  security.pam.services.sudo_local.touchIdAuth = true;
}
