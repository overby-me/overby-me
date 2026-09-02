# Builds an Android boot image that can be flashed to the `boot` partition
# using fastboot.
#
# Replicated from
# https://github.com/gian-reto/nixos-fairphone-fp5/blob/main/flake.nix and
# parameterized for any Android-bootloader device.
#
# Usage:
#   lib.mkBootImage {
#     dtb = "qcom/qcm6490-fairphone-fp5.dtb";
#   } config pkgs;
#
# Parameters:
#   deviceConfig - attrset of device-specific boot image parameters:
#     dtb             - device tree blob path relative to kernel dtbs/ dir
#     headerVersion   - (optional) boot image header version (default: 2)
#     base            - (optional) base address (default: "0x00000000")
#     kernelOffset    - (optional) kernel offset (default: "0x00008000")
#     ramdiskOffset   - (optional) ramdisk offset (default: "0x01000000")
#     dtbOffset       - (optional) DTB offset (default: "0x01f00000")
#     tagsOffset      - (optional) tags offset (default: "0x00000100")
#     pagesize        - (optional) page size (default: 4096)
#   config - the `config` of a NixOS system (e.g. nixosConfigurations.phone.config)
#   pkgs   - nixpkgs package set
#
# Returns: a derivation producing a single boot.img file.
lib: {
  mkBootImage = deviceConfig: config: pkgs: let
    cfg =
      {
        headerVersion = 2;
        base = "0x00000000";
        kernelOffset = "0x00008000";
        ramdiskOffset = "0x01000000";
        dtbOffset = "0x01f00000";
        tagsOffset = "0x00000100";
        pagesize = 4096;
      }
      // deviceConfig;
  in
    pkgs.runCommand "boot.img" {
      nativeBuildInputs = [pkgs.android-tools];
    } ''
      kernelPath="${config.system.build.kernel}"
      initrdPath="${config.system.build.initialRamdisk}/initrd"

      # The bootloader appends `init=/init` and wins anyway, and naming the
      # toplevel here would make the image unbuildable from inside the config.
      kernelParams="${lib.toString config.boot.kernelParams}"
      cmdline="$kernelParams init=/init"

      # The bootloader expects the kernel and the device tree blob as one file.
      echo "Concatenating kernel and DTB..."
      cat "$kernelPath/Image.gz" "$kernelPath/dtbs/${cfg.dtb}" > Image-with-dtb.gz

      echo "Building boot image with mkbootimg..."
      echo "Using cmdline: $cmdline"
      mkbootimg \
        --header_version ${lib.toString cfg.headerVersion} \
        --kernel Image-with-dtb.gz \
        --ramdisk "$initrdPath" \
        --cmdline "$cmdline" \
        --base ${cfg.base} \
        --kernel_offset ${cfg.kernelOffset} \
        --ramdisk_offset ${cfg.ramdiskOffset} \
        --dtb_offset ${cfg.dtbOffset} \
        --tags_offset ${cfg.tagsOffset} \
        --pagesize ${lib.toString cfg.pagesize} \
        --dtb "$kernelPath/dtbs/${cfg.dtb}" \
        -o "$out"

      echo "Boot image created successfully: $out"
      echo "Size: $(stat -c%s "$out") bytes"
    '';
}
