# Buck2 load phase: evaluate BUCK / .bzl files into an unconfigured target
# graph. builtins only (uses the skylark interpreter passed in).
#
# mkLoader { skylark; root; cells; system; } ->
#   { getTarget; evalBUCK; loadFrom; }
# where `root` is the project source (a Nix path), `cells` maps cell names to
# root-relative dirs, and `system` feeds host_info().
{
  skylark,
  root,
  cells,
  system,
}: let
  labels = import ./labels.nix;
  mkGlobals = import ./globals.nix;
  inherit (builtins) filter;

  V = skylark.values;

  readSrc = relPath: builtins.readFile (root + "/${relPath}");
  pkgSrcPath = pkgDir:
    if pkgDir == "" || pkgDir == "."
    then root
    else root + "/${pkgDir}";

  globalsFor = currentFile:
    mkGlobals {
      inherit V system;
      inherit currentFile;
    };

  # Evaluate a .bzl module, returning its (frozen) globals attrset.
  evalBzl = relPath: let
    ast = skylark.parse (readSrc relPath);
    interp = skylark.mkInterp {
      loadModule = loadFrom;
      extraGlobals = globalsFor relPath;
    };
    res = interp.evalModule relPath ast {providerSeq = 0;};
  in
    res.globals;

  loadFrom = currentFile: label: evalBzl (labels.resolveLoadPath currentFile cells label);

  # Evaluate a package's BUCK file, returning its registered target list.
  evalBUCK = {
    buckPath,
    pkgCell,
    pkgName,
    pkgDir,
  }: let
    ast = skylark.parse (readSrc buckPath);
    interp = skylark.mkInterp {
      loadModule = loadFrom;
      extraGlobals = globalsFor buckPath;
    };
    world0 = {
      providerSeq = 0;
      targets = [];
      inherit pkgCell pkgName pkgDir;
      pkgSrc = pkgSrcPath pkgDir;
    };
    res = interp.evalModule buckPath ast world0;
  in
    res.world.targets or [];

  # Resolve a label to its target node (evaluating the package's BUCK file).
  getTarget = fromFile: label: let
    t =
      labels.resolveTarget {
        inherit cells;
        currentFile = fromFile;
      }
      label;
    targets = evalBUCK {
      inherit (t) buckPath pkgDir;
      pkgCell = t.cell;
      pkgName = t.pkg;
    };
    matching = filter (x: x.name == t.name) targets;
  in
    if matching == []
    then throw "buck2: target '${label}' not found in ${t.buckPath}"
    else (builtins.head matching) // {resolved = t;};
in {
  inherit getTarget evalBUCK loadFrom evalBzl;
}
