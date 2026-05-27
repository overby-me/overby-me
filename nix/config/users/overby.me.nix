{inputs, ...}: {
  home = {
    username = "overby.me";
    homeDirectory = "/home/overby.me";
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
    claude-code
  ];
}
