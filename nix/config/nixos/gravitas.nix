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
    .dell-precision-3490-intel
    inputs.catppuccin.nixosModules.catppuccin
    inputs.home-manager.nixosModules.home-manager
    inputs.ragenix.nixosModules.default
    inputs.self.hardware.dell-precision-3491
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
    cloud-hypervisor
    android-tools
    {
      # Decrypt the user SSH keys at boot using the host SSH key (no
      # Nitrokey touch needed) and drop them straight into ~overby.me/.ssh.
      # The keys are also backed up in Bitwarden.
      age.secrets = {
        overby-me-id_ed25519 = {
          file = inputs.self.secrets.id_ed25519;
          path = "/home/overby.me/.ssh/id_ed25519";
          owner = "overby.me";
          group = "users";
          mode = "600";
          symlink = false;
        };
        overby-me-id_rsa = {
          file = inputs.self.secrets.id_rsa;
          path = "/home/overby.me/.ssh/id_rsa";
          owner = "overby.me";
          group = "users";
          mode = "600";
          symlink = false;
        };
      };
    }
  ];
}
