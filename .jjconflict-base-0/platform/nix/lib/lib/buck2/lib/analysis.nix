# Buck2 analysis phase: configured-target analysis. builtins only.
#
# analyzeTarget resolves a target's attrs (coercing sources to artifacts and
# deps/toolchains to analyzed provider sets), builds ctx, runs the rule impl
# through the skylark interpreter, and returns:
#   { label; providers; actions; deps; target; }
# where `providers` are the impl's returned provider instances, `actions` are
# the actions it registered, and `deps` are the analyzed dependency nodes
# (so lowering can walk the whole action graph).
{
  skylark,
  loader,
}: let
  V = skylark.values;
  labels = import ./labels.nix;
  actionsLib = import ./actions.nix {inherit V;};
  interp = skylark.mkInterp {};

  inherit (builtins) elemAt length;

  isLabelStr = s: builtins.isString s && (hasInfix "//" s || hasPrefix ":" s);
  hasPrefix = p: s: builtins.substring 0 (builtins.stringLength p) s == p;
  hasInfix = inf: s: let
    il = builtins.stringLength inf;
    sl = builtins.stringLength s;
    go = i:
      if i + il > sl
      then false
      else if builtins.substring i il s == inf
      then true
      else go (i + 1);
  in
    go 0;

  findProvider = providers: pid: let
    go = i:
      if i >= length providers
      then null
      else if (elemAt providers i).providerId or null == pid
      then elemAt providers i
      else go (i + 1);
  in
    go 0;

  defaultOutputOf = node: let
    di = findProvider node.providers "buck2//builtin:DefaultInfo";
  in
    if di == null
    then null
    else let
      one = di.attrs.default_output or null;
      many = di.attrs.default_outputs or null;
    in
      if one != null
      then one
      else if many != null && V.isList many && many.items != []
      then elemAt many.items 0
      else null;

  # Coerce a single attribute value against its attr-type descriptor.
  # Returns { value; deps; } where deps are analyzed dependency nodes.
  coerceValue = fromFile: target: attrType: raw:
    if attrType == null
    then {
      value = raw;
      deps = [];
    }
    else let
      kind = attrType.kind or "unknown";
    in
      if kind == "list" || kind == "string_list"
      then
        if raw == null || !(V.isList raw || V.isTuple raw)
        then {
          value = V.mkList [];
          deps = [];
        }
        else let
          inner = attrType.inner or null;
          coerced = map (x: coerceValue fromFile target inner x) raw.items;
        in {
          value = V.mkList (map (c: c.value) coerced);
          deps = builtins.concatLists (map (c: c.deps) coerced);
        }
      else if kind == "option"
      then
        if raw == null
        then {
          value = null;
          deps = [];
        }
        else coerceValue fromFile target (attrType.inner or null) raw
      else if kind == "default_only"
      then
        coerceValue fromFile target (attrType.inner or null) (
          if raw != null
          then raw
          else attrType.default or null
        )
      else if kind == "dep" || kind == "toolchain_dep" || kind == "exec_dep"
      then let
        node = analyzeTarget fromFile raw;
      in {
        value = mkDep node;
        deps = [node];
      }
      else if kind == "source"
      then
        if isLabelStr raw
        then let
          node = analyzeTarget fromFile raw;
          out = defaultOutputOf node;
        in {
          value = out;
          deps = [node];
        }
        else {
          value = {
            __sk = "artifact";
            kind = "source";
            srcRel = labels.joinPath [target.pkgDir raw];
            rel = raw;
            name = labels.baseOf raw;
            attrs.short_path = raw;
          };
          deps = [];
        }
      else {
        # string / int / bool / arg / dict / one_of / unknown: passthrough
        value = raw;
        deps = [];
      };

  mkDep = node: {
    __sk = "dep";
    inherit (node) label;
    inherit (node) providers;
    analysis = node;
    subscript = ptype: let
      inst = findProvider node.providers (ptype.id or null);
    in
      if inst == null
      then throw "buck2: dep '${node.label}' has no provider ${ptype.name or "?"}"
      else inst;
    attrs = {
      # dep[DefaultInfo] style also works via subscript; expose label too.
      inherit (node) label;
    };
  };

  # Coerce all of a target's attrs. Returns { attrs; deps; }.
  coerceAttrs = fromFile: target: let
    schema = target.ruleAttrs;
    schemaEntries =
      if V.isDict schema
      then schema.entries
      else [];
    provided = target.providedAttrs;
    step = acc: e: let
      attrName = e.key;
      attrType = e.value;
      raw =
        provided.${attrName} or attrType.default or null;
      c = coerceValue fromFile target attrType raw;
    in {
      attrs = acc.attrs // {${attrName} = c.value;};
      deps = acc.deps ++ c.deps;
    };
    base =
      builtins.foldl' step {
        attrs = {};
        deps = [];
      }
      schemaEntries;
  in {
    attrs =
      base.attrs
      // {
        name = provided.name or target.name;
      };
    inherit (base) deps;
  };

  analyzeTarget = fromFile: label: let
    target = loader.getTarget fromFile label;
    fromFileNext = target.resolved.buckPath;
    coercion = coerceAttrs fromFileNext target;
    ctx = actionsLib.mkCtx {
      coercedAttrs = coercion.attrs;
      inherit (target) name;
      pkg = target.pkgName;
      cell = target.pkgCell;
    };
    world0 = {
      actions = [];
      artifactSeq = 0;
      actionSeq = 0;
    };
    res = interp.callValue target.ruleImpl [ctx] [] world0;
    providersList =
      if res.value != null && (V.isList res.value || V.isTuple res.value)
      then res.value.items
      else [];
  in {
    inherit label target;
    providers = providersList;
    actions = res.world.actions or [];
    inherit (coercion) deps;
    inherit (target) pkgDir;
    inherit defaultOutputForNode;
  };

  defaultOutputForNode = defaultOutputOf;

  # Flatten a target node and all transitive deps into a deduped action list
  # plus the set of source artifacts, for lowering.
  collectActions = node: let
    go = seen: n:
      if builtins.elem n.label seen.labels
      then seen
      else let
        seen1 = {
          labels = seen.labels ++ [n.label];
          actions = seen.actions ++ n.actions;
        };
      in
        builtins.foldl' go seen1 n.deps;
    r =
      go {
        labels = [];
        actions = [];
      }
      node;
    # Dedup actions by id (same toolchain reached via multiple binaries).
    dedup = builtins.foldl' (acc: a:
      if builtins.any (x: x.id == a.id) acc
      then acc
      else acc ++ [a]) []
    r.actions;
  in
    dedup;
in {
  inherit analyzeTarget collectActions defaultOutputForNode findProvider;
}
