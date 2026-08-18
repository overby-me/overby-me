# mkRootfsImage with the Home Manager activation packages in the image closure
# too, so the first boot can activate home-manager profiles. Works on any
# Android-bootloader device running NixOS.
#
# Replicated from
# https://github.com/gian-reto/nixos-fairphone-fp5/blob/main/flake.nix
#
# Parameters:
#   nixosConfig - a NixOS system configuration (with home-manager module)
#   pkgs        - nixpkgs package set
#
# Returns: a derivation producing an ext4 filesystem image.
lib: {
  mkRootfsImageWithHomeManager = nixosConfig: pkgs: let
    hmUsers = lib.attrNames (nixosConfig.config.home-manager.users or {});

    hmActivationPackages =
      map
      (user: nixosConfig.config.home-manager.users.${user}.home.activationPackage)
      hmUsers;
  in
    pkgs.callPackage "${pkgs.path}/nixos/lib/make-ext4-fs.nix" {
      storePaths =
        [
          nixosConfig.config.system.build.toplevel
        ]
        ++ hmActivationPackages;
      # Don't compress, as firmware needs to be uncompressed.
      compressImage = false;
      # Must match `fileSystems."/".device` label defined in the hardware module.
      volumeLabel = "nixos";
      populateImageCommands = ''
        # A first-generation profile pointing at the initial toplevel, with
        # "system" pointing at it in turn.
        mkdir -p ./files/nix/var/nix/profiles
        mkdir -p ./files/nix/var/nix/profiles/per-user
        ln -s ${nixosConfig.config.system.build.toplevel} ./files/nix/var/nix/profiles/system-1-link
        ln -s system-1-link ./files/nix/var/nix/profiles/system

        # The bootloader expects /init.
        ln -s /nix/var/nix/profiles/system/init ./files/init

        # The same first-generation shape per home-manager user, plus the
        # .nix-profile symlink their session expects.
        ${lib.concatStringsSep "\n" (map (user: ''
            mkdir -p ./files/nix/var/nix/profiles/per-user/${user}
            ln -s ${nixosConfig.config.home-manager.users.${user}.home.activationPackage} \
              ./files/nix/var/nix/profiles/per-user/${user}/home-manager-1-link
            ln -s home-manager-1-link \
              ./files/nix/var/nix/profiles/per-user/${user}/home-manager

            mkdir -p ./files/home/${user}
            ln -s /nix/var/nix/profiles/per-user/${user}/home-manager \
              ./files/home/${user}/.nix-profile
          '')
          hmUsers)}
      '';
    };
}
