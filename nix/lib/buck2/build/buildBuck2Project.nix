# buildBuck2Project: parse a Buck2 project's .buckconfig + BUCK/.bzl files and
# lower the requested target(s) to Nix derivations (one per action). See PLAN.md.
#
# Analysis (load + rule impls -> action graph) runs either at eval time (pure,
# default) or, with `ifdAnalysis = true`, inside a cached derivation via one
# import-from-derivation. The IFD variant keys the analysis on the build files
# (.bzl / BUCK / .buckconfig) and the file-name structure only, NOT source
# contents, so editing a source never re-runs the Starlark interpreter and a
# no-op rebuild reuses the cached action graph.
{pkgs}: {
  src,
  target ? null,
  targets ? null,
  system ? pkgs.stdenv.hostPlatform.system,
  toolchainPackages ? import ./toolchains.nix pkgs,
  ifdAnalysis ? false,
}: let
  inherit (pkgs) lib;
  lower = import ./lower.nix {
    inherit pkgs toolchainPackages;
    root = src;
  };

  nameOf = tgt:
    "buck2-"
    + builtins.replaceStrings
    ["/" ":" "#" "!" "." " " "+" "@" ","]
    ["-" "-" "-" "-" "-" "-" "-" "-" "-"]
    tgt;

  # ---- pure eval analysis ------------------------------------------------
  pureGraph = tgt:
    import ../lib/analyze.nix {
      inherit src system;
      target = tgt;
    };

  # ---- IFD analysis ------------------------------------------------------
  # The buck2 + skylark lib subtrees, so the nested eval can import analyze.nix.
  libRoot = toString ../..;
  libStore = builtins.path {
    path = ../..;
    name = "buck2-analysis-libs";
    filter = p: _t: let
      rel =
        if toString p == libRoot
        then ""
        else lib.removePrefix (libRoot + "/") (toString p);
      top =
        if rel == ""
        then ""
        else builtins.head (lib.splitString "/" rel);
    in
      rel == "" || top == "buck2" || top == "skylark";
  };

  isBuildFile = p:
    lib.hasSuffix ".bzl" p
    || builtins.elem (builtins.baseNameOf p) ["BUCK" "BUCK.v2" "PACKAGE" ".buckconfig" ".buckroot"];
  # Relative paths of every file under src (no contents read), so the analysis
  # input changes only on add/remove or build-file edits.
  walk = dir: prefix: let
    entries = builtins.readDir dir;
  in
    lib.concatLists (map (
        name: let
          rel =
            if prefix == ""
            then name
            else prefix + "/" + name;
        in
          if entries.${name} == "directory"
          then walk (dir + "/${name}") rel
          else [rel]
      )
      (builtins.attrNames entries));
  allFiles = walk src "";
  buildFiles = builtins.filter isBuildFile allFiles;
  manifest = pkgs.writeText "buck2-file-manifest" (lib.concatStringsSep "\n" allFiles);
  # Real content for each build file (content-addressed, so this input changes
  # only when a build file changes).
  copyBuild = lib.concatMapStringsSep "\n" (rel: ''cp ${builtins.path {
      path = src + "/${rel}";
      name = "bf";
    }} "$out/${rel}"'')
  buildFiles;
  # A source tree with real build files but empty placeholders for everything
  # else (present so glob's readDir still sees them).
  analysisSrc = pkgs.runCommand "buck2-analysis-src" {} ''
    mkdir -p $out
    while IFS= read -r rel; do
      [ -z "$rel" ] && continue
      mkdir -p "$out/$(dirname "$rel")"
      : > "$out/$rel"
    done < ${manifest}
    ${copyBuild}
  '';
  analysisExpr = builtins.toFile "buck2-analyze-expr.nix" ''
    {src, lib, target, system}:
      import (lib + "/buck2/lib/analyze.nix") {inherit src target system;}
  '';
  ifdGraph = tgt: let
    drv = pkgs.runCommand "buck2-analysis-${nameOf tgt}" {nativeBuildInputs = [pkgs.nix];} ''
      export HOME="$NIX_BUILD_TOP/home"
      mkdir -p "$HOME"
      nix-instantiate --eval --strict --json --readonly-mode \
        --arg src ${analysisSrc} \
        --arg lib ${libStore} \
        --argstr target ${lib.escapeShellArg tgt} \
        --argstr system ${lib.escapeShellArg system} \
        ${analysisExpr} > $out
    '';
  in
    builtins.fromJSON (builtins.readFile drv);

  graphOf = tgt:
    if ifdAnalysis
    then ifdGraph tgt
    else pureGraph tgt;

  buildOne = tgt: let
    lowered = lower.lowerGraph (graphOf tgt);
    inherit (lowered) defaultOutputDrv defaultOutputName defaultOutputRel;
  in
    pkgs.runCommand (nameOf tgt) {
      passthru = {inherit (lowered) drvById actions defaultOutputDrv;};
      meta = lib.optionalAttrs (defaultOutputName != null) {mainProgram = defaultOutputName;};
    } ''
      mkdir -p $out
      cp -r --reflink=auto ${defaultOutputDrv}/${defaultOutputRel} "$out/${defaultOutputName}"
    '';
in
  if target != null
  then buildOne target
  else if targets != null
  then
    pkgs.symlinkJoin {
      name = "buck2-targets";
      paths = map buildOne targets;
    }
  else throw "buildBuck2Project: provide `target` (a label) or `targets` (a list of labels)"
