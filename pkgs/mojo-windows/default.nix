# mojo-windows — KGEN runtime shim for cross-compiling Mojo to Windows
#
# The Mojo compiler (Linux) can emit Windows COFF objects via:
#   mojo build --target-triple x86_64-pc-windows-gnu --emit object
#
# However, linking requires the KGEN CompilerRT symbols that are normally
# provided by libKGENCompilerRTShared.so (Linux-only, closed-source).
# This package cross-compiles a minimal C shim (kgen_rt_shim.c) that
# implements those symbols for Windows, producing a static library
# (libkgen_rt.a) that can be linked via MinGW-w64.
#
# Usage in a dev shell:
#   1. Compile Mojo source to Windows object:
#        mojo build --target-triple x86_64-pc-windows-gnu --emit object \
#          -o app.obj app.mojo
#   2. Link with MinGW-w64:
#        x86_64-w64-mingw32-gcc app.obj -o app.exe \
#          -L$(mojo-windows-libdir) -lkgen_rt
#
# The package also provides helper scripts:
#   - mojo-windows-libdir: prints the path to libkgen_rt.a
#   - mojo-windows-build:  one-shot compile + link pipeline
{
  lib,
  stdenv,
  writeShellScriptBin,
  symlinkJoin,
  pkgsCross,
}: let
  mingwGcc = pkgsCross.mingwW64.stdenv.cc;
  mingwPthreads = pkgsCross.mingwW64.windows.pthreads;
  mingwMcfgthreads = pkgsCross.mingwW64.windows.mcfgthreads;

  # Cross-compile the KGEN runtime shim to a Windows static library
  kgenRtLib = stdenv.mkDerivation {
    pname = "mojo-windows-lib";
    version = "0.1.0";

    src = ./.;

    nativeBuildInputs = [mingwGcc];

    buildPhase = ''
      x86_64-w64-mingw32-gcc \
        -c -O2 -Wall -Wextra -Wno-unused-parameter \
        -D_WIN32 \
        -o kgen_rt_shim.o \
        kgen_rt_shim.c

      x86_64-w64-mingw32-gcc \
        -c -O2 -Wall -Wextra -Wno-unused-parameter \
        -D_WIN32 \
        -o dlfcn_shim.o \
        dlfcn_shim.c

      x86_64-w64-mingw32-ar rcs libkgen_rt.a kgen_rt_shim.o dlfcn_shim.o
    '';

    installPhase = ''
      mkdir -p $out/lib
      cp libkgen_rt.a $out/lib/
    '';

    doInstallCheck = true;
    installCheckPhase = ''
      echo "Checking libkgen_rt.a symbols..."
      x86_64-w64-mingw32-nm $out/lib/libkgen_rt.a | grep -q KGEN_CompilerRT_AlignedAlloc
      x86_64-w64-mingw32-nm $out/lib/libkgen_rt.a | grep -q KGEN_CompilerRT_AsyncRT_CreateRuntime
      x86_64-w64-mingw32-nm $out/lib/libkgen_rt.a | grep -q KGEN_CompilerRT_GetOrCreateGlobal
      x86_64-w64-mingw32-nm $out/lib/libkgen_rt.a | grep -q KGEN_CompilerRT_DestroyGlobals
      x86_64-w64-mingw32-nm $out/lib/libkgen_rt.a | grep -q KGEN_CompilerRT_SetArgV
      x86_64-w64-mingw32-nm $out/lib/libkgen_rt.a | grep -q KGEN_CompilerRT_PrintStackTraceOnFault
      x86_64-w64-mingw32-nm $out/lib/libkgen_rt.a | grep -q dlopen
      x86_64-w64-mingw32-nm $out/lib/libkgen_rt.a | grep -q dlsym
      x86_64-w64-mingw32-nm $out/lib/libkgen_rt.a | grep -q dlclose
      x86_64-w64-mingw32-nm $out/lib/libkgen_rt.a | grep -q dlerror
      echo "All expected symbols present ✅"
    '';

    meta = with lib; {
      description = "KGEN runtime shim static library for Windows (x86_64-pc-windows-gnu)";
      license = licenses.mit;
      platforms = platforms.linux;
    };
  };

  # Helper: print the library directory path
  mojoWindowsLibdir = writeShellScriptBin "mojo-windows-libdir" ''
    echo -n "${kgenRtLib}/lib"
  '';

  # Helper: one-shot cross-compile pipeline
  #   Usage: mojo-windows-build [-I path]... -o output.exe input.mojo
  mojoWindowsBuild = writeShellScriptBin "mojo-windows-build" ''
    set -euo pipefail

    KGEN_LIB="${kgenRtLib}/lib"
    MINGW_PTHREADS_LIB="${mingwPthreads}/lib"
    MINGW_MCF_LIB="${mingwMcfgthreads}/lib"

    # Parse arguments: collect -I, -o, and -D flags for mojo, plus the input file
    MOJO_ARGS=()
    OUTPUT=""
    INPUT=""
    LINK_ARGS=()

    while [[ $# -gt 0 ]]; do
      case "$1" in
        -o)
          OUTPUT="$2"
          shift 2
          ;;
        -I)
          MOJO_ARGS+=("-I" "$2")
          shift 2
          ;;
        -I*)
          MOJO_ARGS+=("$1")
          shift
          ;;
        -D)
          MOJO_ARGS+=("-D" "$2")
          shift 2
          ;;
        -D*)
          MOJO_ARGS+=("$1")
          shift
          ;;
        -Xlinker)
          LINK_ARGS+=("$2")
          shift 2
          ;;
        -l*)
          LINK_ARGS+=("$1")
          shift
          ;;
        -L*)
          LINK_ARGS+=("$1")
          shift
          ;;
        --emit|--target-triple|--target-cpu)
          # Silently drop — we set these ourselves
          shift 2
          ;;
        -*)
          MOJO_ARGS+=("$1")
          shift
          ;;
        *)
          INPUT="$1"
          shift
          ;;
      esac
    done

    if [[ -z "$INPUT" ]]; then
      echo "Usage: mojo-windows-build [-I path]... [-D key=val]... -o output.exe input.mojo" >&2
      exit 1
    fi
    if [[ -z "$OUTPUT" ]]; then
      OUTPUT="''${INPUT%.mojo}.exe"
    fi

    TMPOBJ=$(mktemp --suffix=.obj)
    trap 'rm -f "$TMPOBJ"' EXIT

    echo "mojo-windows-build: compiling $INPUT → object" >&2
    mojo build \
      --target-triple x86_64-pc-windows-gnu \
      --emit object \
      "''${MOJO_ARGS[@]}" \
      -o "$TMPOBJ" \
      "$INPUT"

    echo "mojo-windows-build: linking → $OUTPUT" >&2
    x86_64-w64-mingw32-gcc \
      "$TMPOBJ" \
      -o "$OUTPUT" \
      -L"$KGEN_LIB" \
      -L"$MINGW_PTHREADS_LIB" \
      -L"$MINGW_MCF_LIB" \
      -lkgen_rt \
      "''${LINK_ARGS[@]}"

    echo "mojo-windows-build: ✅ $OUTPUT" >&2
  '';
in
  symlinkJoin {
    name = "mojo-windows-0.1.0";

    paths = [
      kgenRtLib
      mojoWindowsLibdir
      mojoWindowsBuild
    ];

    meta = with lib; {
      description = "KGEN runtime shim for cross-compiling Mojo programs to Windows (x86_64-pc-windows-gnu)";
      license = licenses.mit;
      platforms = platforms.linux;
      maintainers = with lib.maintainers; [overby-me];
    };
  }
