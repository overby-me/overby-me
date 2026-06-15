{
  lib,
  pkgs,
  ...
}: {
  catppuccin = {
    enable = true;
    kvantum.enable = false;
  };
  # qt platform theming (qtct/qt5ct/qt6ct) is Linux-only in home-manager.
  qt = lib.mkIf pkgs.stdenv.isLinux {
    enable = true;
    style.name = "qtct";
    platformTheme.name = "qtct";
  };
}
