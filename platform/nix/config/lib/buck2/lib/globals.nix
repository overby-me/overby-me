# Buck2 Starlark globals (everything except the per-target `ctx`). builtins
# only. Injected as the skylark interpreter's extraGlobals, so rule impls can
# reference cmd_args / DefaultInfo / host_info / ... at analysis time.
#
# mkGlobals { V; currentFile; system; } -> attrset of globals.
#   provider, rule, attrs, struct, host_info, oncall, glob, select,
#   DefaultInfo, RunInfo, cmd_args.
{
  V,
  currentFile,
  system,
  # Parsed .buckconfig sections, for read_root_config / read_config.
  sections ? {},
}: let
  inherit (V) mkList mkDict truthy;
  inherit (builtins) elemAt length substring stringLength filter isString;

  # ---- small helpers -----------------------------------------------------
  namedToAttrs = named: builtins.listToAttrs (map (e: {inherit (e) name value;}) named);
  getNamed = named: nm: default: let
    go = i:
      if i >= length named
      then default
      else if (elemAt named i).name == nm
      then (elemAt named i).value
      else go (i + 1);
  in
    go 0;
  argAt = pos: i: default:
    if i < length pos
    then elemAt pos i
    else default;

  builtin = fn: {
    __sk = "builtin";
    name = "buck2";
    inherit fn;
  };
  obj = attrs: {
    __sk = "object";
    inherit attrs;
  };

  # A field-name list from provider(fields = [...] | {..}).
  fieldNames = v:
    if V.isList v || V.isTuple v
    then map (x: x) v.items
    else if V.isDict v
    then V.dictKeys v
    else [];

  # ---- providers ---------------------------------------------------------
  mkProviderType = {
    id,
    name,
    fields,
  }: {
    __sk = "provider";
    inherit id name fields;
    # Constructing an instance: PInfo(field = value, ...).
    fn = {
      named,
      world,
      ...
    }: {
      value = {
        __sk = "provider_instance";
        providerId = id;
        providerName = name;
        # Declared fields default to None; extra named kept too (lenient).
        attrs =
          builtins.listToAttrs (map (f: {
              name = f;
              value = getNamed named f null;
            })
            fields)
          // namedToAttrs named;
      };
      inherit world;
    };
  };

  providerGlobal = builtin ({
    pos,
    named,
    world,
    ...
  }: let
    seq = world.providerSeq or 0;
    fieldsArg = getNamed named "fields" (argAt pos 0 (mkList []));
  in {
    value = mkProviderType {
      id = "${currentFile}#${toString seq}";
      name = "provider";
      fields = fieldNames fieldsArg;
    };
    world = world // {providerSeq = seq + 1;};
  });

  DefaultInfo = mkProviderType {
    id = "buck2//builtin:DefaultInfo";
    name = "DefaultInfo";
    fields = ["default_output" "default_outputs" "sub_targets" "other_outputs"];
  };
  RunInfo = mkProviderType {
    id = "buck2//builtin:RunInfo";
    name = "RunInfo";
    fields = ["args"];
  };

  # ---- rule --------------------------------------------------------------
  ruleGlobal = builtin ({
    named,
    world,
    ...
  }: let
    impl = getNamed named "impl" null;
    attrsSchema = getNamed named "attrs" (mkDict []);
    isToolchain = truthy (getNamed named "is_toolchain_rule" false);
    ruleVal = {
      __sk = "rule";
      inherit impl attrsSchema isToolchain;
      # Calling the rule in a BUCK file registers a target.
      fn = {
        named,
        world,
        ...
      }: let
        nameV = getNamed named "name" null;
        cell = world.pkgCell or "";
        pkg = world.pkgName or "";
        label = "${
          if cell == ""
          then ""
          else cell
        }//${pkg}:${nameV}";
        target = {
          inherit label;
          name = nameV;
          ruleImpl = impl;
          ruleAttrs = attrsSchema;
          ruleIsToolchain = isToolchain;
          providedAttrs = namedToAttrs named;
          pkgCell = cell;
          pkgName = pkg;
          pkgDir = world.pkgDir or "";
          pkgSrc = world.pkgSrc or null;
        };
      in {
        value = null;
        world = world // {targets = (world.targets or []) ++ [target];};
      };
    };
  in {
    value = ruleVal;
    inherit world;
  });

  # ---- attrs.* -----------------------------------------------------------
  attrType = kind: extra: {__sk = "attr_type";} // {inherit kind;} // extra;
  attrCtor = kind:
    builtin ({
      named,
      world,
      ...
    }: {
      value = attrType kind {
        default = getNamed named "default" null;
        hasDefault = getNamed named "default" null != null;
      };
      inherit world;
    });
  attrCtorInner = kind:
    builtin ({
      pos,
      named,
      world,
      ...
    }: {
      value = attrType kind {
        inner = argAt pos 0 null;
        default = getNamed named "default" null;
        hasDefault = getNamed named "default" null != null;
      };
      inherit world;
    });
  attrsObj = obj {
    string = attrCtor "string";
    bool = attrCtor "bool";
    int = attrCtor "int";
    source = attrCtor "source";
    dep = attrCtor "dep";
    exec_dep = attrCtor "dep";
    toolchain_dep = attrCtor "toolchain_dep";
    configuration_label = attrCtor "string";
    list = attrCtorInner "list";
    option = attrCtorInner "option";
    string_list = attrCtor "string_list";
    default_only = builtin ({
      pos,
      world,
      ...
    }: let
      inner = argAt pos 0 null;
    in {
      value = attrType "default_only" {
        inherit inner;
        default = inner.default or null;
        hasDefault = true;
        defaultOnly = true;
      };
      inherit world;
    });
    dict = builtin ({
      pos,
      named,
      world,
      ...
    }: {
      value = attrType "dict" {
        key = argAt pos 0 null;
        valueType = argAt pos 1 null;
        default = getNamed named "default" null;
      };
      inherit world;
    });
    one_of = builtin ({
      pos,
      world,
      ...
    }: {
      value = attrType "one_of" {options = pos;};
      inherit world;
    });
    arg = attrCtor "arg";
  };

  # ---- struct ------------------------------------------------------------
  structGlobal = builtin ({
    named,
    world,
    ...
  }: {
    value = {
      __sk = "struct";
      attrs = namedToAttrs named;
    };
    inherit world;
  });

  # ---- host_info ---------------------------------------------------------
  sysParts = filter isString (builtins.split "-" system);
  arch = elemAt sysParts 0;
  osName = elemAt sysParts (length sysParts - 1);
  boolStruct = attrs: {
    __sk = "struct";
    inherit attrs;
  };
  osStruct = boolStruct {
    is_linux = osName == "linux";
    is_macos = osName == "darwin";
    is_windows = osName == "windows";
    is_freebsd = false;
    is_unknown = false;
    value = osName;
  };
  archStruct = boolStruct {
    is_x86_64 = arch == "x86_64";
    is_aarch64 = arch == "aarch64" || arch == "arm64";
    is_arm = false;
    is_i386 = arch == "i686" || arch == "i386";
    is_unknown = false;
    value = arch;
  };
  hostInfoVal = {
    __sk = "struct";
    attrs = {
      os = osStruct;
      arch = archStruct;
    };
  };
  hostInfoGlobal = builtin ({world, ...}: {
    value = hostInfoVal;
    inherit world;
  });

  oncallGlobal = builtin ({world, ...}: {
    value = null;
    inherit world;
  });

  # select({...}): no configuration yet; return the DEFAULT branch (or the
  # first entry) so the common unconfigured case degrades gracefully.
  selectGlobal = builtin ({
    pos,
    world,
    ...
  }: let
    d = argAt pos 0 (mkDict []);
    def = V.dictGetOr d "DEFAULT" (
      if d.entries == []
      then null
      else (elemAt d.entries 0).value
    );
  in {
    value = def;
    inherit world;
  });

  # ---- glob --------------------------------------------------------------
  # read_root_config(section, key, default = None): a value from the project's
  # .buckconfig, with .buckconfig.local layered on top. Buck2 uses it for values that
  # are machine-local rather than checked in -- a toolchain's compiler or linker path --
  # so a rule can name one without hardcoding it. read_config is the same lookup for the
  # root cell, which is all this interpreter models.
  readRootConfigGlobal = builtin ({
    pos,
    world,
    ...
  }: let
    # Strings are plain Nix strings in this interpreter, and None is null.
    section = argAt pos 0 "";
    key = argAt pos 1 "";
    fallback = argAt pos 2 V.none;
    have = sections ? ${section} && sections.${section} ? ${key};
  in {
    value =
      if have
      then sections.${section}.${key}
      else fallback;
    inherit world;
  });

  globGlobal = builtin ({
    pos,
    named,
    world,
    ...
  }: let
    includes = map (x: x) (argAt pos 0 (mkList [])).items;
    excludes = map (x: x) (getNamed named "exclude" (argAt pos 1 (mkList []))).items;
    dir = world.pkgSrc;
    matched =
      if dir == null
      then []
      else globFiles dir includes excludes;
  in {
    value = mkList matched;
    inherit world;
  });

  # Match one path segment against a wildcard pattern (* and ?).
  wildMatch = pat: name: let
    pl = stringLength pat;
    nl = stringLength name;
    go = pi: ni:
      if pi >= pl
      then ni >= nl
      else let
        pc = substring pi 1 pat;
      in
        if pc == "*"
        then
          (
            # try to match zero or more chars
            if go (pi + 1) ni
            then true
            else if ni < nl
            then go pi (ni + 1)
            else false
          )
        else if ni >= nl
        then false
        else if pc == "?" || pc == substring ni 1 name
        then go (pi + 1) (ni + 1)
        else false;
  in
    go 0 0;

  # Enumerate files matching include patterns (relative to `dir`), minus
  # excludes. Supports `*`/`?` within a segment and `**` across segments.
  globFiles = dir: includes: excludes: let
    listAt = rel: let
      p =
        if rel == ""
        then dir
        else dir + "/${rel}";
    in
      builtins.readDir p;
    # Expand a pattern (segment list) from a relative dir, collecting files.
    walk = rel: segs:
      if segs == []
      then []
      else let
        seg = builtins.head segs;
        rest = builtins.tail segs;
        entries = listAt rel;
        names = builtins.attrNames entries;
      in
        if seg == "**"
        then let
          here =
            if rest == []
            then []
            else walk rel rest;
          intoDirs = builtins.concatLists (map (
              n:
                if entries.${n} == "directory"
                then walk (joinRel rel n) segs
                else []
            )
            names);
        in
          here ++ intoDirs
        else
          builtins.concatLists (map (
              n:
                if !(wildMatch seg n)
                then []
                else if rest == []
                then
                  (
                    if entries.${n} == "regular" || entries.${n} == "symlink"
                    then [(joinRel rel n)]
                    else []
                  )
                else if entries.${n} == "directory"
                then walk (joinRel rel n) rest
                else []
            )
            names);
    joinRel = rel: n:
      if rel == ""
      then n
      else rel + "/${n}";
    splitPat = p: filter (x: x != "" && isString x) (builtins.split "/" p);
    allMatches = builtins.concatLists (map (inc: walk "" (splitPat inc)) includes);
    deduped = builtins.foldl' (acc: m:
      if builtins.elem m acc
      then acc
      else acc ++ [m]) []
    allMatches;
    excludedSet = builtins.concatLists (map (ex: walk "" (splitPat ex)) excludes);
    final = filter (m: !(builtins.elem m excludedSet)) deduped;
  in
    builtins.sort (a: b: a < b) final;

  cmdArgs = import ./cmd_args.nix {inherit V;};
in {
  provider = providerGlobal;
  rule = ruleGlobal;
  attrs = attrsObj;
  struct = structGlobal;
  read_root_config = readRootConfigGlobal;
  read_config = readRootConfigGlobal;
  host_info = hostInfoGlobal;
  oncall = oncallGlobal;
  glob = globGlobal;
  select = selectGlobal;
  inherit DefaultInfo RunInfo;
  inherit (cmdArgs) cmd_args;
  # Re-export for the analysis layer.
  inherit mkProviderType;
}
