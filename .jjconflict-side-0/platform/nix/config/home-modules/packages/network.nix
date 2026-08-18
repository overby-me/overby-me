{pkgs, ...}: {
  home.packages = with pkgs.pkgsUnstable; [
    xh
    wget
    whois
    openssl
    gping
    bandwhich
    rustscan
    unixtools.route
  ];
}
