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
      uutils-diffutils,
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
          uutils-diffutils
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
          echo "Patchelf: $(patchelf --version | head -1)"
          echo "Strip: $(strip --version | head -1)"
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

        # Force rebuild by changing env
        RUST_STDENV_TEST = "2";

        preBuild = ''
          echo "=== make -n lib/basename-lgpl.o ==="
          make -n lib/basename-lgpl.o 2>&1
          echo "EXIT: $?"
          echo "=== make -n all-am 2>&1 | head -5 ==="
          make -n all-am 2>&1 | head -5
          echo "=== make -d lib/basename-lgpl.o 2>&1 | head -20 ==="
          make -d lib/basename-lgpl.o 2>&1 | head -20 || true
          echo "=== end ==="
        '';

        meta = {
          description = "GNU hello built with the Rust stdenv";
          license = lib.licenses.gpl3Plus;
        };
      };
  };
}
