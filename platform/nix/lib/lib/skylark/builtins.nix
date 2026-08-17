# Standard Starlark builtins and type methods. builtins only.
#
# Returns { globals; getMethod; }:
#   globals   attrset name -> builtin value (len, range, sorted, fail, ...)
#   getMethod typeName -> self -> methodName -> callable-or-null
#
# A builtin/method value is { __sk = "builtin"; name; fn; }. `fn` receives
# { pos; named; world; call; } and returns { value; world; newSelf?; }.
# `newSelf` marks an in-place mutation (list.append, dict.update, ...) which
# the evaluator rebinds onto the receiver lvalue. `call` invokes a callable
# (for higher-order builtins like sorted(key=...)).
{V}: let
  inherit (builtins) elemAt length substring stringLength;
  inherit
    (V)
    mkList
    mkTuple
    mkDict
    isDict
    isSeq
    typeOf
    truthy
    eq
    compare
    str
    repr
    dictGet
    dictHas
    dictSet
    dictKeys
    dictValues
    ;

  mk = name: fn: {
    __sk = "builtin";
    inherit name fn;
  };
  # Pure builtin: value computed from (pos, named); world passes through.
  pb = name: g:
    mk name ({
      pos,
      named,
      world,
      ...
    }: {
      value = g pos named;
      inherit world;
    });

  argAt = pos: i: default:
    if i < length pos
    then elemAt pos i
    else default;
  kwAt = named: nm: default: let
    go = i:
      if i >= length named
      then default
      else if (elemAt named i).name == nm
      then (elemAt named i).value
      else go (i + 1);
  in
    go 0;
  posOrKw = pos: named: i: nm: default:
    if i < length pos
    then elemAt pos i
    else kwAt named nm default;

  iterItems = v:
    if isSeq v
    then v.items
    else if isDict v
    then dictKeys v
    else if builtins.isString v
    then chars v
    else throw "skylark: '${typeOf v}' is not iterable";
  chars = s: map (i: substring i 1 s) (builtins.genList (i: i) (stringLength s));

  # ---- string helpers ----------------------------------------------------
  splitStr = sep: s: let
    sl = stringLength sep;
    L = stringLength s;
    go = i: start: acc:
      if i + sl > L
      then acc ++ [(substring start (L - start) s)]
      else if substring i sl s == sep
      then go (i + sl) (i + sl) (acc ++ [(substring start (i - start) s)])
      else go (i + 1) start acc;
  in
    if sl == 0
    then throw "empty separator"
    else go 0 0 [];

  isWs = c: c == " " || c == "\t" || c == "\n" || c == "\r";
  splitWs = s: let
    L = stringLength s;
    go = i: start: inTok: acc:
      if i >= L
      then
        (
          if inTok
          then acc ++ [(substring start (i - start) s)]
          else acc
        )
      else let
        c = substring i 1 s;
      in
        if isWs c
        then
          (
            if inTok
            then go (i + 1) 0 false (acc ++ [(substring start (i - start) s)])
            else go (i + 1) 0 false acc
          )
        else
          (
            if inTok
            then go (i + 1) start true acc
            else go (i + 1) i true acc
          );
  in
    go 0 0 false [];

  stripChars = chs: s: let
    L = stringLength s;
    inSet = c:
      if chs == null
      then isWs c
      else hasChar chs c;
    l = let
      go = i:
        if i < L && inSet (substring i 1 s)
        then go (i + 1)
        else i;
    in
      go 0;
    r = let
      go = i:
        if i > l && inSet (substring (i - 1) 1 s)
        then go (i - 1)
        else i;
    in
      go L;
  in
    substring l (r - l) s;
  hasChar = set: c: let
    n = stringLength set;
    go = i:
      if i >= n
      then false
      else if substring i 1 set == c
      then true
      else go (i + 1);
  in
    go 0;

  hasPrefix = p: s: substring 0 (stringLength p) s == p;
  hasSuffix = suf: s: let
    sl = stringLength s;
    fl = stringLength suf;
  in
    fl <= sl && substring (sl - fl) fl s == suf;
  findSub = sub: s: from: let
    sl = stringLength sub;
    L = stringLength s;
    go = i:
      if i + sl > L
      then -1
      else if substring i sl s == sub
      then i
      else go (i + 1);
  in
    go (
      if from < 0
      then 0
      else from
    );
  countSub = sub: s: let
    sl = stringLength sub;
    go = i: n: let
      j = findSub sub s i;
    in
      if j < 0
      then n
      else go (j + sl) (n + 1);
  in
    if sl == 0
    then 0
    else go 0 0;

  # str.format: {} auto, {n} indexed, {name} keyword; {{ }} escapes; a
  # trailing :spec is ignored.
  formatStr = fmt: pos: named: let
    L = stringLength fmt;
    fieldName = body: let
      colon = findSub ":" body 0;
      key =
        if colon < 0
        then body
        else substring 0 colon body;
    in
      key;
    go = i: auto: acc:
      if i >= L
      then acc
      else let
        c = substring i 1 fmt;
        c2 =
          if i + 1 < L
          then substring (i + 1) 1 fmt
          else "";
      in
        if c == "{" && c2 == "{"
        then go (i + 2) auto (acc + "{")
        else if c == "}" && c2 == "}"
        then go (i + 2) auto (acc + "}")
        else if c == "{"
        then let
          close = findSub "}" fmt i;
          body = substring (i + 1) (close - i - 1) fmt;
          key = fieldName body;
          value =
            if key == ""
            then str (argAt pos auto null)
            else if isDigit key
            then str (argAt pos (toIntStr key) null)
            else str (kwAt named key null);
          nextAuto =
            if key == ""
            then auto + 1
            else auto;
        in
          go (close + 1) nextAuto (acc + value)
        else go (i + 1) auto (acc + c);
  in
    go 0 0 "";
  isDigit = s: let
    n = stringLength s;
    go = i:
      if i >= n
      then n > 0
      else if let c = substring i 1 s; in c >= "0" && c <= "9"
      then go (i + 1)
      else false;
  in
    go 0;
  toIntStr = s: builtins.fromJSON s;

  replaceAll = old: new: s: builtins.replaceStrings [old] [new] s;
  replaceN = old: new: cnt: s: let
    ol = stringLength old;
    go = i: n: acc: let
      j = findSub old s i;
    in
      if j < 0 || n >= cnt
      then acc + substring i (stringLength s - i) s
      else go (j + ol) (n + 1) (acc + substring i (j - i) s + new);
  in
    if cnt < 0
    then replaceAll old new s
    else go 0 0 "";

  toUpper = s: builtins.replaceStrings (chars "abcdefghijklmnopqrstuvwxyz") (chars "ABCDEFGHIJKLMNOPQRSTUVWXYZ") s;
  toLower = s: builtins.replaceStrings (chars "ABCDEFGHIJKLMNOPQRSTUVWXYZ") (chars "abcdefghijklmnopqrstuvwxyz") s;

  # ---- globals -----------------------------------------------------------
  lenOf = v:
    if isSeq v
    then length v.items
    else if isDict v
    then length v.entries
    else if builtins.isString v
    then stringLength v
    else throw "skylark: object of type '${typeOf v}' has no len()";

  rangeFn = pos: let
    a = argAt pos 0 0;
    hasStop = length pos >= 2;
    start =
      if hasStop
      then a
      else 0;
    stop =
      if hasStop
      then elemAt pos 1
      else a;
    step = argAt pos 2 1;
    count =
      if step > 0
      then
        (
          if stop > start
          then (stop - start + step - 1) / step
          else 0
        )
      else
        (
          if start > stop
          then (start - stop + (-step) - 1) / (-step)
          else 0
        );
  in
    mkList (builtins.genList (i: start + i * step) (
      if count < 0
      then 0
      else count
    ));

  toIntVal = v:
    if builtins.isInt v
    then v
    else if builtins.isBool v
    then
      (
        if v
        then 1
        else 0
      )
    else if builtins.isString v
    then builtins.fromJSON (stripChars null v)
    else throw "skylark: int() argument must be an int, bool, or string";

  sortFn = pos: named: call: world: let
    xs = iterItems (argAt pos 0 (mkList []));
    keyFn = posOrKw pos named 1 "key" null;
    reverse = truthy (kwAt named "reverse" false);
    keyed =
      if keyFn == null
      then
        map (x: {
          k = x;
          v = x;
        })
        xs
      else
        map (x: {
          k = (call keyFn [x] [] world).value;
          v = x;
        })
        xs;
    sorted = builtins.sort (a: b: compare a.k b.k < 0) keyed;
    vals = map (p: p.v) sorted;
  in
    mkList (
      if reverse
      then reverseList vals
      else vals
    );
  reverseList = xs: let
    n = length xs;
  in
    builtins.genList (i: elemAt xs (n - 1 - i)) n;

  reduceCmp = pos: named: call: world: pick: let
    items =
      if length pos == 1
      then iterItems (elemAt pos 0)
      else pos;
    keyFn = kwAt named "key" null;
    keyOf = x:
      if keyFn == null
      then x
      else (call keyFn [x] [] world).value;
    go = i: best:
      if i >= length items
      then best
      else let
        x = elemAt items i;
      in
        if pick (compare (keyOf x) (keyOf best))
        then go (i + 1) x
        else go (i + 1) best;
  in
    if items == []
    then
      throw "skylark: ${
        if pick 1
        then "max"
        else "min"
      }() arg is an empty sequence"
    else go 1 (elemAt items 0);

  globals = {
    len = pb "len" (pos: _: lenOf (elemAt pos 0));
    range = pb "range" (pos: _: rangeFn pos);
    list = pb "list" (pos: _:
      if pos == []
      then mkList []
      else mkList (iterItems (elemAt pos 0)));
    tuple = pb "tuple" (pos: _:
      if pos == []
      then mkTuple []
      else mkTuple (iterItems (elemAt pos 0)));
    dict = pb "dict" (pos: named: let
      base =
        if pos == []
        then []
        else let
          a = elemAt pos 0;
        in
          if isDict a
          then a.entries
          else
            map (p: let
              it = iterItems p;
            in {
              key = elemAt it 0;
              value = elemAt it 1;
            }) (iterItems a);
      withNamed = builtins.foldl' (acc: e: dictSetE acc e.name e.value) base named;
    in
      mkDict withNamed);
    str = pb "str" (pos: _: str (argAt pos 0 null));
    repr = pb "repr" (pos: _: repr (argAt pos 0 null));
    bool = pb "bool" (pos: _: truthy (argAt pos 0 false));
    int = pb "int" (pos: _: toIntVal (argAt pos 0 0));
    float = pb "float" (pos: _:
      V.toF (
        let
          v = argAt pos 0 0;
        in
          if builtins.isString v
          then builtins.fromJSON v
          else v
      ));
    type = pb "type" (pos: _: typeOf (elemAt pos 0));
    enumerate = pb "enumerate" (pos: _: let
      xs = iterItems (elemAt pos 0);
      start = argAt pos 1 0;
    in
      mkList (builtins.genList (i: mkTuple [(start + i) (elemAt xs i)]) (length xs)));
    reversed = pb "reversed" (pos: _: mkList (reverseList (iterItems (elemAt pos 0))));
    zip = pb "zip" (pos: _: let
      lists = map iterItems pos;
      n =
        if lists == []
        then 0
        else
          builtins.foldl' (a: l:
            if length l < a
            then length l
            else a) (length (elemAt lists 0))
          lists;
    in
      mkList (builtins.genList (i: mkTuple (map (l: elemAt l i) lists)) n));
    sum = pb "sum" (pos: _: let
      xs = iterItems (elemAt pos 0);
      start = argAt pos 1 0;
    in
      builtins.foldl' (a: b: a + b) start xs);
    any = pb "any" (pos: _: builtins.any truthy (iterItems (elemAt pos 0)));
    all = pb "all" (pos: _: builtins.all truthy (iterItems (elemAt pos 0)));
    sorted = mk "sorted" ({
      pos,
      named,
      world,
      call,
    }: {
      value = sortFn pos named call world;
      inherit world;
    });
    min = mk "min" ({
      pos,
      named,
      world,
      call,
    }: {
      value = reduceCmp pos named call world (c: c < 0);
      inherit world;
    });
    max = mk "max" ({
      pos,
      named,
      world,
      call,
    }: {
      value = reduceCmp pos named call world (c: c > 0);
      inherit world;
    });
    print = mk "print" ({world, ...}: {
      value = null;
      inherit world;
    });
    fail = pb "fail" (pos: named: let
      sep = kwAt named "sep" " ";
      msg = builtins.concatStringsSep sep (map str pos);
    in
      throw "fail: ${msg}");
    hasattr = pb "hasattr" (pos: _: let
      o = elemAt pos 0;
      name = elemAt pos 1;
    in
      (builtins.isAttrs o && o ? attrs && o.attrs ? ${name})
      || (getMethod (typeOf o) o name != null));
    getattr = pb "getattr" (pos: _: let
      o = elemAt pos 0;
      name = elemAt pos 1;
      hasDefault = length pos >= 3;
      default = argAt pos 2 null;
    in
      if builtins.isAttrs o && o ? attrs && o.attrs ? ${name}
      then o.attrs.${name}
      else let
        m = getMethod (typeOf o) o name;
      in
        if m != null
        then m
        else if hasDefault
        then default
        else throw "skylark: '${typeOf o}' has no attribute '${name}'");
  };

  dictSetE = entries: key: value: let
    n = length entries;
    idx = let
      g = i:
        if i >= n
        then -1
        else if eq (elemAt entries i).key key
        then i
        else g (i + 1);
    in
      g 0;
  in
    if idx < 0
    then entries ++ [{inherit key value;}]
    else V.replaceAt entries idx {inherit key value;};

  # ---- methods -----------------------------------------------------------
  pm = self: name: g:
    mk name ({
      pos,
      named,
      world,
      ...
    }: {
      value = g self pos named;
      inherit world;
    });
  mm = self: name: g:
    mk name ({
      pos,
      named,
      world,
      ...
    }: let
      r = g self pos named;
    in {
      inherit (r) value newSelf;
      inherit world;
    });

  strMethod = self: name:
    if name == "format"
    then
      mk "format" ({
        pos,
        named,
        world,
        ...
      }: {
        value = formatStr self pos named;
        inherit world;
      })
    else if name == "join"
    then pm self "join" (s: pos: _: builtins.concatStringsSep s (map str (iterItems (elemAt pos 0))))
    else if name == "split"
    then
      pm self "split" (s: pos: _: let
        sep = argAt pos 0 null;
      in
        mkList (map (x: x) (
          if sep == null
          then splitWs s
          else splitStr sep s
        )))
    else if name == "rsplit"
    then
      pm self "rsplit" (s: pos: _: let
        sep = argAt pos 0 null;
      in
        mkList (
          if sep == null
          then splitWs s
          else splitStr sep s
        ))
    else if name == "splitlines"
    then pm self "splitlines" (s: _: _: mkList (splitStr "\n" s))
    else if name == "startswith"
    then
      pm self "startswith" (s: pos: _: let
        p = elemAt pos 0;
      in
        if isSeq p
        then builtins.any (x: hasPrefix x s) p.items
        else hasPrefix p s)
    else if name == "endswith"
    then
      pm self "endswith" (s: pos: _: let
        p = elemAt pos 0;
      in
        if isSeq p
        then builtins.any (x: hasSuffix x s) p.items
        else hasSuffix p s)
    else if name == "strip"
    then pm self "strip" (s: pos: _: stripChars (argAt pos 0 null) s)
    else if name == "lstrip"
    then pm self "lstrip" (s: pos: _: lstripChars (argAt pos 0 null) s)
    else if name == "rstrip"
    then pm self "rstrip" (s: pos: _: rstripChars (argAt pos 0 null) s)
    else if name == "replace"
    then pm self "replace" (s: pos: _: replaceN (elemAt pos 0) (elemAt pos 1) (argAt pos 2 (-1)) s)
    else if name == "upper"
    then pm self "upper" (s: _: _: toUpper s)
    else if name == "lower"
    then pm self "lower" (s: _: _: toLower s)
    else if name == "capitalize"
    then
      pm self "capitalize" (s: _: _:
        if s == ""
        then ""
        else toUpper (substring 0 1 s) + toLower (substring 1 (stringLength s - 1) s))
    else if name == "find"
    then pm self "find" (s: pos: _: findSub (elemAt pos 0) s (argAt pos 1 0))
    else if name == "rfind"
    then pm self "rfind" (s: pos: _: rfindSub (elemAt pos 0) s)
    else if name == "index"
    then
      pm self "index" (s: pos: _: let
        j = findSub (elemAt pos 0) s (argAt pos 1 0);
      in
        if j < 0
        then throw "substring not found"
        else j)
    else if name == "count"
    then pm self "count" (s: pos: _: countSub (elemAt pos 0) s)
    else if name == "removeprefix"
    then
      pm self "removeprefix" (s: pos: _: let
        p = elemAt pos 0;
      in
        if hasPrefix p s
        then substring (stringLength p) (stringLength s - stringLength p) s
        else s)
    else if name == "removesuffix"
    then
      pm self "removesuffix" (s: pos: _: let
        p = elemAt pos 0;
      in
        if hasSuffix p s
        then substring 0 (stringLength s - stringLength p) s
        else s)
    else if name == "elems"
    then pm self "elems" (s: _: _: mkList (chars s))
    else null;

  lstripChars = chs: s: let
    L = stringLength s;
    inSet = c:
      if chs == null
      then isWs c
      else hasChar chs c;
    go = i:
      if i < L && inSet (substring i 1 s)
      then go (i + 1)
      else i;
    l = go 0;
  in
    substring l (L - l) s;
  rstripChars = chs: s: let
    L = stringLength s;
    inSet = c:
      if chs == null
      then isWs c
      else hasChar chs c;
    go = i:
      if i > 0 && inSet (substring (i - 1) 1 s)
      then go (i - 1)
      else i;
    r = go L;
  in
    substring 0 r s;
  rfindSub = sub: s: let
    sl = stringLength sub;
    L = stringLength s;
    go = i:
      if i < 0
      then -1
      else if substring i sl s == sub
      then i
      else go (i - 1);
  in
    go (L - sl);

  listMethod = self: name:
    if name == "append"
    then
      mm self "append" (s: pos: _: {
        value = null;
        newSelf = mkList (s.items ++ [(elemAt pos 0)]);
      })
    else if name == "extend"
    then
      mm self "extend" (s: pos: _: {
        value = null;
        newSelf = mkList (s.items ++ iterItems (elemAt pos 0));
      })
    else if name == "insert"
    then
      mm self "insert" (s: pos: _: let
        i = elemAt pos 0;
        x = elemAt pos 1;
        n = length s.items;
        j =
          if i < 0
          then 0
          else if i > n
          then n
          else i;
      in {
        value = null;
        newSelf = mkList (take j s.items ++ [x] ++ drop2 j s.items);
      })
    else if name == "pop"
    then
      mm self "pop" (s: pos: _: let
        n = length s.items;
        i = argAt pos 0 (n - 1);
        j =
          if i < 0
          then i + n
          else i;
      in {
        value = elemAt s.items j;
        newSelf = mkList (take j s.items ++ drop2 (j + 1) s.items);
      })
    else if name == "remove"
    then
      mm self "remove" (s: pos: _: let
        x = elemAt pos 0;
        idx = firstIndex s.items x;
      in {
        value = null;
        newSelf = mkList (take idx s.items ++ drop2 (idx + 1) s.items);
      })
    else if name == "clear"
    then
      mm self "clear" (_: _: _: {
        value = null;
        newSelf = mkList [];
      })
    else if name == "index"
    then
      pm self "index" (s: pos: _: let
        idx = firstIndex s.items (elemAt pos 0);
      in
        if idx < 0
        then throw "ValueError: item not in list"
        else idx)
    else if name == "count"
    then pm self "count" (s: pos: _: length (builtins.filter (x: eq x (elemAt pos 0)) s.items))
    else null;

  take = n: xs:
    if n <= 0
    then []
    else
      builtins.genList (i: elemAt xs i) (
        if n > length xs
        then length xs
        else n
      );
  drop2 = n: xs: let
    len = length xs;
  in
    if n >= len
    then []
    else builtins.genList (i: elemAt xs (i + n)) (len - n);
  firstIndex = xs: x: let
    n = length xs;
    go = i:
      if i >= n
      then -1
      else if eq (elemAt xs i) x
      then i
      else go (i + 1);
  in
    go 0;

  dictMethod = self: name:
    if name == "get"
    then pm self "get" (d: pos: _: dictGetOr' d (elemAt pos 0) (argAt pos 1 null))
    else if name == "keys"
    then pm self "keys" (d: _: _: mkList (dictKeys d))
    else if name == "values"
    then pm self "values" (d: _: _: mkList (dictValues d))
    else if name == "items"
    then pm self "items" (d: _: _: mkList (map (e: mkTuple [e.key e.value]) d.entries))
    else if name == "update"
    then
      mm self "update" (d: pos: named: let
        fromPos =
          if pos == []
          then []
          else let
            a = elemAt pos 0;
          in
            if isDict a
            then a.entries
            else
              map (p: let
                it = iterItems p;
              in {
                key = elemAt it 0;
                value = elemAt it 1;
              }) (iterItems a);
        merged = builtins.foldl' (acc: e: dictSetE acc e.key e.value) d.entries fromPos;
        withNamed = builtins.foldl' (acc: e: dictSetE acc e.name e.value) merged named;
      in {
        value = null;
        newSelf = mkDict withNamed;
      })
    else if name == "setdefault"
    then
      mm self "setdefault" (d: pos: _: let
        k = elemAt pos 0;
        default = argAt pos 1 null;
      in
        if dictHas d k
        then {
          value = dictGet d k;
          newSelf = d;
        }
        else {
          value = default;
          newSelf = dictSet d k default;
        })
    else if name == "pop"
    then
      mm self "pop" (d: pos: _: let
        k = elemAt pos 0;
        hasDefault = length pos >= 2;
      in
        if dictHas d k
        then {
          value = dictGet d k;
          newSelf = mkDict (builtins.filter (e: !eq e.key k) d.entries);
        }
        else if hasDefault
        then {
          value = elemAt pos 1;
          newSelf = d;
        }
        else throw "KeyError: ${repr k}")
    else if name == "popitem"
    then
      mm self "popitem" (d: _: _: let
        last = elemAt d.entries (length d.entries - 1);
      in {
        value = mkTuple [last.key last.value];
        newSelf = mkDict (take (length d.entries - 1) d.entries);
      })
    else if name == "clear"
    then
      mm self "clear" (_: _: _: {
        value = null;
        newSelf = mkDict [];
      })
    else null;
  dictGetOr' = d: k: default:
    if dictHas d k
    then dictGet d k
    else default;

  getMethod = tname: self: name:
    if tname == "string"
    then strMethod self name
    else if tname == "list" || tname == "tuple"
    then listMethod self name
    else if tname == "dict"
    then dictMethod self name
    else null;
in {
  inherit globals getMethod;
}
