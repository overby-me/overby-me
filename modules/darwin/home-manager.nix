{
  pkgs,
  inputs,
  stateVersion,
  src,
  ...
}: {
  home-manager = {
    useGlobalPkgs = true;
    useUserPackages = true;
    backupCommand = ''
      ${pkgs.coreutils}/bin/mv -f "$1" "$1.hm-backup"
    '';
    extraSpecialArgs = {
      inherit
        inputs
        pkgs
        stateVersion
        src
        ;
    };
  };
}
