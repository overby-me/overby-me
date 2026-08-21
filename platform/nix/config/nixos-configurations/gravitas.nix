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
    inputs.home-manager.nixosModules.home-manager
    inputs.self.hardware.dell-precision-3491
    inputs.self.desktops.cosmic
    inputs.self.desktops.gnome
    inputs.self.desktops.xr
    nitrokey
    core
    programs
    services
    home-manager
    cloud-hypervisor
    android-tools
    secretspec
    ({pkgs, ...}: {
      secretspec = {
        enable = true;
        # 26.05 carries 0.10.1; the modules need the 0.19 `--reason` flag.
        package = pkgs.pkgsUnstable.secretspec;
        projectFile = ../secretspec.toml;
        profile = "gravitas";
        provider = "age://secrets/secretspec.age?identity=/etc/ssh/ssh_host_ed25519_key&recipients-file=secrets/secretspec.age.recipients";
        # The user SSH keys land straight in ~overby.me/.ssh at boot (no
        # Nitrokey touch needed); they are also backed up in Bitwarden.
        secrets = {
          SSH_ID_ED25519 = {
            encoding = "base64";
            path = "/home/overby.me/.ssh/id_ed25519";
            owner = "overby.me";
            group = "users";
            mode = "600";
          };
          SSH_ID_RSA = {
            encoding = "base64";
            path = "/home/overby.me/.ssh/id_rsa";
            owner = "overby.me";
            group = "users";
            mode = "600";
          };
        };
      };
    })
  ];
}
