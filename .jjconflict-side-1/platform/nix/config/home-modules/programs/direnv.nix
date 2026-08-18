{
  pkgs,
  lib,
  ...
}: {
  programs.direnv = {
    enable = true;
    nix-direnv.enable = true;
    silent = true;

    # direnv-instant replaces the stock prompt hook; running both means two
    # things racing to rewrite the same environment.  Disabling the nushell
    # integration leaves direnv itself in place, which is what
    # direnv-instant drives in the background.
    enableNushellIntegration = lib.mkForce false;
  };

  home.packages = [pkgs.direnv-instant];

  # Sourced by path rather than `eval`-ed, because nushell's `source` is a
  # parse-time keyword and cannot read command output the way the bash and zsh
  # hooks do.  The file comes from the package's postInstall.
  #
  # The hook polls at pre_prompt and pre_execution rather than reacting to
  # SIGUSR1 as the other shells' hooks do, since nushell has no signal traps.
  # That is upstream's design, not a limitation of this wiring.
  programs.nushell.extraConfig = ''
    source ${pkgs.direnv-instant}/share/direnv-instant/nushell.nu
  '';
}
