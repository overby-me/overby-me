{
  inputs,
  pkgs,
  ...
}: {
  imports = [inputs.peon-ping.homeManagerModules.default];

  home.packages = [
    inputs.peon-ping.packages.${pkgs.system}.default
    pkgs.libnotify
  ];

  programs.peon-ping = {
    enable = true;
    package = inputs.peon-ping.packages.${pkgs.system}.default;
    settings = {
      default_pack = "peon";
      volume = 0.5;
      enabled = true;
      desktop_notifications = true;
    };
    installPacks = ["peon"];
  };
}
