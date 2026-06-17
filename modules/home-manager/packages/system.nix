{pkgs, ...}: {
  home.packages = with pkgs.pkgsUnstable; [
    killall
    uutils-coreutils-noprefix
    xkill
    lsof
    wl-clipboard-rs
    skim
    #waypipe
    wl-color-picker
    cryptsetup
  ];
}
