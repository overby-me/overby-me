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
      experimental-features = [
        "nix-command"
        "flakes"
        "ca-derivations"
        "dynamic-derivations"
        "recursive-nix"
      ];
      download-buffer-size = 1024 * 1024 * 1024;
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
