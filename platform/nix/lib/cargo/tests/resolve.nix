# Run: nix eval -f platform/nix/cargo/tests/resolve.nix
#
# Synthetic workspace "root" depending on fixture crate foo 1.1.0:
#   foo has a build-dep on ab, an optional renamed dep "renamed" -> bar-baz
#   (default-features off, features ["x"], target cfg(unix)), and features
#   extra = ["dep:renamed"], weakon = ["renamed?/z"] (schema v2, features2).
#   bar-baz has features default = ["std"], x = ["y"], y, z.
let
  lockLib = import ../lib/lock.nix;
  cfgLib = import ../lib/cfg.nix;
  resolveLib = import ../lib/resolve.nix;
  indexLib = import ../lib/index.nix;

  lockText = ''
    version = 4

    [[package]]
    name = "root"
    version = "0.1.0"
    dependencies = ["foo"]

    [[package]]
    name = "foo"
    version = "1.1.0"
    source = "registry+https://github.com/rust-lang/crates.io-index"
    checksum = "1111111111111111111111111111111111111111111111111111111111111111"
    dependencies = ["ab", "bar-baz"]

    [[package]]
    name = "ab"
    version = "1.4.2"
    source = "registry+https://github.com/rust-lang/crates.io-index"
    checksum = "2222222222222222222222222222222222222222222222222222222222222222"

    [[package]]
    name = "bar-baz"
    version = "2.0.1"
    source = "registry+https://github.com/rust-lang/crates.io-index"
    checksum = "3333333333333333333333333333333333333333333333333333333333333333"
  '';

  lock = lockLib.parseLock lockText;

  mkRootDep = features: {
    name = "foo";
    package = "foo";
    req = "^1.1";
    kind = "normal";
    optional = false;
    defaultFeatures = true;
    inherit features;
    target = null;
    registry = null;
    path = null;
    git = null;
    rev = null;
  };

  mkWorkspace = rootDepFeatures: {
    byName.root = {
      name = "root";
      version = "0.1.0";
      relDir = "";
      edition = "2021";
      deps = [(mkRootDep rootDepFeatures)];
      features = {};
      lib = null;
      bins = [];
      links = null;
      hasBuildScript = false;
      buildScript = null;
    };
  };

  run = rootDepFeatures:
    (resolveLib.resolve {
      inherit lock;
      indexDir = ./fixtures/index;
      platform = cfgLib.platforms.x86_64-linux;
      workspace = mkWorkspace rootDepFeatures;
      roots = ["root"];
    }).nodes;

  a = run ["extra"];
  b = run ["extra" "weakon"];
  c = run ["weakon"];

  fooEdgesA = a."foo-1.1.0".edges;
  abEdgeA = builtins.filter (e: e.package == "ab") fooEdgesA;

  # Implicit optional-dep feature: foo 1.0.0 never says dep:renamed, so it
  # gets an implicit "renamed" feature.
  foo100 = indexLib.lookup ./fixtures/index "foo" "1.0.0";
  eff100 = resolveLib.effectiveFeatures foo100;

  checks = {
    aActive =
      builtins.sort (x: y: x < y) (builtins.attrNames a)
      == ["ab-1.4.2" "bar-baz-2.0.1" "foo-1.1.0" "root-0.1.0"];
    aFooFeatures = a."foo-1.1.0".features == ["default" "extra"];
    aBarFeatures = a."bar-baz-2.0.1".features == ["x" "y"];
    aAbFeatures = a."ab-1.4.2".features == [];
    aAbKind = (builtins.head abEdgeA).kind == "build";
    aFooEdgeCount = builtins.length fooEdgesA == 2;
    aRootEdges =
      map (e: e.targetId) a."root-0.1.0".edges == ["foo-1.1.0"];
    aRootIsMember = a."root-0.1.0".isWorkspaceMember && !a."foo-1.1.0".isWorkspaceMember;

    bBarFeatures = b."bar-baz-2.0.1".features == ["x" "y" "z"];

    cBarInactive = !(c ? "bar-baz-2.0.1");
    cFooFeatures = c."foo-1.1.0".features == ["default" "weakon"];
    cFooEdgeCount = builtins.length c."foo-1.1.0".edges == 1;

    implicitFeature = eff100.renamed == ["dep:renamed"];
    implicitKeepsExisting = eff100.default == ["std"];
  };

  failures = builtins.filter (n: !checks.${n}) (builtins.attrNames checks);
in
  if failures == []
  then "ok: ${toString (builtins.length (builtins.attrNames checks))} checks"
  else throw "resolve test failures: ${builtins.concatStringsSep ", " failures}"
