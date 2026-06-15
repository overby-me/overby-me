{
  pkgs,
  lib,
  ...
}: {
  imports =
    [
      ./ai.nix
      ./dev.nix
      ./file.nix
      ./fun.nix
      ./media.nix
      ./network.nix
      ./scripts
    ]
    # Linux-only package sets: GNOME/PipeWire/Wayland apps, Linux hardware
    # tooling, LUKS/containers, and the Nitrokey/PCSC stack that don't build
    # or apply on Darwin. (dev.nix is cross-platform; its few Linux-only
    # packages are gated inside that module.)
    ++ lib.optionals pkgs.stdenv.isLinux [
      ./container.nix
      ./general.nix
      ./hardware.nix
      ./security.nix
      ./system.nix
    ];
}
