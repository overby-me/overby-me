{
  config,
  lib,
  ...
}: let
  cfg = config.zswap;
in {
  options.zswap = {
    enable = lib.mkEnableOption "zswap compressed swap cache";

    swapDevice = lib.mkOption {
      type = lib.types.str;
      default = "/swapfile";
      description = "Path to the swap device or file.";
    };

    swapSize = lib.mkOption {
      type = lib.types.int;
      default = 16 * 1024;
      description = "Swap size in MiB.";
    };

    maxPoolPercent = lib.mkOption {
      type = lib.types.int;
      default = 20;
      description = "Maximum percentage of RAM to use for the zswap pool.";
    };
  };

  config = lib.mkIf cfg.enable {
    boot = {
      kernelParams = [
        "zswap.enabled=1"
        "zswap.compressor=zstd"
        "zswap.zpool=zsmalloc"
        "zswap.max_pool_percent=${toString cfg.maxPoolPercent}"
        "zswap.shrinker_enabled=1"
      ];
      initrd.kernelModules = ["zsmalloc"];
      kernel.sysctl = {
        "vm.swappiness" = 180;
        "vm.watermark_boost_factor" = 0;
        "vm.watermark_scale_factor" = 125;
        "vm.page-cluster" = 0;
      };
    };

    swapDevices = [
      {
        device = cfg.swapDevice;
        size = cfg.swapSize;
      }
    ];
  };
}
