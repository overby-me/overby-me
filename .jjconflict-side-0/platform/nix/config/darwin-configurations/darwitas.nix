{
  inputs,
  src,
  lib,
  ...
}: {
  system = "aarch64-darwin";

  specialArgs = {
    inherit src inputs lib;
    # nix-darwin's system.stateVersion (integer).
    darwinStateVersion = 6;
    # home-manager's home.stateVersion (string), passed through to the
    # home-manager modules via extraSpecialArgs as `stateVersion`.
    stateVersion = "24.05";
    hasSecrets = false;
  };

  modules = with inputs.self.darwinModules; [
    inputs.home-manager.darwinModules.home-manager

    core
    home-manager
  ];
}
