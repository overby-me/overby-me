# Lower a Buck2 action graph to Nix derivations: one derivation per action, no
# import-from-derivation.
#
# Actions run against a virtual "buck-out" where every artifact has a stable
# working-directory-relative path. Command lines and script contents name those
# relative paths, honouring cmd_args relative_to, never store paths, so an action
# that needs only a peer's path - a generated script naming an output it does not
# build - creates neither a derivation dependency nor a cycle. Each action stages
# its inputs into a working tree, copying each producer's whole tree so that
# transitive files and symlink targets travel along, runs, and exports that tree
# as $out. Dependencies flow only through store-path interpolation in the staging
# commands.
#
# mkLower { pkgs; root; toolchainPackages; } -> { lowerGraph; }
# lowerGraph { actions; defaultOutput; } -> { defaultOutputDrv; defaultOutputRel; ... }
# The graph may be rich (from analysis.nix, functions ignored) or plain (from
# fromJSON of the IFD analysis); lowering touches only data fields.
{
  pkgs,
  root,
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
  # toolchain build a single-file main without a module). Caches live under
  # $NIX_BUILD_TOP, not the working tree ($out), so they never leak into output.
  runEnvPrelude = ''
    export HOME="$NIX_BUILD_TOP/.home" GOCACHE="$NIX_BUILD_TOP/.gocache" GOPATH="$NIX_BUILD_TOP/.gopath"
    export GO111MODULE=off CGO_ENABLED=0 GOPROXY=off GOTOOLCHAIN=local
    mkdir -p "$HOME"
  '';

  lowerGraph = graph: let
    inherit (graph) actions;
    actionOutputs = a:
      if a.kind == "symlinked_dir" || a.kind == "copy"
      then [a.output]
      else if a.kind == "run"
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
      if a.kind == "symlinked_dir"
      then lib.unique (map producerId (filter (x: x.kind != "source") (collectInputs (map (e: e.src) a.entries))))
      else if a.kind == "copy"
      then lib.unique (map producerId (filter (x: x.kind != "source") (collectInputs [a.src])))
      else if a.kind == "run"
      then lib.unique (map producerId (filter (x: x.kind != "source") (collectInputs (a.cmd.parts ++ a.cmd.hidden))))
      else [];
    # An action needs autoPatchelf only when it materializes real files from a
    # download (e.g. untars the Go toolchain): its direct input is a download.
    # Consumers symlink to the already-patched tree, so they never re-scan.
    # Skipping this for from-source builds (cpp/rust) keeps them fast.
    stagesDownload = a: builtins.any (id: actionById.${id}.kind == "download") (runDepIds a);

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
      patch = stagesDownload a;
      # The working tree is built directly in $out. Dependency trees come in
      # through cp -rs, giving real dirs and file symlinks into the producer's
      # store path, so a large toolchain is never copied and its patched binaries
      # and symlink targets are reached through the link.
      stageDeps = builtins.concatStringsSep "\n" (map (id: "cp -rsf --no-preserve=mode ${drvById.${id}}/. ./") depIds);
      # Sources are copied (real files) so relative #include / sibling lookups
      # resolve within the staged package directory.
      stageSrcs = builtins.concatStringsSep "\n" (map (s: "install -Dm644 ${srcStorePath s} ${esc (artPath s)}") srcs);
      mkOutDirs = builtins.concatStringsSep "\n" (map (o: ''mkdir -p "$(dirname ${esc (artPath o)})"'') outs);
      # autoPatchelf makes freshly-materialized downloaded binaries runnable on
      # Nix by fixing their ELF interpreter (runs once, in the untar action).
      # No extra buildInputs: glibc here would pollute the C++ header path, and
      # the patched interpreter finds libc via its own default search.
      patchCall = lib.optionalString patch ''autoPatchelf "$out" || true'';
    in
      pkgs.runCommand (sanDrv a.id) ({
          nativeBuildInputs =
            # stdenv.cc for compiler toolchains (linking) and for autoPatchelf's
            # dynamic-linker; omitted from pure shell actions (extract, symlink).
            lib.optional (tcPkgs != [] || patch) pkgs.stdenv.cc
            ++ tcPkgs
            ++ lib.optional patch pkgs.autoPatchelfHook;
          dontStrip = true;
          preferLocalBuild = true;
        }
        // lib.optionalAttrs patch {autoPatchelfIgnoreMissingDeps = true;}) ''
        ${runEnvPrelude}
        mkdir -p $out
        cd $out
        ${stageDeps}
        ${stageSrcs}
        ${mkOutDirs}
        ${argv}
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

    # A directory of links to the mapped artifacts. Dependency trees come in the
    # way mkRun stages them, and each entry then links to its file inside that
    # staged tree, so nothing is copied twice and a staged header still resolves.
    mkSymlinkedDir = a: let
      outRel = artPath a.output;
      srcs = filter (x: x.kind == "source") (collectInputs (map (e: e.src) a.entries));
      depIds =
        lib.unique (map producerId (filter (x: x.kind != "source")
            (collectInputs (map (e: e.src) a.entries))));
      stageDeps =
        builtins.concatStringsSep "\n"
        (map (id: "cp -rsf --no-preserve=mode ${drvById.${id}}/. ./staged/") depIds);
      stageSrcs =
        builtins.concatStringsSep "\n"
        (map (x: "install -Dm644 ${srcStorePath x} ./staged/${artPath x}") srcs);
      linkEntries = builtins.concatStringsSep "\n" (map (e: let
          from = artPath (builtins.head (collectInputs [e.src]));
        in ''
          mkdir -p "$out/${outRel}/$(dirname ${esc e.path})"
          cp -a --no-preserve=mode "./staged/${from}" "$out/${outRel}/${e.path}"
        '')
        a.entries);
    in
      pkgs.runCommand (sanDrv a.id) {preferLocalBuild = true;} ''
        mkdir -p ./staged "$out/${outRel}"
        ${stageDeps}
        ${stageSrcs}
        ${linkEntries}
      '';

    # copy_file: one file, same staging rules.
    mkCopy = a: let
      outRel = artPath a.output;
      ins = collectInputs [a.src];
      src = builtins.head ins;
      srcPath =
        if src.kind == "source"
        then srcStorePath src
        else "${drvById.${producerId src}}/${artPath src}";
    in
      pkgs.runCommand (sanDrv a.id) {preferLocalBuild = true;} ''
        install -D ${srcPath} "$out/${outRel}"
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
      else if a.kind == "symlinked_dir"
      then mkSymlinkedDir a
      else if a.kind == "copy"
      then mkCopy a
      else throw "buck2: cannot lower action kind '${a.kind}'";

    drvById = listToAttrs (map (a: {
        name = a.id;
        value = mkDrv a;
      })
      actions);

    defaultOut = graph.defaultOutput;
  in {
    inherit drvById actions;
    defaultOutputDrv =
      if defaultOut == null
      then throw "buck2: target has no DefaultInfo default output"
      # A target whose default output IS a source file, which is exactly what
      # export_file gives. No action produces it, and a source artifact carries no
      # action id, so asking which one does fails before it can say why. Hence
      # materializing the source itself, under the path a consumer expects.
      else if defaultOut.kind == "source"
      then
        pkgs.runCommand (sanDrv "src-${defaultOut.name}") {preferLocalBuild = true;} ''
          install -D ${srcStorePath defaultOut} "$out/${artPath defaultOut}"
        ''
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
  inherit lowerGraph;
}
