# Surface Pro 11 hardware module: kernel, devicetree, initrd and kernel command
# line.
{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.hardware.surfacePro11;
in {
  options.hardware.surfacePro11 = {
    enable =
      (lib.mkEnableOption "hardware support for the Surface Pro 11")
      // {default = true;};
  };

  config = lib.mkIf cfg.enable {
    nixpkgs.hostPlatform = lib.mkDefault "aarch64-linux";

    # Stock nixpkgs kernel, no fork.  Linux 7.0 merged
    # arch/arm64/boot/dts/qcom/x1e80100-microsoft-denali-oled.dts and the
    # "microsoft,denali" match in drivers/platform/surface, and nixpkgs' aarch64
    # config already builds every driver this machine needs as a module
    # (SURFACE_*, ATH12K, DRM_MSM, SND_SOC_X1E80100, I2C_HID_OF_ELAN,
    # NVMEM_SPMI_SDAM, UCSI_PMIC_GLINK, ...).  Hydra builds it, so this is a
    # substitution rather than an hours-long aarch64 kernel compile.
    boot.kernelPackages = lib.mkDefault pkgs.linuxPackages_latest;

    assertions = [
      {
        assertion = lib.versionAtLeast config.boot.kernelPackages.kernel.version "7.0";
        message = ''
          The Surface Pro 11 devicetrees were merged in Linux 7.0;
          boot.kernelPackages is ${config.boot.kernelPackages.kernel.version},
          which has no ${config.hardware.deviceTree.name}.
        '';
      }
    ];

    # The UEFI on Snapdragon X machines describes the hardware with ACPI, for
    # Windows.  Linux needs the devicetree handed to it by the bootloader
    # instead; systemd-boot picks this up through
    # boot.loader.systemd-boot.installDeviceTree, which defaults to on once
    # `name` is set.
    hardware.deviceTree = {
      enable = true;
      name = "qcom/x1e80100-microsoft-denali-oled.dtb";
    };

    # Accelerometer, ambient light sensor and screen rotation.
    hardware.sensor.iio.enable = true;

    boot = {
      # Everything else comes from includeDefaultModules, which stays on.
      # Upstream had to turn it off because the out-of-tree kernel it built was
      # missing several of those modules; nixpkgs' aarch64 kernel has them all.
      initrd.availableKernelModules = [
        # PHYs for PCIe (NVMe, Wi-Fi) and USB.
        "phy-qcom-qmp-pcie"
        "phy-qcom-qmp-usb"
        "phy-qcom-qmp-combo"
        "phy-qcom-eusb2-repeater"
        "phy-snps-eusb2"

        # Without these the NVMe drive is not detected during stage 1.
        "nvme"
        "nvmem_qcom-spmi-sdam"

        # Surface System Aggregator Module: the type cover, and the power and
        # volume keys, hang off it.  Needed early so stage 1 is interactive
        # enough to type a LUKS passphrase.
        "surface-aggregator"
        "surface-aggregator-registry"
        "surface-aggregator-hub"
        "surface-hid"
        "hid-microsoft"
      ];

      # For LVM.
      initrd.kernelModules = ["dm_mod"];

      kernelParams = [
        # Carried over from upstream: ask for the deep sleep state where the
        # firmware offers one.  Suspend is unreliable either way, see the
        # known gaps in default.nix.
        "mem_sleep_default=deep"
        # The X1E clock and power-domain trees are only partly described, so
        # the kernel must not gate anything it cannot account for.
        "clk_ignore_unused"
        "pd_ignore_unused"
      ];
    };

    # Not ported from the upstream flake, all three worked around things this
    # repository never turns on:
    #   services.iptsd.enable        IPTS is the Intel Surface touch stack; the
    #                                Pro 11 panel is an I2C-HID Elan device
    #                                driven by hid-multitouch.
    #   services.tlp.enable = false  only needed to undo nixos-hardware's
    #                                microsoft/surface module.
    #   boot.crashDump.enable        off by default.
  };
}
