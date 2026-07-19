# Starlark value model. builtins only.
#
# Scalars are native Nix values: null (None), bool, int, float, string.
# Compound and host values are tagged attrsets with a `__sk` discriminator.
# Core builds only list/tuple/dict/function/builtin; every other `__sk`
# (struct, object, artifact, cmd_args, provider, rule, dep, ...) is a "host"
# value supplied by a layer above (e.g. Buck2) and handled generically via
# the host protocol: `.attrs`/`.getAttr` (member access), `.subscript`
# (indexing), `.fn` (calling), `.id` (identity for equality).
let
  inherit (builtins) isInt isFloat isBool isString isAttrs elemAt length;

  isNum = v: isInt v || isFloat v;
  toF = v:
    if isInt v
    then v * 1.0
    else v;

  none = null;

  mkList = items: {
    __sk = "list";
    inherit items;
  };
  mkTuple = items: {
    __sk = "tuple";
    inherit items;
  };
  mkDict = entries: {
    __sk = "dict";
    inherit entries;
  };
  mkFn = f:
    {
      __sk = "function";
    }
    // f;
  mkBuiltin = name: fn: {
    __sk = "builtin";
    inherit name fn;
  };

  isList = v: isAttrs v && v ? __sk && v.__sk == "list";
  isTuple = v: isAttrs v && v ? __sk && v.__sk == "tuple";
  isDict = v: isAttrs v && v ? __sk && v.__sk == "dict";
  isSeq = v: isList v || isTuple v;
  isCallable = v: isAttrs v && v ? __sk && (v.__sk == "function" || v.__sk == "builtin" || v ? fn);

  typeOf = v:
    if v == null
    then "NoneType"
    else if isBool v
    then "bool"
    else if isInt v
    then "int"
    else if isFloat v
    then "float"
    else if isString v
    then "string"
    else if isAttrs v && v ? __sk
    then
      (
        if v.__sk == "list"
        then "list"
        else if v.__sk == "tuple"
        then "tuple"
        else if v.__sk == "dict"
        then "dict"
        else if v.__sk == "function" || v.__sk == "builtin"
        then "function"
        else v.__sk
      )
    else throw "skylark: not a Starlark value: ${builtins.typeOf v}";

  truthy = v:
    if v == null
    then false
    else if isBool v
    then v
    else if isInt v || isFloat v
    then v != 0 && v != 0.0
    else if isString v
    then v != ""
    else if isList v || isTuple v
    then v.items != []
    else if isDict v
    then v.entries != []
    else if isAttrs v && v ? truthy
    then v.truthy
    else true; # host values default truthy

  # Structural equality with Starlark cross-numeric semantics.
  eq = a: b:
    if a == null || b == null
    then a == null && b == null
    else if isBool a || isBool b
    then isBool a && isBool b && a == b
    else if isNum a && isNum b
    then toF a == toF b
    else if isString a && isString b
    then a == b
    else if (isList a && isList b) || (isTuple a && isTuple b) || (isList a && isTuple b) || (isTuple a && isList b)
    then a.__sk == b.__sk && seqEq a.items b.items
    else if isDict a && isDict b
    then dictEq a b
    else if isAttrs a && isAttrs b && a ? id && b ? id
    then a.id == b.id
    else if isAttrs a && isAttrs b && a ? __sk && b ? __sk && a.__sk == "struct" && b.__sk == "struct"
    then dictLikeEq (a.attrs or {}) (b.attrs or {})
    else false;

  seqEq = xs: ys:
    length xs == length ys && builtins.all (i: eq (elemAt xs i) (elemAt ys i)) (builtins.genList (i: i) (length xs));

  dictEq = a: b:
    length a.entries
    == length b.entries
    && builtins.all (e: dictHas b e.key && eq (dictGet b e.key) e.value) a.entries;

  dictLikeEq = a: b: let
    ka = builtins.attrNames a;
    kb = builtins.attrNames b;
  in
    ka == kb && builtins.all (k: eq a.${k} b.${k}) ka;

  # ---- dict operations (ordered entries, linear lookup) -----------------
  dictFindIndex = d: key: let
    n = length d.entries;
    go = i:
      if i >= n
      then -1
      else if eq (elemAt d.entries i).key key
      then i
      else go (i + 1);
  in
    go 0;
  dictHas = d: key: dictFindIndex d key >= 0;
  dictGet = d: key: let
    i = dictFindIndex d key;
  in
    if i < 0
    then throw "KeyError: ${repr key}"
    else (elemAt d.entries i).value;
  dictGetOr = d: key: default: let
    i = dictFindIndex d key;
  in
    if i < 0
    then default
    else (elemAt d.entries i).value;
  dictSet = d: key: value: let
    i = dictFindIndex d key;
  in
    if i < 0
    then mkDict (d.entries ++ [{inherit key value;}])
    else mkDict (replaceAt d.entries i {inherit key value;});
  dictKeys = d: map (e: e.key) d.entries;
  dictValues = d: map (e: e.value) d.entries;

  replaceAt = xs: i: v:
    builtins.genList (j:
      if j == i
      then v
      else elemAt xs j) (length xs);

  # ---- string / repr -----------------------------------------------------
  # Float repr: Nix toString yields a fixed 6-decimal form; trim trailing
  # zeros (keeping at least one) so 3.5 -> "3.5", 3.0 -> "3.0".
  floatRepr = f: let
    s = toString f;
    hasDot = builtins.match ".*\\..*" s != null;
    trimmed =
      if hasDot
      then let
        stripEnd = str: let
          n = builtins.stringLength str;
          go = k:
            if k > 0 && builtins.substring (k - 1) 1 str == "0"
            then go (k - 1)
            else k;
          end = go n;
          e2 =
            if end > 0 && builtins.substring (end - 1) 1 str == "."
            then end + 1
            else end;
        in
          builtins.substring 0 e2 str;
      in
        stripEnd s
      else s;
  in
    trimmed;

  str = v:
    if v == null
    then "None"
    else if isBool v
    then
      (
        if v
        then "True"
        else "False"
      )
    else if isInt v
    then toString v
    else if isFloat v
    then floatRepr v
    else if isString v
    then v
    else if isList v
    then "[" + join ", " (map repr v.items) + "]"
    else if isTuple v
    then
      (
        if length v.items == 1
        then "(" + repr (elemAt v.items 0) + ",)"
        else "(" + join ", " (map repr v.items) + ")"
      )
    else if isDict v
    then "{" + join ", " (map (e: repr e.key + ": " + repr e.value) v.entries) + "}"
    else if isAttrs v && v ? strRepr
    then v.strRepr
    else if isAttrs v && v ? __sk
    then "<${v.__sk}${
      if v ? name
      then " ${v.name}"
      else ""
    }>"
    else "<value>";

  repr = v:
    if isString v
    then "\"" + escapeStr v + "\""
    else str v;

  escapeStr = s:
    builtins.replaceStrings ["\\" "\"" "\n" "\t" "\r"] ["\\\\" "\\\"" "\\n" "\\t" "\\r"] s;

  join = sep: xs:
    if xs == []
    then ""
    else builtins.concatStringsSep sep xs;

  # Three-way comparison for ordering (<, sorted). Throws on unorderable.
  compare = a: b:
    if isNum a && isNum b
    then
      (
        if toF a < toF b
        then -1
        else if toF a > toF b
        then 1
        else 0
      )
    else if isString a && isString b
    then
      (
        if a < b
        then -1
        else if a > b
        then 1
        else 0
      )
    else if isBool a && isBool b
    then
      (
        if a == b
        then 0
        else if a
        then 1
        else -1
      )
    else if isSeq a && isSeq b
    then compareSeq a.items b.items 0
    else throw "unorderable types: ${typeOf a} and ${typeOf b}";

  compareSeq = xs: ys: i:
    if i >= length xs && i >= length ys
    then 0
    else if i >= length xs
    then -1
    else if i >= length ys
    then 1
    else let
      c = compare (elemAt xs i) (elemAt ys i);
    in
      if c != 0
      then c
      else compareSeq xs ys (i + 1);
in {
  inherit
    none
    mkList
    mkTuple
    mkDict
    mkFn
    mkBuiltin
    isNum
    toF
    isList
    isTuple
    isDict
    isSeq
    isCallable
    typeOf
    truthy
    eq
    compare
    dictFindIndex
    dictHas
    dictGet
    dictGetOr
    dictSet
    dictKeys
    dictValues
    replaceAt
    str
    repr
    escapeStr
    floatRepr
    join
    ;
}
