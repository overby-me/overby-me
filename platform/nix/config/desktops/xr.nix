{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.desktops.xr;

  # Monado is only ever reached by the XR session, so a machine listing the
  # flatscreen one alone has no reason to carry it - and every reason not to,
  # since enabling it installs monado-vulkan-layers as an implicit layer that
  # loads into every Vulkan process on the system.
  xrSession = lib.elem "stardust-xr" cfg.sessions;
in {
  options.desktops.xr.sessions = lib.mkOption {
    type = lib.types.listOf (lib.types.enum ["stardust-xr" "stardust-xr-flatscreen"]);
    default = ["stardust-xr" "stardust-xr-flatscreen"];
    example = ["stardust-xr-flatscreen"];
    description = ''
      Which Stardust sessions cosmic-greeter lists.

      `stardust-xr` needs monado's vk_display backend, which needs a driver
      that answers vkGetPhysicalDeviceDisplayPropertiesKHR. Mesa's turnip does
      not, so on an Adreno machine that entry can only fail: drop it there and
      leave the flatscreen one.
    '';
  };

  options.desktops.xr.vkDisplay = lib.mkOption {
    type = lib.types.nullOr lib.types.int;
    default = null;
    example = 0;
    description = ''
      Which VkDisplayKHR monado's compositor takes for the `stardust-xr`
      session, as an index into the displays Vulkan enumerates.

      Null leaves monado to choose, and on a session with no compositor it
      finds nothing: `direct_wayland` wants a compositor offering DRM leases,
      `x11_direct` wants an X output with `non-desktop=1`, and the glasses are
      an ordinary monitor with an IMU. `vk_display` needs no display server
      but never detects itself, so naming an index here is the only way to it.

      The index cannot be derived without asking Vulkan on the machine, and a
      wrong one is a session that does not come up - which is what the
      `stardust-xr-flatscreen` entry beside it is for.
    '';
  };

  config = {
    environment.systemPackages = with pkgs;
      lib.optional xrSession monado
      ++ [
        stardust-xr-server
        stardust-xr-non-spatial-input
        stardust-xr-flatland
        stardust-xr-protostar
        stardust-xr-atmosphere
        #weston
      ];
    # stardust-xr-kiara was dropped from nixpkgs on 2026-07-04, "no longer
    # compatible with the latest versions of the Stardust XR server", and the
    # alias now throws.  Surviving clients if a replacement is wanted:
    # stardust-xr-phobetor, -gravity, -sphereland.

    # Socket-activated, so it takes the display when the session's first
    # client connects rather than while the greeter still holds it.
    services.monado = lib.mkIf xrSession {
      enable = true;
      defaultRuntime = true;
    };

    # The compositor reading this lives in monado-service, not in the session,
    # so setting it in the session script would do nothing. The whole service
    # is guarded, not just the variable: naming an attribute of it is enough to
    # declare the unit on a machine that never installs monado.
    systemd.user.services.monado = lib.mkIf xrSession {
      environment.XRT_COMPOSITOR_FORCE_VK_DISPLAY =
        lib.mkIf (cfg.vkDisplay != null) (toString cfg.vkDisplay);
    };

    services.displayManager.sessionPackages = [
      (pkgs.stardust-xr-session.override {enabledSessions = cfg.sessions;})
    ];

    services.udev.extraRules = ''
      # XReal

      SUBSYSTEM=="usb", ACTION=="add", ATTR{idVendor}=="3318", ATTR{idProduct}=="0424|0428|0432", MODE="0666"

      SUBSYSTEM=="input", KERNEL=="event[0-9]*", ATTRS{idVendor}=="3318", ATTRS{idProduct}=="0424|0428|0432", MODE="0666"

      SUBSYSTEM=="sound", KERNEL=="pcmC[0-9]D[0-9]p", ATTRS{idVendor}=="3318", ATTRS{idProduct}=="0424|0428|0432", MODE="0666"
      SUBSYSTEM=="sound", KERNEL=="controlC[0-9]", ATTRS{idVendor}=="3318", ATTRS{idProduct}=="0424|0428|0432", MODE="0666"

      SUBSYSTEM=="hidraw", KERNEL=="hidraw[0-9]*", ATTRS{idVendor}=="3318", ATTRS{idProduct}=="0424|0428|0432", MODE="0666"

      KERNEL=="hiddev[0-9]*", SUBSYSTEM=="usb", ATTRS{idVendor}=="3318", ATTRS{idProduct}=="0424|0428|0432", MODE="0666"
    '';
  };
}
