{
  darwinStateVersion,
  hostname,
  ...
}: {
  # nix-darwin's own state version (separate from home-manager's).
  system.stateVersion = darwinStateVersion;

  networking.hostName = hostname;
  networking.computerName = hostname;

  # Sensible macOS defaults. These map to `defaults write` settings and are
  # applied on `darwin-rebuild switch`.
  system.defaults = {
    NSGlobalDomain = {
      AppleInterfaceStyle = "Dark";
      ApplePressAndHoldEnabled = false;
      InitialKeyRepeat = 15;
      KeyRepeat = 2;
      NSAutomaticCapitalizationEnabled = false;
      NSAutomaticSpellingCorrectionEnabled = false;
      "com.apple.swipescrolldirection" = false;
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

  # Enable Touch ID for sudo.
  security.pam.services.sudo_local.touchIdAuth = true;
}
