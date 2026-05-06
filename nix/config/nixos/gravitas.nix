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
    cloud-hypervisor
    android-tools
    {
      # Decrypt the user SSH keys at boot using the host SSH key (no
      # Nitrokey touch needed) and drop them straight into ~noverby/.ssh.
      # The keys are also backed up in Bitwarden.
      age.secrets = {
        noverby-id_ed25519 = {
          file = inputs.self.secrets.id_ed25519;
          path = "/home/noverby/.ssh/id_ed25519";
          owner = "noverby";
          group = "users";
          mode = "600";
          symlink = false;
        };
        noverby-id_rsa = {
          file = inputs.self.secrets.id_rsa;
          path = "/home/noverby/.ssh/id_rsa";
          owner = "noverby";
          group = "users";
          mode = "600";
          symlink = false;
        };
      };
    }
  ];
}
