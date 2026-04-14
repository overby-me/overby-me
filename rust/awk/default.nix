{
  packages = {
    rust-awk = {
      lib,
      rustPlatform,
    }:
      rustPlatform.buildRustPackage {
        pname = "rust-awk";
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
          ln -s $out/bin/awk $out/bin/gawk
        '';

        meta = {
          description = "A GNU awk-compatible text processing tool written in Rust";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/awk";
          license = lib.licenses.mit;
          mainProgram = "awk";
        };
      };

    rust-awk-dev = {
      lib,
      rustPlatform,
    }:
      rustPlatform.buildRustPackage {
        pname = "rust-awk-dev";
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
          ln -s $out/bin/awk $out/bin/gawk
        '';

        meta = {
          description = "A GNU awk-compatible text processing tool written in Rust (dev build, fast compile)";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/awk";
          license = lib.licenses.mit;
          mainProgram = "awk";
        };
      };
  };

  checks = let
    testNames = [
      "addcomma"
      "anchgsub"
      "anchor"
      "arrayind1"
      "arrayind2"
      "arrayind3"
      "arrayparm"
      "arrayprm2"
      "arrayprm3"
      "arrayref"
      "arrymem1"
      "arryref2"
      "arryref3"
      "arryref4"
      "arryref5"
      "arynasty"
      "aryprm1"
      "aryprm2"
      "aryprm3"
      "aryprm4"
      "aryprm5"
      "aryprm6"
      "aryprm7"
      "aryprm8"
      "aryprm9"
      "arysubnm"
      "aryunasgn"
      "asgext"
      "assignnumfield"
      "assignnumfield2"
      "back89"
      "backgsub"
      "badassign1"
      "badbuild"
      "callparam"
      "childin"
      "close_status"
      "closebad"
      "clsflnam"
      "compare2"
      "concat1"
      "concat2"
      "concat3"
      "concat4"
      "concat5"
      "convfmt"
      "datanonl"
      "delargv"
      "delarpm2"
      "delarprm"
      "delfunc"
      "dfacheck2"
      "dfamb1"
      "dfastress"
      "divzero"
      "divzero2"
      "dynlj"
      "eofsplit"
      "exit2"
      "exitval2"
      "exitval3"
      "fcall_exit"
      "fcall_exit2"
      "fieldassign"
      "fldchg"
      "fldchgnf"
      "fldterm"
      "fnamedat"
      "fnarray"
      "fnarray2"
      "fnaryscl"
      "fnasgnm"
      "fnmisc"
      "fordel"
      "forref"
      "forsimp"
      "fsbs"
      "fscaret"
      "fsnul1"
      "fsrs"
      "fstabplus"
      "funsemnl"
      "funsmnam"
      "funstack"
      "getline"
      "getline3"
      "getline4"
      "getline5"
      "getlnfa"
      "getnr2tb"
      "getnr2tm"
      "gsubasgn"
      "gsubnulli18n"
      "gsubtest"
      "gsubtst2"
      "gsubtst4"
      "gsubtst5"
      "gsubtst6"
      "gsubtst7"
      "gsubtst8"
      "hex"
      "hex2"
      "hsprint"
      "inpref"
      "inputred"
      "intest"
      "intprec"
      "iobug1"
      "leaddig"
      "leadnl"
      "longsub"
      "manglprm"
      "match4"
      "matchuninitialized"
      "math"
      "memleak"
      "membug1"
      "minusstr"
      "mmap8k"
      "nasty"
      "nasty2"
      "negexp"
      "negrange"
      "nested"
      "nfldstr"
      "nfloop"
      "nfneg"
      "nfset"
      "nlfldsep"
      "nlinstr"
      "nlstrina"
      "noloop1"
      "noloop2"
      "noparms"
      "nulinsrc"
      "nulrsend"
      "numindex"
      "numrange"
      "numstr1"
      "numsubstr"
      "octsub"
      "ofmt"
      "ofmta"
      "ofmtbig"
      "ofmtfidl"
      "ofmts"
      "ofmtstrnum"
      "ofs1"
      "onlynl"
      "opasnidx"
      "opasnslf"
      "paramdup"
      "paramres"
      "paramtyp"
      "paramuninitglobal"
      "parse1"
      "parsefld"
      "parseme"
      "pcntplus"
      "prdupval"
      "prec"
      "printf-corners"
      "printf1"
      "printfchar"
      "prmarscl"
      "prmreuse"
      "prt1eval"
      "prtoeval"
      "rand"
      "range1"
      "range2"
      "readbuf"
      "rebrackloc"
      "rebt8b1"
      "rebuild"
      "regeq"
      "regex3minus"
      "regexpbad"
      "regexpbrack"
      "regexpbrack2"
      "regexprange"
      "regrange"
      "reindops"
      "reparse"
      "resplit"
      "rri1"
      "rs"
      "rsnul1nl"
      "rsnullre"
      "rsnulw"
      "rstest1"
      "rstest2"
      "rstest3"
      "rstest4"
      "rstest5"
      "rswhite"
      "scalar"
      "sclforin"
      "sclifin"
      "setrec0"
      "setrec1"
      "sigpipe1"
      "sortempty"
      "sortglos"
      "splitargv"
      "splitarr"
      "splitdef"
      "splitvar"
      "splitwht"
      "splitwht2"
      "status-close"
      "strcat1"
      "strfieldnum"
      "strnum1"
      "strnum2"
      "strsubscript"
      "strtod"
      "subamp"
      "subback"
      "subi18n"
      "subsepnm"
      "subslash"
      "substr"
      "swaplns"
      "synerr1"
      "synerr2"
      "synerr3"
      "tailrecurse"
      "trailbs"
      "unterm"
      "uparrfs"
      "uplus"
      "wideidx"
      "wideidx2"
      "widesub"
      "widesub2"
      "widesub3"
      "widesub4"
      "wjposer1"
      "zero2"
      "zeroe0"
      "zeroflag"
    ];
  in
    builtins.listToAttrs (map (name: {
        name = "rust-awk-test-${name}";
        value = pkgs: import ./testsuite.nix {inherit pkgs name;};
      })
      testNames);
}
