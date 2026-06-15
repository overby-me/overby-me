{
  pkgs,
  lib,
  ...
}: {
  home.packages = with pkgs.pkgsUnstable;
    [
      # Very serious tools
      genact
      fortune-kind
    ]
    # microfetch is Linux-only in nixpkgs.
    ++ lib.optionals pkgs.stdenv.isLinux [
      microfetch
    ];
}
