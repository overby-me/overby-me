{
  pkgs,
  lib,
  ...
}: {
  home.packages = with pkgs.pkgsUnstable;
    [
      # Very serious tools
      genact
    ]
    # microfetch is Linux-only in nixpkgs.
    ++ lib.optionals pkgs.stdenv.isLinux [
      microfetch
    ];
}
