{pkgs, ...}: {
  home.packages = with pkgs.pkgsUnstable; [
    killall
    uutils-coreutils-noprefix
    xkill
    lsof
    skim
    #waypipe
    wl-color-picker
    cryptsetup
  ];
}
