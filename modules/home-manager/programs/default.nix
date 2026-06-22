{
  pkgs,
  lib,
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
    # Browser has no aarch64-darwin build.
    ++ lib.optionals pkgs.stdenv.isLinux [
      ./obs-studio.nix
      ./zen-browser
    ];

  programs.home-manager.enable = true;
}
