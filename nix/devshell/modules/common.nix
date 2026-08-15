# Base tooling shared by the default devshell.
{pkgs, ...}: {
  # rust/perl's build.rs includes two headers that ship only in Perl's source
  # distribution. Without this, plain `cargo build` and the clippy hook fail
  # on that crate, which is how it came to depend on a directory someone had
  # unpacked under /tmp by hand.
  config.env.PERL_SRC = "${pkgs.srcOnly pkgs.perl}";

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
