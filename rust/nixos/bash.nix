# bash → rust-bash replacement
#
# rust-bash is a Bash-compatible shell written in Rust that directly handles
# standard bash flags (-e, -u, -c, -o pipefail, etc.), [[ ]], arrays, (( )),
# nameref, process substitution, and can source nixpkgs setup.sh.
# No C wrapper needed — it's a drop-in replacement.
{pkgs, ...}: let
  # rust-bash already provides /bin/bash and /bin/sh
  rustBash = pkgs.rust-bash;

  # Create replacement packages with matching names for closure rewriting
  mkRustBash = name:
    pkgs.runCommand name {} ''
      mkdir -p $out/bin
      ln -s ${rustBash}/bin/bash $out/bin/bash
      ln -s ${rustBash}/bin/sh $out/bin/sh
    '';
in {
  system.replaceDependencies.replacements = [
    {
      original = pkgs.bashNonInteractive;
      replacement = mkRustBash "bash-${rustBash.version}";
    }
    {
      original = pkgs.bashInteractive;
      replacement = mkRustBash "bash-interactive-${rustBash.version}";
    }
  ];
}
