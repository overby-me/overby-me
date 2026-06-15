{pkgs, ...}: {
  nix = {
    package = pkgs.pkgsUnstable.nixVersions.latest;
    settings = {
      max-jobs = "auto";
      keep-going = true;
      connect-timeout = 10;
      stalled-download-timeout = 10;
      trusted-users = [
        "root"
        "@admin"
        "overby.me"
      ];
      experimental-features = "nix-command flakes ca-derivations dynamic-derivations recursive-nix";
      download-buffer-size = 1024 * 1024 * 1024;
      substituters = [
        "https://overby-me.cachix.org"
        "https://nix-community.cachix.org"
        "https://zed.cachix.org"
      ];
      trusted-public-keys = [
        "overby-me.cachix.org-1:dU7qOj5u97QZz98nqnh+Nwait6c+2d2Eq0KTOAXTyp4="
        "nix-community.cachix.org-1:mB9FSh9qf2dCimDSUo8Zy7bkq5CX+/rkCWyvRCYg3Fs="
        "zed.cachix.org-1:/pHQ6dpMsAZk2DiP4WCL0p9YDNKWj2Q5FL20bNmw1cU="
      ];
    };

    # Garbage collection via launchd.
    gc = {
      automatic = true;
      interval = {
        Weekday = 0;
        Hour = 3;
        Minute = 0;
      };
      options = "--delete-older-than 30d";
    };

    # Optimise the store (deduplicate) on a schedule.
    optimise = {
      automatic = true;
      interval = {
        Weekday = 0;
        Hour = 4;
        Minute = 0;
      };
    };
  };
}
