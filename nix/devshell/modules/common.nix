# Base tooling shared by the default devshell.
{pkgs, ...}: {
  config.packages = with pkgs.pkgsUnstable; [
    # IDE
    harper
    # Common
    just
    # Nix
    nil
    alejandra
    colmena
    rage
    (writeShellScriptBin "ragenix" ''
      exec ${ragenix}/bin/ragenix -i ~/.age/id_fido2 "$@"
    '')
  ];
}
