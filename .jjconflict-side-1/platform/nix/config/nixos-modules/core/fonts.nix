{pkgs, ...}: {
  fonts.packages = with pkgs; [
    nerd-fonts.fira-code
    nerd-fonts.droid-sans-mono
    fira
    roboto
    roboto-slab
    meslo-lgs-nf
    cascadia-code
  ];
}
