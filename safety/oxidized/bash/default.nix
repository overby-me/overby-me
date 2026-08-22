{lib, ...}: {
  packages = {
    default = {lib, ...}:
      lib.buildCargoProject {
        pname = "oxidized-bash";

        src = lib.fileset.toSource {
          root = ./.;
          fileset = lib.fileset.unions [
            ./Cargo.toml
            ./Cargo.lock
            ./src
          ];
        };

        rootAttrs.postInstall = ''
          ln -s $out/bin/bash $out/bin/sh
        '';

        meta = {
          description = "A Bash-compatible shell written in Rust";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/bash";
          license = lib.licenses.mit;
          mainProgram = "bash";
          platforms = lib.platforms.linux;
        };
      };

    dev = {lib, ...}:
      lib.buildCargoProject {
        pname = "oxidized-bash-dev";

        src = lib.fileset.toSource {
          root = ./.;
          fileset = lib.fileset.unions [
            ./Cargo.toml
            ./Cargo.lock
            ./src
          ];
        };

        release = false;

        rootAttrs.postInstall = ''
          ln -s $out/bin/bash $out/bin/sh
        '';

        meta = {
          description = "A Bash-compatible shell written in Rust (dev build, fast compile)";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/bash";
          license = lib.licenses.mit;
          mainProgram = "bash";
          platforms = lib.platforms.linux;
        };
      };
  };

  checks = let
    testNames = [
      "alias"
      "appendop"
      "arith"
      "arith-for"
      "array"
      "array2"
      "assoc"
      "attr"
      "braces"
      "builtins"
      "case"
      "casemod"
      "comsub"
      "comsub2"
      "comsub-eof"
      "comsub-posix"
      "cond"
      "coproc"
      "cprint"
      "dirstack"
      "dollars"
      "dynvar"
      "errors"
      "execscript"
      "exp-tests"
      "exportfunc"
      "extglob"
      "extglob2"
      "extglob3"
      "func"
      "getopts"
      "glob-bracket"
      "glob-test"
      "globstar"
      "heredoc"
      "herestr"
      "ifs"
      "ifs-posix"
      "input-test"
      "invert"
      "iquote"
      "lastpipe"
      "mapfile"
      "more-exp"
      "nameref"
      "new-exp"
      "nquote"
      "nquote1"
      "nquote2"
      "nquote3"
      "nquote4"
      "nquote5"
      "parser"
      "posix2"
      "posixexp"
      "posixexp2"
      "posixpat"
      "posixpipe"
      "precedence"
      "printf"
      "procsub"
      "quote"
      "quotearray"
      "read"
      "redir"
      "rhs-exp"
      "set-e"
      "set-x"
      "shopt"
      "strip"
      "test"
      "tilde"
      "tilde2"
      "trap"
      "type"
      "varenv"
      "vredir"
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
