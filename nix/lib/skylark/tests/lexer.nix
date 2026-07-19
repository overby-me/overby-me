# Run: nix eval -f nix/lib/skylark/tests/lexer.nix
let
  tokenize = import ../lexer.nix;

  # Compact rendering of a token for assertions.
  render = t:
    if t.type == "NAME"
    then "N:${t.value}"
    else if t.type == "STRING"
    then "S=${t.value}"
    else if t.type == "NUMBER"
    then "#:${toString t.value}"
    else if t.type == "NEWLINE"
    then "\\n"
    else if t.type == "INDENT"
    then ">>"
    else if t.type == "DEDENT"
    then "<<"
    else if t.type == "EOF"
    then "EOF"
    else t.type;

  toks = src: map render (tokenize src);

  # [ label source expectedRenderedList ]
  cases = [
    [
      "simple call"
      "cpp_binary(name = \"main\")\n"
      ["N:cpp_binary" "(" "N:name" "=" "S=main" ")" "\\n" "EOF"]
    ]
    [
      "def with body indent/dedent"
      "def f(x):\n    return x\n"
      ["def" "N:f" "(" "N:x" ")" ":" "\\n" ">>" "return" "N:x" "\\n" "<<" "EOF"]
    ]
    [
      "blank and comment lines do not indent"
      "a = 1\n\n# comment\nb = 2\n"
      ["N:a" "=" "#:1" "\\n" "N:b" "=" "#:2" "\\n" "EOF"]
    ]
    [
      "implicit line join inside brackets"
      "x = [\n  1,\n  2,\n]\n"
      ["N:x" "=" "[" "#:1" "," "#:2" "," "]" "\\n" "EOF"]
    ]
    [
      "operators multi-char"
      "a == b != c and not d\n"
      ["N:a" "==" "N:b" "!=" "N:c" "and" "not" "N:d" "\\n" "EOF"]
    ]
    [
      "arrow and type-ish tokens"
      "def g() -> list:\n    pass\n"
      ["def" "N:g" "(" ")" "->" "N:list" ":" "\\n" ">>" "pass" "\\n" "<<" "EOF"]
    ]
    [
      "triple-quoted string with newline"
      "x = \"\"\"a\nb\"\"\"\n"
      ["N:x" "=" "S=a\nb" "\\n" "EOF"]
    ]
    [
      "escapes in string"
      "x = \"a\\tb\\n\"\n"
      ["N:x" "=" "S=a\tb\n" "\\n" "EOF"]
    ]
    [
      "hex and float numbers"
      "a = 0x1f\nb = 3.5\n"
      ["N:a" "=" "#:31" "\\n" "N:b" "=" "#:3.500000" "\\n" "EOF"]
    ]
    [
      "nested indent then double dedent"
      "def f():\n    if x:\n        return 1\n    return 2\n"
      ["def" "N:f" "(" ")" ":" "\\n" ">>" "if" "N:x" ":" "\\n" ">>" "return" "#:1" "\\n" "<<" "return" "#:2" "\\n" "<<" "EOF"]
    ]
    [
      "load statement"
      "load(\"pkg/rules.bzl\", \"cpp_binary\")\n"
      ["load" "(" "S=pkg/rules.bzl" "," "S=cpp_binary" ")" "\\n" "EOF"]
    ]
    [
      "member and subscript"
      "ctx.attrs.toolchain[CxxCompilerInfo]\n"
      ["N:ctx" "." "N:attrs" "." "N:toolchain" "[" "N:CxxCompilerInfo" "]" "\\n" "EOF"]
    ]
    [
      "no trailing newline"
      "x = 1"
      ["N:x" "=" "#:1" "\\n" "EOF"]
    ]
  ];

  results =
    map (
      c: let
        label = builtins.elemAt c 0;
        src = builtins.elemAt c 1;
        expected = builtins.elemAt c 2;
        actual = toks src;
      in {
        inherit label expected actual;
        ok = actual == expected;
      }
    )
    cases;

  failures = builtins.filter (r: !r.ok) results;
in
  if failures == []
  then "ok: ${toString (builtins.length cases)} lexer cases"
  else throw "lexer test failures: ${builtins.toJSON failures}"
