# nix-lib-skylark

A pure-Nix interpreter for [Starlark](https://github.com/bazelbuild/starlark)
(the language formerly called Skylark), used by Bazel, Buck2, and others.
Lexer, recursive-descent parser, and a tree-walking evaluator, written with
`builtins` only (no `pkgs`, no nixpkgs `lib`), so the unit tests run with a
bare `nix eval -f`.

It is host-agnostic on purpose: `load()` is resolved by a caller-supplied
function, extra globals are injectable, and an opaque `world` accumulator is
threaded through evaluation so a host can collect effects (Buck2 uses this for
target and action registration). `platform/nix/lib/buck2` is the first consumer; the
interpreter itself knows nothing about Buck2.

## Usage

```nix
let sk = import ./platform/nix/lib/skylark/api.nix;
in {
  # Evaluate a single expression.
  answer = sk.evalExpr "6 * 7";                       # => 42 (a skylark int)

  # Evaluate a module; returns { globals; world; }.
  mod = sk.exec { src = "x = 1\ny = x + 2\n"; };      # mod.globals.y => 3

  # With load() and injected globals:
  out = sk.exec {
    src = ''
      load("//m.bzl", "helper")
      result = helper(3)
    '';
    loadModule = currentFile: label: /* -> module globals attrset */;
    extraGlobals = { my_builtin = /* a builtin value */; };
    world = null;                                      # threaded, host-defined
  };
}
```

Interpreter API (`api.nix`):

| Entry | Description |
|---|---|
| `parse src` | source string to an AST (`{ k = "module"; body = [...]; }`) |
| `parseExpr src` | a single expression AST |
| `evalExpr src` | evaluate one expression in the base environment |
| `exec { src; currentFile ? ; world ? null; extraGlobals ? {}; loadModule ? ; }` | evaluate a module, returns `{ globals; world; }` |
| `mkInterp { loadModule ? ; extraGlobals ? {}; }` | an interpreter instance: `{ evalModule; callValue; apply; eval; }` |
| `values` | the value model (`values.nix`) |
| `stdlib` | `{ globals; getMethod; }` (the standard library) |

## Value model

Scalars are native Nix values (`null` is `None`, plus `bool`, `int`, `float`,
`string`). Compound and host values are tagged attrsets with a `__sk` field:
`list`, `tuple`, `dict`, `function`, `builtin`, and any host tag (`struct`,
`object`, `artifact`, ...). Host values participate through a small protocol:
`.attrs`/`.getAttr` for member access, `.subscript` for indexing, `.fn` for
calling, `.id` for identity in equality. See `values.nix`.

## Supported language

Functions and closures (with def-time defaults), `if`/`elif`/`else`, `for` with
`break`/`continue`, `return`, list/dict/tuple literals, list and dict
comprehensions, ternary `a if c else b`, boolean short-circuit, the full
operator set (arithmetic, comparison, `in`/`not in`, bitwise), `%` string
formatting, slicing, tuple unpacking, augmented assignment, `load`, and the
common builtins (`len`, `range`, `sorted`, `enumerate`, `zip`, `dict`, `str`,
`type`, `fail`, ...) plus string/list/dict methods (`format`, `join`, `split`,
`startswith`, `replace`, `append`, `extend`, `get`, `items`, `update`, ...).

Free variables in a function resolve against the module's final globals, so a
rule impl may reference a helper defined later in the file (the Buck2 analysis
pattern). `True`/`False`/`None` are ordinary globals.

## Not (yet) supported

Mutation is modeled by statement-level rebinding: `xs.append(x)` and
`cmd.add(...)` rebind the local, which is correct for local-variable mutation
but not for mutation through an alias (`ys = xs; ys.append(1)` will not change
`xs`). There is no object heap. `set`, `\x`/`\u` string escapes, and float
`int()` are not implemented. Starlark's ban on recursion is not enforced.

## Tests

```console
nix eval -f platform/nix/lib/skylark/tests/lexer.nix
nix eval -f platform/nix/lib/skylark/tests/parser.nix
nix eval -f platform/nix/lib/skylark/tests/eval.nix
```

Or as a flake check: `nix build .#checks.x86_64-linux.skylark-lib` (never
`nix flake check` in this repo).
