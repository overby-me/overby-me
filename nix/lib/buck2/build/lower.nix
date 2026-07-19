# Lower a Buck2 action graph to Nix derivations: one derivation per action, no
# import-from-derivation.
#
# Model: a virtual "buck-out". Every artifact has a stable working-directory-
# relative path (artPath); command lines and script contents reference those
# relative paths (honoring cmd_args relative_to), NOT store paths, so an action
# that only needs a peer's *path* (e.g. a generated script naming an output it
# does not build) creates no derivation dependency and no cycle. Each action's
# derivation stages its input artifacts into a working tree (copying each
# producer's whole tree, so transitive files and symlink targets travel along),
# runs, and exports the resulting tree as $out. Dependencies flow through
# store-path interpolation in the staging commands only.
#
# mkLower { pkgs; root; analysis; toolchainPackages; } -> { lowerNode; }
{
  pkgs,
  root,
  analysis,
  toolchainPackages,
}: let
  inherit (pkgs) lib;
  inherit (builtins) filter concatMap isAttrs isString listToAttrs elemAt length genList;

  esc = lib.escapeShellArg;

  sanDrv = s:
    "buck2-"
    + builtins.replaceStrings
    ["/" ":" "#" "!" "." " " "+" "@" "," "(" ")" "[" "]" "="]
    ["-" "-" "-" "-" "-" "-" "-" "-" "-" "-" "-" "-" "-" "-"]
    s;
  sanLabel = builtins.replaceStrings ["/" ":"] ["_" "_"];

  # ---- path helpers ------------------------------------------------------
  segsOf = p: filter (x: x != "" && x != "." && isString x) (builtins.split "/" p);
  joinSegs = xs:
    if xs == []
    then "."
    else builtins.concatStringsSep "/" xs;
  dropLast = n: xs: let
    keep = length xs - n;
  in
    if keep <= 0
    then []
    else genList (i: elemAt xs i) keep;
  dropN = n: xs:
    if n >= length xs
    then []
    else genList (i: elemAt xs (i + n)) (length xs - n);
  commonPrefix = a: b: let
    n = let
      go = i:
        if i < length a && i < length b && elemAt a i == elemAt b i
        then go (i + 1)
        else i;
    in
      go 0;
  in
    n;
  # Path from directory `base` to `target` (both working-dir-relative).
  relativize = target: base: let
    ts = segsOf target;
    bs = segsOf base;
    c = commonPrefix ts bs;
    up = genList (_: "..") (length bs - c);
    down = dropN c ts;
    parts = up ++ down;
  in
    joinSegs parts;

  # ---- artifact paths ----------------------------------------------------
  artPath = art:
    if art.kind == "source"
    then "bo/src/${art.srcRel}"
    else "bo/${sanLabel art.owner}/${art.name}";

  # relative_to spec -> base directory (a working-dir-relative path).
  reltoBase = rt:
    if rt == null
    then null
    else if isAttrs rt && rt ? __sk && rt.__sk == "tuple"
    then joinSegs (dropLast (elemAt rt.items 1) (segsOf (artPath (elemAt rt.items 0))))
    else if isAttrs rt && rt ? __sk && rt.__sk == "output_arg"
    then artPath rt.artifact
    else if isAttrs rt && rt ? __sk && rt.__sk == "artifact"
    then artPath rt
    else null;

  artRef = art: rt: let
    p = artPath art;
    base = reltoBase rt;
  in
    if base == null
    then p
    else relativize p base;

  # ---- cmd_args rendering (to a list of raw strings) ---------------------
  renderPartRaw = rt: part:
    if isString part
    then [part]
    else if !(isAttrs part && part ? __sk)
    then [(toString part)]
    else if part.__sk == "output_arg"
    then [(artRef part.artifact rt)]
    else if part.__sk == "artifact"
    then [(artRef part rt)]
    else if part.__sk == "cmd_args"
    then argStrings part
    else if part.__sk == "list" || part.__sk == "tuple"
    then concatMap (renderPartRaw rt) part.items
    else [];

  argStrings = cav: let
    rt = cav.opts.relative_to or null;
    raw = concatMap (renderPartRaw rt) cav.parts;
    withPrepend =
      if (cav.opts.prepend or null) != null
      then concatMap (x: [cav.opts.prepend x]) raw
      else raw;
    formatted =
      if (cav.opts.format or null) != null
      then map (x: builtins.replaceStrings ["{}"] [x] cav.opts.format) withPrepend
      else withPrepend;
  in
    if (cav.opts.delimiter or null) != null
    then [(builtins.concatStringsSep cav.opts.delimiter formatted)]
    else formatted;

  renderWriteContent = c:
    if isString c
    then c
    else if isAttrs c && c ? __sk && c.__sk == "cmd_args"
    then builtins.concatStringsSep " " (argStrings c)
    else if isAttrs c && c ? __sk && (c.__sk == "list" || c.__sk == "tuple")
    then builtins.concatStringsSep "\n" (map renderWriteContent c.items)
    else if c == null
    then ""
    else toString c;

  # ---- artifact collection ----------------------------------------------
  collectOutputs = xs: concatMap collectOut xs;
  collectOut = v:
    if !(isAttrs v && v ? __sk)
    then []
    else if v.__sk == "output_arg"
    then [v.artifact]
    else if v.__sk == "cmd_args"
    then collectOutputs (v.parts ++ v.hidden)
    else if v.__sk == "list" || v.__sk == "tuple"
    then collectOutputs v.items
    else [];
  # Plain artifact references (inputs): sources and other actions' outputs.
  collectInputs = xs: concatMap collectIn xs;
  collectIn = v:
    if !(isAttrs v && v ? __sk)
    then []
    else if v.__sk == "artifact"
    then [v]
    else if v.__sk == "cmd_args"
    then collectInputs (v.parts ++ v.hidden)
    else if v.__sk == "list" || v.__sk == "tuple"
    then collectInputs v.items
    else [];

  litStrings = xs: concatMap litStr xs;
  litStr = v:
    if isString v
    then [v]
    else if isAttrs v && v ? __sk && v.__sk == "cmd_args"
    then litStrings (v.parts ++ v.hidden)
    else if isAttrs v && v ? __sk && (v.__sk == "list" || v.__sk == "tuple")
    then litStrings v.items
    else [];

  srcStorePath = art:
    builtins.path {
      path = root + "/${art.srcRel}";
      inherit (art) name;
    };

  # Baseline env for run actions (harmless for cc/rustc; lets the vendored Go
  # toolchain build a single-file main without a module).
  runEnvPrelude = ''
    export HOME="$PWD/.home" GOCACHE="$PWD/.gocache" GOPATH="$PWD/.gopath"
    export GO111MODULE=off CGO_ENABLED=0 GOPROXY=off GOTOOLCHAIN=local
    mkdir -p "$HOME"
  '';

  lowerNode = rootNode: let
    actions = analysis.collectActions rootNode;
    actionOutputs = a:
      if a.kind == "run"
      then collectOutputs (a.cmd.parts ++ a.cmd.hidden)
      else if a ? output
      then [a.output]
      else [];
    outputToAction = listToAttrs (concatMap (a:
      map (o: {
        name = o.id;
        value = a.id;
      }) (actionOutputs a))
    actions);
    producerId = art:
      outputToAction.${art.id}
      or (throw "buck2: no action produces artifact '${art.id}'");
    actionById = listToAttrs (map (a: {
        name = a.id;
        value = a;
      })
      actions);
    runDepIds = a:
      if a.kind == "run"
      then lib.unique (map producerId (filter (x: x.kind != "source") (collectInputs (a.cmd.parts ++ a.cmd.hidden))))
      else [];
    # Only actions that carry downloaded prebuilt binaries need autoPatchelf.
    # Skipping it for from-source builds (cpp/rust) keeps them fast; scanning +
    # patching adds seconds even for a tiny tree.
    needsPatch = id: let
      a = actionById.${id};
    in
      a.kind == "download" || builtins.any needsPatch (runDepIds a);

    # ---- per-action derivations ----------------------------------------
    mkRun = a: let
      all = a.cmd.parts ++ a.cmd.hidden;
      ins = collectInputs all;
      srcs = filter (x: x.kind == "source") ins;
      depOuts = filter (x: x.kind != "source") ins;
      depIds = lib.unique (map producerId depOuts);
      outs = collectOutputs all;
      argv = builtins.concatStringsSep " " (map esc (argStrings a.cmd));
      strings = litStrings a.cmd.parts;
      tcPkgs = map (k: toolchainPackages.${k}) (filter (k: builtins.elem k strings) (builtins.attrNames toolchainPackages));
      patch = needsPatch a.id;
      # Copy each producer's whole tree in; chmod +w after each so the next
      # copy can merge into the (store-read-only) directories.
      stageDeps = builtins.concatStringsSep "\n" (map (id: "cp -r --reflink=auto ${drvById.${id}}/. ./\nchmod -R u+w . 2>/dev/null || true") depIds);
      stageSrcs = builtins.concatStringsSep "\n" (map (s: "install -Dm644 ${srcStorePath s} ${esc (artPath s)}") srcs);
      mkOutDirs = builtins.concatStringsSep "\n" (map (o: ''mkdir -p "$(dirname ${esc (artPath o)})"'') outs);
      # autoPatchelfHook makes downloaded prebuilt binaries in $out runnable on
      # Nix by fixing their ELF interpreter. runCommand bypasses fixupPhase, so
      # the hook's function is invoked directly. No extra buildInputs: glibc
      # here would pollute the C++ header path, and the patched interpreter
      # finds libc via its own default search.
      patchCall = lib.optionalString patch ''
        chmod -R u+w "$out" 2>/dev/null || true
        autoPatchelf "$out" || true
      '';
    in
      pkgs.runCommand (sanDrv a.id) ({
          nativeBuildInputs = [pkgs.stdenv.cc] ++ lib.optional patch pkgs.autoPatchelfHook ++ tcPkgs;
          dontStrip = true;
        }
        // lib.optionalAttrs patch {autoPatchelfIgnoreMissingDeps = true;}) ''
        ${runEnvPrelude}
        ${stageDeps}
        ${stageSrcs}
        chmod -R u+w . 2>/dev/null || true
        ${mkOutDirs}
        ${argv}
        mkdir -p $out
        if [ -e bo ]; then cp -r --reflink=auto bo $out/; fi
        ${patchCall}
      '';

    mkWrite = a: let
      contentFile = pkgs.writeText "${sanDrv a.id}-content" (renderWriteContent a.content);
      outRel = artPath a.output;
    in
      pkgs.runCommand (sanDrv a.id) {} ''
        mkdir -p "$out/$(dirname ${esc outRel})"
        cp ${contentFile} "$out/${outRel}"
        ${lib.optionalString a.isExecutable ''chmod +x "$out/${outRel}"''}
      '';

    mkDownload = a: let
      fod = pkgs.fetchurl ({inherit (a) url;}
        // (
          if (a.sha256 or null) != null
          then {inherit (a) sha256;}
          else if (a.sha1 or null) != null
          then {inherit (a) sha1;}
          else {}
        ));
      outRel = artPath a.output;
    in
      pkgs.runCommand (sanDrv a.id) {} ''
        install -D ${fod} "$out/${outRel}"
      '';

    mkDrv = a:
      if a.kind == "run"
      then mkRun a
      else if a.kind == "write"
      then mkWrite a
      else if a.kind == "download"
      then mkDownload a
      else throw "buck2: cannot lower action kind '${a.kind}'";

    drvById = listToAttrs (map (a: {
        name = a.id;
        value = mkDrv a;
      })
      actions);

    defaultOut = analysis.defaultOutputForNode rootNode;
  in {
    inherit drvById actions;
    defaultOutputDrv =
      if defaultOut == null
      then throw "buck2: target '${rootNode.label}' has no DefaultInfo default output"
      else drvById.${producerId defaultOut};
    defaultOutputRel =
      if defaultOut == null
      then null
      else artPath defaultOut;
    defaultOutputName =
      if defaultOut == null
      then null
      else defaultOut.name;
  };
in {
  inherit lowerNode;
}
