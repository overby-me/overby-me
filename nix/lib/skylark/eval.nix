# Starlark tree-walking evaluator. builtins only.
#
# Parameterized by:
#   V           the value model (values.nix)
#   sk          { globals; getMethod; } from builtins.nix
#   loadModule  currentFile -> loadLabel -> moduleGlobals (attrset name->value)
#   extraGlobals extra globals injected by a host layer (Buck2)
#
# Threads `env` (lexical scope) and an opaque `world` accumulator. Host
# effects (Buck2 target/action registration) live in `world`; the core never
# inspects it. Mutation of local collections is modeled by statement-level
# rebinding (a method call that returns `newSelf` on an lvalue rebinds it).
{
  V,
  sk,
  loadModule ? (_currentFile: _label: throw "skylark: load() is not supported in this configuration"),
  extraGlobals ? {},
}: let
  inherit (builtins) elemAt length head tail;
  inherit
    (V)
    typeOf
    truthy
    eq
    compare
    mkList
    mkTuple
    mkDict
    isList
    isTuple
    isDict
    isSeq
    dictGet
    dictHas
    dictSet
    dictKeys
    replaceAt
    str
    repr
    ;

  ctrlNormal = {t = "normal";};

  baseVars =
    sk.globals
    // extraGlobals
    // {
      True = true;
      False = false;
      None = null;
    };
  defaultBaseEnv = {
    vars = baseVars;
    parent = null;
  };

  lookup = env: name:
    if env == null
    then throw "skylark: name '${name}' is not defined"
    else env.vars.${name} or (lookup env.parent name);

  setVar = env: name: value:
    env // {vars = env.vars // {${name} = value;};};

  # ---- numeric helpers ---------------------------------------------------
  pow = base: exp:
    if exp <= 0
    then 1
    else base * (pow base (exp - 1));
  floorDivInt = a: b: let
    q = a / b; # Nix truncates toward zero
    r = a - q * b;
  in
    if r != 0 && ((r < 0) != (b < 0))
    then q - 1
    else q;
  modInt = a: b: a - b * (floorDivInt a b);

  # ---- iteration ---------------------------------------------------------
  iterate = v:
    if isSeq v
    then v.items
    else if isDict v
    then dictKeys v
    else if builtins.isString v
    then map (i: builtins.substring i 1 v) (builtins.genList (i: i) (builtins.stringLength v))
    else if builtins.isAttrs v && v ? iter
    then v.iter
    else throw "skylark: '${typeOf v}' is not iterable";

  # ---- member access (attributes and methods) ----------------------------
  getMember = objVal: name:
    if builtins.isAttrs objVal && objVal ? attrs && objVal.attrs ? ${name}
    then objVal.attrs.${name}
    else if builtins.isAttrs objVal && objVal ? getAttr
    then objVal.getAttr name
    else let
      m = sk.getMethod (typeOf objVal) objVal name;
    in
      if m != null
      then m
      else throw "skylark: '${typeOf objVal}' value has no attribute or method '${name}'";

  # ---- indexing ----------------------------------------------------------
  normIndex = i: n:
    if i < 0
    then i + n
    else i;
  evalSubscript = objVal: idx:
    if isSeq objVal
    then let
      n = length objVal.items;
      j = normIndex idx n;
    in
      if j < 0 || j >= n
      then throw "IndexError: index ${toString idx} out of range"
      else elemAt objVal.items j
    else if builtins.isString objVal
    then let
      n = builtins.stringLength objVal;
      j = normIndex idx n;
    in
      if j < 0 || j >= n
      then throw "IndexError: string index out of range"
      else builtins.substring j 1 objVal
    else if isDict objVal
    then dictGet objVal idx
    else if builtins.isAttrs objVal && objVal ? subscript
    then objVal.subscript idx
    else throw "skylark: '${typeOf objVal}' is not subscriptable";

  evalSliceVal = objVal: lo: hi: step: let
    items =
      if isSeq objVal
      then objVal.items
      else if builtins.isString objVal
      then map (i: builtins.substring i 1 objVal) (builtins.genList (i: i) (builtins.stringLength objVal))
      else throw "skylark: '${typeOf objVal}' is not sliceable";
    n = length items;
    st =
      if step == null
      then 1
      else step;
    lower =
      if lo == null
      then 0
      else let
        x = normIndex lo n;
      in
        if x < 0
        then 0
        else x;
    upper =
      if hi == null
      then n
      else let
        x = normIndex hi n;
      in
        if x > n
        then n
        else x;
    picked = builtins.filter (i: i >= lower && i < upper && (modInt (i - lower) st == 0)) (builtins.genList (i: i) n);
    result = map (i: elemAt items i) picked;
  in
    if builtins.isString objVal
    then builtins.concatStringsSep "" result
    else if isTuple objVal
    then mkTuple result
    else mkList result;

  # ---- binary operators --------------------------------------------------
  binApply = op: a: b:
    if op == "or" || op == "and"
    then throw "skylark: internal: short-circuit op reached binApply"
    else if op == "+"
    then
      (
        if V.isNum a && V.isNum b
        then a + b
        else if builtins.isString a && builtins.isString b
        then a + b
        else if isList a && isList b
        then mkList (a.items ++ b.items)
        else if isTuple a && isTuple b
        then mkTuple (a.items ++ b.items)
        else if isDict a && isDict b
        then V.mkDict (a.entries ++ (builtins.filter (e: !dictHas a e.key) b.entries))
        else throw "skylark: unsupported operand types for +: ${typeOf a} and ${typeOf b}"
      )
    else if op == "-"
    then numOp a b (x: y: x - y)
    else if op == "*"
    then
      (
        if V.isNum a && V.isNum b
        then a * b
        else if builtins.isString a && builtins.isInt b
        then repeatStr a b
        else if builtins.isInt a && builtins.isString b
        then repeatStr b a
        else if isList a && builtins.isInt b
        then mkList (repeatList a.items b)
        else if builtins.isInt a && isList b
        then mkList (repeatList b.items a)
        else throw "skylark: unsupported operand types for *: ${typeOf a} and ${typeOf b}"
      )
    else if op == "/"
    then V.toF a / V.toF b
    else if op == "//"
    then
      (
        if builtins.isInt a && builtins.isInt b
        then floorDivInt a b
        else throw "skylark: // on non-integers is unsupported"
      )
    else if op == "%"
    then
      (
        if builtins.isString a
        then formatPercent a b
        else if builtins.isInt a && builtins.isInt b
        then modInt a b
        else throw "skylark: %% on ${typeOf a} is unsupported"
      )
    else if op == "**"
    then
      (
        if builtins.isInt a && builtins.isInt b
        then pow a b
        else throw "skylark: ** on non-integers is unsupported"
      )
    else if op == "=="
    then eq a b
    else if op == "!="
    then !(eq a b)
    else if op == "<"
    then compare a b < 0
    else if op == ">"
    then compare a b > 0
    else if op == "<="
    then compare a b <= 0
    else if op == ">="
    then compare a b >= 0
    else if op == "in"
    then contains b a
    else if op == "not in"
    then !(contains b a)
    else if op == "&"
    then builtins.bitAnd a b
    else if op == "|"
    then
      (
        if isDict a && isDict b
        then binApply "+" a b
        else builtins.bitOr a b
      )
    else if op == "^"
    then builtins.bitXor a b
    else if op == "<<"
    then a * (pow 2 b)
    else if op == ">>"
    then floorDivInt a (pow 2 b)
    else throw "skylark: unknown operator '${op}'";

  numOp = a: b: f:
    if V.isNum a && V.isNum b
    then f a b
    else throw "skylark: unsupported numeric operands: ${typeOf a} and ${typeOf b}";

  repeatStr = s: n: let
    go = i: acc:
      if i <= 0
      then acc
      else go (i - 1) (acc + s);
  in
    go n "";
  repeatList = xs: n:
    builtins.concatLists (builtins.genList (_: xs) (
      if n < 0
      then 0
      else n
    ));

  contains = container: item:
    if isSeq container
    then builtins.any (x: eq x item) container.items
    else if isDict container
    then dictHas container item
    else if builtins.isString container && builtins.isString item
    then hasInfix item container
    else throw "skylark: 'in' unsupported for ${typeOf container}";

  hasInfix = needle: hay: let
    nl = builtins.stringLength needle;
    hl = builtins.stringLength hay;
    go = i:
      if i + nl > hl
      then false
      else if builtins.substring i nl hay == needle
      then true
      else go (i + 1);
  in
    if nl == 0
    then true
    else go 0;

  # Minimal printf-style % formatting: %s %r %d %% (single arg or tuple).
  formatPercent = fmt: arg: let
    args =
      if isTuple arg
      then arg.items
      else [arg];
    len = builtins.stringLength fmt;
    go = i: ai: acc:
      if i >= len
      then acc
      else let
        c = builtins.substring i 1 fmt;
      in
        if c == "%" && i + 1 < len
        then let
          spec = builtins.substring (i + 1) 1 fmt;
        in
          if spec == "%"
          then go (i + 2) ai (acc + "%")
          else if spec == "s"
          then go (i + 2) (ai + 1) (acc + str (elemAt args ai))
          else if spec == "r"
          then go (i + 2) (ai + 1) (acc + repr (elemAt args ai))
          else if spec == "d" || spec == "i"
          then go (i + 2) (ai + 1) (acc + toString (elemAt args ai))
          else go (i + 1) ai (acc + c)
        else go (i + 1) ai (acc + c);
  in
    go 0 0 "";

  # ---- argument evaluation ----------------------------------------------
  evalArgs = node: env: world: let
    posBase = foldExprs node.args env world;
    posStar =
      if node.star != null
      then let
        r = evalExpr node.star env posBase.world;
      in {
        items = posBase.items ++ (iterate r.value);
        inherit (r) world;
      }
      else {
        inherit (posBase) items;
        inherit (posBase) world;
      };
    namedBase = foldNamed node.kwargs env posStar.world;
    namedStar =
      if node.dstar != null
      then let
        r = evalExpr node.dstar env namedBase.world;
      in {
        named =
          namedBase.named
          ++ map (e: {
            name = e.key;
            inherit (e) value;
          })
          r.value.entries;
        inherit (r) world;
      }
      else {
        inherit (namedBase) named;
        inherit (namedBase) world;
      };
  in {
    pos = posStar.items;
    inherit (namedStar) named;
    inherit (namedStar) world;
  };

  # foldl' rather than a recursive `go`: Nix has no tail-call elimination, so one
  # frame per element overflows the C stack on a big literal. Buck2 projects have
  # those -- a generated SDK header map is thousands of entries in one dict.
  foldExprs = nodes: env: world:
    builtins.foldl' (acc: node: let
      r = evalExpr node env acc.world;
    in {
      items = acc.items ++ [r.value];
      inherit (r) world;
    }) {
      items = [];
      inherit world;
    }
    nodes;

  # Same reason as foldExprs: iterative, so a call with many keyword arguments
  # cannot exhaust the stack.
  foldNamed = kwargs: env: world:
    builtins.foldl' (acc: kw: let
      r = evalExpr kw.value env acc.world;
    in {
      named =
        acc.named
        ++ [
          {
            inherit (kw) name;
            inherit (r) value;
          }
        ];
      inherit (r) world;
    }) {
      named = [];
      inherit world;
    }
    kwargs;

  findNamed = named: name: let
    go = i:
      if i >= length named
      then null
      else if (elemAt named i).name == name
      then (elemAt named i).value
      else go (i + 1);
  in
    go 0;

  # ---- calling -----------------------------------------------------------
  # Returns { value; world; newSelf?; }.
  apply = fnVal: pos: named: world:
    if !(builtins.isAttrs fnVal && fnVal ? __sk)
    then throw "skylark: '${typeOf fnVal}' object is not callable"
    else if fnVal.__sk == "function"
    then let
      vars = bindArgs fnVal.params pos named fnVal.name;
      frame = {
        inherit vars;
        parent = fnVal.closure;
      };
      r = execStmts "<fn>" fnVal.body frame world;
    in {
      value =
        if r.ctrl.t == "return"
        then r.ctrl.value
        else null;
      inherit (r) world;
    }
    else if fnVal ? fn
    then
      fnVal.fn {
        inherit pos named world;
        call = callValue;
      }
    else throw "skylark: '${typeOf fnVal}' object is not callable";

  callValue = fnVal: pos: named: world: let
    r = apply fnVal pos named world;
  in {
    inherit (r) value world;
  };

  bindArgs = params: pos: named: fnName: let
    go = i: posIdx: acc:
      if i >= length params
      then acc
      else let
        p = elemAt params i;
      in
        if p.kind == "star" && p.name == null
        then go (i + 1) posIdx acc # keyword-only marker
        else if p.kind == "star"
        then go (i + 1) (length pos) (acc // {${p.name} = mkTuple (drop posIdx pos);})
        else if p.kind == "dstar"
        then go (i + 1) posIdx (acc // {${p.name} = mkDict (leftoverNamed params named);})
        else let
          fromNamed = findNamed named p.name;
        in
          if posIdx < length pos
          then go (i + 1) (posIdx + 1) (acc // {${p.name} = elemAt pos posIdx;})
          else if fromNamed != null
          then go (i + 1) posIdx (acc // {${p.name} = fromNamed;})
          else if p.default != null || (p ? hasDefault && p.hasDefault)
          then go (i + 1) posIdx (acc // {${p.name} = p.defaultValue;})
          else throw "skylark: ${fnName}() missing required argument '${p.name}'";
  in
    go 0 0 {};

  drop = n: xs: let
    len = length xs;
  in
    if n >= len
    then []
    else builtins.genList (i: elemAt xs (i + n)) (len - n);

  paramNames = params: map (p: p.name) (builtins.filter (p: p.kind == "normal") params);
  leftoverNamed = params: named: let
    declared = paramNames params;
  in
    map (e: {
      key = e.name;
      inherit (e) value;
    }) (builtins.filter (e: !(builtins.elem e.name declared)) named);

  # ---- expression evaluation --------------------------------------------
  evalExpr = node: env: world: let
    inherit (node) k;
  in
    if k == "num" || k == "str"
    then {
      inherit (node) value;
      inherit world;
    }
    else if k == "name"
    then {
      value = lookup env node.id;
      inherit world;
    }
    else if k == "list"
    then let
      r = foldExprs node.elts env world;
    in {
      value = mkList r.items;
      inherit (r) world;
    }
    else if k == "tuple"
    then let
      r = foldExprs node.elts env world;
    in {
      value = mkTuple r.items;
      inherit (r) world;
    }
    else if k == "dict"
    then evalDictLiteral node env world
    else if k == "listcomp"
    then let
      r = evalComp node.elt null node.clauses env world;
    in {
      value = mkList r.items;
      inherit (r) world;
    }
    else if k == "dictcomp"
    then let
      r = evalComp node.key node.value node.clauses env world;
    in {
      value = mkDict (map (p: {
          key = elemAt p 0;
          value = elemAt p 1;
        })
        r.items);
      inherit (r) world;
    }
    else if k == "unary"
    then let
      r = evalExpr node.operand env world;
      v = r.value;
    in {
      value =
        if node.op == "not"
        then !(truthy v)
        else if node.op == "-"
        then -v
        else if node.op == "+"
        then v
        else if node.op == "~"
        then (-v) - 1
        else throw "skylark: unknown unary '${node.op}'";
      inherit (r) world;
    }
    else if k == "binop"
    then evalBinop node env world
    else if k == "ternary"
    then let
      t = evalExpr node.test env world;
    in
      if truthy t.value
      then evalExpr node.body env t.world
      else evalExpr node.orelse env t.world
    else if k == "call"
    then evalCall node env world
    else if k == "attr"
    then let
      r = evalExpr node.obj env world;
    in {
      value = getMember r.value node.name;
      inherit (r) world;
    }
    else if k == "subscript"
    then let
      o = evalExpr node.obj env world;
      idx = evalExpr node.index env o.world;
    in {
      value = evalSubscript o.value idx.value;
      inherit (idx) world;
    }
    else if k == "slice"
    then let
      o = evalExpr node.obj env world;
      lo =
        if node.lower == null
        then {
          value = null;
          inherit (o) world;
        }
        else evalExpr node.lower env o.world;
      hi =
        if node.upper == null
        then {
          value = null;
          inherit (lo) world;
        }
        else evalExpr node.upper env lo.world;
      stp =
        if node.step == null
        then {
          value = null;
          inherit (hi) world;
        }
        else evalExpr node.step env hi.world;
    in {
      value = evalSliceVal o.value lo.value hi.value stp.value;
      inherit (stp) world;
    }
    else if k == "lambda"
    then {
      value = mkLambda node env world;
      inherit world;
    }
    else throw "skylark: cannot evaluate node '${k}'";

  # A dict LITERAL is built iteratively, and string keys (the overwhelmingly common
  # case, and the only one a generated header map uses) dedup through an attrset
  # index rather than dictSetEntries' linear scan. Recursion here overflowed the
  # stack on a 4000-entry literal, and the linear scan made it O(n^2) besides.
  evalDictLiteral = node: env: world: let
    step = acc: e: let
      kr = evalExpr e.key env acc.world;
      vr = evalExpr e.value env kr.world;
      k = kr.value;
      isStrKey = builtins.isString k;
    in
      if isStrKey && !(acc.idx ? ${k})
      then {
        entries =
          acc.entries
          ++ [
            {
              key = k;
              inherit (vr) value;
            }
          ];
        idx = acc.idx // {${k} = true;};
        inherit (vr) world;
      }
      else {
        entries = dictSetEntries acc.entries k vr.value;
        inherit (acc) idx;
        inherit (vr) world;
      };
    r =
      builtins.foldl' step {
        entries = [];
        idx = {};
        inherit world;
      }
      node.entries;
  in {
    value = mkDict r.entries;
    inherit (r) world;
  };

  dictSetEntries = entries: key: value: let
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
    else replaceAt entries idx {inherit key value;};

  evalBinop = node: env: world:
    if node.op == "and"
    then let
      l = evalExpr node.left env world;
    in
      if !(truthy l.value)
      then l
      else evalExpr node.right env l.world
    else if node.op == "or"
    then let
      l = evalExpr node.left env world;
    in
      if truthy l.value
      then l
      else evalExpr node.right env l.world
    else let
      l = evalExpr node.left env world;
      r = evalExpr node.right env l.world;
    in {
      value = binApply node.op l.value r.value;
      inherit (r) world;
    };

  evalCall = node: env: world:
    if node.func.k == "attr"
    then let
      objR = evalExpr node.func.obj env world;
      member = getMember objR.value node.func.name;
      a = evalArgs node env objR.world;
      r = apply member a.pos a.named a.world;
    in {
      inherit (r) value world;
    }
    else let
      fnR = evalExpr node.func env world;
      a = evalArgs node env fnR.world;
      r = apply fnR.value a.pos a.named a.world;
    in {
      inherit (r) value world;
    };

  mkLambda = node: env: world: {
    __sk = "function";
    name = "lambda";
    params = precomputeDefaults node.params env world;
    body = [
      {
        k = "return";
        value = node.body;
      }
    ];
    closure = env;
  };

  precomputeDefaults = params: env: world:
    map (
      p:
        if p.kind == "normal" && (p.default or null) != null
        then
          p
          // {
            hasDefault = true;
            defaultValue = (evalExpr p.default env world).value;
          }
        else p
    )
    params;

  # ---- comprehensions ----------------------------------------------------
  # keyNode/valNode: for listcomp valNode is null and each result is one item;
  # for dictcomp each result is a [key value] pair.
  evalComp = keyNode: valNode: clauses: env: world: let
    go = cls: e: w: acc:
      if cls == []
      then
        (
          if valNode == null
          then let
            r = evalExpr keyNode e w;
          in {
            items = acc ++ [r.value];
            inherit (r) world;
          }
          else let
            kr = evalExpr keyNode e w;
            vr = evalExpr valNode e kr.world;
          in {
            items = acc ++ [[kr.value vr.value]];
            inherit (vr) world;
          }
        )
      else let
        clause = head cls;
        rest = tail cls;
      in
        if clause.k == "for"
        then let
          iterR = evalExpr clause.iter e w;
          items = iterate iterR.value;
          # foldl' for the same reason as execFor: a frame per ITEM overflows the C stack,
          # and a comprehension is how a BUCK file declares one target per entry of a
          # generated map (xnu exports 1,252 files that way).
          step = accum: item: let
            e2 = (assignLvalue clause.targets item e accum.world).env;
            sub = go rest e2 accum.world accum.items;
          in
            # Force the accumulated list every step. foldl' is strict only in the
            # accumulator's outermost value, so a lazy `acc ++ [x]` chain survives it and
            # is still N thunks deep when something finally forces it -- the same C-stack
            # overflow, one step later and in a confusing place (dict.items()).
            builtins.seq (length sub.items) sub;
        in
          builtins.foldl' step {
            world = iterR.world;
            items = acc;
          }
          items
        else let
          testR = evalExpr clause.test e w;
        in
          if truthy testR.value
          then go rest e testR.world acc
          else {
            items = acc;
            inherit (testR) world;
          };
  in
    go clauses env world [];

  # ---- assignment --------------------------------------------------------
  assignLvalue = target: value: env: world:
    if target.k == "name"
    then {
      env = setVar env target.id value;
      inherit world;
    }
    else if target.k == "tuple"
    then let
      items = iterate value;
      n = length target.elts;
    in
      if length items != n
      then throw "skylark: cannot unpack ${toString (length items)} values into ${toString n} targets"
      else let
        go = i: e: w:
          if i >= n
          then {
            env = e;
            world = w;
          }
          else let
            r = assignLvalue (elemAt target.elts i) (elemAt items i) e w;
          in
            go (i + 1) r.env r.world;
      in
        go 0 env world
    else if target.k == "subscript"
    then let
      idxR = evalExpr target.index env world;
      contR = evalExpr target.obj env idxR.world;
      newCont = setIndexVal contR.value idxR.value value;
    in
      assignLvalue target.obj newCont env contR.world
    else if target.k == "attr"
    then let
      objR = evalExpr target.obj env world;
      newObj = objR.value // {attrs = (objR.value.attrs or {}) // {${target.name} = value;};};
    in
      assignLvalue target.obj newObj env objR.world
    else throw "skylark: cannot assign to ${target.k}";

  setIndexVal = container: idx: value:
    if isList container
    then let
      n = length container.items;
      j = normIndex idx n;
    in
      mkList (replaceAt container.items j value)
    else if isDict container
    then dictSet container idx value
    else throw "skylark: '${typeOf container}' does not support item assignment";

  isLvalue = node: builtins.elem node.k ["name" "subscript" "attr" "tuple"];

  augToBin = op: builtins.substring 0 (builtins.stringLength op - 1) op;

  # ---- statements --------------------------------------------------------
  execStmts = currentFile: stmts: env: world: let
    go = i: e: w:
      if i >= length stmts
      then {
        env = e;
        world = w;
        ctrl = ctrlNormal;
      }
      else let
        r = execStmt currentFile (elemAt stmts i) e w;
      in
        if r.ctrl.t == "normal"
        then go (i + 1) r.env r.world
        else r;
  in
    go 0 env world;

  execStmt = currentFile: stmt: env: world: let
    inherit (stmt) k;
  in
    if k == "expr_stmt"
    then execExprStmt stmt env world
    else if k == "assign"
    then let
      r = evalExpr stmt.value env world;
      a = assignLvalue stmt.target r.value env r.world;
    in {
      inherit (a) env world;
      ctrl = ctrlNormal;
    }
    else if k == "augassign"
    then let
      cur = evalExpr stmt.target env world;
      rhs = evalExpr stmt.value env cur.world;
      newVal = binApply (augToBin stmt.op) cur.value rhs.value;
      a = assignLvalue stmt.target newVal env rhs.world;
    in {
      inherit (a) env world;
      ctrl = ctrlNormal;
    }
    else if k == "def"
    then let
      fnVal = {
        __sk = "function";
        inherit (stmt) name;
        params = precomputeDefaults stmt.params env world;
        inherit (stmt) body;
        closure = env;
      };
    in {
      env = setVar env stmt.name fnVal;
      inherit world;
      ctrl = ctrlNormal;
    }
    else if k == "return"
    then
      if stmt.value == null
      then {
        inherit env world;
        ctrl = {
          t = "return";
          value = null;
        };
      }
      else let
        r = evalExpr stmt.value env world;
      in {
        inherit env;
        inherit (r) world;
        ctrl = {
          t = "return";
          inherit (r) value;
        };
      }
    else if k == "pass"
    then {
      inherit env world;
      ctrl = ctrlNormal;
    }
    else if k == "break"
    then {
      inherit env world;
      ctrl = {t = "break";};
    }
    else if k == "continue"
    then {
      inherit env world;
      ctrl = {t = "continue";};
    }
    else if k == "if"
    then execIf currentFile stmt env world
    else if k == "for"
    then execFor currentFile stmt env world
    else if k == "load"
    then execLoad currentFile stmt env world
    else throw "skylark: cannot execute statement '${k}'";

  execExprStmt = stmt: env: world: let
    e = stmt.expr;
  in
    if e.k == "call" && e.func.k == "attr" && isLvalue e.func.obj
    then let
      objR = evalExpr e.func.obj env world;
      member = getMember objR.value e.func.name;
      a = evalArgs e env objR.world;
      r = apply member a.pos a.named a.world;
      env' =
        if r ? newSelf
        then (assignLvalue e.func.obj r.newSelf env r.world).env
        else env;
    in {
      env = env';
      inherit (r) world;
      ctrl = ctrlNormal;
    }
    else let
      r = evalExpr e env world;
    in {
      inherit env;
      inherit (r) world;
      ctrl = ctrlNormal;
    };

  execIf = currentFile: stmt: env: world: let
    go = branches: w:
      if branches == []
      then execStmts currentFile stmt.orelse env w
      else let
        b = head branches;
        t = evalExpr b.test env w;
      in
        if truthy t.value
        then execStmts currentFile b.body env t.world
        else go (tail branches) t.world;
  in
    go stmt.branches world;

  execFor = currentFile: stmt: env: world: let
    iterR = evalExpr stmt.iter env world;
    items = iterate iterR.value;
    # foldl', not recursion: Nix has no tail-call elimination, so a frame per ITERATION
    # overflows the C stack on a real loop -- Darling's BUCK files iterate the generated
    # SDK maps, 4,178 entries, and that is `max-call-depth exceeded` rather than a slow
    # evaluation. A fold cannot stop early, so `done` carries break/return and the
    # remaining items fall through untouched.
    step = acc: item:
      if acc.done
      then acc
      else let
        e1 = (assignLvalue stmt.targets item acc.env acc.world).env;
        r = execStmts currentFile stmt.body e1 acc.world;
      in
        if r.ctrl.t == "return"
        then {
          inherit (r) env world ctrl;
          done = true;
        }
        else if r.ctrl.t == "break"
        then {
          inherit (r) env world;
          ctrl = ctrlNormal;
          done = true;
        }
        else
          # Same reason as the comprehension fold: the env an assignment produces is an
          # attrset UPDATE, and a chain of those is as deep as the loop unless each step
          # collapses it.
          builtins.seq r.env (builtins.seq r.world {
            inherit (r) env world;
            ctrl = ctrlNormal;
            done = false;
          });
    final =
      builtins.foldl' step {
        inherit env;
        world = iterR.world;
        ctrl = ctrlNormal;
        done = false;
      }
      items;
  in {
    inherit (final) env world ctrl;
  };

  execLoad = currentFile: stmt: env: world: let
    modGlobals = loadModule currentFile stmt.module;
    bound =
      builtins.foldl' (
        e: sym:
          if modGlobals ? ${sym.from}
          then setVar e sym.name modGlobals.${sym.from}
          else throw "skylark: load: '${sym.from}' not found in ${stmt.module}"
      )
      env
      stmt.symbols;
  in {
    env = bound;
    inherit world;
    ctrl = ctrlNormal;
  };

  # ---- module driver -----------------------------------------------------
  # Function closures resolve module-level names against the FINAL module
  # globals (so a rule impl may reference a helper defined later in the
  # file). Achieved by chaining the module frame's parent to a lazy env whose
  # vars are the finished globals; builtins sit before it so ordinary lookups
  # never force the fixpoint mid-evaluation.
  evalModule = currentFile: ast: world: let
    lateEnv = {
      vars = res.env.vars;
      parent = null;
    };
    withLate = {
      vars = baseVars;
      parent = lateEnv;
    };
    moduleFrame = {
      vars = {};
      parent = withLate;
    };
    res = execStmts currentFile ast.body moduleFrame world;
  in {
    globals = res.env.vars;
    inherit (res) world;
  };

  evalExprTop = node: (evalExpr node defaultBaseEnv null).value;
in {
  inherit evalModule callValue apply evalExpr;
  # Evaluate a single expression AST in the base environment (for tests).
  eval = evalExprTop;
  baseEnv = defaultBaseEnv;
}
