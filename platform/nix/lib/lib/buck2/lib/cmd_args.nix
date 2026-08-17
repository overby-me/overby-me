# Buck2 cmd_args: an ordered argument builder carrying formatting options and
# hidden inputs. builtins only. Rendered to an argv at lowering time
# (build/lower.nix); here it is just data plus mutating-by-rebind methods
# (.add / .hidden return newSelf, which the evaluator rebinds).
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

  builtin = fn: {
    __sk = "builtin";
    name = "cmd_args";
    inherit fn;
  };

  # Flatten list/tuple arguments into a flat list of leaf values; artifacts,
  # strings, output wrappers, and nested cmd_args stay as leaves (rendered
  # recursively later).
  flatten = xs:
    builtins.concatLists (map (
        x:
          if V.isList x || V.isTuple x
          then flatten x.items
          else [x]
      )
      xs);

  toItems = v:
    if v == null
    then []
    else if V.isList v || V.isTuple v
    then flatten v.items
    else [v];

  mkCmdArgs = {
    parts,
    hidden,
    opts,
  }: {
    __sk = "cmd_args";
    inherit parts hidden opts;
    attrs = {
      add = builtin ({
        pos,
        world,
        ...
      }: {
        value = null;
        newSelf = mkCmdArgs {
          parts = parts ++ flatten pos;
          inherit hidden opts;
        };
        inherit world;
      });
      hidden = builtin ({
        pos,
        world,
        ...
      }: {
        value = null;
        newSelf = mkCmdArgs {
          inherit parts opts;
          hidden = hidden ++ flatten pos;
        };
        inherit world;
      });
      copy = builtin ({world, ...}: {
        value = mkCmdArgs {inherit parts hidden opts;};
        inherit world;
      });
    };
  };

  cmd_args = builtin ({
    pos,
    named,
    world,
    ...
  }: {
    value = mkCmdArgs {
      parts = flatten pos;
      hidden = toItems (getNamed named "hidden" null);
      opts = {
        format = getNamed named "format" null;
        delimiter = getNamed named "delimiter" null;
        prepend = getNamed named "prepend" null;
        relative_to = getNamed named "relative_to" null;
      };
    };
    inherit world;
  });
in {
  inherit cmd_args mkCmdArgs flatten;
}
