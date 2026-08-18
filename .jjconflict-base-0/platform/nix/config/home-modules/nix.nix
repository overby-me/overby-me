{
  pkgs,
  lib,
  ...
}: {
  nix = {
    package = lib.mkDefault pkgs.pkgsUnstable.nixVersions.latest;
    settings = {
      allow-import-from-derivation = true;
    };
  };
}
