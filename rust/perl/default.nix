{
  packages = {
    rust-perl = {
      lib,
      rustPlatform,
    }:
      rustPlatform.buildRustPackage {
        pname = "rust-perl";
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

        meta = {
          description = "A Perl interpreter written in Rust";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/perl";
          license = lib.licenses.mit;
          mainProgram = "perl";
        };
      };

    rust-perl-dev = {
      lib,
      rustPlatform,
    }:
      rustPlatform.buildRustPackage {
        pname = "rust-perl-dev";
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

        meta = {
          description = "A Perl interpreter written in Rust (dev build, fast compile)";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/perl";
          license = lib.licenses.mit;
          mainProgram = "perl";
        };
      };
  };

  checks = let
    # Tests organized by category from the upstream Perl test suite.
    # Start with t/base/ (absolute fundamentals) and t/opbasic/ (core operators),
    # then expand outward as the interpreter matures.
    testDefs = [
      # --- t/base/ (9 tests) ---
      # Absolute basics: if these fail, nothing else matters.
      # These use raw "print ok/not ok" — no test libraries.
      {category = "base"; name = "cond";}
      {category = "base"; name = "if";}
      {category = "base"; name = "lex";}
      {category = "base"; name = "num";}
      {category = "base"; name = "pat";}
      {category = "base"; name = "rs";}
      {category = "base"; name = "term";}
      {category = "base"; name = "translate";}
      {category = "base"; name = "while";}

      # --- t/opbasic/ (5 tests) ---
      # Core operators that t/test.pl itself depends on.
      {category = "opbasic"; name = "arith";}
      {category = "opbasic"; name = "cmp";}
      {category = "opbasic"; name = "concat";}
      {category = "opbasic"; name = "magic_phase";}
      {category = "opbasic"; name = "qq";}

      # --- t/cmd/ (5 tests) ---
      # Control flow: for, elsif, statement modifiers, subroutines.
      {category = "cmd"; name = "elsif";}
      {category = "cmd"; name = "for";}
      {category = "cmd"; name = "mod";}
      {category = "cmd"; name = "subval";}
      {category = "cmd"; name = "switch";}

      # --- t/op/ (selected core tests) ---
      # Operators and builtins — the heart of the language.
      {category = "op"; name = "arith2";}
      {category = "op"; name = "array";}
      {category = "op"; name = "auto";}
      {category = "op"; name = "bop";}
      {category = "op"; name = "chop";}
      {category = "op"; name = "chr";}
      {category = "op"; name = "closure";}
      {category = "op"; name = "cond";}
      {category = "op"; name = "context";}
      {category = "op"; name = "defined";}
      {category = "op"; name = "delete";}
      {category = "op"; name = "die";}
      {category = "op"; name = "do";}
      {category = "op"; name = "each";}
      {category = "op"; name = "eval";}
      {category = "op"; name = "grep";}
      {category = "op"; name = "hash";}
      {category = "op"; name = "heredoc";}
      {category = "op"; name = "inc";}
      {category = "op"; name = "index";}
      {category = "op"; name = "join";}
      {category = "op"; name = "lc";}
      {category = "op"; name = "length";}
      {category = "op"; name = "list";}
      {category = "op"; name = "local";}
      {category = "op"; name = "my";}
      {category = "op"; name = "not";}
      {category = "op"; name = "oct";}
      {category = "op"; name = "ord";}
      {category = "op"; name = "pack";}
      {category = "op"; name = "pos";}
      {category = "op"; name = "print";}
      {category = "op"; name = "push";}
      {category = "op"; name = "quotemeta";}
      {category = "op"; name = "range";}
      {category = "op"; name = "ref";}
      {category = "op"; name = "repeat";}
      {category = "op"; name = "reverse";}
      {category = "op"; name = "sort";}
      {category = "op"; name = "splice";}
      {category = "op"; name = "split";}
      {category = "op"; name = "sprintf";}
      {category = "op"; name = "sub";}
      {category = "op"; name = "substr";}
      {category = "op"; name = "tr";}
      {category = "op"; name = "undef";}
      {category = "op"; name = "unshift";}
      {category = "op"; name = "vec";}
      {category = "op"; name = "wantarray";}

      # --- t/io/ (selected) ---
      {category = "io"; name = "argv";}
      {category = "io"; name = "fs";}
      {category = "io"; name = "open";}
      {category = "io"; name = "print";}
      {category = "io"; name = "read";}
      {category = "io"; name = "tell";}

      # --- t/re/ (selected) ---
      {category = "re"; name = "pat";}
      {category = "re"; name = "regexp";}
      {category = "re"; name = "subst";}

      # --- t/run/ (selected) ---
      {category = "run"; name = "exit";}
      {category = "run"; name = "switches";}
    ];
  in
    builtins.listToAttrs (map (t: {
        name = "rust-perl-test-${t.category}-${t.name}";
        value = pkgs: import ./testsuite.nix {inherit pkgs; inherit (t) category name;};
      })
      testDefs);
}
