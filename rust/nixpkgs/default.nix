{
  devShells.rust-nixpkgs = pkgs: {
    packages = with pkgs; [
      just
      nix-tree
    ];
  };

  overlays.rust-nixpkgs = final: prev: let
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

    # Shell replacement: rust-bash provides /bin/bash and /bin/sh
    shellPkg = let
      shellMatch = lib.filter (c: c.name == "shell") available;
    in
      if shellMatch != []
      then (lib.head shellMatch).replacement
      else prev.bash;
  in {
    # Expose the component registry for introspection
    rust-nixpkgs-components = components;

    # A stdenv with all available Rust replacements swapped in.
    # We disable allowedRequisites because Rust replacement packages
    # are built with the normal stdenv, so their closures transitively
    # reference the C originals (e.g. rust-grep depends on coreutils).
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
    rust-nixpkgs-test = {
      stdenv,
      lib,
      rust-bash,
      uutils-coreutils-noprefix,
      rust-sed,
      rust-grep,
      rust-awk,
      uutils-findutils,
      rust-diffutils,
      rust-file,
      rust-tar,
      rust-gzip,
      rust-bzip2,
      rust-xz,
      rust-make,
      rust-patch,
      rust-patchelf,
      rust-strip,
    }:
      stdenv.mkDerivation {
        pname = "rust-nixpkgs-test";
        version = "0.1.0";

        dontUnpack = true;

        nativeBuildInputs = [
          rust-bash
          uutils-coreutils-noprefix
          rust-sed
          rust-grep
          rust-awk
          uutils-findutils
          rust-diffutils
          rust-file
          rust-tar
          rust-gzip
          rust-bzip2
          rust-xz
          rust-make
          rust-patch
          rust-patchelf
          rust-strip
        ];

        buildPhase = ''
          echo "=== rust-nixpkgs component status ==="
          echo ""
          echo "All 15 components available:"
          echo "  Phase 1: shell (rust-bash), coreutils (uutils)"
          echo "  Phase 2: sed (uutils-sed), grep, awk, findutils (uutils), diffutils (uutils)"
          echo "  Phase 3: tar, gzip, bzip2, xz"
          echo "  Phase 4: make, patch"
          echo "  Phase 5: patchelf, strip"
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
          strip --version | head -1
          echo ""
          echo "All components verified."
        '';

        installPhase = ''
          mkdir -p $out
          echo "rust-nixpkgs component test passed" > $out/result
        '';

        meta = {
          description = "Test derivation for rust-nixpkgs component availability";
          license = lib.licenses.mit;
        };
      };

    # Test building a trivial derivation using the Rust stdenv.
    # Constructs a stdenv with Rust tools directly from flake packages,
    # bypassing the overlay to avoid needing all overlays composed.
    rust-nixpkgs-stdenv-test = {
      lib,
      stdenv,
      uutils-coreutils-noprefix,
      rust-sed,
      rust-grep,
      rust-awk,
      uutils-findutils,
      rust-diffutils,
      rust-file,
      rust-tar,
      rust-gzip,
      rust-bzip2,
      rust-xz,
      rust-make,
      rust-patch,
    }: let
      rustStdenv = import ./stdenv-test.nix {
        inherit
          stdenv
          uutils-coreutils-noprefix
          rust-sed
          rust-grep
          rust-awk
          uutils-findutils
          rust-diffutils
          rust-file
          rust-tar
          rust-gzip
          rust-bzip2
          rust-xz
          rust-make
          rust-patch
          ;
      };
    in
      rustStdenv.mkDerivation {
        pname = "rust-nixpkgs-stdenv-test";
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
          echo "rust-nixpkgs stdenv test passed" > $out/result
        '';

        meta = {
          description = "Test building with the Rust stdenv";
          license = lib.licenses.mit;
        };
      };

    # Test building GNU hello (a real autotools package) with the Rust stdenv.
    # This exercises configure scripts, make, install, and fixup phases.
    rust-nixpkgs-hello-test = {
      lib,
      stdenv,
      fetchurl,
      uutils-coreutils-noprefix,
      rust-sed,
      rust-grep,
      rust-awk,
      uutils-findutils,
      rust-diffutils,
      rust-file,
      rust-tar,
      rust-gzip,
      rust-bzip2,
      rust-xz,
      rust-make,
      rust-patch,
      rust-texinfo,
      rust-help2man,
    }: let
      rustStdenv = import ./stdenv-test.nix {
        inherit
          stdenv
          uutils-coreutils-noprefix
          rust-sed
          rust-grep
          rust-awk
          uutils-findutils
          rust-diffutils
          rust-file
          rust-tar
          rust-gzip
          rust-bzip2
          rust-xz
          rust-make
          rust-patch
          ;
      };
    in
      rustStdenv.mkDerivation {
        pname = "rust-nixpkgs-hello-test";
        version = "2.12.1";

        nativeBuildInputs = [rust-texinfo rust-help2man];

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
          description = "GNU hello built with the Rust stdenv";
          license = lib.licenses.gpl3Plus;
        };
      };

    # Test building zlib — a critical C library used by nearly everything.
    # Uses a simple configure + make (not autotools).
    rust-nixpkgs-zlib-test = {
      lib,
      stdenv,
      fetchurl,
      uutils-coreutils-noprefix,
      rust-sed,
      rust-grep,
      rust-awk,
      uutils-findutils,
      rust-diffutils,
      rust-file,
      rust-tar,
      rust-gzip,
      rust-bzip2,
      rust-xz,
      rust-make,
      rust-patch,
    }: let
      rustStdenv = import ./stdenv-test.nix {
        inherit
          stdenv
          uutils-coreutils-noprefix
          rust-sed
          rust-grep
          rust-awk
          uutils-findutils
          rust-diffutils
          rust-file
          rust-tar
          rust-gzip
          rust-bzip2
          rust-xz
          rust-make
          rust-patch
          ;
      };
    in
      rustStdenv.mkDerivation {
        pname = "rust-nixpkgs-zlib-test";
        version = "1.3.1";

        src = fetchurl {
          url = "https://github.com/madler/zlib/releases/download/v1.3.1/zlib-1.3.1.tar.gz";
          sha256 = "sha256-mpOyt9/ax3zrpaVYpYDnRmfdb+3kWFuR7vtg8Dty3yM=";
        };

        meta = {
          description = "zlib built with the Rust stdenv";
          license = lib.licenses.zlib;
        };
      };

    # Test building GNU patch — an autotools C package.
    rust-nixpkgs-gnupatch-test = {
      lib,
      stdenv,
      fetchurl,
      uutils-coreutils-noprefix,
      rust-sed,
      rust-grep,
      rust-awk,
      uutils-findutils,
      rust-diffutils,
      rust-file,
      rust-tar,
      rust-gzip,
      rust-bzip2,
      rust-xz,
      rust-make,
      rust-patch,
      rust-texinfo,
      rust-help2man,
    }: let
      rustStdenv = import ./stdenv-test.nix {
        inherit
          stdenv
          uutils-coreutils-noprefix
          rust-sed
          rust-grep
          rust-awk
          uutils-findutils
          rust-diffutils
          rust-file
          rust-tar
          rust-gzip
          rust-bzip2
          rust-xz
          rust-make
          rust-patch
          ;
      };
    in
      rustStdenv.mkDerivation {
        pname = "rust-nixpkgs-gnupatch-test";
        version = "2.8";

        nativeBuildInputs = [rust-texinfo rust-help2man];

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
          description = "GNU patch built with the Rust stdenv";
          license = lib.licenses.gpl3Plus;
        };
      };

    # Test building GNU coreutils — a large autotools package with 100+ programs.
    rust-nixpkgs-coreutils-test = {
      lib,
      stdenv,
      fetchurl,
      uutils-coreutils-noprefix,
      rust-sed,
      rust-grep,
      rust-awk,
      uutils-findutils,
      rust-diffutils,
      rust-file,
      rust-tar,
      rust-gzip,
      rust-bzip2,
      rust-xz,
      rust-make,
      rust-patch,
      rust-texinfo,
      rust-help2man,
    }: let
      rustStdenv = import ./stdenv-test.nix {
        inherit
          stdenv
          uutils-coreutils-noprefix
          rust-sed
          rust-grep
          rust-awk
          uutils-findutils
          rust-diffutils
          rust-file
          rust-tar
          rust-gzip
          rust-bzip2
          rust-xz
          rust-make
          rust-patch
          ;
      };
    in
      rustStdenv.mkDerivation {
        pname = "rust-nixpkgs-coreutils-test";
        version = "9.6";

        nativeBuildInputs = [rust-texinfo rust-help2man];

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
          description = "GNU coreutils built with the Rust stdenv";
          license = lib.licenses.gpl3Plus;
        };
      };
    # Test building GNU grep — autotools with regex library.
    rust-nixpkgs-gnugrep-test = {
      lib,
      stdenv,
      fetchurl,
      uutils-coreutils-noprefix,
      rust-sed,
      rust-grep,
      rust-awk,
      uutils-findutils,
      rust-diffutils,
      rust-file,
      rust-tar,
      rust-gzip,
      rust-bzip2,
      rust-xz,
      rust-make,
      rust-patch,
      rust-texinfo,
    }: let
      rustStdenv = import ./stdenv-test.nix {
        inherit
          stdenv
          uutils-coreutils-noprefix
          rust-sed
          rust-grep
          rust-awk
          uutils-findutils
          rust-diffutils
          rust-file
          rust-tar
          rust-gzip
          rust-bzip2
          rust-xz
          rust-make
          rust-patch
          ;
      };
    in
      rustStdenv.mkDerivation {
        pname = "rust-nixpkgs-gnugrep-test";
        version = "3.11";

        nativeBuildInputs = [rust-texinfo];

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
          description = "GNU grep built with the Rust stdenv";
          license = lib.licenses.gpl3Plus;
        };
      };

    # Test building GNU sed — autotools, exercises sed replacement compatibility.
    rust-nixpkgs-gnused-test = {
      lib,
      stdenv,
      fetchurl,
      uutils-coreutils-noprefix,
      rust-sed,
      rust-grep,
      rust-awk,
      uutils-findutils,
      rust-diffutils,
      rust-file,
      rust-tar,
      rust-gzip,
      rust-bzip2,
      rust-xz,
      rust-make,
      rust-patch,
      rust-texinfo,
    }: let
      rustStdenv = import ./stdenv-test.nix {
        inherit
          stdenv
          uutils-coreutils-noprefix
          rust-sed
          rust-grep
          rust-awk
          uutils-findutils
          rust-diffutils
          rust-file
          rust-tar
          rust-gzip
          rust-bzip2
          rust-xz
          rust-make
          rust-patch
          ;
      };
    in
      rustStdenv.mkDerivation {
        pname = "rust-nixpkgs-gnused-test";
        version = "4.9";

        nativeBuildInputs = [rust-texinfo];

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
          description = "GNU sed built with the Rust stdenv";
          license = lib.licenses.gpl3Plus;
        };
      };

    # Test building GNU diffutils — exercises diff/cmp/sdiff compatibility.
    rust-nixpkgs-gnudiffutils-test = {
      lib,
      stdenv,
      fetchurl,
      uutils-coreutils-noprefix,
      rust-sed,
      rust-grep,
      rust-awk,
      uutils-findutils,
      rust-diffutils,
      rust-file,
      rust-tar,
      rust-gzip,
      rust-bzip2,
      rust-xz,
      rust-make,
      rust-patch,
      rust-texinfo,
      rust-help2man,
    }: let
      rustStdenv = import ./stdenv-test.nix {
        inherit
          stdenv
          uutils-coreutils-noprefix
          rust-sed
          rust-grep
          rust-awk
          uutils-findutils
          rust-diffutils
          rust-file
          rust-tar
          rust-gzip
          rust-bzip2
          rust-xz
          rust-make
          rust-patch
          ;
      };
    in
      rustStdenv.mkDerivation {
        pname = "rust-nixpkgs-gnudiffutils-test";
        version = "3.10";

        nativeBuildInputs = [rust-texinfo rust-help2man];

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
          description = "GNU diffutils built with the Rust stdenv";
          license = lib.licenses.gpl3Plus;
        };
      };

    # Test building GNU make — builds make with rust-make (self-referential!).
    rust-nixpkgs-gnumake-test = {
      lib,
      stdenv,
      fetchurl,
      uutils-coreutils-noprefix,
      rust-sed,
      rust-grep,
      rust-awk,
      uutils-findutils,
      rust-diffutils,
      rust-file,
      rust-tar,
      rust-gzip,
      rust-bzip2,
      rust-xz,
      rust-make,
      rust-patch,
      rust-texinfo,
    }: let
      rustStdenv = import ./stdenv-test.nix {
        inherit
          stdenv
          uutils-coreutils-noprefix
          rust-sed
          rust-grep
          rust-awk
          uutils-findutils
          rust-diffutils
          rust-file
          rust-tar
          rust-gzip
          rust-bzip2
          rust-xz
          rust-make
          rust-patch
          ;
      };
    in
      rustStdenv.mkDerivation {
        pname = "rust-nixpkgs-gnumake-test";
        version = "4.4.1";

        nativeBuildInputs = [rust-texinfo];

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
          description = "GNU make built with the Rust stdenv (using rust-make!)";
          license = lib.licenses.gpl3Plus;
        };
      };

    # Test building GNU gawk — autotools with complex configure.
    rust-nixpkgs-gnuawk-test = {
      lib,
      stdenv,
      fetchurl,
      uutils-coreutils-noprefix,
      rust-sed,
      rust-grep,
      rust-awk,
      uutils-findutils,
      rust-diffutils,
      rust-file,
      rust-tar,
      rust-gzip,
      rust-bzip2,
      rust-xz,
      rust-make,
      rust-patch,
      rust-texinfo,
    }: let
      rustStdenv = import ./stdenv-test.nix {
        inherit
          stdenv
          uutils-coreutils-noprefix
          rust-sed
          rust-grep
          rust-awk
          uutils-findutils
          rust-diffutils
          rust-file
          rust-tar
          rust-gzip
          rust-bzip2
          rust-xz
          rust-make
          rust-patch
          ;
      };
    in
      rustStdenv.mkDerivation {
        pname = "rust-nixpkgs-gnuawk-test";
        version = "5.3.1";

        nativeBuildInputs = [rust-texinfo];

        src = fetchurl {
          url = "mirror://gnu/gawk/gawk-5.3.1.tar.xz";
          sha256 = "sha256-aU23ZIEqYjZCPU/0DOt7bExEEwG3KtUCu1wn4AzVb3g=";
        };

        postPatch = ''
          find . -name '*.in' -o -name configure -o -name aclocal.m4 \
            -o -name config.h.in -o -name Makefile.in -o -name config.in \
            | xargs touch
        '';

        doCheck = false;

        meta = {
          description = "GNU awk built with the Rust stdenv";
          license = lib.licenses.gpl3Plus;
        };
      };

    # Test building GNU findutils — autotools.
    rust-nixpkgs-gnufindutils-test = {
      lib,
      stdenv,
      fetchurl,
      uutils-coreutils-noprefix,
      rust-sed,
      rust-grep,
      rust-awk,
      uutils-findutils,
      rust-diffutils,
      rust-file,
      rust-tar,
      rust-gzip,
      rust-bzip2,
      rust-xz,
      rust-make,
      rust-patch,
      rust-texinfo,
    }: let
      rustStdenv = import ./stdenv-test.nix {
        inherit
          stdenv
          uutils-coreutils-noprefix
          rust-sed
          rust-grep
          rust-awk
          uutils-findutils
          rust-diffutils
          rust-file
          rust-tar
          rust-gzip
          rust-bzip2
          rust-xz
          rust-make
          rust-patch
          ;
      };
    in
      rustStdenv.mkDerivation {
        pname = "rust-nixpkgs-gnufindutils-test";
        version = "4.10.0";

        nativeBuildInputs = [rust-texinfo];

        src = fetchurl {
          url = "mirror://gnu/findutils/findutils-4.10.0.tar.xz";
          sha256 = "sha256-E4fgtn/yR9Kr3pmPkN+/cMFJE5Glnd/suK5ph4nwpPU=";
        };

        postPatch = ''
          find . -name '*.in' -o -name configure -o -name aclocal.m4 \
            -o -name config.h.in -o -name Makefile.in -o -name config.in \
            | xargs touch
        '';

        doCheck = false;

        meta = {
          description = "GNU findutils built with the Rust stdenv";
          license = lib.licenses.gpl3Plus;
        };
      };
  };
}
