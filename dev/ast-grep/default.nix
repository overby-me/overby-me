# The structural lint layer as a checked project.
#
# The pre-commit hooks run these rules only against changed files, and the
# rule-test hook fires only when dev/ast-grep itself changes - so a nixpkgs
# bump that updates ast-grep or rebuilds the tree-sitter-mojo grammar could
# break rules with nothing noticing until the next rule edit. A dead rule and
# a clean tree are indistinguishable from scan output, which is how one
# shipped silently once already. This check binds the fixture tests and a
# full-tree scan to exactly the pinned inputs that can break them.
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
        cat > sgconfig.yml <<EOF
        ruleDirs:
          - dev/ast-grep/rules
        customLanguages:
          mojo:
            libraryPath: ${pkgs.tree-sitter-mojo}/lib/mojo.so
            extensions: [mojo]
            expandoChar: _
        EOF

        echo "==> Rule fixture tests"
        ast-grep test -t dev/ast-grep/tests --skip-snapshot-tests

        echo "==> Full-tree scan"
        ast-grep scan .
      '';

      installPhase = "touch $out";

      meta.description = "ast-grep rule fixtures + full-tree scan against the pinned engine and grammar";
    };
  };
}
