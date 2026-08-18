# Serialize a Buck2 action graph to a JSON-safe form. builtins only.
#
# The rich values built during analysis carry Nix functions (artifact
# `.as_output`, cmd_args `.add`, dep `.subscript`) that `builtins.toJSON`
# cannot encode. Lowering never uses those, only the data fields, so this
# rebuilds each value keeping `__sk` + data and dropping the callables. The
# result round-trips through toJSON/fromJSON and feeds build/lower.nix
# unchanged (rich or plain both work).
let
  inherit (builtins) isString isInt isBool isList isAttrs map;

  # Output artifacts carry id/owner; source artifacts carry srcRel/rel. Keep
  # whichever are present (lowering reads only the data it needs per kind).
  plainArtifact = a:
    {
      __sk = "artifact";
      inherit (a) kind name;
    }
    // (
      if a ? id
      then {inherit (a) id;}
      else {}
    )
    // (
      if a ? owner
      then {inherit (a) owner;}
      else {}
    )
    // (
      if a ? srcRel
      then {inherit (a) srcRel;}
      else {}
    )
    // (
      if a ? rel
      then {inherit (a) rel;}
      else {}
    );

  plainVal = v:
    if isString v || isInt v || isBool v || v == null
    then v
    else if isList v
    then map plainVal v
    else if isAttrs v && v ? __sk
    then let
      sk = v.__sk;
    in
      if sk == "artifact"
      then plainArtifact v
      else if sk == "output_arg"
      then {
        __sk = "output_arg";
        artifact = plainArtifact v.artifact;
      }
      else if sk == "cmd_args"
      then {
        __sk = "cmd_args";
        parts = map plainVal v.parts;
        hidden = map plainVal v.hidden;
        opts = plainOpts v.opts;
      }
      else if sk == "tuple"
      then {
        __sk = "tuple";
        items = map plainVal v.items;
      }
      else if sk == "list"
      then {
        __sk = "list";
        items = map plainVal v.items;
      }
      else throw "buck2 serialize: cannot serialize value of kind '${sk}'"
    else throw "buck2 serialize: cannot serialize non-tagged value";

  plainOpts = o: {
    format = o.format or null;
    delimiter = o.delimiter or null;
    prepend = o.prepend or null;
    relative_to =
      if (o.relative_to or null) == null
      then null
      else plainVal o.relative_to;
  };

  plainAction = a:
    if a.kind == "run"
    then {
      inherit (a) kind id;
      category = a.category or null;
      cmd = plainVal a.cmd;
    }
    else if a.kind == "write"
    then {
      inherit (a) kind id;
      output = plainArtifact a.output;
      content = plainVal a.content;
      isExecutable = a.isExecutable or false;
    }
    else if a.kind == "download"
    then {
      inherit (a) kind id url;
      output = plainArtifact a.output;
      sha256 = a.sha256 or null;
      sha1 = a.sha1 or null;
    }
    else if a.kind == "symlinked_dir"
    then {
      inherit (a) kind id;
      output = plainArtifact a.output;
      entries =
        map (e: {
          inherit (e) path;
          src = plainVal e.src;
        })
        a.entries;
    }
    else if a.kind == "copy"
    then {
      inherit (a) kind id;
      output = plainArtifact a.output;
      src = plainVal a.src;
    }
    else throw "buck2 serialize: unknown action kind '${a.kind}'";

  plainGraph = {
    actions,
    defaultOutput,
  }: {
    actions = map plainAction actions;
    defaultOutput =
      if defaultOutput == null
      then null
      else plainArtifact defaultOutput;
  };
in {
  inherit plainGraph plainAction plainVal plainArtifact;
}
