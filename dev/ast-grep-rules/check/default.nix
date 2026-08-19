# The structural lint layer as a checked project.
#
# The pre-commit hooks run these rules only against changed files, and the
# rule-test hook fires only when dev/ast-grep-rules itself changes - so a nixpkgs
# bump that updates ast-grep or rebuilds the tree-sitter-mojo grammar could
# break rules with nothing noticing until the next rule edit. A dead rule and
# a clean tree are indistinguishable from scan output, which is how one
# shipped silently once already. This check binds the fixture tests and a
# full-tree scan to exactly the pinned inputs that can break them.
#
# `here` is where this directory sits inside `src`: `dev/ast-grep-rules` when the
# monorepo evaluates it, `.` in the published repo, whose root is this
# directory. The same file serves both, so the check cannot pass in one place
# and rot in the other.
{src, ...}: {
  checks = pkgs: {
    ast-grep-rules = pkgs.stdenv.mkDerivation {
      name = "check-ast-grep-rules";
      inherit src;

      nativeBuildInputs = [pkgs.ast-grep];

      # The generated root sgconfig.yml is gitignored (it carries a store
      # path), so the check writes its own against the same grammar package.
      buildPhase = ''
        export HOME=$TMPDIR
        here=.
        if [ -d dev/ast-grep-rules ]; then here=dev/ast-grep-rules; fi
        cat > sgconfig.yml <<EOF
        ruleDirs:
          - $here/rules
        customLanguages:
          mojo:
            libraryPath: ${pkgs.tree-sitter-mojo}/lib/mojo.so
            extensions: [mojo]
            expandoChar: _
        EOF

        echo "==> Rule fixture tests"
        ast-grep test -t $here/tests --skip-snapshot-tests

        echo "==> Full-tree scan"
        ast-grep scan .
      '';

      installPhase = "touch $out";

      meta.description = "ast-grep rule fixtures + full-tree scan against the pinned engine and grammar";
    };
  };
}
