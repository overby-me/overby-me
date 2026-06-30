{
  config,
  lib,
  ...
}: let
  cfg = config.zram;
in {
  options.zram = {
    enable = lib.mkEnableOption "zram compressed swap in RAM";

    memoryPercent = lib.mkOption {
      type = lib.types.int;
      default = 100;
      description = "Percentage of RAM to use for zram swap.";
    };
  };

  config = lib.mkIf cfg.enable {
    zramSwap = {
      enable = true;
      algorithm = "zstd";
      inherit (cfg) memoryPercent;
    };

    boot.kernel.sysctl = {
      "vm.swappiness" = 180;
      "vm.watermark_boost_factor" = 0;
      "vm.watermark_scale_factor" = 125;
      "vm.page-cluster" = 0;
    };
  };
}
