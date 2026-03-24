{
  packages = {
    rust-bash = {
      lib,
      rustPlatform,
    }:
      rustPlatform.buildRustPackage {
        pname = "rust-bash";
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
          ln -s $out/bin/bash $out/bin/sh
        '';

        meta = {
          description = "A Bash-compatible shell written in Rust";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/bash";
          license = lib.licenses.mit;
          mainProgram = "bash";
        };
      };

    rust-bash-dev = {
      lib,
      rustPlatform,
    }:
      rustPlatform.buildRustPackage {
        pname = "rust-bash-dev";
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

        buildType = "debug";

        postInstall = ''
          ln -s $out/bin/bash $out/bin/sh
        '';

        meta = {
          description = "A Bash-compatible shell written in Rust (dev build, fast compile)";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/bash";
          license = lib.licenses.mit;
          mainProgram = "bash";
        };
      };

    rust-bash-drowse = {
      drowse,
      lib,
    }:
      drowse.crate2nix {
        pname = "rust-bash";
        version = "0.1.0";

        src = lib.fileset.toSource {
          root = ./.;
          fileset = lib.fileset.unions [
            ./Cargo.toml
            ./Cargo.lock
            ./src
          ];
        };

        select = ''
          project:
          let
            pkgs = import <nixpkgs> {};
            build = project.workspaceMembers.rust-bash.build;
          in
          pkgs.runCommand "rust-bash-0.1.0" {} '''
            mkdir -p $out/bin
            cp -a ''${build}/bin/bash $out/bin/bash
            ln -s $out/bin/bash $out/bin/sh
          '''
        '';

        doCheck = false;

        meta = {
          description = "A Bash-compatible shell written in Rust (incremental crate-level build)";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/bash";
          license = lib.licenses.mit;
          mainProgram = "bash";
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
    builtins.listToAttrs (map (name: {
        name = "rust-bash-test-${name}";
        value = pkgs: import ./testsuite.nix {inherit pkgs name;};
      })
      testNames);
}
