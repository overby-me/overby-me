# Buck2 label + cell resolution. builtins only.
#
# Parses target labels (@cell//pkg:name, //pkg:name, cell//pkg:name, :name)
# and load labels (@cell//pkg:file.bzl, //pkg:file.bzl, :file.bzl), and
# resolves them to project-root-relative file paths via the cell map. Paths
# are strings; the driver turns them into real paths for readFile/readDir.
let
  inherit (builtins) substring stringLength filter isString elemAt length;

  charAt = s: i: substring i 1 s;
  hasPrefix = p: s: substring 0 (stringLength p) s == p;

  # First index of a 2-char needle (for "//"), or -1.
  indexOf2 = needle: s: let
    L = stringLength s;
    go = i:
      if i + 2 > L
      then -1
      else if substring i 2 s == needle
      then i
      else go (i + 1);
  in
    go 0;
  lastIndexOfChar = c: s: let
    L = stringLength s;
    go = i:
      if i < 0
      then -1
      else if charAt s i == c
      then i
      else go (i - 1);
  in
    go (L - 1);

  segsOf = p: filter (x: x != "" && x != ".") (filter isString (builtins.split "/" p));
  joinPath = parts: let
    segs = builtins.concatLists (map segsOf parts);
  in
    if segs == []
    then "."
    else builtins.concatStringsSep "/" segs;
  dirOf = p: let
    i = lastIndexOfChar "/" p;
  in
    if i < 0
    then ""
    else substring 0 i p;
  baseOf = p: let
    i = lastIndexOfChar "/" p;
  in
    if i < 0
    then p
    else substring (i + 1) (stringLength p - i - 1) p;

  # Strip a trailing [subtarget].
  stripSub = name: let
    i = lastIndexOfChar "[" name;
  in
    if i < 0
    then name
    else substring 0 i name;

  # parseLabel "@cell//pkg:name" / "//pkg:name" / "cell//pkg:name" / ":name"
  # -> { relative; cell; pkg; name; }.
  parseLabel = str:
    if hasPrefix ":" str
    then {
      relative = true;
      cell = null;
      pkg = null;
      name = stripSub (substring 1 (stringLength str - 1) str);
    }
    else let
      s =
        if hasPrefix "@" str
        then substring 1 (stringLength str - 1) str
        else str;
      ss = indexOf2 "//" s;
    in
      if ss < 0
      then throw "buck2: malformed label '${str}' (missing //)"
      else let
        cell = substring 0 ss s;
        rest = substring (ss + 2) (stringLength s - ss - 2) s;
        colon = lastIndexOfChar ":" rest;
        pkg =
          if colon < 0
          then rest
          else substring 0 colon rest;
        name =
          if colon < 0
          then baseOf rest
          else substring (colon + 1) (stringLength rest - colon - 1) rest;
      in {
        relative = false;
        inherit cell;
        inherit pkg;
        name = stripSub name;
      };

  cellDir = cells: cell:
    if cell == null || cell == ""
    then cells.root or "."
    else cells.${cell} or (throw "buck2: unknown cell '${cell}'");

  isPrefixSegs = a: b:
    length a <= length b && builtins.all (i: elemAt a i == elemAt b i) (builtins.genList (i: i) (length a));

  # The cell that a project-root-relative path belongs to: the cell whose
  # directory is the longest segment-prefix of the path. `//` in a load or
  # target is relative to THIS cell's root, not the root cell.
  cellOf = cells: path: let
    pathSegs = segsOf path;
    scored = map (n: {
      name = n;
      depth = length (segsOf cells.${n});
      ok = isPrefixSegs (segsOf cells.${n}) pathSegs;
    }) (builtins.attrNames cells);
    oks = filter (x: x.ok) scored;
    best =
      builtins.foldl' (a: b:
        if b.depth > a.depth
        then b
        else a) {
        name = "root";
        depth = -1;
      }
      oks;
  in
    best.name;

  # Canonical "cell//pkg:name" (root cell rendered as "//pkg:name").
  canonical = cell: pkg: name: "${
    if cell == null || cell == ""
    then ""
    else cell
  }//${pkg}:${name}";

  # Resolve a load() label from the loading file to a root-relative path.
  # An empty cell (`//...`) resolves against the loading file's own cell.
  resolveLoadPath = currentFile: cells: label: let
    p = parseLabel label;
    cellName =
      if p.cell == ""
      then cellOf cells currentFile
      else p.cell;
  in
    if p.relative
    then joinPath [(dirOf currentFile) p.name]
    else joinPath [(cellDir cells cellName) p.pkg p.name];

  # Resolve a target label to its package BUCK path and canonical key.
  # `currentPkgDir`/`currentCanon` are used for relative (:name) targets.
  resolveTarget = {
    cells,
    currentFile ? "",
    currentCell ? "",
    currentPkg ? "",
  }: label: let
    p = parseLabel label;
  in
    if p.relative
    then let
      pkgDir = dirOf currentFile;
    in {
      cell = currentCell;
      pkg = currentPkg;
      inherit (p) name;
      inherit pkgDir;
      buckPath = joinPath [pkgDir "BUCK"];
      label = canonical currentCell currentPkg p.name;
    }
    else let
      cellName =
        if p.cell == ""
        then cellOf cells currentFile
        else p.cell;
      canonCell =
        if cellName == "root"
        then ""
        else cellName;
      pkgDir = joinPath [(cellDir cells cellName) p.pkg];
    in {
      cell = canonCell;
      inherit (p) pkg name;
      inherit pkgDir;
      buckPath = joinPath [pkgDir "BUCK"];
      label = canonical canonCell p.pkg p.name;
    };
in {
  inherit
    parseLabel
    resolveLoadPath
    resolveTarget
    canonical
    cellDir
    joinPath
    dirOf
    baseOf
    segsOf
    ;
}
