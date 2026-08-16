{
  pkgs,
  lib,
  ...
}: {
  home.packages = with pkgs.pkgsUnstable;
    [
      killall
      uutils-coreutils-noprefix
      xkill
      lsof
      #waypipe
      wl-color-picker
      cryptsetup
    ]
    # rust-wclip is a Wayland clipboard tool (Linux-only).
    ++ lib.optionals pkgs.stdenv.isLinux [
      rust-wclip
    ];
}
