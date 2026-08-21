{lib, ...}: {
  packages = {
    default = {lib, ...}:
      lib.buildCargoProject {
        pname = "rust-sed";

        src = lib.fileset.toSource {
          root = ./.;
          fileset = lib.fileset.unions [
            ./Cargo.toml
            ./Cargo.lock
            ./src
          ];
        };

        index = ../../../platform/nix/lib/lib/cargo/index;

        meta = {
          description = "A GNU sed-compatible stream editor written in Rust";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/sed";
          license = lib.licenses.mit;
          mainProgram = "sed";
          platforms = lib.platforms.linux;
        };
      };

    dev = {lib, ...}:
      lib.buildCargoProject {
        pname = "rust-sed-dev";

        src = lib.fileset.toSource {
          root = ./.;
          fileset = lib.fileset.unions [
            ./Cargo.toml
            ./Cargo.lock
            ./src
          ];
        };

        index = ../../../platform/nix/lib/lib/cargo/index;

        release = false;

        meta = {
          description = "A GNU sed-compatible stream editor written in Rust (dev build, fast compile)";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/sed";
          license = lib.licenses.mit;
          mainProgram = "sed";
          platforms = lib.platforms.linux;
        };
      };
  };

  checks = let
    testNames = [
      "8to7"
      "binary"
      "bsd"
      "bsd-wrapper"
      "bug32082"
      "bug32271-1"
      "bug32271-2"
      "cmd-0r"
      "cmd-l"
      "cmd-R"
      "colon-with-no-label"
      "command-endings"
      "comment-n"
      "compile-errors"
      "compile-tests"
      "convert-number"
      "dc"
      "distrib"
      "eval"
      "execute-tests"
      "follow-symlinks"
      "follow-symlinks-stdin"
      "help"
      "in-place-hyphen"
      "in-place-suffix-backup"
      "inplace-hold"
      "inplace-selinux"
      "mac-mf"
      "madding"
      "mb-bad-delim"
      "mb-charclass-non-utf8"
      "mb-match-slash"
      "mb-y-translate"
      "missing-filename"
      "newline-dfa-bug"
      "normalize-text"
      "nulldata"
      "obinary"
      "panic-tests"
      "posix-char-class"
      "posix-mode-addr"
      "posix-mode-bad-ref"
      "posix-mode-ERE"
      "posix-mode-N"
      "posix-mode-s"
      "range-overlap"
      "recursive-escape-c"
      "regex-errors"
      "regex-max-int"
      "sandbox"
      "stdin"
      "stdin-prog"
      "subst-mb-incomplete"
      "subst-options"
      "subst-replacement"
      "temp-file-cleanup"
      "title-case"
      "unbuffered"
      "uniq"
      "word-delim"
      "xemacs"
    ];
  in
    lib.listToAttrs (
      map (name: {
        name = "test-${name}";
        value = pkgs: import ./testsuite.nix {inherit pkgs name;};
      })
      testNames
    );
}
