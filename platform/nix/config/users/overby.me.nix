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
      nushell-plugin-tramp
      nix
      home
      packages
      xdg
      programs
      services
      claude-code
      opencode
    ])
    # Darwin-only: make home-manager GUI apps show up in Spotlight/Launchpad.
    ++ lib.optionals pkgs.stdenv.isDarwin (
      with inputs.self.homeModules; [
        darwin-apps
      ]
    )
    # Linux-only modules: the home modules that rely on systemd user units,
    # plus the Zen Browser app when the evaluating tree declares its input.
    ++ lib.optionals pkgs.stdenv.isLinux (
      with inputs.self.homeModules; [
        systemd
      ]
    )
    ++ lib.optionals (pkgs.stdenv.isLinux && inputs ? zen-browser) [
      inputs.zen-browser.homeModules.default
    ]
    # vibe is built from platform/nix/packages, so no binary cache has it and an aarch64
    # host compiles it under emulation.  It is a desktop audio visualiser,
    # not worth that on armitas or phone.
    ++ lib.optionals (pkgs.stdenv.isLinux && pkgs.stdenv.hostPlatform.isx86_64) (
      with inputs.self.homeModules; [
        vibe
      ]
    );
}
