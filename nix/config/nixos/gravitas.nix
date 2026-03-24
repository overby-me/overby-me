{
  inputs,
  src,
  lib,
  ...
}: {
  system = "x86_64-linux";

  specialArgs = {
    inherit src inputs lib;
    stateVersion = "24.05";
    hasSecrets = true;
  };

  modules = with inputs.self.nixosModules; [
    inputs.nixos-hardware
    .nixosModules
    .lenovo-thinkpad-p14s-amd-gen5
    inputs.catppuccin.nixosModules.catppuccin
    inputs.home-manager.nixosModules.home-manager
    inputs.ragenix.nixosModules.default
    inputs.self.hardware.thinkpad-t14-ryzen-7-pro
    inputs.self.desktops.cosmic
    inputs.self.desktops.gnome
    inputs.self.desktops.xr
    nitrokey
    age
    core
    programs
    services
    catppuccin
    home-manager
    veo
    cloud-hypervisor
    android-tools
    {zswap.swapSize = 64 * 1024;}
  ];
}
