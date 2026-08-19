# ast-grep rules

<!-- publish:begin -->
> Part of the [overby.me monorepo](https://tangled.org/overby.me/overby.me), where this lives in
> [`dev/ast-grep`](https://tangled.org/overby.me/overby.me/tree/main/dev/ast-grep) and where all development happens.
>
> It is also published on its own, as
> [tangled.org/overby.me/ast-grep-rules](https://tangled.org/overby.me/ast-grep-rules) and
> [github.com/overby-me/ast-grep-rules](https://github.com/overby-me/ast-grep-rules). Both
> are read-only mirrors, rebuilt from the monorepo with
> [josh](https://github.com/josh-project/josh): a commit made to either is
> overwritten by the next sync, so please open issues and pull requests on the
> monorepo.
<!-- publish:end -->

Structural lint rules for [ast-grep](https://ast-grep.github.io/), written for
and measured against one monorepo: 39 for Rust, 4 for Nix, 5 for Mojo. Every
rule is `severity: error`, because ast-grep exits 0 on warnings and a gate
that cannot fail is decoration.

Each rule's header comment records why it exists and what it measured when it
landed: how many sites it found, which were fixed, and which idioms it was
narrowed to spare. Rules clippy or statix can already express are not here;
[CANDIDATES.md](CANDIDATES.md) is the ledger of everything evaluated, kept so
a rejected rule is not proposed twice.

## Layout

- `rules/` - one rule per file, `<language>-<what-it-catches>.yml`.
- `tests/` - one fixture file per rule, `valid:` and `invalid:` snippets.
  A dead rule and a clean tree are indistinguishable from scan output, so a
  rule without a fixture proving both directions does not land.
- `check/default.nix` - the check: fixture tests plus a full scan, bound to
  the pinned ast-grep and grammar so an engine bump cannot break rules
  silently.
- `flake.nix` - the same check, runnable from a clone:
  `nix flake check`.

## Using the rules

Point an `sgconfig.yml` at a checkout:

```yaml
ruleDirs:
  - <this-repo>/rules
```

The Mojo rules additionally need a grammar, because ast-grep has no Mojo
built in. The one these rules are written against is the patched
`tree-sitter-mojo` from
[nix-packages](https://tangled.org/overby.me/nix-packages):

```yaml
customLanguages:
  mojo:
    libraryPath: <nix-packages>#tree-sitter-mojo/lib/mojo.so
    extensions: [mojo]
    expandoChar: _
```

**Register only languages ast-grep does not have built in.** A
`customLanguages` entry for a built-in name (rust, nix, ...) makes every rule
for that language silently match nothing: rules resolve the name through the
built-in table, files resolve it through the custom extension map, and the
two compare unequal. The scan reports zero findings and exits 0, which looks
exactly like a clean tree.

## Running the tests

```sh
ast-grep test -t tests --skip-snapshot-tests
```