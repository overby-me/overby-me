{
  lib,
  pkgs,
  ...
}: {
  nix = {
    package = pkgs.pkgsUnstable.nixVersions.latest;
    settings = {
      max-jobs = "auto";
      connect-timeout = 10;
      stalled-download-timeout = 10;
      trusted-users = ["root" "noverby"];
      experimental-features = "nix-command flakes ca-derivations dynamic-derivations recursive-nix";
      system-features = ["benchmark" "big-parallel" "kvm" "nixos-test" "recursive-nix"];
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
    daemonCPUSchedPolicy = "idle";
    daemonIOSchedClass = "idle";
    extraOptions = ''
      min-free = ${toString (30 * 1024 * 1024 * 1024)}
      max-free = ${toString (40 * 1024 * 1024 * 1024)}
    '';
  };

  # Enforce Niceness
  systemd.services.nix-daemon.serviceConfig = {
    Nice = lib.mkForce 15;
    IOSchedulingClass = lib.mkForce "idle";
    IPEgressPriority = 7;
    IPIngressPriority = 7;
  };
}
