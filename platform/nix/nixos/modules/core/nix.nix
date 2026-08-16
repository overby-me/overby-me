{
  lib,
  pkgs,
  ...
}: {
  nix = {
    package = pkgs.pkgsUnstable.nixVersions.latest;
    settings = {
      max-jobs = "auto";
      keep-going = true;
      # A .drv is a GC root for the whole build-time closure of what it built,
      # so keeping them holds on to the sources and compilers behind everything
      # installed, which is most of what a store grows. Evaluating writes one
      # back whenever it is actually needed again.
      #
      # Prompted by this machine running out of room in a way df cannot show:
      # btrfs metadata at 99.7% of 172 GiB with no unallocated space left, which
      # fails a rename with ENOSPC while still reporting 210 GiB free.
      keep-derivations = false;
      connect-timeout = 10;
      stalled-download-timeout = 10;
      trusted-users = ["root" "overby.me"];
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
    # Collect on a schedule rather than when the disk looks full, because on
    # btrfs it never does.
    #
    # This replaces a min-free/max-free pair that used to sit in extraOptions.
    # Those ask the filesystem how many bytes are free, and btrfs answers about
    # DATA: this machine reported 210 GiB free while its metadata stood at 99.7%
    # of 172 GiB with no unallocated space left to grow into. That is a
    # filesystem which fails a rename with ENOSPC and a daemon which sees no
    # reason to collect anything. Gone rather than tuned, since no threshold
    # helps against a number that does not move. What actually reclaims the
    # space metadata needs is `btrfs balance`, which is not nix's to run.
    gc = {
      automatic = true;
      dates = "weekly";
      options = "--delete-older-than 30d";
    };
    daemonCPUSchedPolicy = "idle";
    daemonIOSchedClass = "idle";
  };

  # Enforce Niceness
  systemd.services.nix-daemon.serviceConfig = {
    Nice = lib.mkForce 15;
    IOSchedulingClass = lib.mkForce "idle";
    IPEgressPriority = 7;
    IPIngressPriority = 7;
    MemoryMax = "90%";
    MemorySwapMax = "64G";
  };
}
