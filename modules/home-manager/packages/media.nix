{
  pkgs,
  lib,
  ...
}: {
  home.packages = with pkgs.pkgsUnstable;
    [
      imagemagick
      oxipng
    ]
    # gimp3 is marked Linux-only in nixpkgs (not available on aarch64-darwin).
    ++ lib.optionals pkgs.stdenv.isLinux [
      gimp3
    ];
}
