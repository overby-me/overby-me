# Buck2 analysis-time artifact model and ctx / ctx.actions. builtins only.
#
# The action registry lives in the threaded `world`:
#   world.actions      list of action records (run/write/download/...)
#   world.artifactSeq  counter for deterministic artifact ids
#   world.actionSeq    counter for deterministic action ids
# ctx.actions.* mint artifacts and append actions; lowering (build/lower.nix)
# turns each action into one derivation.
{V}: let
  inherit (builtins) elemAt length;

  getNamed = named: nm: default: let
    go = i:
      if i >= length named
      then default
      else if (elemAt named i).name == nm
      then (elemAt named i).value
      else go (i + 1);
  in
    go 0;
  inherit (V) truthy;

  builtin = fn: {
    __sk = "builtin";
    name = "action";
    inherit fn;
  };

  # ---- artifacts ---------------------------------------------------------
  mkArtifact = {
    id,
    name,
    owner,
    kind ? "output",
  }: {
    __sk = "artifact";
    inherit id name owner kind;
    attrs = {
      as_output = builtin ({world, ...}: {
        value = {
          __sk = "output_arg";
          artifact = mkArtifact {inherit id name owner kind;};
        };
        inherit world;
      });
      short_path = name;
      basename = baseName name;
    };
  };
  baseName = p: let
    parts = builtins.filter builtins.isString (builtins.split "/" p);
    ne = builtins.filter (x: x != "") parts;
  in
    if ne == []
    then p
    else elemAt ne (length ne - 1);

  # Collect the output artifacts referenced (via as_output()) anywhere in a
  # command value (cmd_args / list / output_arg).
  collectOutputs = v:
    if !(builtins.isAttrs v)
    then []
    else if v ? __sk && v.__sk == "output_arg"
    then [v.artifact]
    else if v ? __sk && v.__sk == "cmd_args"
    then builtins.concatLists (map collectOutputs (v.parts ++ v.hidden))
    else if v ? __sk && (v.__sk == "list" || v.__sk == "tuple")
    then builtins.concatLists (map collectOutputs v.items)
    else [];

  # ---- ctx.actions -------------------------------------------------------
  mkActions = targetLabel: let
    mkId = seq: kind: "${targetLabel}#${kind}${toString seq}";
    declareArtifact = world: name: let
      seq = world.artifactSeq or 0;
    in {
      artifact = mkArtifact {
        id = "${targetLabel}!${toString seq}:${name}";
        inherit name;
        owner = targetLabel;
      };
      world = world // {artifactSeq = seq + 1;};
    };
  in {
    __sk = "object";
    attrs = {
      declare_output = builtin ({
        pos,
        world,
        ...
      }: let
        # declare_output(prefix, filename?) or declare_output(filename)
        name =
          if length pos >= 2 && builtins.isString (elemAt pos 1)
          then "${elemAt pos 0}/${elemAt pos 1}"
          else elemAt pos 0;
        d = declareArtifact world name;
      in {
        value = d.artifact;
        inherit (d) world;
      });

      run = builtin ({
        pos,
        named,
        world,
        ...
      }: let
        cmd = elemAt pos 0;
        seq = world.actionSeq or 0;
        action = {
          kind = "run";
          id = mkId seq "run";
          inherit cmd;
          category = getNamed named "category" null;
          identifier = getNamed named "identifier" null;
          env = getNamed named "env" null;
          outputs = collectOutputs cmd;
        };
      in {
        value = null;
        world =
          world
          // {
            actions = (world.actions or []) ++ [action];
            actionSeq = seq + 1;
          };
      });

      write = builtin ({
        pos,
        named,
        world,
        ...
      }: let
        name = elemAt pos 0;
        content = elemAt pos 1;
        d = declareArtifact world name;
        seq = d.world.actionSeq or 0;
        allowArgs = truthy (getNamed named "allow_args" false);
        action = {
          kind = "write";
          id = mkId seq "write";
          output = d.artifact;
          inherit content;
          isExecutable = truthy (getNamed named "is_executable" false);
          inherit allowArgs;
        };
        world' =
          d.world
          // {
            actions = (d.world.actions or []) ++ [action];
            actionSeq = seq + 1;
          };
      in {
        # With allow_args, buck2 returns (artifact, macro_files); we return an
        # empty macro-file list (no cmd_args macros in the corpus).
        value =
          if allowArgs
          then {
            __sk = "tuple";
            items = [d.artifact (V.mkList [])];
          }
          else d.artifact;
        world = world';
      });

      download_file = builtin ({
        pos,
        named,
        world,
        ...
      }: let
        outArg = elemAt pos 0;
        artifact =
          if builtins.isAttrs outArg && outArg ? __sk && outArg.__sk == "output_arg"
          then outArg.artifact
          else outArg;
        url = elemAt pos 1;
        seq = world.actionSeq or 0;
        action = {
          kind = "download";
          id = mkId seq "download";
          output = artifact;
          inherit url;
          sha256 = getNamed named "sha256" null;
          sha1 = getNamed named "sha1" null;
          isExecutable = truthy (getNamed named "is_executable" false);
        };
      in {
        value = artifact;
        world =
          world
          // {
            actions = (world.actions or []) ++ [action];
            actionSeq = seq + 1;
          };
      });

      copy_file = builtin ({
        pos,
        world,
        ...
      }: let
        dest = elemAt pos 0;
        src = elemAt pos 1;
        artifact =
          if builtins.isAttrs dest && dest ? __sk && dest.__sk == "output_arg"
          then dest.artifact
          else dest;
        seq = world.actionSeq or 0;
        action = {
          kind = "copy";
          id = mkId seq "copy";
          output = artifact;
          inherit src;
        };
      in {
        value = artifact;
        world =
          world
          // {
            actions = (world.actions or []) ++ [action];
            actionSeq = seq + 1;
          };
      });
    };
  };

  # ---- ctx ---------------------------------------------------------------
  mkLabelStruct = {
    name,
    pkg,
    cell,
  }: {
    __sk = "struct";
    attrs = {
      inherit name;
      package = pkg;
      cell =
        if cell == ""
        then "root"
        else cell;
      raw_target = builtin ({world, ...}: {
        value = "${cell}//${pkg}:${name}";
        inherit world;
      });
    };
  };

  mkCtx = {
    coercedAttrs,
    name,
    pkg,
    cell,
  }: {
    __sk = "object";
    attrs = {
      attrs = {
        __sk = "object";
        attrs = coercedAttrs;
      };
      label = mkLabelStruct {inherit name pkg cell;};
      actions = mkActions "${
        if cell == ""
        then ""
        else cell
      }//${pkg}:${name}";
    };
  };
in {
  inherit mkArtifact mkActions mkCtx collectOutputs;
}
