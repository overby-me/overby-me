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
    backupFileExtension = "hm-backup";
    extraSpecialArgs = {
      inherit inputs pkgs stateVersion src;
    };
  };
}
