# Run: nix eval -f platform/nix/config/lib/cargo/tests/shapes.nix
#
# Polymorphic manifest shapes (nocargo issue #10 class): integer edition,
# bare-string crate-type, single [bin] table instead of [[bin]].
let
  manifest = import ../lib/manifest.nix;

  pkg = manifest.normalizePackage {
    dir = ./fixtures/shapes;
    manifest = manifest.loadManifest ./fixtures/shapes/Cargo.toml;
  };

  checks = {
    intEdition = pkg.edition == "2021";
    stringCrateType = pkg.lib.crateTypes == ["cdylib"];
    singleBinTable =
      pkg.bins
      == [
        {
          name = "one";
          path = "src/one.rs";
          requiredFeatures = [];
        }
      ];
    versionIsString = pkg.version == "0.1.0";
  };

  failures = builtins.filter (n: !checks.${n}) (builtins.attrNames checks);
in
  if failures == []
  then "ok: ${toString (builtins.length (builtins.attrNames checks))} checks"
  else throw "shape test failures: ${builtins.concatStringsSep ", " failures}"
