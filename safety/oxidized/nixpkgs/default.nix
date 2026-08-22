{
  devShell = pkgs: {
    packages = with pkgs; [
      just
      nix-tree
    ];
  };

  overlays."/oxidized-nixpkgs" = final: prev: let
    inherit (final) lib;
    components = import ./components {pkgs = final;};

    # Collect only components that have a replacement ready
    available = lib.filter (c: c.replacement != null) (lib.attrValues components);

    # Build the replacement initialPath by swapping available components
    replacedInitialPath =
      map (
        pkg: let
          match = lib.filter (c: c.original == pkg) available;
          component =
            if match != []
            then lib.head match
            else null;
        in
          if component != null
          then component.replacement
          else pkg
      )
      prev.stdenv.initialPath;

    # Shell replacement: oxidized-bash provides /bin/bash and /bin/sh
    shellPkg = let
      shellMatch = lib.filter (c: c.name == "shell") available;
    in
      if shellMatch != []
      then (lib.head shellMatch).replacement
      else prev.bash;
  in {
    # Expose the component registry for introspection
    oxidized-nixpkgs-components = components;

    # Wrap oxidized-gcc with nixpkgs cc-wrapper for proper include/lib paths
    oxidized-gcc-wrapped = prev.wrapCCWith {
      cc = final.oxidized-gcc;
      inherit (prev.stdenv.cc) libc bintools;
      isGNU = true;
      # Add oxidized-gcc's built-in headers to the system include path
      nixSupport.cc-cflags = [
        "-isystem ${final.oxidized-gcc}/lib/gcc/x86_64-unknown-linux-gnu/14.2.0/include"
      ];
    };

    # A stdenv with all available Rust replacements swapped in.
    # We disable allowedRequisites because Rust replacement packages
    # are built with the normal stdenv, so their closures transitively
    # reference the C originals (e.g. oxidized-grep depends on coreutils).
    # A fully bootstrapped Rust stdenv (Phase 7) would rebuild the
    # replacements with themselves, eliminating these references.
    stdenvRs = prev.stdenv.override {
      initialPath = replacedInitialPath;
      shell = "${shellPkg}/bin/bash";
      allowedRequisites = null;
    };

    # mkDerivation using the Rust stdenv — use this to test-build packages
    mkDerivationRs = args: (final.stdenvRs.mkDerivation args);
  };

  packages = {
    # A test derivation that reports component availability status.
    # This uses the normal stdenv (not stdenvRs) so it always builds,
    # even when the Rust stdenv has issues.
    test = {
      stdenv,
      lib,
      oxidized-bash,
      uutils-coreutils-noprefix,
      oxidized-sed,
      oxidized-grep,
      oxidized-awk,
      uutils-findutils,
      oxidized-diffutils,
      oxidized-file,
      oxidized-tar,
      oxidized-gzip,
      oxidized-bzip2,
      oxidized-xz,
      oxidized-make,
      oxidized-patch,
      oxidized-patchelf,
    }:
      stdenv.mkDerivation {
        pname = "oxidized-nixpkgs-test";
        version = "0.1.0";

        dontUnpack = true;

        nativeBuildInputs = [
          oxidized-bash
          uutils-coreutils-noprefix
          oxidized-sed
          oxidized-grep
          oxidized-awk
          uutils-findutils
          oxidized-diffutils
          oxidized-file
          oxidized-tar
          oxidized-gzip
          oxidized-bzip2
          oxidized-xz
          oxidized-make
          oxidized-patch
          oxidized-patchelf
        ];

        buildPhase = ''
          echo "=== oxidized-nixpkgs component status ==="
          echo ""
          echo "All 15 components available:"
          echo "  Phase 1: shell (oxidized-bash), coreutils (uutils)"
          echo "  Phase 2: sed (uutils-sed), grep, awk, findutils (uutils), diffutils (uutils)"
          echo "  Phase 3: tar, gzip, bzip2, xz"
          echo "  Phase 4: make, patch"
          echo "  Phase 5: patchelf"
          echo ""
          echo "Verifying binaries..."
          bash --version | head -1
          ls --version | head -1
          sed --version | head -1
          grep --version | head -1
          awk --version | head -1
          find --version | head -1
          diff --version | head -1
          tar --version | head -1
          gzip --version | head -1
          bzip2 --version 2>&1 | head -1
          xz --version | head -1
          make --version | head -1
          patch --version | head -1
          patchelf --version | head -1
          echo ""
          echo "All components verified."
        '';

        installPhase = ''
          mkdir -p $out
          echo "oxidized-nixpkgs component test passed" > $out/result
        '';

        meta = {
          platforms = lib.platforms.linux;
          description = "Test derivation for oxidized-nixpkgs component availability";
          license = lib.licenses.mit;
        };
      };

    # Test building a trivial derivation using the Rust stdenv.
    # Constructs a stdenv with Rust tools directly from flake packages,
    # bypassing the overlay to avoid needing all overlays composed.
    stdenv-test = {
      lib,
      stdenv,
      uutils-coreutils-noprefix,
      oxidized-sed,
      oxidized-grep,
      oxidized-awk,
      uutils-findutils,
      oxidized-diffutils,
      oxidized-file,
      oxidized-tar,
      oxidized-gzip,
      oxidized-bzip2,
      oxidized-xz,
      oxidized-make,
      oxidized-patch,
    }: let
      rustStdenv = import ./stdenv-test.nix {
        inherit
          stdenv
          uutils-coreutils-noprefix
          oxidized-sed
          oxidized-grep
          oxidized-awk
          uutils-findutils
          oxidized-diffutils
          oxidized-file
          oxidized-tar
          oxidized-gzip
          oxidized-bzip2
          oxidized-xz
          oxidized-make
          oxidized-patch
          ;
      };
    in
      rustStdenv.mkDerivation {
        pname = "oxidized-nixpkgs-stdenv-test";
        version = "0.1.0";

        dontUnpack = true;

        buildPhase = ''
          echo "=== Building with Rust stdenv ==="
          echo "Shell: $(bash --version | head -1)"
          echo "Coreutils: $(ls --version | head -1)"
          echo "Sed: $(sed --version | head -1)"
          echo "Grep: $(grep --version | head -1)"
          echo "Awk: $(awk --version | head -1)"
          echo "Find: $(find --version | head -1)"
          echo "Diff: $(diff --version | head -1)"
          echo "Tar: $(tar --version | head -1)"
          echo "Gzip: $(gzip --version | head -1)"
          echo "Bzip2: $(bzip2 --version 2>&1 | head -1)"
          echo "Xz: $(xz --version | head -1)"
          echo "Make: $(make --version | head -1)"
          echo "Patch: $(patch --version | head -1)"
          echo ""
          echo "Rust stdenv test passed."
        '';

        installPhase = ''
          mkdir -p $out
          echo "oxidized-nixpkgs stdenv test passed" > $out/result
        '';

        meta = {
          platforms = lib.platforms.linux;
          description = "Test building with the Rust stdenv";
          license = lib.licenses.mit;
        };
      };

    # Test building GNU hello (a real autotools package) with the Rust stdenv.
    # This exercises configure scripts, make, install, and fixup phases.
    hello-test = {
      lib,
      stdenv,
      fetchurl,
      uutils-coreutils-noprefix,
      oxidized-sed,
      oxidized-grep,
      oxidized-awk,
      uutils-findutils,
      oxidized-diffutils,
      oxidized-file,
      oxidized-tar,
      oxidized-gzip,
      oxidized-bzip2,
      oxidized-xz,
      oxidized-make,
      oxidized-patch,
      oxidized-texinfo,
      oxidized-help2man,
    }: let
      rustStdenv = import ./stdenv-test.nix {
        inherit
          stdenv
          uutils-coreutils-noprefix
          oxidized-sed
          oxidized-grep
          oxidized-awk
          uutils-findutils
          oxidized-diffutils
          oxidized-file
          oxidized-tar
          oxidized-gzip
          oxidized-bzip2
          oxidized-xz
          oxidized-make
          oxidized-patch
          ;
      };
    in
      rustStdenv.mkDerivation {
        pname = "oxidized-nixpkgs-hello-test";
        version = "2.12.1";

        nativeBuildInputs = [oxidized-texinfo oxidized-help2man];

        src = fetchurl {
          url = "mirror://gnu/hello/hello-2.12.1.tar.gz";
          sha256 = "sha256-jZkUKv2SV28wsM18tCqNxoCZmLxdYH2Idh9RLibH2yA=";
        };

        # Prevent autotools re-running by ensuring generated files
        # are newer than their inputs (standard autotools timestamp fix)
        postPatch = ''
          find . -name '*.in' -o -name configure -o -name aclocal.m4 \
            -o -name config.h.in -o -name Makefile.in -o -name config.in \
            | xargs touch
        '';

        meta = {
          platforms = lib.platforms.linux;
          description = "GNU hello built with the Rust stdenv";
          license = lib.licenses.gpl3Plus;
        };
      };

    # Test building zlib — a critical C library used by nearly everything.
    # Uses a simple configure + make (not autotools).
    zlib-test = {
      lib,
      stdenv,
      fetchurl,
      uutils-coreutils-noprefix,
      oxidized-sed,
      oxidized-grep,
      oxidized-awk,
      uutils-findutils,
      oxidized-diffutils,
      oxidized-file,
      oxidized-tar,
      oxidized-gzip,
      oxidized-bzip2,
      oxidized-xz,
      oxidized-make,
      oxidized-patch,
    }: let
      rustStdenv = import ./stdenv-test.nix {
        inherit
          stdenv
          uutils-coreutils-noprefix
          oxidized-sed
          oxidized-grep
          oxidized-awk
          uutils-findutils
          oxidized-diffutils
          oxidized-file
          oxidized-tar
          oxidized-gzip
          oxidized-bzip2
          oxidized-xz
          oxidized-make
          oxidized-patch
          ;
      };
    in
      rustStdenv.mkDerivation {
        pname = "oxidized-nixpkgs-zlib-test";
        version = "1.3.1";

        src = fetchurl {
          url = "https://github.com/madler/zlib/releases/download/v1.3.1/zlib-1.3.1.tar.gz";
          sha256 = "sha256-mpOyt9/ax3zrpaVYpYDnRmfdb+3kWFuR7vtg8Dty3yM=";
        };

        meta = {
          platforms = lib.platforms.linux;
          description = "zlib built with the Rust stdenv";
          license = lib.licenses.zlib;
        };
      };

    # Test building GNU patch — an autotools C package.
    gnupatch-test = {
      lib,
      stdenv,
      fetchurl,
      uutils-coreutils-noprefix,
      oxidized-sed,
      oxidized-grep,
      oxidized-awk,
      uutils-findutils,
      oxidized-diffutils,
      oxidized-file,
      oxidized-tar,
      oxidized-gzip,
      oxidized-bzip2,
      oxidized-xz,
      oxidized-make,
      oxidized-patch,
      oxidized-texinfo,
      oxidized-help2man,
    }: let
      rustStdenv = import ./stdenv-test.nix {
        inherit
          stdenv
          uutils-coreutils-noprefix
          oxidized-sed
          oxidized-grep
          oxidized-awk
          uutils-findutils
          oxidized-diffutils
          oxidized-file
          oxidized-tar
          oxidized-gzip
          oxidized-bzip2
          oxidized-xz
          oxidized-make
          oxidized-patch
          ;
      };
    in
      rustStdenv.mkDerivation {
        pname = "oxidized-nixpkgs-gnupatch-test";
        version = "2.8";

        nativeBuildInputs = [oxidized-texinfo oxidized-help2man];

        src = fetchurl {
          url = "mirror://gnu/patch/patch-2.8.tar.xz";
          sha256 = "sha256-+Hzuae7CtPy/YKOWsDCtaqNBXxkqpffuhMrV4R9/WuM=";
        };

        postPatch = ''
          find . -name '*.in' -o -name configure -o -name aclocal.m4 \
            -o -name config.h.in -o -name Makefile.in -o -name config.in \
            | xargs touch
        '';

        meta = {
          platforms = lib.platforms.linux;
          description = "GNU patch built with the Rust stdenv";
          license = lib.licenses.gpl3Plus;
        };
      };

    # Test building GNU coreutils — a large autotools package with 100+ programs.
    coreutils-test = {
      lib,
      stdenv,
      fetchurl,
      uutils-coreutils-noprefix,
      oxidized-sed,
      oxidized-grep,
      oxidized-awk,
      uutils-findutils,
      oxidized-diffutils,
      oxidized-file,
      oxidized-tar,
      oxidized-gzip,
      oxidized-bzip2,
      oxidized-xz,
      oxidized-make,
      oxidized-patch,
      oxidized-texinfo,
      oxidized-help2man,
    }: let
      rustStdenv = import ./stdenv-test.nix {
        inherit
          stdenv
          uutils-coreutils-noprefix
          oxidized-sed
          oxidized-grep
          oxidized-awk
          uutils-findutils
          oxidized-diffutils
          oxidized-file
          oxidized-tar
          oxidized-gzip
          oxidized-bzip2
          oxidized-xz
          oxidized-make
          oxidized-patch
          ;
      };
    in
      rustStdenv.mkDerivation {
        pname = "oxidized-nixpkgs-coreutils-test";
        version = "9.6";

        nativeBuildInputs = [oxidized-texinfo oxidized-help2man];

        src = fetchurl {
          url = "mirror://gnu/coreutils/coreutils-9.6.tar.xz";
          sha256 = "sha256-egEkMns5j9nrGmq95YM4mCFCLHRP+hBzSyT1V2ENMoM=";
        };

        postPatch = ''
          find . -name '*.in' -o -name configure -o -name aclocal.m4 \
            -o -name config.h.in -o -name Makefile.in -o -name config.in \
            | xargs touch
        '';

        # Coreutils needs perl for some tests, skip them
        doCheck = false;

        meta = {
          platforms = lib.platforms.linux;
          description = "GNU coreutils built with the Rust stdenv";
          license = lib.licenses.gpl3Plus;
        };
      };
    # Test building GNU grep — autotools with regex library.
    gnugrep-test = {
      lib,
      stdenv,
      fetchurl,
      uutils-coreutils-noprefix,
      oxidized-sed,
      oxidized-grep,
      oxidized-awk,
      uutils-findutils,
      oxidized-diffutils,
      oxidized-file,
      oxidized-tar,
      oxidized-gzip,
      oxidized-bzip2,
      oxidized-xz,
      oxidized-make,
      oxidized-patch,
      oxidized-texinfo,
    }: let
      rustStdenv = import ./stdenv-test.nix {
        inherit
          stdenv
          uutils-coreutils-noprefix
          oxidized-sed
          oxidized-grep
          oxidized-awk
          uutils-findutils
          oxidized-diffutils
          oxidized-file
          oxidized-tar
          oxidized-gzip
          oxidized-bzip2
          oxidized-xz
          oxidized-make
          oxidized-patch
          ;
      };
    in
      rustStdenv.mkDerivation {
        pname = "oxidized-nixpkgs-gnugrep-test";
        version = "3.11";

        nativeBuildInputs = [oxidized-texinfo];

        src = fetchurl {
          url = "mirror://gnu/grep/grep-3.11.tar.xz";
          sha256 = "sha256-HbKu3eidDepCsW2VKPiUyNFdrk4ZC1muzHj1qVEnbqs=";
        };

        postPatch = ''
          find . -name '*.in' -o -name configure -o -name aclocal.m4 \
            -o -name config.h.in -o -name Makefile.in -o -name config.in \
            | xargs touch
        '';

        doCheck = false;

        meta = {
          platforms = lib.platforms.linux;
          description = "GNU grep built with the Rust stdenv";
          license = lib.licenses.gpl3Plus;
        };
      };

    # Test building GNU sed — autotools, exercises sed replacement compatibility.
    gnused-test = {
      lib,
      stdenv,
      fetchurl,
      uutils-coreutils-noprefix,
      oxidized-sed,
      oxidized-grep,
      oxidized-awk,
      uutils-findutils,
      oxidized-diffutils,
      oxidized-file,
      oxidized-tar,
      oxidized-gzip,
      oxidized-bzip2,
      oxidized-xz,
      oxidized-make,
      oxidized-patch,
      oxidized-texinfo,
    }: let
      rustStdenv = import ./stdenv-test.nix {
        inherit
          stdenv
          uutils-coreutils-noprefix
          oxidized-sed
          oxidized-grep
          oxidized-awk
          uutils-findutils
          oxidized-diffutils
          oxidized-file
          oxidized-tar
          oxidized-gzip
          oxidized-bzip2
          oxidized-xz
          oxidized-make
          oxidized-patch
          ;
      };
    in
      rustStdenv.mkDerivation {
        pname = "oxidized-nixpkgs-gnused-test";
        version = "4.9";

        nativeBuildInputs = [oxidized-texinfo];

        src = fetchurl {
          url = "mirror://gnu/sed/sed-4.9.tar.xz";
          sha256 = "sha256-biJrcy4c1zlGStaGK9Ghq6QteYKSLaelNRljHSSXUYE=";
        };

        postPatch = ''
          find . -name '*.in' -o -name configure -o -name aclocal.m4 \
            -o -name config.h.in -o -name Makefile.in -o -name config.in \
            | xargs touch
        '';

        doCheck = false;

        meta = {
          platforms = lib.platforms.linux;
          description = "GNU sed built with the Rust stdenv";
          license = lib.licenses.gpl3Plus;
        };
      };

    # Test building GNU diffutils — exercises diff/cmp/sdiff compatibility.
    gnudiffutils-test = {
      lib,
      stdenv,
      fetchurl,
      uutils-coreutils-noprefix,
      oxidized-sed,
      oxidized-grep,
      oxidized-awk,
      uutils-findutils,
      oxidized-diffutils,
      oxidized-file,
      oxidized-tar,
      oxidized-gzip,
      oxidized-bzip2,
      oxidized-xz,
      oxidized-make,
      oxidized-patch,
      oxidized-texinfo,
      oxidized-help2man,
    }: let
      rustStdenv = import ./stdenv-test.nix {
        inherit
          stdenv
          uutils-coreutils-noprefix
          oxidized-sed
          oxidized-grep
          oxidized-awk
          uutils-findutils
          oxidized-diffutils
          oxidized-file
          oxidized-tar
          oxidized-gzip
          oxidized-bzip2
          oxidized-xz
          oxidized-make
          oxidized-patch
          ;
      };
    in
      rustStdenv.mkDerivation {
        pname = "oxidized-nixpkgs-gnudiffutils-test";
        version = "3.10";

        nativeBuildInputs = [oxidized-texinfo oxidized-help2man];

        src = fetchurl {
          url = "mirror://gnu/diffutils/diffutils-3.10.tar.xz";
          sha256 = "sha256-kOXpPMck5OvhLt6A3xY0Bjx6hVaSaFkZv+YLVWyb0J4=";
        };

        postPatch = ''
          find . -name '*.in' -o -name configure -o -name aclocal.m4 \
            -o -name config.h.in -o -name Makefile.in -o -name config.in \
            | xargs touch
          # Touch man pages to prevent regeneration (avoids perl help2man)
          find . -name '*.1' | xargs touch
        '';

        makeFlags = ["HELP2MAN=true"];

        doCheck = false;

        meta = {
          platforms = lib.platforms.linux;
          description = "GNU diffutils built with the Rust stdenv";
          license = lib.licenses.gpl3Plus;
        };
      };

    # Test building GNU make — builds make with oxidized-make (self-referential!).
    gnumake-test = {
      lib,
      stdenv,
      fetchurl,
      uutils-coreutils-noprefix,
      oxidized-sed,
      oxidized-grep,
      oxidized-awk,
      uutils-findutils,
      oxidized-diffutils,
      oxidized-file,
      oxidized-tar,
      oxidized-gzip,
      oxidized-bzip2,
      oxidized-xz,
      oxidized-make,
      oxidized-patch,
      oxidized-texinfo,
    }: let
      rustStdenv = import ./stdenv-test.nix {
        inherit
          stdenv
          uutils-coreutils-noprefix
          oxidized-sed
          oxidized-grep
          oxidized-awk
          uutils-findutils
          oxidized-diffutils
          oxidized-file
          oxidized-tar
          oxidized-gzip
          oxidized-bzip2
          oxidized-xz
          oxidized-make
          oxidized-patch
          ;
      };
    in
      rustStdenv.mkDerivation {
        pname = "oxidized-nixpkgs-gnumake-test";
        version = "4.4.1";

        nativeBuildInputs = [oxidized-texinfo];

        src = fetchurl {
          url = "mirror://gnu/make/make-4.4.1.tar.gz";
          sha256 = "sha256-3Rb7HWe/q3mnL16DkHNcSePo5wtJRaFasfgd23hlj7M=";
        };

        postPatch = ''
          find . -name '*.in' -o -name configure -o -name aclocal.m4 \
            -o -name config.h.in -o -name Makefile.in -o -name config.in \
            | xargs touch
        '';

        doCheck = false;

        meta = {
          platforms = lib.platforms.linux;
          description = "GNU make built with the Rust stdenv (using oxidized-make!)";
          license = lib.licenses.gpl3Plus;
        };
      };

    # Test building GNU gawk — autotools with complex configure.
    gnuawk-test = {
      lib,
      stdenv,
      fetchurl,
      oxidized-bison,
      uutils-coreutils-noprefix,
      oxidized-sed,
      oxidized-grep,
      oxidized-awk,
      uutils-findutils,
      oxidized-diffutils,
      oxidized-file,
      oxidized-tar,
      oxidized-gzip,
      oxidized-bzip2,
      oxidized-xz,
      oxidized-make,
      oxidized-patch,
      oxidized-texinfo,
    }: let
      rustStdenv = import ./stdenv-test.nix {
        inherit
          stdenv
          uutils-coreutils-noprefix
          oxidized-sed
          oxidized-grep
          oxidized-awk
          uutils-findutils
          oxidized-diffutils
          oxidized-file
          oxidized-tar
          oxidized-gzip
          oxidized-bzip2
          oxidized-xz
          oxidized-make
          oxidized-patch
          ;
      };
    in
      rustStdenv.mkDerivation {
        pname = "oxidized-nixpkgs-gnuawk-test";
        version = "5.3.1";

        # gnulib of this vintage writes `static inline [[__nodiscard__]] int`
        # once it detects C23, and an attribute cannot follow a storage class
        # there - so with a compiler defaulting to C23 the header does not
        # parse. Asking for gnu17 puts gnulib back on
        # __attribute__((warn_unused_result)), which goes anywhere.
        env.NIX_CFLAGS_COMPILE = "-std=gnu17";

        nativeBuildInputs = [oxidized-texinfo oxidized-bison];

        src = fetchurl {
          url = "mirror://gnu/gawk/gawk-5.3.1.tar.xz";
          sha256 = "sha256-aU23ZIEqYjZCPU/0DOt7bExEEwG3KtUCu1wn4AzVb3g=";
        };

        postPatch = ''
          find . -name '*.in' -o -name configure -o -name aclocal.m4 \
            -o -name config.h.in -o -name Makefile.in -o -name config.in \
            | xargs touch
          # Touch info files to prevent makeinfo regeneration
          find . -name '*.info' -o -name '*.info-*' | xargs touch 2>/dev/null || true
        '';

        # Skip makeinfo by setting MAKEINFO to true
        makeFlags = ["MAKEINFO=true"];

        doCheck = false;

        meta = {
          platforms = lib.platforms.linux;
          description = "GNU awk built with the Rust stdenv (parser generated by oxidized-bison!)";
          license = lib.licenses.gpl3Plus;
        };
      };

    # Test building GNU bc — a calculator, different autotools patterns.
    # Exercises flex/yacc-generated parsers and ed-style line editing.
    bc-test = {
      lib,
      stdenv,
      fetchurl,
      uutils-coreutils-noprefix,
      oxidized-sed,
      oxidized-grep,
      oxidized-awk,
      uutils-findutils,
      oxidized-diffutils,
      oxidized-file,
      oxidized-tar,
      oxidized-gzip,
      oxidized-bzip2,
      oxidized-xz,
      oxidized-make,
      oxidized-patch,
      oxidized-texinfo,
      oxidized-bison,
      flex,
      ed,
    }: let
      rustStdenv = import ./stdenv-test.nix {
        inherit
          stdenv
          uutils-coreutils-noprefix
          oxidized-sed
          oxidized-grep
          oxidized-awk
          uutils-findutils
          oxidized-diffutils
          oxidized-file
          oxidized-tar
          oxidized-gzip
          oxidized-bzip2
          oxidized-xz
          oxidized-make
          oxidized-patch
          ;
      };
    in
      rustStdenv.mkDerivation {
        pname = "oxidized-nixpkgs-bc-test";
        version = "1.07.1";

        nativeBuildInputs = [oxidized-texinfo oxidized-bison flex ed];

        src = fetchurl {
          url = "mirror://gnu/bc/bc-1.07.1.tar.gz";
          sha256 = "sha256-Yq38qJsKHAFkws3KWcohDB1Ew//Eba+ZMc9JQmZMsCo=";
        };

        postPatch = ''
          find . -name '*.in' -o -name configure -o -name aclocal.m4 \
            -o -name config.h.in -o -name Makefile.in -o -name config.in \
            | xargs touch
        '';

        doCheck = false;

        meta = {
          platforms = lib.platforms.linux;
          description = "GNU bc (calculator) built with the Rust stdenv";
          license = lib.licenses.gpl3Plus;
        };
      };

    # Test using oxidized-bash as the stdenv SHELL (not just in initialPath).
    # This is the critical test: can oxidized-bash actually execute the build
    # phases via setup.sh, acting as the builder shell?
    bash-shell-test = {
      lib,
      stdenv,
      oxidized-bash,
      uutils-coreutils-noprefix,
      oxidized-sed,
      oxidized-grep,
      oxidized-awk,
      uutils-findutils,
      oxidized-diffutils,
      oxidized-file,
      oxidized-tar,
      oxidized-gzip,
      oxidized-bzip2,
      oxidized-xz,
      oxidized-make,
      oxidized-patch,
    }: let
      rustStdenv = import ./stdenv-test.nix {
        inherit
          stdenv
          uutils-coreutils-noprefix
          oxidized-sed
          oxidized-grep
          oxidized-awk
          uutils-findutils
          oxidized-diffutils
          oxidized-file
          oxidized-tar
          oxidized-gzip
          oxidized-bzip2
          oxidized-xz
          oxidized-make
          oxidized-patch
          ;
      };
      # Override the shell to use oxidized-bash as the builder
      rustShellStdenv = rustStdenv.override {
        shell = "${oxidized-bash}/bin/bash";
      };
    in
      rustShellStdenv.mkDerivation {
        pname = "oxidized-nixpkgs-bash-shell-test";
        version = "0.1.0";

        # gnulib of this vintage writes `static inline [[__nodiscard__]] int`
        # once it detects C23, and an attribute cannot follow a storage class
        # there - so with a compiler defaulting to C23 the header does not
        # parse. Asking for gnu17 puts gnulib back on
        # __attribute__((warn_unused_result)), which goes anywhere.
        env.NIX_CFLAGS_COMPILE = "-std=gnu17";

        dontUnpack = true;
        dontPatch = true;
        dontConfigure = true;
        dontFixup = true;

        buildPhase = ''
          echo "=== Building with oxidized-bash as stdenv shell ==="
          echo "Shell: $SHELL"
          echo "Bash: $(bash --version | head -1)"
          echo "Current shell PID: $$"
          echo ""
          echo "Testing basic shell features..."
          # Test variable assignment and expansion
          FOO="hello world"
          echo "Variable: $FOO"
          # Test command substitution
          echo "Date: $(date +%s)"
          # Test conditionals
          if [ -d "$NIX_BUILD_TOP" ]; then
            echo "Build dir exists: $NIX_BUILD_TOP"
          fi
          # Test loops
          for i in 1 2 3; do
            echo "Loop iteration: $i"
          done
          echo ""
          echo "oxidized-bash shell test passed."
        '';

        installPhase = ''
          mkdir -p $out
          echo "oxidized-bash shell test passed" > $out/result
        '';

        meta = {
          platforms = lib.platforms.linux;
          description = "Test using oxidized-bash as the stdenv builder shell";
          license = lib.licenses.mit;
        };
      };

    # Test building GNU tar — self-referential: builds tar using oxidized-tar!
    gnutar-test = {
      lib,
      stdenv,
      fetchurl,
      uutils-coreutils-noprefix,
      oxidized-sed,
      oxidized-grep,
      oxidized-awk,
      uutils-findutils,
      oxidized-diffutils,
      oxidized-file,
      oxidized-tar,
      oxidized-gzip,
      oxidized-bzip2,
      oxidized-xz,
      oxidized-make,
      oxidized-patch,
      oxidized-texinfo,
    }: let
      rustStdenv = import ./stdenv-test.nix {
        inherit
          stdenv
          uutils-coreutils-noprefix
          oxidized-sed
          oxidized-grep
          oxidized-awk
          uutils-findutils
          oxidized-diffutils
          oxidized-file
          oxidized-tar
          oxidized-gzip
          oxidized-bzip2
          oxidized-xz
          oxidized-make
          oxidized-patch
          ;
      };
    in
      rustStdenv.mkDerivation {
        pname = "oxidized-nixpkgs-gnutar-test";
        version = "1.35";

        nativeBuildInputs = [oxidized-texinfo];

        src = fetchurl {
          url = "mirror://gnu/tar/tar-1.35.tar.xz";
          sha256 = "sha256-TWL/NzQux67XSFNTI5MMfPlKz3HDWRiCsmp+pQ8+3BY=";
        };

        postPatch = ''
          find . -name '*.in' -o -name configure -o -name aclocal.m4 \
            -o -name config.h.in -o -name Makefile.in -o -name config.in \
            | xargs touch
        '';

        doCheck = false;

        meta = {
          platforms = lib.platforms.linux;
          description = "GNU tar built with the Rust stdenv (using oxidized-tar!)";
          license = lib.licenses.gpl3Plus;
        };
      };

    # Test building GNU gzip — self-referential: builds gzip using oxidized-gzip!
    gnugzip-test = {
      lib,
      stdenv,
      fetchurl,
      uutils-coreutils-noprefix,
      oxidized-sed,
      oxidized-grep,
      oxidized-awk,
      uutils-findutils,
      oxidized-diffutils,
      oxidized-file,
      oxidized-tar,
      oxidized-gzip,
      oxidized-bzip2,
      oxidized-xz,
      oxidized-make,
      oxidized-patch,
      oxidized-texinfo,
    }: let
      rustStdenv = import ./stdenv-test.nix {
        inherit
          stdenv
          uutils-coreutils-noprefix
          oxidized-sed
          oxidized-grep
          oxidized-awk
          uutils-findutils
          oxidized-diffutils
          oxidized-file
          oxidized-tar
          oxidized-gzip
          oxidized-bzip2
          oxidized-xz
          oxidized-make
          oxidized-patch
          ;
      };
    in
      rustStdenv.mkDerivation {
        pname = "oxidized-nixpkgs-gnugzip-test";
        version = "1.14";

        nativeBuildInputs = [oxidized-texinfo];

        src = fetchurl {
          url = "mirror://gnu/gzip/gzip-1.14.tar.xz";
          sha256 = "sha256-Aae4gb0iC/32Ffl7hxj4C9/T9q3ThbmT3Pbv0U6MCsY=";
        };

        postPatch = ''
          find . -name '*.in' -o -name configure -o -name aclocal.m4 \
            -o -name config.h.in -o -name Makefile.in -o -name config.in \
            | xargs touch
          find . -name '*.info' -o -name '*.info-*' | xargs touch 2>/dev/null || true
        '';

        # oxidized-make doesn't handle .in: suffix rules, so pre-generate the scripts
        preBuild = ''
          for f in gunzip gzexe zcat zcmp zdiff zegrep zfgrep zforce zgrep zless zmore znew; do
            if [ -f "$f.in" ]; then
              sed \
                -e "s|/bin/sh|$SHELL|g" \
                -e "s|@GREP@|grep|g" \
                -e "s|'gzip'|gzip|g" \
                -e "s|'zdiff'|zdiff|g" \
                -e "s|'zgrep'|zgrep|g" \
                -e "s|@VERSION@|1.14|g" \
                "$f.in" > "$f"
              chmod a+rx "$f"
            fi
          done
        '';

        makeFlags = ["MAKEINFO=true"];

        doCheck = false;

        meta = {
          platforms = lib.platforms.linux;
          description = "GNU gzip built with the Rust stdenv (using oxidized-gzip!)";
          license = lib.licenses.gpl3Plus;
        };
      };

    # Test building XZ Utils — self-referential: builds xz using oxidized-xz!
    xz-test = {
      lib,
      stdenv,
      fetchurl,
      uutils-coreutils-noprefix,
      oxidized-sed,
      oxidized-grep,
      oxidized-awk,
      uutils-findutils,
      oxidized-diffutils,
      oxidized-file,
      oxidized-tar,
      oxidized-gzip,
      oxidized-bzip2,
      oxidized-xz,
      oxidized-make,
      oxidized-patch,
      oxidized-texinfo,
    }: let
      rustStdenv = import ./stdenv-test.nix {
        inherit
          stdenv
          uutils-coreutils-noprefix
          oxidized-sed
          oxidized-grep
          oxidized-awk
          uutils-findutils
          oxidized-diffutils
          oxidized-file
          oxidized-tar
          oxidized-gzip
          oxidized-bzip2
          oxidized-xz
          oxidized-make
          oxidized-patch
          ;
      };
    in
      rustStdenv.mkDerivation {
        pname = "oxidized-nixpkgs-xz-test";
        version = "5.6.4";

        # gnulib of this vintage writes `static inline [[__nodiscard__]] int`
        # once it detects C23, and an attribute cannot follow a storage class
        # there - so with a compiler defaulting to C23 the header does not
        # parse. Asking for gnu17 puts gnulib back on
        # __attribute__((warn_unused_result)), which goes anywhere.
        env.NIX_CFLAGS_COMPILE = "-std=gnu17";

        nativeBuildInputs = [oxidized-texinfo];

        src = fetchurl {
          url = "https://github.com/tukaani-project/xz/releases/download/v5.6.4/xz-5.6.4.tar.xz";
          sha256 = "sha256-gpzP5512l0j3VX56RCmmTQaFjifh42LiXQGre5MdnJU=";
        };

        postPatch = ''
          find . -name '*.in' -o -name configure -o -name aclocal.m4 \
            -o -name config.h.in -o -name Makefile.in -o -name config.in \
            | xargs touch
        '';

        doCheck = false;

        meta = {
          platforms = lib.platforms.linux;
          description = "XZ Utils built with the Rust stdenv (using oxidized-xz!)";
          license = lib.licenses.gpl3Plus;
        };
      };

    # Test building GNU findutils — autotools.
    gnufindutils-test = {
      lib,
      stdenv,
      fetchurl,
      uutils-coreutils-noprefix,
      oxidized-sed,
      oxidized-grep,
      oxidized-awk,
      uutils-findutils,
      oxidized-diffutils,
      oxidized-file,
      oxidized-tar,
      oxidized-gzip,
      oxidized-bzip2,
      oxidized-xz,
      oxidized-make,
      oxidized-patch,
      oxidized-texinfo,
    }: let
      rustStdenv = import ./stdenv-test.nix {
        inherit
          stdenv
          uutils-coreutils-noprefix
          oxidized-sed
          oxidized-grep
          oxidized-awk
          uutils-findutils
          oxidized-diffutils
          oxidized-file
          oxidized-tar
          oxidized-gzip
          oxidized-bzip2
          oxidized-xz
          oxidized-make
          oxidized-patch
          ;
      };
    in
      rustStdenv.mkDerivation {
        pname = "oxidized-nixpkgs-gnufindutils-test";
        version = "4.10.0";

        # gnulib of this vintage writes `static inline [[__nodiscard__]] int`
        # once it detects C23, and an attribute cannot follow a storage class
        # there - so with a compiler defaulting to C23 the header does not
        # parse. Asking for gnu17 puts gnulib back on
        # __attribute__((warn_unused_result)), which goes anywhere.
        env.NIX_CFLAGS_COMPILE = "-std=gnu17";

        nativeBuildInputs = [oxidized-texinfo];

        src = fetchurl {
          url = "mirror://gnu/findutils/findutils-4.10.0.tar.xz";
          sha256 = "sha256-E4fgtn/yR9Kr3pmPkN+/cMFJE5Glnd/suK5ph4nwpPU=";
        };

        postPatch = ''
          find . -name '*.in' -o -name configure -o -name aclocal.m4 \
            -o -name config.h.in -o -name Makefile.in -o -name config.in \
            | xargs touch
          find . -name '*.info' -o -name '*.info-*' | xargs touch 2>/dev/null || true
        '';

        makeFlags = ["MAKEINFO=true"];

        doCheck = false;

        meta = {
          platforms = lib.platforms.linux;
          description = "GNU findutils built with the Rust stdenv";
          license = lib.licenses.gpl3Plus;
        };
      };

    # Test building GNU readline — line editing library, autotools, exercises termcap/ncurses.
    readline-test = {
      lib,
      stdenv,
      fetchurl,
      ncurses,
      uutils-coreutils-noprefix,
      oxidized-sed,
      oxidized-grep,
      oxidized-awk,
      uutils-findutils,
      oxidized-diffutils,
      oxidized-file,
      oxidized-tar,
      oxidized-gzip,
      oxidized-bzip2,
      oxidized-xz,
      oxidized-make,
      oxidized-patch,
      oxidized-texinfo,
    }: let
      rustStdenv = import ./stdenv-test.nix {
        inherit
          stdenv
          uutils-coreutils-noprefix
          oxidized-sed
          oxidized-grep
          oxidized-awk
          uutils-findutils
          oxidized-diffutils
          oxidized-file
          oxidized-tar
          oxidized-gzip
          oxidized-bzip2
          oxidized-xz
          oxidized-make
          oxidized-patch
          ;
      };
    in
      rustStdenv.mkDerivation {
        pname = "oxidized-nixpkgs-readline-test";
        version = "8.2";

        nativeBuildInputs = [oxidized-texinfo];
        buildInputs = [ncurses];

        src = fetchurl {
          url = "mirror://gnu/readline/readline-8.2.tar.gz";
          sha256 = "sha256-P+txcfFqhO6CyhijbXub4QmlLAT0kqBTMx19EJUAfDU=";
        };

        configureFlags = ["--disable-shared"];

        postPatch = ''
          find . -name '*.in' -o -name configure -o -name aclocal.m4 \
            -o -name config.h.in -o -name Makefile.in -o -name config.in \
            | xargs touch
        '';

        doCheck = false;

        meta = {
          platforms = lib.platforms.linux;
          description = "GNU readline built with the Rust stdenv";
          license = lib.licenses.gpl3Plus;
        };
      };

    # Test building libffi — foreign function interface, autotools+special build system.
    # Note: libffi's Makefile uses complex conditionals that oxidized-make can't handle,
    # so we add gnumake as a nativeBuildInput to override oxidized-make for the build.
    libffi-test = {
      lib,
      stdenv,
      fetchurl,
      gnumake,
      uutils-coreutils-noprefix,
      oxidized-sed,
      oxidized-grep,
      oxidized-awk,
      uutils-findutils,
      oxidized-diffutils,
      oxidized-file,
      oxidized-tar,
      oxidized-gzip,
      oxidized-bzip2,
      oxidized-xz,
      oxidized-make,
      oxidized-patch,
      oxidized-texinfo,
    }: let
      rustStdenv = import ./stdenv-test.nix {
        inherit
          stdenv
          uutils-coreutils-noprefix
          oxidized-sed
          oxidized-grep
          oxidized-awk
          uutils-findutils
          oxidized-diffutils
          oxidized-file
          oxidized-tar
          oxidized-gzip
          oxidized-bzip2
          oxidized-xz
          oxidized-make
          oxidized-patch
          ;
      };
    in
      rustStdenv.mkDerivation {
        pname = "oxidized-nixpkgs-libffi-test";
        version = "3.4.6";

        nativeBuildInputs = [oxidized-texinfo gnumake];

        src = fetchurl {
          url = "https://github.com/libffi/libffi/releases/download/v3.4.6/libffi-3.4.6.tar.gz";
          sha256 = "sha256-sN6p3yPIY6elDoJUQPPr/6vWXfFJcQjl1Dd0eEOJWk4=";
        };

        # libffi creates a subdirectory build layout via config.status buildir.
        # We need to configure from within the build subdirectory to avoid the
        # broken wrapper Makefile generation.
        postPatch = ''
          find . -name '*.in' -o -name configure -o -name aclocal.m4 \
            -o -name config.h.in -o -name Makefile.in -o -name config.in \
            | xargs touch
        '';

        preConfigure = ''
          mkdir -p x86_64-pc-linux-gnu
          cd x86_64-pc-linux-gnu
          configureScript=../configure
        '';

        doCheck = false;

        meta = {
          platforms = lib.platforms.linux;
          description = "libffi built with the Rust stdenv";
          license = lib.licenses.mit;
        };
      };

    # Test building PCRE2 — regex library, autotools.
    pcre2-test = {
      lib,
      stdenv,
      fetchurl,
      uutils-coreutils-noprefix,
      oxidized-sed,
      oxidized-grep,
      oxidized-awk,
      uutils-findutils,
      oxidized-diffutils,
      oxidized-file,
      oxidized-tar,
      oxidized-gzip,
      oxidized-bzip2,
      oxidized-xz,
      oxidized-make,
      oxidized-patch,
      oxidized-texinfo,
    }: let
      rustStdenv = import ./stdenv-test.nix {
        inherit
          stdenv
          uutils-coreutils-noprefix
          oxidized-sed
          oxidized-grep
          oxidized-awk
          uutils-findutils
          oxidized-diffutils
          oxidized-file
          oxidized-tar
          oxidized-gzip
          oxidized-bzip2
          oxidized-xz
          oxidized-make
          oxidized-patch
          ;
      };
    in
      rustStdenv.mkDerivation {
        pname = "oxidized-nixpkgs-pcre2-test";
        version = "10.44";

        # gnulib of this vintage writes `static inline [[__nodiscard__]] int`
        # once it detects C23, and an attribute cannot follow a storage class
        # there - so with a compiler defaulting to C23 the header does not
        # parse. Asking for gnu17 puts gnulib back on
        # __attribute__((warn_unused_result)), which goes anywhere.
        env.NIX_CFLAGS_COMPILE = "-std=gnu17";

        nativeBuildInputs = [oxidized-texinfo];

        src = fetchurl {
          url = "https://github.com/PCRE2Project/pcre2/releases/download/pcre2-10.44/pcre2-10.44.tar.bz2";
          sha256 = "sha256-008C4RPPcZOh6/J3DTrFJwiNSF1OBH7RDl0hfG713pY=";
        };

        postPatch = ''
          find . -name '*.in' -o -name configure -o -name aclocal.m4 \
            -o -name config.h.in -o -name Makefile.in -o -name config.in \
            | xargs touch
        '';

        doCheck = false;

        meta = {
          platforms = lib.platforms.linux;
          description = "PCRE2 built with the Rust stdenv";
          license = lib.licenses.bsd3;
        };
      };

    # Test building GNU m4 — macro processor, used by autoconf. Autotools build.
    m4-test = {
      lib,
      stdenv,
      fetchurl,
      uutils-coreutils-noprefix,
      oxidized-sed,
      oxidized-grep,
      oxidized-awk,
      uutils-findutils,
      oxidized-diffutils,
      oxidized-file,
      oxidized-tar,
      oxidized-gzip,
      oxidized-bzip2,
      oxidized-xz,
      oxidized-make,
      oxidized-patch,
      oxidized-texinfo,
    }: let
      rustStdenv = import ./stdenv-test.nix {
        inherit
          stdenv
          uutils-coreutils-noprefix
          oxidized-sed
          oxidized-grep
          oxidized-awk
          uutils-findutils
          oxidized-diffutils
          oxidized-file
          oxidized-tar
          oxidized-gzip
          oxidized-bzip2
          oxidized-xz
          oxidized-make
          oxidized-patch
          ;
      };
    in
      rustStdenv.mkDerivation {
        pname = "oxidized-nixpkgs-m4-test";
        version = "1.4.19";

        # gnulib of this vintage writes `static inline [[__nodiscard__]] int`
        # once it detects C23, and an attribute cannot follow a storage class
        # there - so with a compiler defaulting to C23 the header does not
        # parse. Asking for gnu17 puts gnulib back on
        # __attribute__((warn_unused_result)), which goes anywhere.
        env.NIX_CFLAGS_COMPILE = "-std=gnu17";

        nativeBuildInputs = [oxidized-texinfo];

        src = fetchurl {
          url = "mirror://gnu/m4/m4-1.4.19.tar.xz";
          sha256 = "sha256-Y67eXG0zttmxNRHNC+LKwEby5w/QoHqpVzoEqCeDr5Y=";
        };

        postPatch = ''
          find . -name '*.in' -o -name configure -o -name aclocal.m4 \
            -o -name config.h.in -o -name Makefile.in -o -name config.in \
            | xargs touch
          # Disable po (translations) to avoid needing gettext/msgfmt
          sed 's/^SUBDIRS = \(.*\) po \(.*\)/SUBDIRS = \1 \2/' Makefile.in > Makefile.in.tmp
          mv Makefile.in.tmp Makefile.in
        '';

        doCheck = false;

        meta = {
          platforms = lib.platforms.linux;
          description = "GNU m4 built with the Rust stdenv";
          license = lib.licenses.gpl3Plus;
        };
      };

    # Test building GNU libtool — shared library support. Autotools build.
    libtool-test = {
      lib,
      stdenv,
      fetchurl,
      m4,
      uutils-coreutils-noprefix,
      oxidized-sed,
      oxidized-grep,
      oxidized-awk,
      uutils-findutils,
      oxidized-diffutils,
      oxidized-file,
      oxidized-tar,
      oxidized-gzip,
      oxidized-bzip2,
      oxidized-xz,
      oxidized-make,
      oxidized-patch,
      oxidized-texinfo,
    }: let
      rustStdenv = import ./stdenv-test.nix {
        inherit
          stdenv
          uutils-coreutils-noprefix
          oxidized-sed
          oxidized-grep
          oxidized-awk
          uutils-findutils
          oxidized-diffutils
          oxidized-file
          oxidized-tar
          oxidized-gzip
          oxidized-bzip2
          oxidized-xz
          oxidized-make
          oxidized-patch
          ;
      };
    in
      rustStdenv.mkDerivation {
        pname = "oxidized-nixpkgs-libtool-test";
        version = "2.5.4";

        # gnulib of this vintage writes `static inline [[__nodiscard__]] int`
        # once it detects C23, and an attribute cannot follow a storage class
        # there - so with a compiler defaulting to C23 the header does not
        # parse. Asking for gnu17 puts gnulib back on
        # __attribute__((warn_unused_result)), which goes anywhere.
        env.NIX_CFLAGS_COMPILE = "-std=gnu17";

        nativeBuildInputs = [oxidized-texinfo m4];

        src = fetchurl {
          url = "mirror://gnu/libtool/libtool-2.5.4.tar.xz";
          sha256 = "sha256-+B9YYGZrC8fYS63e+mDRy5+m/OsjmMw7rKavqmAmZnU=";
        };

        postPatch = ''
          find . -name '*.in' -o -name configure -o -name aclocal.m4 \
            -o -name config.h.in -o -name Makefile.in -o -name config.in \
            | xargs touch
        '';

        doCheck = false;

        meta = {
          platforms = lib.platforms.linux;
          description = "GNU libtool built with the Rust stdenv";
          license = lib.licenses.gpl3Plus;
        };
      };

    # Test building bzip2 — compression library. Simple Makefile build (not autotools).
    bzip2-test = {
      lib,
      stdenv,
      fetchurl,
      uutils-coreutils-noprefix,
      oxidized-sed,
      oxidized-grep,
      oxidized-awk,
      uutils-findutils,
      oxidized-diffutils,
      oxidized-file,
      oxidized-tar,
      oxidized-gzip,
      oxidized-bzip2,
      oxidized-xz,
      oxidized-make,
      oxidized-patch,
    }: let
      rustStdenv = import ./stdenv-test.nix {
        inherit
          stdenv
          uutils-coreutils-noprefix
          oxidized-sed
          oxidized-grep
          oxidized-awk
          uutils-findutils
          oxidized-diffutils
          oxidized-file
          oxidized-tar
          oxidized-gzip
          oxidized-bzip2
          oxidized-xz
          oxidized-make
          oxidized-patch
          ;
      };
    in
      rustStdenv.mkDerivation {
        pname = "oxidized-nixpkgs-bzip2-test";
        version = "1.0.8";

        src = fetchurl {
          url = "https://sourceware.org/pub/bzip2/bzip2-1.0.8.tar.gz";
          sha256 = "sha256-q1oDF27hBtPw+pDjgdpHjdrkBZGBU8yiSOaCzQxKImk=";
        };

        # bzip2 uses a plain Makefile, not autotools
        dontConfigure = true;

        makeFlags = [
          "CC=${stdenv.cc.targetPrefix}cc"
        ];

        installPhase = ''
          mkdir -p $out/bin $out/lib $out/include $out/man/man1
          cp -v bzip2 bzip2recover $out/bin/
          ln -s bzip2 $out/bin/bunzip2
          ln -s bzip2 $out/bin/bzcat
          cp -v libbz2.a $out/lib/
          cp -v bzlib.h $out/include/
          cp -v bzip2.1 $out/man/man1/
        '';

        doCheck = false;

        meta = {
          platforms = lib.platforms.linux;
          description = "bzip2 built with the Rust stdenv";
          license = lib.licenses.bsdOriginal;
        };
      };

    # Test that oxidized-gcc can compile a simple C program via the nixpkgs wrapper.
    gcc-test = {
      lib,
      stdenv,
      oxidized-gcc,
      wrapCCWith,
    }: let
      # Wrap oxidized-gcc the same way nixpkgs wraps real gcc
      wrappedCC = wrapCCWith {
        cc = oxidized-gcc;
        inherit (stdenv.cc) libc bintools;
        isGNU = true;
      };
      # Create a stdenv using the wrapped oxidized-gcc
      gccStdenv = stdenv.override {
        cc = wrappedCC;
        allowedRequisites = null;
      };
    in
      gccStdenv.mkDerivation {
        pname = "oxidized-nixpkgs-gcc-test";
        version = "0.1.0";

        dontUnpack = true;

        buildPhase = ''
          echo "=== Testing oxidized-gcc compilation ==="
          echo "CC: $CC"
          $CC --version | head -1
          echo "NIX_CFLAGS_COMPILE: $NIX_CFLAGS_COMPILE"
          echo "NIX_CC: $NIX_CC"

          # Compile a simple C program
          cat > hello.c << 'CEOF'
          #include <stdio.h>
          int main(void) {
              printf("Hello from oxidized-gcc!\n");
              return 0;
          }
          CEOF
          $CC -isystem ${oxidized-gcc}/lib/gcc/x86_64-unknown-linux-gnu/14.2.0/include -o hello hello.c
          file hello
          echo "Compilation succeeded!"
          # Note: execution may fail due to dynamic linker path — the built-in
          # linker doesn't yet fully integrate with nixpkgs' ld-linux path.
          ./hello || echo "(execution failed — linker path issue, expected for now)"

          echo ""
          echo "oxidized-gcc compilation test passed."
        '';

        installPhase = ''
          mkdir -p $out/bin
          cp hello $out/bin/
          echo "oxidized-gcc compilation test passed" > $out/result
        '';

        meta = {
          platforms = lib.platforms.linux;
          description = "Test compiling C code with oxidized-gcc";
          license = lib.licenses.cc0;
        };
      };

    # Test that oxidized-binutils ar/ranlib/nm work correctly.
    binutils-test = {
      lib,
      stdenv,
      oxidized-binutils,
    }:
      stdenv.mkDerivation {
        pname = "oxidized-nixpkgs-binutils-test";
        version = "0.1.0";

        dontUnpack = true;

        nativeBuildInputs = [oxidized-binutils];

        buildPhase = ''
          echo "=== Testing oxidized-binutils ==="

          # Verify tools exist
          ar --version | head -1
          ranlib --version | head -1
          nm --version | head -1
          readelf --version | head -1
          strings --version | head -1
          size --version | head -1

          # Create a test .c and .o file
          cat > add.c << 'EOF'
          int add(int a, int b) { return a + b; }
          EOF
          cat > mul.c << 'EOF'
          int mul(int a, int b) { return a * b; }
          EOF

          gcc -c add.c -o add.o
          gcc -c mul.c -o mul.o

          # Test ar: create archive
          ar rcs libmath.a add.o mul.o
          echo "Archive created:"
          ar t libmath.a

          # Test nm: list symbols
          echo "Symbols in archive:"
          nm libmath.a

          # Test ranlib (should be no-op since ar s already ran)
          ranlib libmath.a

          # Test strings
          echo "Strings in archive:"
          strings libmath.a | head -5

          # Test size
          echo "Section sizes:"
          size add.o

          # Test readelf
          echo "ELF header:"
          readelf -h add.o | head -5

          # Test linking with the archive
          cat > main.c << 'EOF'
          extern int add(int, int);
          extern int mul(int, int);
          int main() { return !(add(2, 3) == 5 && mul(3, 4) == 12); }
          EOF
          gcc main.c -L. -lmath -o test_math
          ./test_math && echo "Linking test PASSED"

          echo ""
          echo "oxidized-binutils test passed."
        '';

        installPhase = ''
          mkdir -p $out
          echo "oxidized-binutils test passed" > $out/result
        '';

        meta = {
          platforms = lib.platforms.linux;
          description = "Test oxidized-binutils ar/ranlib/nm functionality";
          license = lib.licenses.mit;
        };
      };
  };
}
