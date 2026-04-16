{
  packages = {
    rust-grep = {
      lib,
      rustPlatform,
    }:
      rustPlatform.buildRustPackage {
        pname = "rust-grep";
        version = "0.1.0";

        src = lib.fileset.toSource {
          root = ./.;
          fileset = lib.fileset.unions [
            ./Cargo.toml
            ./Cargo.lock
            ./src
          ];
        };

        cargoLock.lockFile = ./Cargo.lock;

        postInstall = ''
          ln -s $out/bin/grep $out/bin/egrep
          ln -s $out/bin/grep $out/bin/fgrep
        '';

        meta = {
          description = "A GNU grep-compatible pattern matching tool written in Rust";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/grep";
          license = lib.licenses.mit;
          mainProgram = "grep";
        };
      };
  };

  checks = let
    testNames = [
      "backref"
      "backref-alt"
      "backref-word"
      "backslash-dot"
      "bre"
      "context-0"
      "count-newline"
      "empty"
      "empty-line"
      "ere"
      "fedora"
      "fgrep-longest"
      "file"
      "foad1"
      "grep-dir"
      "high-bit-range"
      "include-exclude"
      "initial-tab"
      "khadafy"
      "match-lines"
      "max-count-vs-context"
      "multiple-begin-or-end-line"
      "null-byte"
      "options"
      "posix-bracket"
      "r-dot"
      "spencer1"
      "status"
      "two-chars"
      "two-files"
      "word-multi-file"
      "z-anchor-newline"
      # Multibyte / locale-dependent tests
      "backref-multibyte-slow"
      "backslash-s-and-repetition-operators"
      "backslash-s-vs-invalid-multibyte"
      "case-fold-backref"
      "case-fold-backslash-w"
      "case-fold-char-class"
      "case-fold-char-range"
      "case-fold-char-type"
      "case-fold-titlecase"
      "char-class-multibyte"
      "char-class-multibyte2"
      "c-locale"
      "dfaexec-multibyte"
      "empty-line-mb"
      "encoding-error"
      "equiv-classes"
      "euc-mb"
      "false-match-mb-non-utf8"
      "hangul-syllable"
      "mb-dot-newline"
      "mb-non-UTF8-overrun"
      "mb-non-UTF8-word-boundary"
      "multibyte-white-space"
      "prefix-of-multibyte"
      "sjis-mb"
      "surrogate-pair"
      "surrogate-search"
      "turkish-eyes"
      "turkish-I"
      "turkish-I-without-dot"
      "unibyte-binary"
      "unibyte-bracket-expr"
      "unibyte-negated-circumflex"
      "utf8-bracket"
      "word-delim-multibyte"
      "word-multibyte"
      # Color output tests
      "color-colors"
      # PCRE tests
      "pcre"
      "pcre-abort"
      "pcre-ascii-digits"
      "pcre-context"
      "pcre-count"
      "pcre-infloop"
      "pcre-invalid-utf8-infloop"
      "pcre-invalid-utf8-input"
      "pcre-jitstack"
      "pcre-o"
      "pcre-utf8"
      "pcre-utf8-bug224"
      "pcre-utf8-w"
      "pcre-w"
      "pcre-wx-backref"
      "pcre-z"
      # Performance / stress tests
      "big-match"
      "dfa-coverage"
      "dfa-heap-overrun"
      "dfa-infloop"
      "dfa-invalid-utf8"
      "fgrep-infloop"
      "fillbuf-long-line"
      "fmbtest"
      "hash-collision-perf"
      "inconsistent-range"
      "invalid-multibyte-infloop"
      "kwset-abuse"
      "long-pattern-perf"
      "many-regex-performance"
      "repetition-overflow"
      "reversed-range-endpoints"
      "stack-overflow"
      "triple-backref"
      # System / I/O tests
      "binary-file-matches"
      "big-hole"
      "epipe"
      "grep-dev-null"
      "grep-dev-null-out"
      "in-eq-out-infloop"
      "max-count-overread"
      "proc"
      "skip-device"
      "skip-read"
      "symlink"
      "warn-char-classes"
      "write-error-msg"
      # Misc
      "100k-entries"
      "bogus-wctob"
      "no-perl"
      "version-pcre"
      "y2038-vs-32-bit"
    ];
  in
    builtins.listToAttrs (map (name: {
        name = "rust-grep-test-${name}";
        value = pkgs: import ./testsuite.nix {inherit pkgs name;};
      })
      testNames);
}
