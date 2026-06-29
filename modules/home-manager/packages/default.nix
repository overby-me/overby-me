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
      ./general.nix
      ./media.nix
      ./network.nix
      ./scripts
    ]
    # Linux-only package sets (hardware tooling, containers, Nitrokey/PCSC) not available on Darwin.
    ++ lib.optionals pkgs.stdenv.isLinux [
      ./container.nix
      ./hardware.nix
      ./security.nix
      ./system.nix
    ];
}
