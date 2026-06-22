{pkgs, ...}: let
  lacy = pkgs.pkgsUnstable.lacy;

  # Lacy has no home-manager module, so generate its nushell integration at
  # build time and source it from the nushell config. `lacy init nu` emits a
  # `lacy` module exporting a `y` command (aliased to `cd` via shellAliases);
  # running it eagerly keeps the generated script pure and reproducible.
  lacyNuInit =
    pkgs.runCommand "lacy-init.nu" {
      nativeBuildInputs = [lacy];
    } ''
      lacy init nu > $out
    '';
in {
  home.packages = [lacy];

  programs.nushell.extraConfig = ''
    source ${lacyNuInit}
  '';
}
