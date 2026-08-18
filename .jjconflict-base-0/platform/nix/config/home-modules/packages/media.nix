{
  pkgs,
  lib,
  ...
}: {
  home.packages = with pkgs.pkgsUnstable;
    [
      imagemagick
      oxipng
      # Spotify ships no aarch64 Linux client.  librespot needs no Widevine,
      # unlike the web player.  Premium only.  Default features carry
      # streaming and the rodio backend.
      spotify-player
    ]
    # gimp3 is marked Linux-only in nixpkgs (not available on aarch64-darwin).
    ++ lib.optionals pkgs.stdenv.isLinux [
      gimp3
    ];
}
