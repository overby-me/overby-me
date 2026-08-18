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

    # Stock kernel, no fork: Linux 7.0 merged the denali devicetree and the
    # "microsoft,denali" match, and nixpkgs' aarch64 config already builds every
    # driver this machine needs (SURFACE_*, ATH12K, DRM_MSM, SND_SOC_X1E80100,
    # I2C_HID_OF_ELAN, NVMEM_SPMI_SDAM, UCSI_PMIC_GLINK). Hydra builds it, so
    # this substitutes rather than compiling an aarch64 kernel for hours.
    boot.kernelPackages = lib.mkDefault pkgs.linuxPackages_latest;

    # The one thing that genuinely needs patching. ath12k learns that a machine
    # wants rfkill disabled only from an ACPI bitflag, which a devicetree-booted
    # machine never reads, so Wi-Fi comes up hard-blocked with nothing in
    # userspace able to clear it. The chip is otherwise fine: firmware loads and
    # the radio is detected. Two commits from dwhinham's tree fix it, five lines
    # them: one adds a `disable-rfkill` devicetree property to the driver, the
    # other sets it on this board.  Neither is upstream as of 7.1, and
    # mainline still only has ath12k_acpi_get_disable_rfkill.
    #
    # This costs a full kernel build on every nixpkgs bump, since
    # linuxPackages_latest is no longer the cached article. Build on the machine
    # rather than emulated:
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

      # Free once a kernel build is being paid for anyway. Flips
      # d3_closes_handle false in ssam_controller_caps_load_from_of, the
      # devicetree path only, so it cannot affect an x86 Surface.
      #
      # A workaround, not a fix: the SSDT agrees with upstream's default, so the
      # real bug is in the EC blocking path
      # (dwhinham/linux-surface-pro-11#23). Resume from power key, external
      # keyboard or lid is stable with it, with one quirk: waking by lid can
      # leave the Surface keyboard and touchpad dead until another
      # suspend/resume by power key.
      {
        name = "ssam-sp11-suspend-resume";
        patch = ./patches/ssam-sp11-suspend-resume.patch;
      }

      # The touchscreen, which mainline cannot see at all: the digitizer is a
      # quad-SPI MSHW0485 on QUP1 SE2 and every QUP SPI node in the denali
      # devicetree is disabled, so there is no device to bind a driver to. The
      # patch header says what the three pieces are and where they came from.
      #
      # This is the one patch here that is not a small fix. It is ~5,800 lines,
      # its SPI half is register replay captured from Windows, and it is keyed
      # to one serial engine by name. Expect it to be the thing that breaks on
      # a kernel bump, and read the header before trying to forward-port it.
      #
      # It does not get the pen working. Nothing does, on any tree.
      {
        name = "mshw0485-touchscreen";
        patch = ./patches/mshw0485-touchscreen.patch;
        # Not in nixpkgs' aarch64 config, since the driver is not in any
        # kernel it builds. SPI_QCOM_GENI and QCOM_GPI_DMA, which the patch
        # also touches, are already modules there.
        structuredExtraConfig.TOUCHSCREEN_MSHW0485 = lib.kernel.module;
      }
    ];

    # Two further patches from that tree are deliberately not taken.
    #
    # The Wi-Fi MAC one is redundant: networking.nix sets it from userspace, and
    # the patch reads a local-mac-address property mainline does not set.
    #
    # The eDP v1.4+ link rates one fixes a panel probe failure this machine does
    # not have - the Samsung ATNA30DW01-1 probes fine and offers two modes - and
    # is 120 lines rewriting DP link training. Revisit only if the display turns
    # out capped below its native 120 Hz.

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

    # Snapdragon X UEFI describes the hardware with ACPI, for Windows; Linux
    # needs the devicetree from the bootloader instead. systemd-boot picks this
    # up via installDeviceTree, which defaults on once `name` is set.
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
    #   services.iptsd.enable        iptsd is the Intel Surface touch stack and
    #                                cannot reach this panel, which is a
    #                                quad-SPI MSHW0485 rather than an Intel
    #                                PCIe one. The shape is the same, a raw
    #                                capacitance heatmap somebody has to
    #                                classify; the patch above does it in the
    #                                kernel instead of in a daemon.
    #   services.tlp.enable = false  only needed to undo nixos-hardware's
    #                                microsoft/surface module.
    #   boot.crashDump.enable        off by default.
  };
}
