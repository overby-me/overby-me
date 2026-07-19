# Lower a Buck2 action graph to Nix derivations: one derivation per action, no
# import-from-derivation. Dependencies between actions/targets are wired
# through store-path interpolation.
#
# mkLower { pkgs; root; analysis; toolchainPackages; } -> { lowerNode; }
#   lowerNode node -> { defaultOutputDrv; defaultOutputName; drvById; actions; }
{
  pkgs,
  root,
  analysis,
  toolchainPackages,
}: let
  inherit (pkgs) lib;
  inherit (builtins) filter concatMap isAttrs isString listToAttrs;

  esc = lib.escapeShellArg;

  sanitize = s:
    "buck2-"
    + builtins.replaceStrings
    ["/" ":" "#" "!" "." " " "+" "@" "," "(" ")" "[" "]" "="]
    ["-" "-" "-" "-" "-" "-" "-" "-" "-" "-" "-" "-" "-" "-"]
    s;

  srcStorePath = art:
    builtins.path {
      path = root + "/${art.srcRel}";
      inherit (art) name;
    };

  # Source artifacts anywhere in a command (for staging into the sandbox).
  collectSources = xs: concatMap collectSrc xs;
  collectSrc = v:
    if !(isAttrs v) || !(v ? __sk)
    then []
    else if v.__sk == "artifact" && v.kind == "source"
    then [v]
    else if v.__sk == "cmd_args"
    then collectSources (v.parts ++ v.hidden)
    else if v.__sk == "list" || v.__sk == "tuple"
    then collectSources v.items
    else [];

  # Literal string leaves of a command (for toolchain-package detection).
  litStrings = xs: concatMap litStr xs;
  litStr = v:
    if isString v
    then [v]
    else if isAttrs v && v ? __sk && v.__sk == "cmd_args"
    then litStrings v.parts
    else if isAttrs v && v ? __sk && (v.__sk == "list" || v.__sk == "tuple")
    then litStrings v.items
    else [];

  lowerNode = rootNode: let
    actions = analysis.collectActions rootNode;
    actionById = listToAttrs (map (a: {
        name = a.id;
        value = a;
      })
      actions);
    actionOutputs = a:
      if a.kind == "run"
      then a.outputs
      else if a ? output
      then [a.output]
      else [];
    outputToAction = listToAttrs (concatMap (a:
      map (o: {
        name = o.id;
        value = a.id;
      }) (actionOutputs a))
    actions);

    # Store-path string of an output artifact (produced by some action).
    outputPath = art: let
      aid =
        outputToAction.${art.id}
        or (throw "buck2: no action produces artifact '${art.id}'");
      a = actionById.${aid};
    in
      if a.kind == "download"
      then "${drvById.${aid}}"
      else "${drvById.${aid}}/${art.name}";

    # Render a command part to a list of shell tokens (already quoted).
    renderToken = part:
      if isString part
      then [(esc part)]
      else if !(isAttrs part && part ? __sk)
      then [(esc (toString part))]
      else if part.__sk == "output_arg"
      then ["\"$out/${part.artifact.name}\""]
      else if part.__sk == "artifact"
      then
        (
          if part.kind == "source"
          then [(esc part.rel)]
          else [(esc (outputPath part))]
        )
      else if part.__sk == "cmd_args"
      then renderCmdArgs part
      else [];

    # Nested cmd_args: render parts, apply delimiter/format/prepend. Enough
    # for the go vertical's script/symlink args; relative_to is unsupported.
    renderCmdArgs = cav: let
      raw = concatMap renderRaw cav.parts;
      withPrepend =
        if cav.opts.prepend or null != null
        then concatMap (x: [cav.opts.prepend x]) raw
        else raw;
      formatted =
        if cav.opts.format or null != null
        then map (x: builtins.replaceStrings ["{}"] [x] cav.opts.format) withPrepend
        else withPrepend;
      joined =
        if cav.opts.delimiter or null != null
        then [(builtins.concatStringsSep cav.opts.delimiter formatted)]
        else formatted;
    in
      map esc joined;

    # Raw (unquoted) rendering of a part, for use inside format/delimiter.
    renderRaw = part:
      if isString part
      then [part]
      else if !(isAttrs part && part ? __sk)
      then [(toString part)]
      else if part.__sk == "output_arg"
      then ["$out/${part.artifact.name}"]
      else if part.__sk == "artifact"
      then
        (
          if part.kind == "source"
          then [part.rel]
          else [(outputPath part)]
        )
      else if part.__sk == "cmd_args"
      then concatMap renderRaw part.parts
      else [];

    # ---- per-action derivations ----------------------------------------
    mkRun = a: let
      srcs = collectSources (a.cmd.parts ++ a.cmd.hidden);
      stage = builtins.concatStringsSep "\n" (map (s: "install -Dm644 ${srcStorePath s} ${esc s.rel}") srcs);
      argv = builtins.concatStringsSep " " (concatMap renderToken a.cmd.parts);
      strings = litStrings a.cmd.parts;
      tcPkgs = map (k: toolchainPackages.${k}) (filter (k: builtins.elem k strings) (builtins.attrNames toolchainPackages));
      mkOutDirs = builtins.concatStringsSep "\n" (map (o: ''mkdir -p "$(dirname "$out/${o.name}")"'') a.outputs);
    in
      pkgs.runCommand (sanitize a.id) {
        nativeBuildInputs = [pkgs.stdenv.cc] ++ tcPkgs;
      } ''
        mkdir -p $out
        ${mkOutDirs}
        ${stage}
        ${argv}
      '';

    mkWrite = a: let
      contentFile = pkgs.writeText "${sanitize a.id}-content" (renderWriteContent a.content);
    in
      pkgs.runCommand (sanitize a.id) {} ''
        mkdir -p "$(dirname "$out/${a.output.name}")"
        cp ${contentFile} "$out/${a.output.name}"
        ${lib.optionalString a.isExecutable ''chmod +x "$out/${a.output.name}"''}
      '';

    renderWriteContent = c:
      if isString c
      then c
      else if isAttrs c && c ? __sk && c.__sk == "cmd_args"
      then builtins.concatStringsSep " " (concatMap renderRaw c.parts)
      else if isAttrs c && c ? __sk && (c.__sk == "list" || c.__sk == "tuple")
      then builtins.concatStringsSep "\n" (map renderWriteContent c.items)
      else if c == null
      then ""
      else toString c;

    mkDownload = a:
      pkgs.fetchurl ({inherit (a) url;}
        // (
          if a.sha256 or null != null
          then {inherit (a) sha256;}
          else if a.sha1 or null != null
          then {inherit (a) sha1;}
          else {}
        ));

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
      else drvById.${outputToAction.${defaultOut.id}};
    defaultOutputName =
      if defaultOut == null
      then null
      else defaultOut.name;
    defaultOutputIsDownload =
      defaultOut != null && actionById.${outputToAction.${defaultOut.id}}.kind == "download";
  };
in {
  inherit lowerNode;
}
