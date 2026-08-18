{
  pkgs,
  lib,
  ...
}: {
  environment = {
    systemPackages = with pkgs;
      [
        helix
        tailspin
      ]
      # cosmic-osk comes from platform/nix/packages/packages, so it is never in a binary cache and
      # every aarch64 host compiles it under emulation.  Hosts that actually
      # need an on-screen keyboard ask for it themselves; phone.nix does.
      ++ lib.optional pkgs.stdenv.hostPlatform.isx86_64 cosmic-osk;
    sessionVariables = {
      PAGER = "${pkgs.tailspin}/bin/tspin";
      SYSTEMD_PAGERSECURE = "1";
      NIXOS_OZONE_WL = "1";
    };
  };
}
