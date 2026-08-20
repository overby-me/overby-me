{
  pkgs,
  lib,
  inputs,
  ...
}: {
  imports =
    [
      ./atuin.nix
      ./bash.nix
      ./bat.nix
      ./bottom.nix
      ./carapace.nix
      ./delta.nix
      ./direnv.nix
      ./gh.nix
      ./git.nix
      ./jujutsu.nix
      ./lacy.nix
      ./mergiraf.nix
      ./nix-index.nix
      ./nushell
      ./readline.nix
      ./ssh.nix
      ./starship.nix
      ./tealdeer.nix
      ./vscodium
      ./wezterm.nix
      ./zed-editor
      ./zellij.nix
    ]
    # Linux-only programs: obs-studio's plugin set is Linux-only, and Zen
    # Browser has no aarch64-darwin build. Its config module only makes
    # sense once the upstream input's home module declares the options.
    ++ lib.optionals pkgs.stdenv.isLinux [
      ./obs-studio.nix
    ]
    ++ lib.optionals (pkgs.stdenv.isLinux && inputs ? zen-browser) [
      ./zen-browser
    ];

  programs.home-manager.enable = true;
}
