{
  pkgs,
  inputs,
  stateVersion,
  src,
  ...
}: {
  home-manager = {
    inherit (inputs.self) users;
    useGlobalPkgs = true;
    useUserPackages = true;
    backupCommand = ''
      ${pkgs.coreutils}/bin/mv -f "$1" "$1.hm-backup"
    '';
    extraSpecialArgs = {
      inherit inputs pkgs stateVersion src;
    };
  };
}
