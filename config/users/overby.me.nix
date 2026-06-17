{
  inputs,
  pkgs,
  lib,
  ...
}: {
  home = {
    username = "overby.me";
    homeDirectory =
      if pkgs.stdenv.isDarwin
      then "/Users/overby.me"
      else "/home/overby.me";
  };
  imports =
    (with inputs.self.homeModules; [
      inputs.ragenix.homeManagerModules.default
      nushell-plugin-tramp
      nix
      home
      packages
      xdg
      programs
      services
      claude-code
    ])
    # Linux-only modules: the Zen Browser app plus the home modules that rely
    # on systemd user units (systemd, vibe).
    ++ lib.optionals pkgs.stdenv.isLinux (
      with inputs.self.homeModules; [
        inputs.zen-browser.homeModules.default
        systemd
        vibe
      ]
    );
}
