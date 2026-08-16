# Run: nix eval -f platform/nix/lib/buck2/tests/analysis.nix
let
  skylark = import ../../skylark/api.nix;
  buckconfig = import ../lib/buckconfig.nix;
  mkLoader = import ../lib/loader.nix;
  mkAnalysis = import ../lib/analysis.nix;

  root = ./fixtures/no_prelude;
  inherit ((buckconfig.parse (builtins.readFile (root + "/.buckconfig")))) cells;
  loader = mkLoader {
    inherit skylark root cells;
    system = "x86_64-linux";
  };
  analysis = mkAnalysis {inherit skylark loader;};

  rp = p:
    if builtins.isString p
    then p
    else if p ? __sk && p.__sk == "output_arg"
    then "OUT:${p.artifact.name}"
    else if p ? __sk && p.__sk == "artifact"
    then "${
      if p.kind == "source"
      then "SRC"
      else "REF"
    }:${p.name}"
    else if p ? __sk && p.__sk == "cmd_args"
    then "CMD"
    else "?";

  main = analysis.analyzeTarget "" "//cpp/hello_world:main";
  runAction = builtins.head main.actions;
  parts = map rp runAction.cmd.parts;
  hidden = map rp runAction.cmd.hidden;
  outNames = map (a: a.name) runAction.outputs;
  provIds = map (p: p.providerId) main.providers;
  defOut = analysis.defaultOutputForNode main;

  rustMain = analysis.analyzeTarget "" "//rust:main";
  rustAction = builtins.head rustMain.actions;
  rustParts = map rp rustAction.cmd.parts;

  checks = [
    ["one run action" (builtins.length main.actions == 1 && runAction.kind == "run")]
    ["run category" (runAction.category == "compile")]
    ["cmd parts wired (clang++, -o, out, srcs)" (parts == ["clang++" "-o" "OUT:main" "SRC:func.cpp" "SRC:main.cpp"])]
    ["hidden headers" (hidden == ["SRC:func.hpp"])]
    ["output artifact name" (outNames == ["main"])]
    ["providers are DefaultInfo + RunInfo" (provIds == ["buck2//builtin:DefaultInfo" "buck2//builtin:RunInfo"])]
    ["default output is main" (defOut != null && defOut.name == "main")]
    ["one dep (toolchain)" (builtins.length main.deps == 1 && (builtins.head main.deps).label == "toolchains//:cpp")]

    ["rust run action" (rustAction.kind == "run")]
    ["rust cmd wired (rustc, --crate-type, file, -o, out)" (rustParts == ["rustc" "--crate-type=bin" "SRC:main.rs" "-o" "OUT:main"])]
  ];
  failures = builtins.filter (c: !(builtins.elemAt c 1)) checks;
  names = map (c: builtins.elemAt c 0) failures;
in
  if failures == []
  then "ok: ${toString (builtins.length checks)} analysis cases"
  else throw "analysis test failures: ${builtins.toJSON names}"
