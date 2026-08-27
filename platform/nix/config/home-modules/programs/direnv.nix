{
  pkgs,
  lib,
  ...
}: let
  # Upstream calls `direnv-instant start` unguarded from a prompt hook, so a
  # Ctrl+C landing while it runs leaves nushell printing a
  # `terminated_by_signal` trace over the prompt - which is what interrupting
  # a `cd` completion does. Swallowing it costs nothing: the hook runs again on
  # the next prompt, and the cached env file is what it would have reused.
  nushellHook = pkgs.runCommand "direnv-instant-nushell.nu" {} ''
    install -m 644 ${pkgs.direnv-instant}/share/direnv-instant/nushell.nu $out
    substituteInPlace $out --replace-fail \
      'let raw = (^direnv-instant start | str trim)' \
      'let raw = (try { ^direnv-instant start | str trim } catch { "" })'
  '';
in {
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
    source ${nushellHook}
  '';
}
