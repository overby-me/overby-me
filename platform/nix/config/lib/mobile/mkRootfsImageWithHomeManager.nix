# Like mkRootfsImage, but also includes Home Manager activation packages
# in the image closure so the first boot can activate home-manager profiles.
#
# Works with any Android-bootloader device running NixOS.
# Originally replicated from
# https://github.com/gian-reto/nixos-fairphone-fp5/blob/main/flake.nix
#
# Parameters:
#   nixosConfig - a NixOS system configuration (with home-manager module)
#   pkgs        - nixpkgs package set
#
# Returns: a derivation producing an ext4 filesystem image.
lib: {
  mkRootfsImageWithHomeManager = nixosConfig: pkgs: let
    # Get all users that have home-manager configurations.
    hmUsers = lib.attrNames (nixosConfig.config.home-manager.users or {});

    # Collect all home-manager activation packages.
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
        # Create the profile directory structure.
        mkdir -p ./files/nix/var/nix/profiles
        mkdir -p ./files/nix/var/nix/profiles/per-user

        # Create first-generation NixOS profile.
        ln -s ${nixosConfig.config.system.build.toplevel} ./files/nix/var/nix/profiles/system-1-link
        # Set "system" to point to first-generation profile.
        ln -s system-1-link ./files/nix/var/nix/profiles/system

        # The bootloader expects /init.
        ln -s /nix/var/nix/profiles/system/init ./files/init

        # Create home-manager profiles for each user.
        ${lib.concatStringsSep "\n" (map (user: ''
            # Create profile directory for ${user}.
            mkdir -p ./files/nix/var/nix/profiles/per-user/${user}

            # Create first-generation home-manager profile for ${user}.
            ln -s ${nixosConfig.config.home-manager.users.${user}.home.activationPackage} \
              ./files/nix/var/nix/profiles/per-user/${user}/home-manager-1-link
            # Set home-manager to point to first-generation home profile.
            ln -s home-manager-1-link \
              ./files/nix/var/nix/profiles/per-user/${user}/home-manager

            # Create user's .nix-profile symlink.
            mkdir -p ./files/home/${user}
            ln -s /nix/var/nix/profiles/per-user/${user}/home-manager \
              ./files/home/${user}/.nix-profile
          '')
          hmUsers)}
      '';
    };
}
