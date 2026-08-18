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
    # wclip is a Wayland clipboard tool (Linux-only).
    ++ lib.optionals pkgs.stdenv.isLinux [
      wclip
    ];
}
