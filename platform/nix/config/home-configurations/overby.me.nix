{
  inputs,
  src,
  lib,
  ...
}: {
  # No system: builds once per framework system as overby.me@<system>
  # (nix-workspace's fallback for a systemless home configuration).
  inherit lib;
  extraSpecialArgs = {
    inherit inputs src;
    stateVersion = "24.05";
  };
  modules = [inputs.self.users."overby.me"];
}
