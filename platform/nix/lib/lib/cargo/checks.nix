# Checks for the cargo library, imported explicitly because the lib autoloader
# routes only default.nix - checks are not lib content.
#
#   nix build .#checks.x86_64-linux.cargo-lib
#
# Never `nix flake check`; see the repo rules.
{
  checks = {
    # Pure eval unit tests: importing a test file throws on failure, so
    # instantiating this derivation is the assertion.
    cargo-lib = pkgs: let
      names = ["semver" "cfg" "lock" "index" "manifest" "shapes" "profile" "resolve" "patch"];
      results = map (n: "${n}: ${import (./tests + "/${n}.nix")}") names;
    in
      pkgs.writeText "cargo-lib-tests" (pkgs.lib.concatStringsSep "\n" results);
  };
}
