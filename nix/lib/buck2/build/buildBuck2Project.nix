# buildBuck2Project: parse a Buck2 project's .buckconfig + BUCK/.bzl files at
# eval time and lower the requested target(s) to Nix derivations (one per
# action, no IFD). See nix/lib/buck2/PLAN.md.
{pkgs}: {
  src,
  target ? null,
  targets ? null,
  system ? pkgs.stdenv.hostPlatform.system,
  toolchainPackages ? import ./toolchains.nix pkgs,
}: let
  skylark = import ../../skylark/api.nix;
  buckconfig = import ../lib/buckconfig.nix;
  mkLoader = import ../lib/loader.nix;
  mkAnalysis = import ../lib/analysis.nix;
  mkLower = import ./lower.nix;

  inherit ((buckconfig.parse (builtins.readFile (src + "/.buckconfig")))) cells;
  loader = mkLoader {
    inherit skylark cells system;
    root = src;
  };
  analysis = mkAnalysis {inherit skylark loader;};
  lower = mkLower {
    inherit pkgs analysis toolchainPackages;
    root = src;
  };

  nameOf = tgt:
    "buck2-"
    + builtins.replaceStrings
    ["/" ":" "#" "!" "." " " "+" "@" ","]
    ["-" "-" "-" "-" "-" "-" "-" "-" "-"]
    tgt;

  buildOne = tgt: let
    node = analysis.analyzeTarget "" tgt;
    lowered = lower.lowerNode node;
    inherit (lowered) defaultOutputDrv defaultOutputName;
  in
    pkgs.runCommand (nameOf tgt) {
      passthru = {
        inherit node defaultOutputDrv;
        inherit (lowered) drvById actions;
      };
      meta = pkgs.lib.optionalAttrs (defaultOutputName != null) {
        mainProgram = defaultOutputName;
      };
    } ''
      mkdir -p $out
      if [ -d ${defaultOutputDrv} ]; then
        cp -r ${defaultOutputDrv}/. $out/
      else
        cp ${defaultOutputDrv} "$out/${defaultOutputName}"
      fi
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
