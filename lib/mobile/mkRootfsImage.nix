# Builds an ext4 image containing the NixOS system that can be flashed to the
# `userdata` partition of an Android-bootloader device using fastboot.
#
# Originally replicated from
# https://github.com/gian-reto/nixos-fairphone-fp5/blob/main/flake.nix
# and generalized to work with any Android-bootloader NixOS device.
#
# Usage:
#   lib.mkRootfsImage nixosConfig pkgs;
#   lib.mkRootfsImage nixosConfig pkgs { sshHostKeyDir = /tmp/phone-hostkeys; };
#
# Parameters:
#   nixosConfig   - a NixOS system configuration
#   pkgs          - nixpkgs package set
#   opts          - (optional) attrset of extra options:
#     sshHostKeyDir - path to a directory containing pre-generated SSH host
#                     keys (e.g. ssh_host_ed25519_key + .pub).  These are
#                     injected into /etc/ssh/ in the image so that agenix
#                     can decrypt secrets on first boot.  The directory is
#                     expected to contain files named ssh_host_*_key (and
#                     corresponding .pub files).  Private keys are installed
#                     with mode 0600, public keys with 0644.
#
#                     To produce this directory from an age-encrypted key:
#                       mkdir -p /tmp/phone-hostkeys
#                       rage -d -i ~/.ssh/id_ed25519 \
#                         config/secrets/phone-host-key.age \
#                         -o /tmp/phone-hostkeys/ssh_host_ed25519_key
#                       ssh-keygen -y -f /tmp/phone-hostkeys/ssh_host_ed25519_key \
#                         > /tmp/phone-hostkeys/ssh_host_ed25519_key.pub
#
# Returns: a derivation producing an ext4 filesystem image.
{
  mkRootfsImage = nixosConfig: pkgs: opts: let
    options =
      {
        sshHostKeyDir = null;
      }
      // opts;

    sshHostKeyCommands =
      if options.sshHostKeyDir != null
      then ''
        # Inject pre-generated SSH host keys so agenix can decrypt
        # secrets on first boot.
        mkdir -p ./files/etc/ssh
        for privkey in ${options.sshHostKeyDir}/ssh_host_*_key; do
          [ -f "$privkey" ] || continue
          name="$(basename "$privkey")"
          install -m 0600 "$privkey" "./files/etc/ssh/$name"
          if [ -f "''${privkey}.pub" ]; then
            install -m 0644 "''${privkey}.pub" "./files/etc/ssh/''${name}.pub"
          fi
        done
      ''
      else "";
  in
    pkgs.callPackage "${pkgs.path}/nixos/lib/make-ext4-fs.nix" {
      storePaths = [nixosConfig.config.system.build.toplevel];
      # Don't compress, as firmware needs to be uncompressed.
      compressImage = false;
      # Must match `fileSystems."/".device` label defined in the hardware module.
      volumeLabel = "nixos";
      populateImageCommands = ''
        # Create the profile directory structure.
        mkdir -p ./files/nix/var/nix/profiles

        # Create first-generation NixOS profile and point to our initial toplevel.
        ln -s ${nixosConfig.config.system.build.toplevel} ./files/nix/var/nix/profiles/system-1-link

        # Set "system" to point to first-generation profile.
        ln -s system-1-link ./files/nix/var/nix/profiles/system

        # The Android bootloader appends init=/init to the kernel cmdline, which
        # overrides our init=/nix/var/.../init parameter. Instead of fighting the
        # bootloader, we create the symlink it expects. This symlink is stable and
        # always points to the current generation.
        ln -s /nix/var/nix/profiles/system/init ./files/init

        ${sshHostKeyCommands}
      '';
    };
}
