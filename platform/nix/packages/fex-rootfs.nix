# FEX's own prebuilt x86-64 root filesystem, as an EROFS image.
#
# Valve's FEX compatibility tool expects the OS to hand it one of these: it
# defaults to /usr/share/guestos/fex-mesa, which is where SteamOS puts a
# combined FEX-and-Mesa image, and the rootfs/ directory in the tool's own
# depot ships empty. Without it every x86 process dies on exec with "Invalid
# or Unsupported elf file ... RootFS path doesn't exist", because FEX resolves
# the x86 interpreter and every x86 library through the rootfs, not through
# the host.
#
# FEX's index of these lives at https://rootfs.fex-emu.gg/RootFS_links.json;
# this is the newest entry it offers. EROFS rather than the squashfs of the
# same contents, because the kernel can mount it read-only off a loop device
# with no userspace helper.
{fetchurl}:
fetchurl {
  name = "fex-rootfs-fedora-44.ero";
  url = "https://rootfs.fex-emu.gg/Fedora_44/2026-08-11/Fedora_44.ero";
  hash = "sha256-2yUDxP8kouDTmrFp0FoTqRICsm0uhzI0AMxRtGpuHFg=";
}
