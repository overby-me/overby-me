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

    # A stdenv with all available Rust replacements swapped in
    stdenvRs = prev.stdenv.override {
      initialPath = replacedInitialPath;
      shell = "${shellPkg}/bin/bash";
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
      uutils-sed,
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
          uutils-sed
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
  };
}
