{
  packages = {
    rust-tar = {
      lib,
      rustPlatform,
    }:
      rustPlatform.buildRustPackage {
        pname = "rust-tar";
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
          description = "A GNU tar-compatible archive tool written in Rust";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/tar";
          license = lib.licenses.mit;
          mainProgram = "tar";
        };
      };

    rust-tar-dev = {
      lib,
      rustPlatform,
    }:
      rustPlatform.buildRustPackage {
        pname = "rust-tar-dev";
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
          description = "A GNU tar-compatible archive tool written in Rust (dev build, fast compile)";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/tar";
          license = lib.licenses.mit;
          mainProgram = "tar";
        };
      };

    # Shared helper: a prepared GNU tar source tree with the autom4te-
    # generated `tests/testsuite` script and the compiled helper programs
    # (`genfile`, `checkseekhole`, `ckmtime`). Used by `testsuite.nix` as
    # the base tree for every per-test derivation so we only run the
    # autoconf + build chain once.
    gnutar-test-harness = {
      stdenv,
      gnutar,
      autoconf,
      automake,
      m4,
      gawk,
      gettext,
      help2man,
      texinfo,
      perl,
      bison,
      pkg-config,
    }:
      stdenv.mkDerivation {
        pname = "gnutar-test-harness";
        version = "1.35";
        inherit (gnutar) src;

        nativeBuildInputs = [
          autoconf
          automake
          m4
          gawk
          gettext
          help2man
          texinfo
          perl
          bison
          pkg-config
        ];

        dontConfigure = false;
        dontBuild = false;

        # Build the core tar and test helper C programs (genfile,
        # checkseekhole, ckmtime) that the testsuite invokes.
        buildPhase = ''
          runHook preBuild
          make
          make -C tests genfile checkseekhole ckmtime
          runHook postBuild
        '';

        # We keep everything: `src/tar` (stub; will be replaced with
        # rust-tar at test time), the `tests/` helper programs, and the
        # autom4te-generated `tests/testsuite` script.
        installPhase = ''
          runHook preInstall
          mkdir -p $out/tar-1.35
          cp -r . $out/tar-1.35/
          # Strip generated object files and binaries that would bloat
          # the store but keep `tests/testsuite` (shell script) and the
          # helper programs under `tests/` that are needed at runtime.
          find $out/tar-1.35 -name '*.o' -delete
          find $out/tar-1.35 -name '*.lo' -delete
          find $out/tar-1.35 -name '.deps' -type d -exec rm -rf {} + 2>/dev/null || true
          runHook postInstall
        '';

        meta.description = "GNU tar 1.35 source with tests/testsuite prebuilt";
      };
  };

  checks = let
    # The 225 upstream Autotest names (one per `tests/*.at` in tar-1.35,
    # excluding `testsuite.at` itself).  Keep alphabetical.
    testNames = [
      "acls01"
      "acls02"
      "acls03"
      "add-file"
      "append"
      "append01"
      "append02"
      "append03"
      "append04"
      "append05"
      "backup01"
      "capabs_raw01"
      "chtype"
      "comperr"
      "comprec"
      "delete01"
      "delete02"
      "delete03"
      "delete04"
      "delete05"
      "delete06"
      "difflink"
      "dirrem01"
      "dirrem02"
      "exclude"
      "exclude01"
      "exclude02"
      "exclude03"
      "exclude04"
      "exclude05"
      "exclude06"
      "exclude07"
      "exclude08"
      "exclude09"
      "exclude10"
      "exclude11"
      "exclude12"
      "exclude13"
      "exclude14"
      "exclude15"
      "exclude16"
      "extrac01"
      "extrac02"
      "extrac03"
      "extrac04"
      "extrac05"
      "extrac06"
      "extrac07"
      "extrac08"
      "extrac09"
      "extrac10"
      "extrac11"
      "extrac12"
      "extrac13"
      "extrac14"
      "extrac15"
      "extrac16"
      "extrac17"
      "extrac18"
      "extrac19"
      "extrac20"
      "extrac21"
      "extrac22"
      "extrac23"
      "extrac24"
      "extrac25"
      "filerem01"
      "filerem02"
      "grow"
      "gzip"
      "ignfail"
      "incr01"
      "incr02"
      "incr03"
      "incr04"
      "incr05"
      "incr06"
      "incr07"
      "incr08"
      "incr09"
      "incr10"
      "incr11"
      "incremental"
      "indexfile"
      "label01"
      "label02"
      "label03"
      "label04"
      "label05"
      "link01"
      "link02"
      "link03"
      "link04"
      "listed01"
      "listed02"
      "listed03"
      "listed04"
      "listed05"
      "long01"
      "longv7"
      "lustar01"
      "lustar02"
      "lustar03"
      "map"
      "multiv01"
      "multiv02"
      "multiv03"
      "multiv04"
      "multiv05"
      "multiv06"
      "multiv07"
      "multiv08"
      "multiv09"
      "multiv10"
      "numeric"
      "old"
      "onetop01"
      "onetop02"
      "onetop03"
      "onetop04"
      "onetop05"
      "opcomp01"
      "opcomp02"
      "opcomp03"
      "opcomp04"
      "opcomp05"
      "opcomp06"
      "options"
      "options02"
      "options03"
      "owner"
      "pipe"
      "positional01"
      "positional02"
      "positional03"
      "recurs02"
      "recurse"
      "remfiles01"
      "remfiles02"
      "remfiles03"
      "remfiles04a"
      "remfiles04b"
      "remfiles04c"
      "remfiles05a"
      "remfiles05b"
      "remfiles05c"
      "remfiles06a"
      "remfiles06b"
      "remfiles06c"
      "remfiles07a"
      "remfiles07b"
      "remfiles07c"
      "remfiles08a"
      "remfiles08b"
      "remfiles08c"
      "remfiles09a"
      "remfiles09b"
      "remfiles09c"
      "remfiles10"
      "rename01"
      "rename02"
      "rename03"
      "rename04"
      "rename05"
      "rename06"
      "same-order01"
      "same-order02"
      "selacl01"
      "selnx01"
      "shortfile"
      "shortrec"
      "shortupd"
      "sigpipe"
      "sparse01"
      "sparse02"
      "sparse03"
      "sparse04"
      "sparse05"
      "sparse06"
      "sparse07"
      "sparsemv"
      "spmvp00"
      "spmvp01"
      "spmvp10"
      "sptrcreat"
      "sptrdiff00"
      "sptrdiff01"
      "T-cd"
      "T-dir00"
      "T-dir01"
      "T-empty"
      "T-mult"
      "T-nest"
      "T-nonl"
      "T-null"
      "T-null2"
      "T-rec"
      "T-recurse"
      "T-zfile"
      "time01"
      "time02"
      "truncate"
      "update"
      "update01"
      "update02"
      "update03"
      "update04"
      "verbose"
      "verify"
      "version"
      "volsize"
      "volume"
      "xattr01"
      "xattr02"
      "xattr03"
      "xattr04"
      "xattr05"
      "xattr06"
      "xattr07"
      "xattr08"
      "xform-h"
      "xform01"
      "xform02"
      "xform03"
    ];
  in
    builtins.listToAttrs (map (name: {
        name = "rust-tar-test-${name}";
        value = pkgs: import ./testsuite.nix {inherit pkgs name;};
      })
      testNames);
}
