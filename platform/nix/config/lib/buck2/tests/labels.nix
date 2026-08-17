# Run: nix eval -f platform/nix/config/lib/buck2/tests/labels.nix
let
  buckconfig = import ../lib/buckconfig.nix;
  labels = import ../lib/labels.nix;

  inherit ((buckconfig.parse (builtins.readFile ./fixtures/no_prelude/.buckconfig))) cells;

  rt = labels.resolveTarget {inherit cells;};

  checks = [
    [
      "buckconfig cells"
      (cells
        == {
          root = ".";
          toolchains = "toolchains";
        })
    ]

    ["parse root target" (let p = labels.parseLabel "//cpp/hello_world:main"; in p.cell == "" && p.pkg == "cpp/hello_world" && p.name == "main")]
    ["parse cell load @" (let p = labels.parseLabel "@toolchains//:cpp_toolchain.bzl"; in p.cell == "toolchains" && p.pkg == "" && p.name == "cpp_toolchain.bzl")]
    ["parse bare cell target" (let p = labels.parseLabel "toolchains//:cpp"; in p.cell == "toolchains" && p.pkg == "" && p.name == "cpp")]
    ["parse relative load" (let p = labels.parseLabel ":export_file.bzl"; in p.relative && p.name == "export_file.bzl")]
    ["parse root load" (let p = labels.parseLabel "//cpp:rules.bzl"; in p.cell == "" && p.pkg == "cpp" && p.name == "rules.bzl")]
    ["parse pkg-only defaults name" (let p = labels.parseLabel "//foo/bar"; in p.pkg == "foo/bar" && p.name == "bar")]
    ["parse subtarget stripped" (let p = labels.parseLabel "//a:b[sub]"; in p.name == "b")]

    ["resolve load @cell" (labels.resolveLoadPath "toolchains/BUCK" cells "@toolchains//:cpp_toolchain.bzl" == "toolchains/cpp_toolchain.bzl")]
    ["resolve load //pkg" (labels.resolveLoadPath "cpp/hello_world/BUCK" cells "//cpp:rules.bzl" == "cpp/rules.bzl")]
    ["resolve load relative" (labels.resolveLoadPath "toolchains/BUCK" cells ":export_file.bzl" == "toolchains/export_file.bzl")]

    ["resolve target root" (let t = rt "//cpp/hello_world:main"; in t.buckPath == "cpp/hello_world/BUCK" && t.pkgDir == "cpp/hello_world" && t.name == "main" && t.label == "//cpp/hello_world:main")]
    ["resolve target cell" (let t = rt "toolchains//:cpp"; in t.buckPath == "toolchains/BUCK" && t.pkgDir == "toolchains" && t.name == "cpp" && t.label == "toolchains//:cpp")]
  ];
  failures = builtins.filter (c: !(builtins.elemAt c 1)) checks;
  names = map (c: builtins.elemAt c 0) failures;
in
  if failures == []
  then "ok: ${toString (builtins.length checks)} label cases"
  else throw "label test failures: ${builtins.toJSON names}"
