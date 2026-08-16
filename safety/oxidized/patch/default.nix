{lib, ...}: {
  packages = {
    oxidized-patch = {lib, ...}:
      lib.buildCargoProject {
        pname = "rust-patch";

        src = lib.fileset.toSource {
          root = ./.;
          fileset = lib.fileset.unions [
            ./Cargo.toml
            ./Cargo.lock
            ./src
          ];
        };

        index = ../../../platform/nix/lib/cargo/index;

        meta = {
          description = "A GNU patch-compatible diff application tool written in Rust";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/patch";
          license = lib.licenses.mit;
          mainProgram = "patch";
          platforms = lib.platforms.linux;
        };
      };

    oxidized-patch-dev = {lib, ...}:
      lib.buildCargoProject {
        pname = "rust-patch-dev";

        src = lib.fileset.toSource {
          root = ./.;
          fileset = lib.fileset.unions [
            ./Cargo.toml
            ./Cargo.lock
            ./src
          ];
        };

        index = ../../../platform/nix/lib/cargo/index;

        release = false;

        meta = {
          description = "A GNU patch-compatible diff application tool written in Rust (dev build, fast compile)";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/patch";
          license = lib.licenses.mit;
          mainProgram = "patch";
          platforms = lib.platforms.linux;
        };
      };
  };

  checks = let
    # Upstream test list from tests/Makefile.am (GNU patch 2.8).
    # Exit 77 from a test counts as skipped (required utility missing).
    testNames = [
      "asymmetric-hunks"
      "backup-prefix-suffix"
      "bad-filenames"
      "bad-usage"
      "concat-git-diff"
      "context-format"
      "copy-rename"
      "corrupt-patch"
      "corrupt-reject-files"
      "create-delete"
      "create-directory"
      "criss-cross"
      "crlf-handling"
      "dash-o-append"
      "deep-directories"
      "ed-style"
      "empty-files"
      "false-match"
      "fifo"
      "file-create-modes"
      "file-modes"
      "filename-choice"
      "garbage"
      "git-binary-diff"
      "git-cleanup"
      "global-reject-files"
      "hardlinks"
      "inname"
      "line-numbers"
      "mangled-numbers-abort"
      "merge"
      "mixed-patch-types"
      "munged-context-format"
      "need-filename"
      "no-backup"
      "no-mode-change-git-diff"
      "no-newline-triggers-assert"
      "preserve-c-function-names"
      "preserve-mode-and-timestamp"
      "quoted-filenames"
      "read-only-files"
      "regression-abe92e8010ab"
      "reject-format"
      "remember-backup-files"
      "remember-reject-files"
      "remove-directories"
      "symlinks"
      "unmodified-files"
      "unusual-blanks"
    ];
  in
    lib.listToAttrs (
      map (name: {
        name = "oxidized-patch-test-${name}";
        value = pkgs: import ./testsuite.nix {inherit pkgs name;};
      })
      testNames
    );
}
