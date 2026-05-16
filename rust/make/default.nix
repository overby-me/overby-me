{
  packages = {
    rust-make = {
      lib,
      rustPlatform,
    }:
      rustPlatform.buildRustPackage {
        pname = "rust-make";
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
          ln -s $out/bin/make $out/bin/gmake
        '';

        meta = {
          description = "A GNU Make-compatible build system driver written in Rust";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/make";
          license = lib.licenses.mit;
          mainProgram = "make";
        };
      };

    rust-make-dev = {
      lib,
      rustPlatform,
    }:
      rustPlatform.buildRustPackage {
        pname = "rust-make-dev";
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
          ln -s $out/bin/make $out/bin/gmake
        '';

        meta = {
          description = "A GNU Make-compatible build system driver written in Rust (dev build, fast compile)";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/make";
          license = lib.licenses.mit;
          mainProgram = "make";
        };
      };
  };

  checks = let
    testNames = [
      # features (42)
      "features/archives"
      "features/comments"
      "features/conditionals"
      "features/default_names"
      "features/dircache"
      "features/double_colon"
      "features/echoing"
      "features/errors"
      "features/escape"
      "features/exec"
      "features/export"
      "features/grouped_targets"
      "features/implicit_search"
      "features/include"
      "features/jobserver"
      "features/load"
      "features/loadapi"
      "features/mult_rules"
      "features/mult_targets"
      "features/order_only"
      "features/output-sync"
      "features/override"
      "features/parallelism"
      "features/patspecific_vars"
      "features/patternrules"
      "features/quoting"
      "features/recursion"
      "features/reinvoke"
      "features/rule_glob"
      "features/se_explicit"
      "features/se_implicit"
      "features/se_statpat"
      "features/shell_assignment"
      "features/statipattrules"
      "features/suffixrules"
      "features/targetvars"
      "features/temp_stdin"
      "features/utf8"
      "features/varnesting"
      "features/vpath"
      "features/vpathgpath"
      "features/vpathplus"
      # functions (31)
      "functions/abspath"
      "functions/addprefix"
      "functions/addsuffix"
      "functions/andor"
      "functions/basename"
      "functions/call"
      "functions/dir"
      "functions/error"
      "functions/eval"
      "functions/file"
      "functions/filter-out"
      "functions/findstring"
      "functions/flavor"
      "functions/foreach"
      "functions/guile"
      "functions/if"
      "functions/intcmp"
      "functions/join"
      "functions/let"
      "functions/notdir"
      "functions/origin"
      "functions/realpath"
      "functions/shell"
      "functions/sort"
      "functions/strip"
      "functions/substitution"
      "functions/suffix"
      "functions/value"
      "functions/warning"
      "functions/wildcard"
      "functions/word"
      # misc (9)
      "misc/bs-nl"
      "misc/close_stdout"
      "misc/failure"
      "misc/fopen-fail"
      "misc/general1"
      "misc/general2"
      "misc/general3"
      "misc/general4"
      "misc/utf8"
      # options (20)
      "options/dash-B"
      "options/dash-C"
      "options/dash-d"
      "options/dash-e"
      "options/dash-f"
      "options/dash-I"
      "options/dash-k"
      "options/dash-l"
      "options/dash-n"
      "options/dash-q"
      "options/dash-r"
      "options/dash-s"
      "options/dash-t"
      "options/dash-W"
      "options/eval"
      "options/general"
      "options/print-directory"
      "options/shuffle"
      "options/symlinks"
      "options/warn-undefined-variables"
      # targets (12)
      "targets/clean"
      "targets/DEFAULT"
      "targets/DELETE_ON_ERROR"
      "targets/FORCE"
      "targets/INTERMEDIATE"
      "targets/NOTINTERMEDIATE"
      "targets/ONESHELL"
      "targets/PHONY"
      "targets/POSIX"
      "targets/SECONDARY"
      "targets/SILENT"
      "targets/WAIT"
      # variables (21)
      "variables/automatic"
      "variables/CURDIR"
      "variables/DEFAULT_GOAL"
      "variables/define"
      "variables/EXTRA_PREREQS"
      "variables/flavors"
      "variables/GNUMAKEFLAGS"
      "variables/INCLUDE_DIRS"
      "variables/LIBPATTERNS"
      "variables/MAKE"
      "variables/MAKECMDGOALS"
      "variables/MAKEFILES"
      "variables/MAKEFLAGS"
      "variables/MAKELEVEL"
      "variables/MAKE_RESTARTS"
      "variables/MFILE_LIST"
      "variables/negative"
      "variables/private"
      "variables/SHELL"
      "variables/special"
      "variables/undefine"
    ];
    # Flatten "features/comments" to attr "rust-make-test-features-comments".
    mkCheck = path: let
      parts = builtins.split "/" path;
      category = builtins.elemAt parts 0;
      name = builtins.elemAt parts 2;
    in {
      name = "rust-make-test-${category}-${name}";
      value = pkgs: import ./testsuite.nix {inherit pkgs category name;};
    };
  in
    builtins.listToAttrs (map mkCheck testNames);
}
