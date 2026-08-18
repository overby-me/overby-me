{pkgs, ...}: {
  boot = {
    loader = {
      timeout = 3;
      systemd-boot = {
        enable = true;
        # Cap kernels + initrds copied to the ESP.  Older generations
        # remain in the Nix store and are still rollback-able via
        # `nixos-rebuild`, they just won't appear in the boot menu.
        configurationLimit = 10;
        # Disallow editing the kernel command line from the boot menu.
        # Without this anyone with physical access can append
        # `init=/bin/sh` and bypass login entirely — relevant for
        # encrypted-disk setups where the rest of the system is
        # protected at rest.
        editor = false;
      };
      efi.canTouchEfiVariables = true;
    };
    plymouth.enable = true;
    consoleLogLevel = 0;
    initrd = {
      verbose = false;
      systemd.enable = true;
    };
    kernelParams = [
      "boot.shell_on_fail"
      "loglevel=3"
      "plymouth.use-simpledrm"
      "quiet"
      "rd.systemd.show_status=false"
      "rd.udev.log_level=3"
      "splash"
      "udev.log_priority=3"
    ];
    kernelModules = ["v4l2loopback"];
    kernelPackages = pkgs.linuxPackages_zen;
    extraModulePackages = [pkgs.linuxPackages_zen.v4l2loopback];
    binfmt = {
      emulatedSystems = ["aarch64-linux"];
      # Needed for Docker emulation
      preferStaticEmulators = true;
    };
  };
}
