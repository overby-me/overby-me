# Run: nix eval -f platform/nix/lib/buck2/tests/load.nix
let
  skylark = import ../../skylark/api.nix;
  buckconfig = import ../lib/buckconfig.nix;
  mkLoader = import ../lib/loader.nix;

  root = ./fixtures/no_prelude;
  inherit ((buckconfig.parse (builtins.readFile (root + "/.buckconfig")))) cells;
  loader = mkLoader {
    inherit skylark root cells;
    system = "x86_64-linux";
  };

  renderList = v: map (x: x) v.items;

  main = loader.getTarget "" "//cpp/hello_world:main";
  lib = loader.getTarget "" "//cpp/library:library";
  cppTc = loader.getTarget "" "toolchains//:cpp";
  rustMain = loader.getTarget "" "//rust:main";
  goMain = loader.getTarget "" "//go:main";

  checks = [
    ["main is cpp_binary target" (main.name == "main" && !main.ruleIsToolchain)]
    ["main has an impl function" (main.ruleImpl.__sk == "function")]
    ["main name attr" (main.providedAttrs.name == "main")]
    ["main srcs from glob" (renderList main.providedAttrs.srcs == ["func.cpp" "main.cpp"])]
    ["main headers from glob" (renderList main.providedAttrs.headers == ["func.hpp"])]
    ["main toolchain label" (main.providedAttrs.toolchain == "toolchains//:cpp")]
    ["main label canonical" (main.label == "//cpp/hello_world:main")]

    ["library target srcs" (renderList lib.providedAttrs.srcs == ["library.cpp"])]
    ["library visibility" (renderList lib.providedAttrs.visibility == ["PUBLIC"])]

    ["cpp toolchain is toolchain rule" (cppTc.ruleIsToolchain == true)]
    ["cpp toolchain command" (cppTc.providedAttrs.command == "clang++")]

    ["rust main file attr" (rustMain.providedAttrs.file == "main.rs")]
    ["rust main toolchain" (rustMain.providedAttrs.toolchain == "toolchains//:rust")]

    ["go main srcs from glob" (renderList goMain.providedAttrs.srcs == ["main.go"])]
    ["go main toolchain" (goMain.providedAttrs.toolchain == "toolchains//:go")]
  ];
  failures = builtins.filter (c: !(builtins.elemAt c 1)) checks;
  names = map (c: builtins.elemAt c 0) failures;
in
  if failures == []
  then "ok: ${toString (builtins.length checks)} load cases"
  else throw "load test failures: ${builtins.toJSON names}"
