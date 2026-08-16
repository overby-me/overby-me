{pkgs, ...}: {
  programs.atuin = {
    enable = true;
    package = pkgs.pkgsUnstable.atuin;
    settings = {
      inline_height = 10;
    };
  };
}
