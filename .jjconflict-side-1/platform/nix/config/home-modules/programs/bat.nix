{pkgs, ...}: {
  programs.bat = {
    enable = true;
    extraPackages = with pkgs.bat-extras; [
      prettybat
      batgrep
      batdiff
    ];
  };
}
