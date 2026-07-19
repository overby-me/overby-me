# Run: nix eval -f nix/lib/skylark/tests/eval.nix
let
  api = import ../api.nix;
  V = api.values;

  # Render a skylark value to plain Nix for assertions.
  render = v:
    if v == null || builtins.isBool v || builtins.isInt v || builtins.isFloat v || builtins.isString v
    then v
    else if V.isList v
    then map render v.items
    else if V.isTuple v
    then {__tuple = map render v.items;}
    else if V.isDict v
    then {__dict = map (e: [(render e.key) (render e.value)]) v.entries;}
    else v;

  e = src: render (api.evalExpr src);

  # Expression cases: [ label actual expected ].
  exprCases = [
    ["arith precedence" (e "1 + 2 * 3") 7]
    ["paren" (e "(1 + 2) * 3") 9]
    ["float div" (e "7 / 2") 3.5]
    ["floor div" (e "7 // 2") 3]
    ["floor div negative" (e "(0 - 7) // 2") (-4)]
    ["modulo" (e "7 % 3") 1]
    ["power" (e "2 ** 10") 1024]
    ["string concat" (e "\"a\" + \"b\"") "ab"]
    ["string repeat" (e "\"ab\" * 3") "ababab"]
    ["list concat" (e "[1, 2] + [3]") [1 2 3]]
    ["compare" (e "3 < 5") true]
    ["eq mixed numeric" (e "1 == 1.0") true]
    ["and short-circuit value" (e "0 and 5") 0]
    ["or short-circuit value" (e "0 or 5") 5]
    ["not" (e "not (1 == 2)") true]
    ["ternary true" (e "\"y\" if 1 else \"n\"") "y"]
    ["ternary false" (e "\"y\" if 0 else \"n\"") "n"]
    ["in list" (e "2 in [1, 2, 3]") true]
    ["not in" (e "5 not in [1, 2, 3]") true]
    ["in string" (e "\"bc\" in \"abcd\"") true]
    ["listcomp" (e "[x * x for x in range(4)]") [0 1 4 9]]
    ["listcomp filter" (e "[x for x in range(6) if x % 2 == 0]") [0 2 4]]
    ["nested listcomp" (e "[x + y for x in [1, 2] for y in [10, 20]]") [11 21 12 22]]
    ["dictcomp" (e "{k: k * 2 for k in range(3)}") {__dict = [[0 0] [1 2] [2 4]];}]
    ["len list" (e "len([1, 2, 3])") 3]
    ["len string" (e "len(\"hello\")") 5]
    ["sorted" (e "sorted([3, 1, 2])") [1 2 3]]
    ["sorted reverse" (e "sorted([1, 3, 2], reverse = True)") [3 2 1]]
    ["enumerate" (e "[i for i, x in enumerate([\"a\", \"b\"])]") [0 1]]
    ["str.format" (e "\"{} and {}\".format(\"a\", \"b\")") "a and b"]
    ["str.format indexed" (e "\"{1}{0}\".format(\"x\", \"y\")") "yx"]
    ["str.join" (e "\",\".join([\"a\", \"b\", \"c\"])") "a,b,c"]
    ["str.split" (e "\"a,b,c\".split(\",\")") ["a" "b" "c"]]
    ["str.startswith" (e "\"hello\".startswith(\"he\")") true]
    ["str.replace" (e "\"a.b.c\".replace(\".\", \"/\")") "a/b/c"]
    ["str.upper" (e "\"abc\".upper()") "ABC"]
    ["subscript negative" (e "[10, 20, 30][-1]") 30]
    ["slice" (e "[0, 1, 2, 3, 4][1:3]") [1 2]]
    ["string index" (e "\"hello\"[1]") "e"]
    ["tuple literal" (e "(1, 2, 3)") {__tuple = [1 2 3];}]
  ];

  # Module cases via exec: [ label globalKey actual expected ].
  execSrc = src: (api.exec {inherit src;}).globals;
  g = src: key: render (execSrc src).${key};

  moduleCases = [
    ["assignment chain" (g "x = 1\ny = x + 2\n" "y") 3]
    ["function default arg" (g "def f(a, b = 10):\n    return a + b\nr = f(5)\n" "r") 15]
    ["function override default" (g "def f(a, b = 10):\n    return a + b\nr = f(5, 20)\n" "r") 25]
    ["kwarg call" (g "def f(a, b):\n    return a - b\nr = f(b = 1, a = 10)\n" "r") 9]
    ["mutation append rebind" (g "xs = []\nxs.append(1)\nxs.append(2)\n" "xs") [1 2]]
    ["mutation in loop" (g "xs = []\nfor i in [3, 4, 5]:\n    xs.append(i)\n" "xs") [3 4 5]]
    ["mutation extend" (g "xs = [1]\nxs.extend([2, 3])\n" "xs") [1 2 3]]
    ["dict item assign" (g "d = {}\nd[\"a\"] = 1\nd[\"b\"] = 2\n" "d") {__dict = [["a" 1] ["b" 2]];}]
    ["dict update method" (g "d = {\"a\": 1}\nd.update({\"b\": 2})\n" "d") {__dict = [["a" 1] ["b" 2]];}]
    ["tuple unpack" (g "a, b = (1, 2)\nc = a + b\n" "c") 3]
    ["augassign" (g "x = 5\nx += 3\n" "x") 8]
    ["if elif else" (g "def f(n):\n    if n < 0:\n        return \"neg\"\n    elif n == 0:\n        return \"zero\"\n    else:\n        return \"pos\"\nr = f(0)\n" "r") "zero"]
    ["for with break" (g "r = 0\nfor i in range(10):\n    if i == 3:\n        break\n    r = r + i\n" "r") 3]
    ["for with continue" (g "r = 0\nfor i in range(5):\n    if i % 2 == 0:\n        continue\n    r = r + i\n" "r") 4]
    ["module-level call using builtins" (g "def cfg():\n    return len(\"abcd\")\nn = cfg()\n" "n") 4]
    ["struct-free provider-ish via dict return" (g "def mk():\n    return {\"k\": 1}\nr = mk()\n" "r") {__dict = [["k" 1]];}]
  ];

  # Late binding: a function defined before a helper it calls, invoked AFTER
  # the module finishes (the Buck2 analysis pattern). Uses callValue on the
  # resulting globals so free names resolve against final module globals.
  interp = api.mkInterp {};
  lateMod =
    interp.evalModule "<late>" (api.parse ''
      def outer(x):
          return helper(x) + 1

      def helper(x):
          return x * 10
    '')
    null;
  lateResult = render (interp.callValue lateMod.globals.outer [4] [] null).value;

  # load(): resolve a module and call a function it exports.
  loaded =
    interp.evalModule "<m>" (api.parse ''
      X = 40

      def inc(n):
          return n + 1
    '')
    null;
  loadStub = _currentFile: label:
    if label == "//m.bzl"
    then loaded.globals
    else throw "unknown module ${label}";
  loadResult =
    render
    (api.exec {
      src = ''
        load("//m.bzl", "X", "inc")
        r = inc(X) + 1
      '';
      loadModule = loadStub;
    }).globals.r;

  specialCases = [
    ["late-binding closure (analysis pattern)" lateResult 41]
    ["load binds and calls" loadResult 42]
  ];

  allCases = exprCases ++ moduleCases ++ specialCases;
  failures =
    builtins.filter (
      c: let
        actual = builtins.elemAt c 1;
        expected = builtins.elemAt c 2;
      in
        actual != expected
    )
    allCases;
  fmt = c: {
    label = builtins.elemAt c 0;
    actual = builtins.elemAt c 1;
    expected = builtins.elemAt c 2;
  };
in
  if failures == []
  then "ok: ${toString (builtins.length allCases)} eval cases"
  else throw "eval test failures: ${builtins.toJSON (map fmt failures)}"
