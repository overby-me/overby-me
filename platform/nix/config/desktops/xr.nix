{
  config,
  lib,
  pkgs,
  ...
}: {
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
    environment.systemPackages = with pkgs; [
      monado
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
    services.monado = {
      enable = true;
      defaultRuntime = true;
    };

    # The compositor reading this lives in monado-service, not in the session,
    # so setting it in the session script would do nothing.
    systemd.user.services.monado.environment.XRT_COMPOSITOR_FORCE_VK_DISPLAY =
      lib.mkIf (config.desktops.xr.vkDisplay != null)
      (toString config.desktops.xr.vkDisplay);

    services.displayManager.sessionPackages = [pkgs.stardust-xr-session];

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
