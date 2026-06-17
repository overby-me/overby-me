{
  pkgs,
  inputs,
  ...
}: {
  imports = with inputs.self.devenvModules; [
    devenv-root
    git-hooks
    configs
    cachix
  ];

  packages = with pkgs.pkgsUnstable; [
    # IDE
    harper
    # Common
    just
    # Nix
    nixd
    nil
    alejandra
    colmena
    rage
    (writeShellScriptBin "ragenix" ''
      exec ${ragenix}/bin/ragenix -i ~/.age/id_fido2 "$@"
    '')
  ];
}
