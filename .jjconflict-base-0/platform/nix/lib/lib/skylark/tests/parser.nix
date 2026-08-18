# Run: nix eval -f platform/nix/lib/lib/skylark/tests/parser.nix
let
  parser = import ../parser.nix;
  p = parser.parseExpr;

  # Expression-shape assertions: [ label boolean ].
  exprChecks = [
    ["add/mul precedence" (let e = p "a + b * c"; in e.k == "binop" && e.op == "+" && e.right.k == "binop" && e.right.op == "*")]
    ["mul/add precedence" (let e = p "a * b + c"; in e.k == "binop" && e.op == "+" && e.left.k == "binop" && e.left.op == "*")]
    ["call args/kwargs/star" (let e = p "f(x, y = 1, *a, **b)"; in e.k == "call" && builtins.length e.args == 1 && builtins.length e.kwargs == 1 && (builtins.head e.kwargs).name == "y" && e.star != null && e.dstar != null)]
    ["attr chain + subscript" (let e = p "o.a.b[c]"; in e.k == "subscript" && e.index.id == "c" && e.obj.k == "attr" && e.obj.name == "b" && e.obj.obj.name == "a")]
    ["ternary" (let e = p "x if c else y"; in e.k == "ternary" && e.body.id == "x" && e.test.id == "c" && e.orelse.id == "y")]
    ["listcomp two clauses" (let e = p "[i for i in xs if i]"; in e.k == "listcomp" && builtins.length e.clauses == 2 && (builtins.head e.clauses).k == "for")]
    ["dictcomp" (let e = p "{k: v for k in ks}"; in e.k == "dictcomp" && e.key.id == "k")]
    ["not binds tighter than and" (let e = p "not a and b"; in e.k == "binop" && e.op == "and" && e.left.k == "unary" && e.left.op == "not")]
    ["power binds tighter than unary minus" (let e = p "-2 ** 2"; in e.k == "unary" && e.op == "-" && e.operand.k == "binop" && e.operand.op == "**")]
    ["equality" (let e = p "a == b"; in e.k == "binop" && e.op == "==")]
    ["not in" (let e = p "a not in b"; in e.k == "binop" && e.op == "not in")]
    ["in" (let e = p "a in b"; in e.k == "binop" && e.op == "in")]
    ["triple tuple" (let e = p "(1, 2, 3)"; in e.k == "tuple" && builtins.length e.elts == 3)]
    ["singleton tuple" (let e = p "(1,)"; in e.k == "tuple" && builtins.length e.elts == 1)]
    ["paren grouping is not a tuple" (let e = p "(x)"; in e.k == "name" && e.id == "x")]
    ["list trailing comma" (let e = p "[1, 2,]"; in e.k == "list" && builtins.length e.elts == 2)]
    ["adjacent string concat" (let e = p "\"a\" \"b\""; in e.k == "str" && e.value == "ab")]
    ["list concat expr" (let e = p "[a] + srcs"; in e.k == "binop" && e.op == "+" && e.left.k == "list")]
    ["index by provider type" (let e = p "dep[CxxCompilerInfo]"; in e.k == "subscript" && e.index.id == "CxxCompilerInfo")]
    ["method call" (let e = p "cmd.add([x])"; in e.k == "call" && e.func.k == "attr" && e.func.name == "add")]
    ["slice" (let e = p "xs[1:2]"; in e.k == "slice" && e.lower.value == 1 && e.upper.value == 2)]
  ];

  # Statement/module-shape assertions.
  kinds = src: map (s: s.k) (parser.parse src).body;

  stmtChecks = [
    ["assign" (kinds "x = 1\n" == ["assign"])]
    [
      "tuple unpack assign"
      (let
        m = (parser.parse "a, b = f()\n").body;
        s = builtins.head m;
      in
        s.k == "assign" && s.target.k == "tuple")
    ]
    ["augassign" (let s = builtins.head (parser.parse "x += 1\n").body; in s.k == "augassign" && s.op == "+=")]
    ["def/if/return" (kinds "def f(x):\n    if x:\n        return 1\n    return 2\n" == ["def"])]
    ["if elif else" (let s = builtins.head (parser.parse "if a:\n    x = 1\nelif b:\n    x = 2\nelse:\n    x = 3\n").body; in s.k == "if" && builtins.length s.branches == 2 && builtins.length s.orelse == 1)]
    ["for loop" (let s = builtins.head (parser.parse "for x in xs:\n    pass\n").body; in s.k == "for")]
    ["load with alias" (let s = builtins.head (parser.parse "load(\"m\", \"a\", b = \"c\")\n").body; in s.k == "load" && s.module == "m" && builtins.length s.symbols == 2 && (builtins.elemAt s.symbols 1).name == "b" && (builtins.elemAt s.symbols 1).from == "c")]
    ["module-level call assign" (kinds "toolchain_config = _toolchain_config()\n" == ["assign"])]
    [
      "return tuple"
      (let
        s = builtins.head (parser.parse "def f():\n    return a, b\n").body;
        b = builtins.head s.body;
      in
        b.k == "return" && b.value.k == "tuple")
    ]
    ["def with type annotations" (let s = builtins.head (parser.parse "def f(ctx: AnalysisContext) -> list[Provider]:\n    return []\n").body; in s.k == "def" && builtins.length s.params == 1 && (builtins.head s.params).name == "ctx")]
  ];

  # Parse every real no_prelude file and force the whole AST (deepSeq catches
  # any lazily-thrown parse error). Also spot-check two module shapes.
  corpus = ../../buck2/tests/fixtures/no_prelude;
  corpusFiles = [
    "cpp/rules.bzl"
    "rust/rules.bzl"
    "go/rules.bzl"
    "toolchains/cpp_toolchain.bzl"
    "toolchains/rust_toolchain.bzl"
    "toolchains/go_toolchain.bzl"
    "toolchains/export_file.bzl"
    "cpp/hello_world/BUCK"
    "cpp/library/BUCK"
    "rust/BUCK"
    "go/BUCK"
    "toolchains/BUCK"
  ];
  parseFile = f: parser.parse (builtins.readFile (corpus + "/${f}"));
  corpusForced = builtins.deepSeq (map parseFile corpusFiles) true;

  corpusChecks = [
    ["corpus parses (deepSeq)" corpusForced]
    ["cpp/rules.bzl top-level shape" (map (s: s.k) (parseFile "cpp/rules.bzl").body == ["load" "assign" "def" "assign" "def" "assign"])]
    ["toolchains/BUCK top-level shape" (map (s: s.k) (parseFile "toolchains/BUCK").body == ["load" "load" "load" "load" "expr_stmt" "expr_stmt" "expr_stmt" "expr_stmt" "expr_stmt"])]
    ["go_toolchain.bzl parses to 6 top-level stmts" (builtins.length (parseFile "toolchains/go_toolchain.bzl").body == 6)]
  ];

  allChecks = exprChecks ++ stmtChecks ++ corpusChecks;
  failures = builtins.filter (c: !(builtins.elemAt c 1)) allChecks;
  names = map (c: builtins.elemAt c 0) failures;
in
  if failures == []
  then "ok: ${toString (builtins.length allChecks)} parser cases"
  else throw "parser test failures: ${builtins.toJSON names}"
