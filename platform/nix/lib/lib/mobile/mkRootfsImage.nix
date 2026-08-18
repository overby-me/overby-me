# Builds an ext4 image containing the NixOS system that can be flashed to the
# `userdata` partition of an Android-bootloader device using fastboot.
#
# Replicated from
# https://github.com/gian-reto/nixos-fairphone-fp5/blob/main/flake.nix and
# generalized to any Android-bootloader NixOS device.
#
# Usage:
#   lib.mkRootfsImage nixosConfig pkgs;
#   lib.mkRootfsImage nixosConfig pkgs { sshHostKeyDir = /tmp/phone-hostkeys; };
#
# Parameters:
#   nixosConfig   - a NixOS system configuration
#   pkgs          - nixpkgs package set
#   opts          - (optional) attrset of extra options:
#     sshHostKeyDir - a directory of pre-generated SSH host keys, named
#                     ssh_host_*_key with matching .pub files. They land in
#                     /etc/ssh/ in the image, private keys 0600 and public 0644,
#                     so agenix can decrypt secrets on first boot.
#
#                     To produce it from an age-encrypted key:
#                       mkdir -p /tmp/phone-hostkeys
#                       rage -d -i ~/.ssh/id_ed25519 \
#                         secrets/phone-host-key.age \
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
        # A first-generation profile pointing at the initial toplevel, with
        # "system" pointing at it in turn.
        mkdir -p ./files/nix/var/nix/profiles
        ln -s ${nixosConfig.config.system.build.toplevel} ./files/nix/var/nix/profiles/system-1-link
        ln -s system-1-link ./files/nix/var/nix/profiles/system

        # The Android bootloader appends init=/init to the kernel cmdline, which
        # overrides the init=/nix/var/.../init parameter. Rather than fight it,
        # create the symlink it expects; it is stable and always points at the
        # current generation.
        ln -s /nix/var/nix/profiles/system/init ./files/init

        ${sshHostKeyCommands}
      '';
    };
}
