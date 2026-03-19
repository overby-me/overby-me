{
  imports = [
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
    ./mergiraf.nix
    ./nix-index.nix
    ./nushell
    ./obs-studio.nix
    ./readline.nix
    ./spicetify.nix
    ./ssh.nix
    ./starship.nix
    ./tealdeer.nix
    ./vscode
    ./wezterm.nix
    ./zed-editor
    ./zellij.nix
    ./zen-browser
    ./zoxide.nix
  ];

  programs.home-manager.enable = true;
}
