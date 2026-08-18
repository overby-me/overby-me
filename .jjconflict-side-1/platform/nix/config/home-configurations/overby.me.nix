{
  inputs,
  src,
  lib,
  ...
}: {
  system = "x86_64-linux";
  inherit lib;
  extraSpecialArgs = {
    inherit inputs src;
    stateVersion = "24.05";
  };
  modules = [inputs.self.users."overby.me"];
}
