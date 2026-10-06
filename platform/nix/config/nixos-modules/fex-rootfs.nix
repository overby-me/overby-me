# The x86-64 rootfs SteamOS hands Valve's FEX compatibility tool and no other
# distribution does. FEX resolves the x86 interpreter and every x86 library
# through it, so without one each x86 process dies on exec with "Invalid or
# Unsupported elf file".
#
# The kernel mounts it so that FEXServer need not, which would want erofs-utils
# and FUSE on the path of whatever launched Steam.
#
# No x86 binfmt handler on purpose: Valve's tool gives pressure-vessel a
# STEAM_COMPAT_EMULATOR manifest naming its own FEX, and exports FEX_PORTABLE=1
# to every child, which sends a store-based interpreter looking for its server
# pipe under /nix/store. Adding one later means giving it somewhere writable,
# and not combining it with boot.binfmt.emulatedSystems = ["x86_64-linux"].
{pkgs, ...}: {
  boot.kernelModules = ["erofs" "loop"];

  # nofail: a machine that cannot mount this should still boot, having lost
  # only the x86 half of Steam.
  fileSystems."/run/fex-emu/rootfs" = {
    device = "${pkgs.fex-rootfs}";
    fsType = "erofs";
    options = ["loop" "ro" "nofail"];
  };
}
