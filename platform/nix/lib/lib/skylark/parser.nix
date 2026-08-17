# Starlark parser: token list -> AST. builtins only.
#
# Recursive descent; expressions via precedence climbing. Each parse function
# has the shape `toks: i: { node; pos; }`, threading the token index like the
# yaml parser threads string indices. Type annotations on params and returns
# are parsed and discarded (Starlark treats them as documentation).
#
# AST nodes are tagged attrsets with a `k` field. See parseStatement /
# parseAtom for the full set.
let
  tokenize = import ./lexer.nix;
  inherit (builtins) elemAt length elem;

  tok = toks: i:
    if i < length toks
    then elemAt toks i
    else {
      type = "EOF";
      line = 0;
    };
  ty = toks: i: (tok toks i).type;
  at = toks: i: t: (ty toks i) == t;

  expect = toks: i: t:
    if at toks i t
    then i + 1
    else throw "skylark parser: expected '${t}' but found '${ty toks i}' at line ${toString (tok toks i).line}";

  # Skip any run of NEWLINE tokens (used between statements defensively).

  compareOps = ["==" "!=" "<" ">" "<=" ">="];

  # ---- Expressions -------------------------------------------------------

  # A single expression ("test"): lambda / ternary / boolean / ...
  parseTest = toks: i:
    if at toks i "lambda"
    then parseLambda toks i
    else let
      c = parseOr toks i;
    in
      if at toks c.pos "if"
      then let
        test = parseOr toks (c.pos + 1);
        e = expect toks test.pos "else";
        orelse = parseTest toks e;
      in {
        node = {
          k = "ternary";
          body = c.node;
          test = test.node;
          orelse = orelse.node;
        };
        inherit (orelse) pos;
      }
      else c;

  parseLambda = toks: i: let
    p = parseParams toks (i + 1) ":";
    colon = expect toks p.pos ":";
    body = parseTest toks colon;
  in {
    node = {
      k = "lambda";
      inherit (p) params;
      body = body.node;
    };
    inherit (body) pos;
  };

  # Drive a parser step until it reports `done`, in CHUNKS. Nix has no tail-call
  # elimination, so a `go acc (i + 1)` loop costs one C-stack frame per element and
  # overflows on a big literal -- a generated SDK header map is thousands of entries
  # in a single dict. foldl' over chunk indices is iterative, and each chunk recurses
  # only chunkSize deep. Overshooting the step bound is harmless: a state that is
  # already done is returned unchanged.
  iterateSteps = {
    step,
    init,
    maxSteps,
  }: let
    chunkSize = 200;
    stepN = n: st:
      if n == 0 || st.done
      then st
      else stepN (n - 1) (step st);
  in
    builtins.foldl' (st: _: stepN chunkSize st) init
    (builtins.genList (i: i) (1 + maxSteps / chunkSize));

  # Left-associative binary operator level.
  leftAssoc = sub: opTypes: toks: i: let
    first = sub toks i;
    go = acc: j:
      if elem (ty toks j) opTypes
      then let
        r = sub toks (j + 1);
      in
        go {
          k = "binop";
          op = ty toks j;
          left = acc;
          right = r.node;
        }
        r.pos
      else {
        node = acc;
        pos = j;
      };
  in
    go first.node first.pos;

  parseOr = leftAssoc parseAnd ["or"];
  parseAnd = leftAssoc parseNot ["and"];

  parseNot = toks: i:
    if at toks i "not"
    then let
      r = parseNot toks (i + 1);
    in {
      node = {
        k = "unary";
        op = "not";
        operand = r.node;
      };
      inherit (r) pos;
    }
    else parseComparison toks i;

  # Single (non-chaining) comparison, including `in` and `not in`.
  parseComparison = toks: i: let
    l = parseBitOr toks i;
    t = ty toks l.pos;
  in
    if elem t compareOps
    then let
      r = parseBitOr toks (l.pos + 1);
    in {
      node = {
        k = "binop";
        op = t;
        left = l.node;
        right = r.node;
      };
      inherit (r) pos;
    }
    else if t == "in"
    then let
      r = parseBitOr toks (l.pos + 1);
    in {
      node = {
        k = "binop";
        op = "in";
        left = l.node;
        right = r.node;
      };
      inherit (r) pos;
    }
    else if t == "not" && (ty toks (l.pos + 1)) == "in"
    then let
      r = parseBitOr toks (l.pos + 2);
    in {
      node = {
        k = "binop";
        op = "not in";
        left = l.node;
        right = r.node;
      };
      inherit (r) pos;
    }
    else l;

  parseBitOr = leftAssoc parseBitXor ["|"];
  parseBitXor = leftAssoc parseBitAnd ["^"];
  parseBitAnd = leftAssoc parseShift ["&"];
  parseShift = leftAssoc parseArith ["<<" ">>"];
  parseArith = leftAssoc parseTerm ["+" "-"];
  parseTerm = leftAssoc parseUnary ["*" "/" "//" "%"];

  parseUnary = toks: i: let
    t = ty toks i;
  in
    if t == "-" || t == "+" || t == "~"
    then let
      r = parseUnary toks (i + 1);
    in {
      node = {
        k = "unary";
        op = t;
        operand = r.node;
      };
      inherit (r) pos;
    }
    else parsePower toks i;

  parsePower = toks: i: let
    base = parsePostfix toks i;
  in
    if at toks base.pos "**"
    then let
      r = parseUnary toks (base.pos + 1);
    in {
      node = {
        k = "binop";
        op = "**";
        left = base.node;
        right = r.node;
      };
      inherit (r) pos;
    }
    else base;

  # Postfix chain: attribute access, call, subscript.
  parsePostfix = toks: i: let
    atom = parseAtom toks i;
    go = acc: j:
      if at toks j "."
      then
        go {
          k = "attr";
          obj = acc;
          name = (tok toks (j + 1)).value;
        } (expect toks (j + 1) "NAME")
      else if at toks j "("
      then let
        c = parseCallArgs toks (j + 1);
      in
        go {
          k = "call";
          func = acc;
          inherit (c) args;
          inherit (c) kwargs;
          inherit (c) star;
          inherit (c) dstar;
        }
        c.pos
      else if at toks j "["
      then let
        s = parseSubscript toks (j + 1);
      in
        go (s.node // {obj = acc;}) s.pos
      else {
        node = acc;
        pos = j;
      };
  in
    go atom.node atom.pos;

  # Inside `[` ... `]`: either an index or a slice lower:upper:step.
  parseSubscript = toks: i: let
    lower =
      if at toks i ":"
      then {
        node = null;
        pos = i;
      }
      else parseTest toks i;
  in
    if at toks lower.pos ":"
    then let
      upper =
        if at toks (lower.pos + 1) ":" || at toks (lower.pos + 1) "]"
        then {
          node = null;
          pos = lower.pos + 1;
        }
        else parseTest toks (lower.pos + 1);
      step =
        if at toks upper.pos ":"
        then
          (
            if at toks (upper.pos + 1) "]"
            then {
              node = null;
              pos = upper.pos + 1;
            }
            else parseTest toks (upper.pos + 1)
          )
        else {
          node = null;
          inherit (upper) pos;
        };
      close = expect toks step.pos "]";
    in {
      node = {
        k = "slice";
        lower = lower.node;
        upper = upper.node;
        step = step.node;
      };
      pos = close;
    }
    else let
      close = expect toks lower.pos "]";
    in {
      node = {
        k = "subscript";
        index = lower.node;
      };
      pos = close;
    };

  # Call arguments: positional, keyword (name=expr), *args, **kwargs.
  parseCallArgs = toks: i: let
    go = args: kwargs: star: dstar: j:
      if at toks j ")"
      then {
        inherit args kwargs star dstar;
        pos = j + 1;
      }
      else let
        j2 =
          if at toks j ","
          then j + 1
          else j;
      in
        if at toks j2 ")"
        then {
          inherit args kwargs star dstar;
          pos = j2 + 1;
        }
        else if at toks j2 "**"
        then let
          r = parseTest toks (j2 + 1);
        in
          go args kwargs star r.node r.pos
        else if at toks j2 "*"
        then let
          r = parseTest toks (j2 + 1);
        in
          go args kwargs r.node dstar r.pos
        else if (ty toks j2) == "NAME" && (ty toks (j2 + 1)) == "="
        then let
          name = (tok toks j2).value;
          r = parseTest toks (j2 + 2);
        in
          go args (kwargs
            ++ [
              {
                inherit name;
                value = r.node;
              }
            ])
          star
          dstar
          r.pos
        else let
          r = parseTest toks j2;
        in
          go (args ++ [r.node]) kwargs star dstar r.pos;
  in
    go [] [] null null i;

  parseAtom = toks: i: let
    t = ty toks i;
  in
    if t == "NUMBER"
    then {
      node = {
        k = "num";
        inherit ((tok toks i)) value;
      };
      pos = i + 1;
    }
    else if t == "STRING"
    then parseStringConcat toks i
    else if t == "NAME"
    then {
      node = {
        k = "name";
        id = (tok toks i).value;
      };
      pos = i + 1;
    }
    else if t == "("
    then parseParenOrTuple toks (i + 1)
    else if t == "["
    then parseListOrComp toks (i + 1)
    else if t == "{"
    then parseDictOrComp toks (i + 1)
    else throw "skylark parser: unexpected token '${t}' at line ${toString (tok toks i).line}";

  # Adjacent string literals concatenate ("a" "b" -> "ab").
  parseStringConcat = toks: i: let
    go = acc: j:
      if at toks j "STRING"
      then go (acc + (tok toks j).value) (j + 1)
      else {
        node = {
          k = "str";
          value = acc;
        };
        pos = j;
      };
  in
    go (tok toks i).value (i + 1);

  parseParenOrTuple = toks: i:
    if at toks i ")"
    then {
      node = {
        k = "tuple";
        elts = [];
      };
      pos = i + 1;
    }
    else let
      first = parseTest toks i;
    in
      if at toks first.pos ","
      then parseTupleRest toks [first.node] (first.pos + 1) ")"
      else let
        close = expect toks first.pos ")";
      in {
        inherit (first) node; # parenthesized grouping
        pos = close;
      };

  # After a leading elt and a comma, gather the rest of a tuple up to `closer`.
  parseTupleRest = toks: elts: i: closer: let
    go = acc: j:
      if at toks j closer
      then {
        node = {
          k = "tuple";
          elts = acc;
        };
        pos = j + 1;
      }
      else let
        r = parseTest toks j;
        j2 = r.pos;
      in
        if at toks j2 ","
        then go (acc ++ [r.node]) (j2 + 1)
        else {
          node = {
            k = "tuple";
            elts = acc ++ [r.node];
          };
          pos = expect toks j2 closer;
        };
  in
    go elts i;

  parseListOrComp = toks: i:
    if at toks i "]"
    then {
      node = {
        k = "list";
        elts = [];
      };
      pos = i + 1;
    }
    else let
      first = parseTest toks i;
    in
      if at toks first.pos "for"
      then let
        c = parseComprehensionClauses toks first.pos "]";
      in {
        node = {
          k = "listcomp";
          elt = first.node;
          inherit (c) clauses;
        };
        inherit (c) pos;
      }
      else parseListRest toks [first.node] first.pos;

  parseListRest = toks: elts: i: let
    go = acc: j:
      if at toks j "]"
      then {
        node = {
          k = "list";
          elts = acc;
        };
        pos = j + 1;
      }
      else let
        j2 = expect toks j ",";
      in
        if at toks j2 "]"
        then {
          node = {
            k = "list";
            elts = acc;
          };
          pos = j2 + 1;
        }
        else let
          r = parseTest toks j2;
        in
          go (acc ++ [r.node]) r.pos;
  in
    go elts i;

  parseDictOrComp = toks: i:
    if at toks i "}"
    then {
      node = {
        k = "dict";
        entries = [];
      };
      pos = i + 1;
    }
    else let
      key = parseTest toks i;
      colon = expect toks key.pos ":";
      val = parseTest toks colon;
    in
      if at toks val.pos "for"
      then let
        c = parseComprehensionClauses toks val.pos "}";
      in {
        node = {
          k = "dictcomp";
          key = key.node;
          value = val.node;
          inherit (c) clauses;
        };
        inherit (c) pos;
      }
      else
        parseDictRest toks [
          {
            key = key.node;
            value = val.node;
          }
        ]
        val.pos;

  parseDictRest = toks: entries: i: let
    step = st:
      if at toks st.pos "}"
      then
        st
        // {
          done = true;
          pos = st.pos + 1;
        }
      else let
        j2 = expect toks st.pos ",";
      in
        if at toks j2 "}"
        then
          st
          // {
            done = true;
            pos = j2 + 1;
          }
        else let
          key = parseTest toks j2;
          colon = expect toks key.pos ":";
          val = parseTest toks colon;
        in
          st
          // {
            acc =
              st.acc
              ++ [
                {
                  key = key.node;
                  value = val.node;
                }
              ];
            inherit (val) pos;
          };
    r = iterateSteps {
      inherit step;
      init = {
        acc = entries;
        pos = i;
        done = false;
      };
      # Each entry consumes at least three tokens (key, colon, value).
      maxSteps = length toks + 2;
    };
  in {
    node = {
      k = "dict";
      entries = r.acc;
    };
    inherit (r) pos;
  };

  # `for <targets> in <expr>` and `if <expr>` clauses, ending at `closer`.
  parseComprehensionClauses = toks: i: closer: let
    go = acc: j:
      if at toks j closer
      then {
        clauses = acc;
        pos = j + 1;
      }
      else if at toks j "for"
      then let
        targets = parseTargetList toks (j + 1);
        inKw = expect toks targets.pos "in";
        iter = parseOr toks inKw;
      in
        go (acc
          ++ [
            {
              k = "for";
              targets = targets.node;
              iter = iter.node;
            }
          ])
        iter.pos
      else if at toks j "if"
      then let
        test = parseOr toks (j + 1);
      in
        go (acc
          ++ [
            {
              k = "if";
              test = test.node;
            }
          ])
        test.pos
      else throw "skylark parser: bad comprehension clause '${ty toks j}' at line ${toString (tok toks j).line}";
  in
    go [] i;

  # A comma-separated target list (for-loop / comprehension / unpack LHS).
  parseTargetList = toks: i: let
    first = parsePostfix toks i;
  in
    if at toks first.pos ","
    then parseTupleRestTargets toks [first.node] (first.pos + 1)
    else {
      inherit (first) node;
      inherit (first) pos;
    };

  parseTupleRestTargets = toks: elts: i: let
    go = acc: j:
      if (ty toks j) == "in" || (ty toks j) == "=" || (ty toks j) == "]" || (ty toks j) == ")"
      then {
        node = {
          k = "tuple";
          elts = acc;
        };
        pos = j;
      }
      else let
        r = parsePostfix toks j;
      in
        if at toks r.pos ","
        then go (acc ++ [r.node]) (r.pos + 1)
        else {
          node = {
            k = "tuple";
            elts = acc ++ [r.node];
          };
          inherit (r) pos;
        };
  in
    go elts i;

  # ---- Statements --------------------------------------------------------

  # Parameter list for def/lambda, ending at `endTok`.
  parseParams = toks: i: endTok: let
    go = acc: j:
      if at toks j endTok
      then {
        params = acc;
        pos = j;
      }
      else let
        j2 =
          if at toks j ","
          then j + 1
          else j;
      in
        if at toks j2 endTok
        then {
          params = acc;
          pos = j2;
        }
        else if at toks j2 "**"
        then let
          name = (tok toks (j2 + 1)).value;
        in
          go (acc
            ++ [
              {
                inherit name;
                kind = "dstar";
              }
            ]) (j2 + 2)
        else if at toks j2 "*"
        then
          (
            if (ty toks (j2 + 1)) == "NAME"
            then
              go (acc
                ++ [
                  {
                    name = (tok toks (j2 + 1)).value;
                    kind = "star";
                  }
                ]) (j2 + 2)
            else
              go (acc
                ++ [
                  {
                    name = null;
                    kind = "star";
                  }
                ]) (j2 + 1)
          )
        else let
          name = (tok toks j2).value;
          afterName = expect toks j2 "NAME";
          # optional `: type` annotation (discarded)
          afterType =
            if at toks afterName ":"
            then (parseTest toks (afterName + 1)).pos
            else afterName;
          # optional `= default`
          hasDefault = at toks afterType "=";
          def =
            if hasDefault
            then parseTest toks (afterType + 1)
            else {
              node = null;
              pos = afterType;
            };
        in
          go (acc
            ++ [
              {
                inherit name;
                kind = "normal";
                default = def.node;
              }
            ])
          def.pos;
  in
    go [] i;

  parseDef = toks: i: let
    name = (tok toks (i + 1)).value;
    lp = expect toks (i + 1) "NAME";
    op = expect toks lp "(";
    p = parseParams toks op ")";
    cp = expect toks p.pos ")";
    # optional `-> type` return annotation (discarded)
    afterRet =
      if at toks cp "->"
      then (parseTest toks (cp + 1)).pos
      else cp;
    colon = expect toks afterRet ":";
    body = parseBlock toks colon;
  in {
    node = {
      k = "def";
      inherit name;
      inherit (p) params;
      body = body.node;
    };
    inherit (body) pos;
  };

  parseIf = toks: i: let
    test = parseTest toks (i + 1);
    colon = expect toks test.pos ":";
    body = parseBlock toks colon;
    branch = {
      test = test.node;
      body = body.node;
    };
    rest =
      if at toks body.pos "elif"
      then let
        r = parseIf toks body.pos;
      in {
        branches = [branch] ++ r.node.branches;
        orelse = r.node.orelse;
        inherit (r) pos;
      }
      else if at toks body.pos "else"
      then let
        colon2 = expect toks (body.pos + 1) ":";
        eb = parseBlock toks colon2;
      in {
        branches = [branch];
        orelse = eb.node;
        inherit (eb) pos;
      }
      else {
        branches = [branch];
        orelse = [];
        inherit (body) pos;
      };
  in {
    node = {
      k = "if";
      inherit (rest) branches orelse;
    };
    inherit (rest) pos;
  };

  parseFor = toks: i: let
    targets = parseTargetList toks (i + 1);
    inKw = expect toks targets.pos "in";
    iter = parseTest toks inKw;
    colon = expect toks iter.pos ":";
    body = parseBlock toks colon;
  in {
    node = {
      k = "for";
      targets = targets.node;
      iter = iter.node;
      body = body.node;
    };
    inherit (body) pos;
  };

  parseLoad = toks: i: let
    op = expect toks (i + 1) "(";
    module = (tok toks op).value;
    afterMod = expect toks op "STRING";
    go = syms: j:
      if at toks j ")"
      then {
        symbols = syms;
        pos = j + 1;
      }
      else let
        j2 = expect toks j ",";
      in
        if at toks j2 ")"
        then {
          symbols = syms;
          pos = j2 + 1;
        }
        else if (ty toks j2) == "NAME" && (ty toks (j2 + 1)) == "="
        then let
          alias = (tok toks j2).value;
          from = (tok toks (j2 + 2)).value;
          j3 = expect toks (j2 + 2) "STRING";
        in
          go (syms
            ++ [
              {
                name = alias;
                inherit from;
              }
            ])
          j3
        else let
          nm = (tok toks j2).value;
          j3 = expect toks j2 "STRING";
        in
          go (syms
            ++ [
              {
                name = nm;
                from = nm;
              }
            ])
          j3;
    r = go [] afterMod;
    nl = expectNewline toks r.pos;
  in {
    node = {
      k = "load";
      inherit module;
      inherit (r) symbols;
    };
    pos = nl;
  };

  augOps = ["+=" "-=" "*=" "/=" "//=" "%=" "&=" "|=" "^=" ">>=" "<<="];

  # A simple statement (expr / assignment / return / pass / break / continue),
  # terminated by NEWLINE.
  parseSimpleStmt = toks: i: let
    t = ty toks i;
  in
    if t == "return"
    then
      if at toks (i + 1) "NEWLINE"
      then {
        node = {
          k = "return";
          value = null;
        };
        pos = expectNewline toks (i + 1);
      }
      else let
        v = parseTestListStmt toks (i + 1);
      in {
        node = {
          k = "return";
          value = v.node;
        };
        pos = expectNewline toks v.pos;
      }
    else if t == "pass"
    then {
      node = {k = "pass";};
      pos = expectNewline toks (i + 1);
    }
    else if t == "break"
    then {
      node = {k = "break";};
      pos = expectNewline toks (i + 1);
    }
    else if t == "continue"
    then {
      node = {k = "continue";};
      pos = expectNewline toks (i + 1);
    }
    else let
      lhs = parseTestListStmt toks i;
      nt = ty toks lhs.pos;
    in
      if nt == "="
      then let
        rhs = parseTestListStmt toks (lhs.pos + 1);
      in {
        node = {
          k = "assign";
          target = lhs.node;
          value = rhs.node;
        };
        pos = expectNewline toks rhs.pos;
      }
      else if elem nt augOps
      then let
        rhs = parseTestListStmt toks (lhs.pos + 1);
      in {
        node = {
          k = "augassign";
          target = lhs.node;
          op = nt;
          value = rhs.node;
        };
        pos = expectNewline toks rhs.pos;
      }
      else {
        node = {
          k = "expr_stmt";
          expr = lhs.node;
        };
        pos = expectNewline toks lhs.pos;
      };

  # Testlist in statement context (bare tuples: `a, b = ...`, `return a, b`).
  parseTestListStmt = toks: i: let
    first = parseTest toks i;
  in
    if at toks first.pos ","
    then let
      go = acc: j:
        if (ty toks j) == "NEWLINE" || (ty toks j) == "=" || elem (ty toks j) augOps
        then {
          node = {
            k = "tuple";
            elts = acc;
          };
          pos = j;
        }
        else let
          r = parseTest toks j;
        in
          if at toks r.pos ","
          then go (acc ++ [r.node]) (r.pos + 1)
          else {
            node = {
              k = "tuple";
              elts = acc ++ [r.node];
            };
            inherit (r) pos;
          };
    in
      go [first.node] (first.pos + 1)
    else first;

  expectNewline = toks: i:
    if at toks i "NEWLINE"
    then i + 1
    else if at toks i "EOF"
    then i
    else if at toks i ";"
    then i + 1 # tolerate a trailing semicolon before newline handling
    else throw "skylark parser: expected end of statement but found '${ty toks i}' at line ${toString (tok toks i).line}";

  parseStatement = toks: i: let
    t = ty toks i;
  in
    if t == "def"
    then parseDef toks i
    else if t == "if"
    then parseIf toks i
    else if t == "for"
    then parseFor toks i
    else if t == "load"
    then parseLoad toks i
    else parseSimpleStmt toks i;

  # A `: <block>` suite: either NEWLINE INDENT stmts DEDENT, or an inline
  # simple statement on the same line.
  parseBlock = toks: i:
    if at toks i "NEWLINE"
    then let
      ind = expect toks (i + 1) "INDENT";
      stmts = parseStmtsUntilDedent toks ind;
      dedent = expect toks stmts.pos "DEDENT";
    in {
      node = stmts.nodes;
      pos = dedent;
    }
    else let
      s = parseSimpleStmt toks i;
    in {
      node = [s.node];
      inherit (s) pos;
    };

  parseStmtsUntilDedent = toks: i: let
    step = st:
      if at toks st.pos "DEDENT" || at toks st.pos "EOF"
      then st // {done = true;}
      else if at toks st.pos "NEWLINE"
      then st // {pos = st.pos + 1;}
      else let
        s = parseStatement toks st.pos;
      in
        st
        // {
          acc = st.acc ++ [s.node];
          inherit (s) pos;
        };
    r = iterateSteps {
      inherit step;
      init = {
        acc = [];
        pos = i;
        done = false;
      };
      maxSteps = length toks + 2;
    };
  in {
    nodes = r.acc;
    inherit (r) pos;
  };

  # A module body is the other unbounded list: a generated BUCK file is tens of
  # thousands of statements, so this iterates for the same reason parseDictRest does.
  parseModule = toks: let
    step = st:
      if at toks st.pos "EOF"
      then st // {done = true;}
      else if at toks st.pos "NEWLINE" || at toks st.pos "INDENT" || at toks st.pos "DEDENT"
      then st // {pos = st.pos + 1;}
      else let
        s = parseStatement toks st.pos;
      in
        st
        // {
          acc = st.acc ++ [s.node];
          inherit (s) pos;
        };
    r = iterateSteps {
      inherit step;
      init = {
        acc = [];
        pos = 0;
        done = false;
      };
      maxSteps = length toks + 2;
    };
  in {
    k = "module";
    body = r.acc;
  };

  parse = src: parseModule (tokenize src);
in {
  inherit parse parseModule;
  # Expose expression parsing for tests.
  parseExpr = src: let
    toks = tokenize src;
    r = parseTest toks 0;
  in
    r.node;
}
