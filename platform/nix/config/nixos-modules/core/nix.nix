{
  lib,
  pkgs,
  ...
}: {
  nix = {
    package = pkgs.pkgsUnstable.nixVersions.latest;
    settings = {
      max-jobs = "auto";

      # Cores per derivation, which `auto` leaves at all of them. That
      # multiplies: 22 builders each entitled to 22 compilers put this machine
      # far enough into swap to need a power cycle. A build is rarely 22-way
      # parallel for long, so the ceiling costs little.
      cores = 4;

      keep-going = true;
      # A .drv is a GC root for the whole build-time closure of what it built,
      # so keeping them holds the sources and compilers behind everything
      # installed - most of what a store grows. Evaluating writes one back when
      # it is needed again.
      keep-derivations = false;
      connect-timeout = 10;
      stalled-download-timeout = 10;
      trusted-users = ["root" "overby.me"];
      experimental-features = "nix-command flakes ca-derivations dynamic-derivations recursive-nix";
      system-features = ["benchmark" "big-parallel" "kvm" "nixos-test" "recursive-nix"];
      download-buffer-size = 1024 * 1024 * 1024;
      substituters = [
        "https://nix-community.cachix.org"
        "https://zed.cachix.org"
      ];
      trusted-public-keys = [
        "nix-community.cachix.org-1:mB9FSh9qf2dCimDSUo8Zy7bkq5CX+/rkCWyvRCYg3Fs="
        "zed.cachix.org-1:/pHQ6dpMsAZk2DiP4WCL0p9YDNKWj2Q5FL20bNmw1cU="
      ];
    };
    # On a schedule rather than when the disk looks full, because on btrfs it
    # never does. min-free/max-free ask how many bytes are free and btrfs
    # answers about DATA: this machine reported 210 GiB free with metadata at
    # 99.7% of 172 GiB and nothing left to grow into, so renames failed with
    # ENOSPC while the daemon saw no reason to collect. No threshold helps
    # against a number that does not move; what reclaims metadata space is
    # `btrfs balance`, which is not nix's to run.
    gc = {
      automatic = true;
      dates = "weekly";
      options = "--delete-older-than 30d";
    };
    daemonCPUSchedPolicy = "idle";
    daemonIOSchedClass = "idle";
  };

  systemd.services.nix-daemon.serviceConfig = {
    Nice = lib.mkForce 15;
    IOSchedulingClass = lib.mkForce "idle";
    IPEgressPriority = 7;
    IPIngressPriority = 7;

    # Throttle before the ceiling rather than only at it. Above MemoryHigh the
    # kernel puts the cgroup under reclaim pressure and slows its allocations,
    # which is what turns "too many builders" into a slow build instead of a
    # dead machine. There was only MemoryMax here, so there was nothing
    # between fine and killed.
    MemoryHigh = "60%";

    # 90% left about three gigabytes for the session on a thirty gigabyte
    # machine, which is not enough to stay usable while the daemon is at its
    # limit - and the limit is what a hard cap is for. This leaves a quarter
    # of the machine outside the builds.
    MemoryMax = "75%";

    # Swap here is zram: compressed, and living in the same RAM the limit is
    # counting. Letting the daemon push 64 GiB into it does not buy memory,
    # it spends memory to store memory, and vm.swappiness = 180 makes that
    # the first thing the kernel reaches for. Bounded so it stays a cushion.
    MemorySwapMax = "8G";
  };
}
