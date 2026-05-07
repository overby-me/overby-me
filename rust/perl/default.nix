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
      {
        category = "base";
        name = "cond";
      }
      {
        category = "base";
        name = "if";
      }
      {
        category = "base";
        name = "lex";
      }
      {
        category = "base";
        name = "num";
      }
      {
        category = "base";
        name = "pat";
      }
      {
        category = "base";
        name = "rs";
      }
      {
        category = "base";
        name = "term";
      }
      {
        category = "base";
        name = "translate";
      }
      {
        category = "base";
        name = "while";
      }

      # --- t/opbasic/ (5 tests) ---
      # Core operators that t/test.pl itself depends on.
      {
        category = "opbasic";
        name = "arith";
      }
      {
        category = "opbasic";
        name = "cmp";
      }
      {
        category = "opbasic";
        name = "concat";
      }
      {
        category = "opbasic";
        name = "magic_phase";
      }
      {
        category = "opbasic";
        name = "qq";
      }

      # --- t/cmd/ (5 tests) ---
      # Control flow: for, elsif, statement modifiers, subroutines.
      {
        category = "cmd";
        name = "elsif";
      }
      {
        category = "cmd";
        name = "for";
      }
      {
        category = "cmd";
        name = "mod";
      }
      {
        category = "cmd";
        name = "subval";
      }
      {
        category = "cmd";
        name = "switch";
      }

      # --- t/op/ (selected core tests) ---
      # Operators and builtins — the heart of the language.
      {
        category = "op";
        name = "arith2";
      }
      {
        category = "op";
        name = "array";
      }
      {
        category = "op";
        name = "auto";
      }
      {
        category = "op";
        name = "bop";
      }
      {
        category = "op";
        name = "chop";
      }
      {
        category = "op";
        name = "chr";
      }
      {
        category = "op";
        name = "closure";
      }
      {
        category = "op";
        name = "cond";
      }
      {
        category = "op";
        name = "context";
      }
      {
        category = "op";
        name = "defined";
      }
      {
        category = "op";
        name = "delete";
      }
      {
        category = "op";
        name = "die";
      }
      {
        category = "op";
        name = "do";
      }
      {
        category = "op";
        name = "each";
      }
      {
        category = "op";
        name = "eval";
      }
      {
        category = "op";
        name = "grep";
      }
      {
        category = "op";
        name = "hash";
      }
      {
        category = "op";
        name = "heredoc";
      }
      {
        category = "op";
        name = "inc";
      }
      {
        category = "op";
        name = "index";
      }
      {
        category = "op";
        name = "join";
      }
      {
        category = "op";
        name = "lc";
      }
      {
        category = "op";
        name = "length";
      }
      {
        category = "op";
        name = "list";
      }
      {
        category = "op";
        name = "local";
      }
      {
        category = "op";
        name = "my";
      }
      {
        category = "op";
        name = "not";
      }
      {
        category = "op";
        name = "oct";
      }
      {
        category = "op";
        name = "ord";
      }
      {
        category = "op";
        name = "pack";
      }
      {
        category = "op";
        name = "pos";
      }
      {
        category = "op";
        name = "print";
      }
      {
        category = "op";
        name = "push";
      }
      {
        category = "op";
        name = "quotemeta";
      }
      {
        category = "op";
        name = "range";
      }
      {
        category = "op";
        name = "ref";
      }
      {
        category = "op";
        name = "repeat";
      }
      {
        category = "op";
        name = "reverse";
      }
      {
        category = "op";
        name = "sort";
      }
      {
        category = "op";
        name = "splice";
      }
      {
        category = "op";
        name = "split";
      }
      {
        category = "op";
        name = "sprintf";
      }
      {
        category = "op";
        name = "sub";
      }
      {
        category = "op";
        name = "substr";
      }
      {
        category = "op";
        name = "tr";
      }
      {
        category = "op";
        name = "undef";
      }
      {
        category = "op";
        name = "unshift";
      }
      {
        category = "op";
        name = "vec";
      }
      {
        category = "op";
        name = "wantarray";
      }
      {
        category = "op";
        name = "avhv";
      }
      {
        category = "op";
        name = "coresubs";
      }
      {
        category = "op";
        name = "dbm";
      }
      {
        category = "op";
        name = "overload_integer";
      }
      {
        category = "op";
        name = "svflags";
      }
      {
        category = "op";
        name = "taint";
      }
      {
        category = "op";
        name = "tie_fetch_count";
      }
      {
        category = "op";
        name = "tiehandle";
      }
      {
        category = "op";
        name = "utftaint";
      }
      {
        category = "op";
        name = "mydef";
      }
      {
        category = "op";
        name = "append";
      }
      {
        category = "op";
        name = "aassign";
      }
      {
        category = "op";
        name = "attrhand";
      }
      {
        category = "op";
        name = "chained";
      }
      {
        category = "op";
        name = "chdir";
      }
      {
        category = "op";
        name = "concat";
      }
      {
        category = "op";
        name = "eq";
      }
      {
        category = "op";
        name = "exists";
      }
      {
        category = "op";
        name = "exp";
      }
      {
        category = "op";
        name = "feature_class";
      }
      {
        category = "op";
        name = "fork";
      }
      {
        category = "op";
        name = "getpid";
      }
      {
        category = "op";
        name = "glob";
      }
      {
        category = "op";
        name = "goto";
      }
      {
        category = "op";
        name = "inccode";
      }
      {
        category = "op";
        name = "incfilter";
      }
      {
        category = "op";
        name = "lc_mg";
      }
      {
        category = "op";
        name = "let";
      }
      {
        category = "op";
        name = "lexsub";
      }
      {
        category = "op";
        name = "lfs";
      }
      {
        category = "op";
        name = "mul";
      }
      {
        category = "op";
        name = "multideref";
      }
      {
        category = "op";
        name = "my_stash";
      }
      {
        category = "op";
        name = "myinit";
      }
      {
        category = "op";
        name = "overload_int";
      }
      {
        category = "op";
        name = "pat_rt_report";
      }
      {
        category = "op";
        name = "sprintf2";
      }
      {
        category = "op";
        name = "stash";
      }
      {
        category = "op";
        name = "stat";
      }
      {
        category = "op";
        name = "streaming";
      }
      {
        category = "op";
        name = "study";
      }
      {
        category = "op";
        name = "svleak";
      }
      {
        category = "op";
        name = "turkish";
      }
      {
        category = "op";
        name = "utf8cache";
      }
      {
        category = "op";
        name = "wantarray_thr";
      }
      {
        category = "op";
        name = "64bitint";
      }
      {
        category = "op";
        name = "alarm";
      }
      {
        category = "op";
        name = "blocks";
      }
      {
        category = "op";
        name = "dump";
      }
      {
        category = "op";
        name = "groups";
      }
      {
        category = "op";
        name = "hexfp";
      }
      {
        category = "op";
        name = "int";
      }
      {
        category = "op";
        name = "isa";
      }
      {
        category = "op";
        name = "numify_chkflags";
      }
      {
        category = "op";
        name = "override";
      }
      {
        category = "op";
        name = "rand";
      }
      {
        category = "op";
        name = "readdir";
      }
      {
        category = "op";
        name = "readline";
      }
      {
        category = "op";
        name = "readline_nb";
      }
      {
        category = "op";
        name = "smartmatch";
      }
      {
        category = "op";
        name = "stat_errors";
      }
      {
        category = "op";
        name = "substr_left";
      }
      {
        category = "op";
        name = "goto_xs";
      }
      {
        category = "op";
        name = "hash-clear-placeholders";
      }
      {
        category = "op";
        name = "hash-rt85026";
      }

      # --- t/comp/ (selected) ---
      {
        category = "comp";
        name = "ourwarn";
      }
      {
        category = "comp";
        name = "uxxxxx";
      }

      # --- t/lib/ (selected) ---
      {
        category = "lib";
        name = "POSIX";
      }
      {
        category = "lib";
        name = "locale";
      }
      {
        category = "lib";
        name = "strict";
      }
      {
        category = "lib";
        name = "warnings";
      }

      # --- t/mro/ (selected) ---
      {
        category = "mro";
        name = "basic";
      }
      {
        category = "mro";
        name = "c3";
      }
      {
        category = "mro";
        name = "dbic";
      }
      {
        category = "mro";
        name = "inconsistent_MRO";
      }
      {
        category = "mro";
        name = "next";
      }
      {
        category = "mro";
        name = "recursion";
      }
      {
        category = "mro";
        name = "recurse";
      }
      {
        category = "mro";
        name = "recursion_c3";
      }

      # --- t/io/ (selected) ---
      {
        category = "io";
        name = "binmode";
      }
      {
        category = "io";
        name = "closepid";
      }
      {
        category = "io";
        name = "dup";
      }
      {
        category = "io";
        name = "iofile";
      }
      {
        category = "io";
        name = "openpid";
      }
      {
        category = "io";
        name = "pipe";
      }
      {
        category = "io";
        name = "say";
      }
      {
        category = "io";
        name = "scalar";
      }
      {
        category = "io";
        name = "scalar_ungetc";
      }
      {
        category = "io";
        name = "socket";
      }
      {
        category = "io";
        name = "socketpair";
      }
      {
        category = "io";
        name = "argv";
      }
      {
        category = "io";
        name = "fs";
      }
      {
        category = "io";
        name = "open";
      }
      {
        category = "io";
        name = "print";
      }
      {
        category = "io";
        name = "read";
      }
      {
        category = "io";
        name = "tell";
      }

      # --- t/re/ (selected) ---
      {
        category = "re";
        name = "pat";
      }
      {
        category = "re";
        name = "regexp";
      }
      {
        category = "re";
        name = "subst";
      }
      {
        category = "re";
        name = "anyof";
      }
      {
        category = "re";
        name = "charset";
      }
      {
        category = "re";
        name = "no_utf8_pm";
      }
      {
        category = "re";
        name = "qrstack";
      }
      {
        category = "re";
        name = "regexp_email";
      }
      {
        category = "re";
        name = "regexp_email_full";
      }
      {
        category = "re";
        name = "reg_fold";
      }
      {
        category = "re";
        name = "reg_x";
      }
      {
        category = "re";
        name = "runtime";
      }
      {
        category = "re";
        name = "regex_sets";
      }

      # --- t/run/ (selected) ---
      {
        category = "run";
        name = "exit";
      }
      {
        category = "run";
        name = "switches";
      }
      {
        category = "run";
        name = "dtrace";
      }
      {
        category = "run";
        name = "locale";
      }
      {
        category = "run";
        name = "runenv_hashseed";
      }
      {
        category = "run";
        name = "runenv_randseed";
      }
      {
        category = "run";
        name = "runenv";
      }
      {
        category = "run";
        name = "switchDx";
      }
      {
        category = "run";
        name = "switch-I-and-M";
      }
      {
        category = "run";
        name = "switchM";
      }
      {
        category = "run";
        name = "todo";
      }
    ];
  in
    builtins.listToAttrs (map (t: {
        name = "rust-perl-test-${t.category}-${t.name}";
        value = pkgs:
          import ./testsuite.nix {
            inherit pkgs;
            inherit (t) category name;
          };
      })
      testDefs);
}
