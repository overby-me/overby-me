{pkgs, ...}: {
  home.packages = with pkgs.pkgsUnstable; [
    helix
    file
    unixtools.xxd
    fd
    hexyl
    dust
    ripgrep
    ripgrep-all
    tokei
    zip
    unzip
    uutils-diffutils
    ast-grep
    # diffoscope  # disabled: pulls broken systemd-260.1 build from nixpkgs-unstable
    jless
    television
    yq-go # Needed by prettybat
  ];
}
