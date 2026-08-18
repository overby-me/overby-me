_: {
  # A lightweight NixOS VM (Apple Virtualization) that lets this Apple Silicon
  # Mac build Linux derivations. Several of this flake's NixOS configurations
  # (e.g. `phone`) use import-from-derivation that must realize aarch64-linux
  # store paths during evaluation, so `nix flake check`/`nix build` of those
  # outputs fails on a bare Darwin host. With the builder enabled, those builds
  # are offloaded to the VM.
  #
  # Only aarch64-linux is configured: it is native to the VM (no Rosetta/binfmt
  # emulation needed) and is the single Linux system the NixOS configs require
  # to be realized on Darwin. x86_64-linux configs are evaluated without IFD, so
  # no x86_64 emulation is necessary here.
  #
  # After enabling, activate once with `darwin-rebuild switch --flake .#darwitas`
  # so the builder daemon/VM is created and registered as a build machine.
  nix.linux-builder = {
    enable = true;
    ephemeral = true;
    maxJobs = 4;
    systems = ["aarch64-linux"];
    config = {
      virtualisation = {
        cores = 6;
        darwin-builder = {
          diskSize = 40 * 1024;
          memorySize = 8 * 1024;
        };
      };
    };
  };

  # `@admin` is already in nix.settings.trusted-users (see ./nix.nix), which the
  # builder requires in order to be registered and used.
}
