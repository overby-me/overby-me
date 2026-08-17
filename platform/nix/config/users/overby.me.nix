{
  inputs,
  pkgs,
  lib,
  config,
  ...
}: {
  home = {
    username = "overby.me";
    homeDirectory =
      if pkgs.stdenv.isDarwin
      then "/Users/${config.home.username}"
      else "/home/${config.home.username}";
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
    # Darwin-only: make home-manager GUI apps show up in Spotlight/Launchpad.
    ++ lib.optionals pkgs.stdenv.isDarwin (
      with inputs.self.homeModules; [
        darwin-apps
      ]
    )
    # Linux-only modules: the Zen Browser app plus the home modules that rely
    # on systemd user units.
    ++ lib.optionals pkgs.stdenv.isLinux (
      with inputs.self.homeModules; [
        inputs.zen-browser.homeModules.default
        systemd
      ]
    )
    # vibe is built from platform/nix/config/packages, so no binary cache has it and an aarch64
    # host compiles it under emulation.  It is a desktop audio visualiser,
    # not worth that on armitas or phone.
    ++ lib.optionals (pkgs.stdenv.isLinux && pkgs.stdenv.hostPlatform.isx86_64) (
      with inputs.self.homeModules; [
        vibe
      ]
    );
}
