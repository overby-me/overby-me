# Run: nix eval -f platform/nix/lib/lib/cargo/tests/index.nix
let
  index = import ../lib/index.nix;

  dir = ./fixtures/index;

  foo100 = index.lookup dir "foo" "1.0.0";
  foo110 = index.lookup dir "foo" "1.1.0";
  ab = index.lookup dir "ab" "1.4.2";

  renamed = builtins.elemAt foo100.deps 1;

  checks = {
    relPath1 = index.relPath "a" == "1/a";
    relPath2 = index.relPath "ab" == "2/ab";
    relPath3 = index.relPath "foo" == "3/f/foo";
    relPath4 = index.relPath "serde" == "se/rd/serde";
    relPathCase = index.relPath "Inflector" == "in/fl/inflector";

    fooVersion = foo100.version == "1.0.0";
    fooFeatures =
      foo100.features
      == {
        default = ["std"];
        std = [];
      };
    fooDepCount = builtins.length foo100.deps == 2;

    depDefaults =
      builtins.head foo100.deps
      // {}
      == {
        name = "ab";
        package = "ab";
        req = "^1";
        kind = "normal";
        optional = false;
        defaultFeatures = true;
        features = [];
        target = null;
        registry = null;
        path = null;
        git = null;
        rev = null;
      };

    renameHandled = renamed.name == "renamed" && renamed.package == "bar-baz";
    renameOptional = renamed.optional && !renamed.defaultFeatures;
    renameTarget = renamed.target == "cfg(unix)";

    features2Merged =
      foo110.features
      == {
        default = [];
        extra = ["dep:renamed"];
        weakon = ["renamed?/z"];
      };
    buildKind = (builtins.head foo110.deps).kind == "build";

    emptyDeps = ab.deps == [] && ab.features == {};
  };

  failures = builtins.filter (n: !checks.${n}) (builtins.attrNames checks);
in
  if failures == []
  then "ok: ${toString (builtins.length (builtins.attrNames checks))} checks"
  else throw "index test failures: ${builtins.concatStringsSep ", " failures}"
