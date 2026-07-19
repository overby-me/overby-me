# Flakelight module: checks for the ninja library. Imported explicitly from
# flake.nix (the nix/lib autoloader only routes default.nix).
#
# Run one: nix build .#checks.x86_64-linux.ninja-build-trivial
# (never `nix flake check`, see the repo rules)
{
  checks = {
    # End-to-end: extract the graph of the hand-written trivial manifest, lower
    # each edge to a derivation, build the `hello` target, and run it.
    ninja-build-trivial = pkgs: let
      drv = pkgs.lib.buildNinjaProject {
        src = ./tests/fixtures/trivial;
        target = "hello";
      };
    in
      pkgs.runCommand "ninja-build-trivial" {} ''
        ${drv}/hello > actual.txt
        grep -q "Hello from nix-ninja!" actual.txt
        touch $out
      '';

    # End-to-end: configure a real CMake project with `-G Ninja`, extract the
    # graph, lower each edge (two compiles + a link, plus CMake's ordering
    # edges), build the `greet` executable, and run it. Exercises absolute
    # store-path sources/includes and no-op ordering edges.
    ninja-build-cmake = pkgs: let
      drv = pkgs.lib.buildNinjaProject {
        cmakeSource = ./tests/fixtures/cmake;
        target = "greet";
      };
    in
      pkgs.runCommand "ninja-build-cmake" {} ''
        ${drv}/greet > actual.txt
        grep -q "Hello from cmake+nix-ninja!" actual.txt
        touch $out
      '';

    # End-to-end: a CMake project with a static library and a `configure_file`
    # generated header (in the build dir). Exercises multi-root rewriting
    # (source tree + configured build dir), `ar`/`ranlib`, and multiple targets.
    ninja-build-cmake-lib = pkgs: let
      drv = pkgs.lib.buildNinjaProject {
        cmakeSource = ./tests/fixtures/cmake-lib;
        target = "app";
      };
    in
      pkgs.runCommand "ninja-build-cmake-lib" {} ''
        ${drv}/app > actual.txt
        grep -q "Hello from cmake lib v1.2.3" actual.txt
        touch $out
      '';

    # M4: build the real Darling launcher (src/startup/darling) via nix-ninja —
    # configure the 38k-edge tree, target-filter to the launcher subtree, lower
    # and build. Per-file (depfile-precise) mode: the compile scan discovers the
    # source-relative `#include "../shellspawn/shellspawn.h"` and stages exactly
    # the files read. Impure (references the Darling source store path); build
    # with `nix build .#checks.x86_64-linux._ninja-darling-launcher --impure`.
    _ninja-darling-launcher = pkgs: let
      drv = pkgs.lib.buildNinjaProject {
        cmakeSource = builtins.storePath "/nix/store/zb85186kdllqgqdhnc08zmkh5iqrnr7v-qwyd0df0afrwmfdjy68a4gvjyhh30m6d-source";
        target = "src/startup/darling";
        cmakeFlags = [
          "-DTARGET_i386=OFF"
          "-DCOMPILE_PY2_BYTECODE=OFF"
          "-DCMAKE_C_COMPILER=clang"
          "-DCMAKE_CXX_COMPILER=clang++"
        ];
        configureNativeBuildInputs = with pkgs; [clang python3 bison flex pkg-config libcap makeWrapper];
        configureBuildInputs = with pkgs; [
          freetype
          libjpeg
          libpng
          libtiff
          giflib
          xorg.libX11
          xorg.libXext
          xorg.libXrandr
          xorg.libXcursor
          xorg.libxkbfile
          cairo
          libglvnd
          fontconfig
          dbus
          libGLU
          fuse
          ffmpeg
          pulseaudio
          libbsd
          openssl
          systemdLibs
          expat
          xorg.libXau
          xorg.libXdmcp
        ];
        toolchain = [pkgs.clang];
      };
    in
      pkgs.runCommand "ninja-darling-launcher" {} ''
        test -x ${drv}/src/startup/darling
        touch $out
      '';
  };
}
