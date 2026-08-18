# The Mojo grammar ast-grep loads, so structural rules can reach the 260 .mojo
# files that clippy and statix cannot see.
#
# Four grammars for Mojo exist and three of them are forks of a tree-sitter-python
# that predates modern Mojo. Parsed against this tree's 260 files they leave
# 225 of them broken; this one leaves 8. Star counts run the other way, which is
# why the choice was made by parsing the corpus rather than by reading READMEs.
#
# The patch closes three gaps that the remaining 8 files exposed, all of them
# ordinary Mojo rather than exotica: `@parameter if` (the compile-time
# conditional), `fn(A) -> R` used as a type (the FFI signature idiom, which is
# most of the wasmtime bindings), and backtick raw identifiers (`` `global` ``,
# which escapes a keyword for use as a name). With it applied, 258 of 260 files
# parse clean.
#
# The last two are `return x ^ y`. Upstream removed binary `^` deliberately,
# because `^` is also Mojo's postfix ownership-transfer sigil, and this tree
# uses the sigil at 2431 sites against 2 uses as XOR. Admitting both needs a
# GLR conflict between binary_operator, unary_operator and transfer_expression;
# losing that bet would break the 2431 to fix the 2. Those two expressions parse
# as ERROR, which costs a missed match inside them and nothing else: an ERROR
# node yields no rule hits, never a wrong one.
{
  lib,
  stdenv,
  fetchFromGitHub,
  tree-sitter,
  nodejs,
}:
stdenv.mkDerivation {
  pname = "tree-sitter-mojo";
  version = "0-unstable-2026-05-25";

  src = fetchFromGitHub {
    owner = "oaustegard";
    repo = "tree-sitter-mojo";
    rev = "406abfdf4d2070d742e56d5e74261b5a944f729f";
    hash = "sha256-ifnNO+DHwaSw/jxbVgxIvkqRJE+ONYBp/fWNCFrZ8YQ=";
  };

  patches = [./mojo-grammar.patch];

  nativeBuildInputs = [tree-sitter nodejs];

  # `generate` rewrites src/parser.c from the patched grammar.js. It is the
  # step that fails loudly on an ambiguity, so it must not be folded into a
  # chain whose exit status a later command can mask.
  buildPhase = ''
    runHook preBuild
    # tree-sitter caches the compiled parser under $HOME, which in the sandbox
    # is an unwritable /homeless-shelter.
    export HOME="$TMPDIR"
    tree-sitter generate
    tree-sitter build --output mojo.so
    runHook postBuild
  '';

  installPhase = ''
    runHook preInstall
    install -D -m 755 mojo.so $out/lib/mojo.so
    runHook postInstall
  '';

  meta = {
    description = "Tree-sitter grammar for Mojo, patched for fn-types, @parameter and raw identifiers";
    homepage = "https://github.com/oaustegard/tree-sitter-mojo";
    license = lib.licenses.mit;
    maintainers = with lib.maintainers; [overby-me];
    platforms = lib.platforms.all;
  };
}
