{inputs, ...}: {
  home = {
    username = "noverby";
    homeDirectory = "/home/noverby";
  };
  imports = with inputs.self.homeModules; [
    inputs.zen-browser.homeModules.default
    inputs.spicetify-nix.homeManagerModules.spicetify
    inputs.catppuccin.homeModules.catppuccin
    inputs.ragenix.homeManagerModules.default
    nushell-plugin-tramp
    nix
    home
    systemd
    packages
    xdg
    programs
    services
    catppuccin
    vibe
    peon-ping
  ];
}
