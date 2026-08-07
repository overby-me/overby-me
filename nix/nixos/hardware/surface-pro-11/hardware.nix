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

    # The one thing that genuinely needs patching.  ath12k only learns that a
    # machine wants rfkill disabled from an ACPI bitflag, which a
    # devicetree-booted machine never reads, so Wi-Fi comes up hard-blocked
    # with nothing in userspace able to clear it:
    #
    #   1: phy0: Wireless LAN
    #     Soft blocked: no
    #     Hard blocked: yes
    #
    # The chip is otherwise fine; the firmware loads and the radio is
    # detected.  Two commits from dwhinham's tree fix it, five lines between
    # them: one adds a `disable-rfkill` devicetree property to the driver, the
    # other sets it on this board.  Neither is upstream as of 7.1, and
    # mainline still only has ath12k_acpi_get_disable_rfkill.
    #
    # This costs a full kernel build, and keeps costing one on every nixpkgs
    # bump, because linuxPackages_latest is no longer the cached article.
    # Build it on the machine itself rather than emulated:
    #
    #   nixos-rebuild switch --flake .#armitas \
    #     --target-host root@armitas --build-host root@armitas
    #
    # Drop both patches if the series ever lands upstream.
    boot.kernelPatches = [
      {
        name = "ath12k-disable-rfkill-via-devicetree";
        patch = ./patches/ath12k-disable-rfkill-via-devicetree.patch;
      }
      {
        name = "denali-disable-rfkill";
        patch = ./patches/denali-disable-rfkill.patch;
      }

      # Once a kernel build is being paid for anyway, this one is free.  It
      # flips d3_closes_handle to false in ssam_controller_caps_load_from_of,
      # the defaults the Surface aggregator picks when it was described by a
      # devicetree rather than ACPI, so it cannot affect an x86 Surface.
      #
      # It is a workaround, not a fix.  dwhinham checked the Surface Pro 11's
      # SSDT and the upstream default of true matches what the ACPI tables
      # say, so the real bug is somewhere in the EC blocking path
      # (dwhinham/linux-surface-pro-11#23).  With the flag off, resume from
      # the power key, an external keyboard or the lid is reported stable,
      # with one known quirk: waking by opening the lid can leave the Surface
      # keyboard and touchpad dead, and unplugging them does not help.
      # Another suspend/resume cycle using the power key brings them back.
      {
        name = "ssam-sp11-suspend-resume";
        patch = ./patches/ssam-sp11-suspend-resume.patch;
      }
    ];

    # Two further patches from that tree are deliberately not taken.
    #
    # "HACK: Allow setting Wi-Fi MAC address via devicetree" is redundant:
    # networking.nix sets the MAC from userspace with a udev rule, and the
    # patch reads a local-mac-address property that mainline's devicetree does
    # not set, so on its own it would do nothing.
    #
    # "drm/msm/dp: Enable support for eDP v1.4+ link rates table" fixes a
    # panel probe failure on the Samsung ATNA30DW01-1 this machine has, but
    # the panel probes fine here and offers two modes, so the failure it
    # describes is not happening.  It is also the largest of them by far, 120
    # lines rewriting DP link training.  Worth revisiting only if the display
    # turns out to be capped below its native 120 Hz.

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

      initrd.kernelModules = [
        # For LVM.
        "dm_mod"

        # Load the GPU and display clock controllers before stage 2, or there
        # is no display at all.
        #
        # arm-smmu is built into the kernel and probes during init.  The GPU's
        # own SMMU at 3da0000 takes its clocks and CX power domain from gpucc,
        # which nixpkgs builds as a module (CLK_X1E80100_GPUCC=m), so udev
        # only loads it in stage 2.  By then the deferred-probe window has
        # closed and the SMMU has given up:
        #
        #   arm-smmu 3da0000.iommu: deferred probe timeout, ignoring dependency
        #   arm-smmu 3da0000.iommu: probe with driver arm-smmu failed with error -110
        #
        # The GPU then never gets an IOMMU, msm_dpu never binds, simpledrm
        # keeps /dev/dri/card0 with no render node, and the first compositor
        # to start spins on it at 100%+ CPU behind a black screen.  The
        # give-away is gpucc_x1e80100 sitting in lsmod with a refcount of 0.
        "gpucc-x1e80100"
        "dispcc-x1e80100"
      ];

      kernelParams = [
        # Carried over from upstream: ask for the deep sleep state where the
        # firmware offers one.  Suspend is unreliable either way, see the
        # known gaps in default.nix.
        "mem_sleep_default=deep"
        # The X1E clock and power-domain trees are only partly described, so
        # the kernel must not gate anything it cannot account for.
        "clk_ignore_unused"
        "pd_ignore_unused"

        # Do NOT add iommu.passthrough=1 here.  It was tried against USB
        # squashfs corruption on the installer and boots to a black screen a
        # few seconds in: the display pipeline scans out through the SMMU, so
        # forcing every device onto an identity domain takes the panel with
        # it.  Removing it again fixed the display.
        #
        # If DMA corruption does come back, the knob to reach for instead is
        # arm-smmu.disable_bypass=0, which permits bypass only for stream IDs
        # that never got attached to a context, rather than turning
        # translation off machine-wide.  One SMMU instance does fail to probe
        # here (arm-smmu 3da0000.iommu, error -110), so something is genuinely
        # unattached.
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
