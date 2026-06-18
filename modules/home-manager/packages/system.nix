{pkgs, ...}: {
  home.packages = with pkgs.pkgsUnstable; [
    killall
    uutils-coreutils-noprefix
    xkill
    lsof
    #waypipe
    wl-color-picker
    cryptsetup
  ];
}
