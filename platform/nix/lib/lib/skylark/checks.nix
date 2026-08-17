# Flakelight module: checks for the skylark interpreter. Imported explicitly
# from flake.nix (the platform/nix/lib/lib autoloader only routes default.nix).
#
# Run one: nix build .#checks.x86_64-linux.skylark-lib
# (never `nix flake check`, see the repo rules)
{
  checks.skylark-lib = pkgs: let
    names = ["lexer" "parser" "eval"];
    results = map (n: "${n}: ${import (./tests + "/${n}.nix")}") names;
  in
    pkgs.writeText "skylark-lib-tests" (builtins.concatStringsSep "\n" results);
}
