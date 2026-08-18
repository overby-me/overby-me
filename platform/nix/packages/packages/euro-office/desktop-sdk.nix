# `ascdocumentscore`, the CEF client library that hosts the editors payload:
# desktop-sdk's own ABI-matched libcef_dll wrapper for CEF branch 109, linked
# against the prebuilt framework. Phase 6; see ./PLAN.md.
#
# The Qt `qtascdocumentscore`/QCefView wrapper is deliberately not built: its
# platform impl is X11/Win32-only, so it is the Linux and Windows hosting path.
# On macOS the Cocoa app links this library and embeds CEF in NSViews itself.
#
# Recompiles `core` alongside the desktop bits, because desktop-sdk's
# CMakeLists pulls core in as add_subdirectory()s. That is the upstream build
# graph; core.nix is the converter-only subset of it.
#
# What distinguishes it from core.nix: Qt 5.15 (reached through common.cmake's
# EO_USE_SYSTEM_LIBS + BUILD_DESKTOP, added in ./patches/0008-*), the CEF 109
# package passed via CEF_ROOT, and hunspell for the spellchecker wrapper.
#
# The GUI bundle itself is a separate Xcode project; this builds only the
# libraries it links.
{
  lib,
  stdenv,
  fetchFromGitHub,
  cmake,
  ninja,
  pkg-config,
  python3,
  boost,
  icu,
  openssl,
  curl,
  zlib,
  hunspell,
  libsForQt5,
  # Linux desktop host deps: ascdocumentscore links GTK3 + X11/xcb/xkbcommon
  # (the QCefView/X11 hosting path), not Qt (Qt is only needed at configure for
  # common.cmake's find_package(Qt5)). macOS uses the NSView path instead.
  gtk3,
  glib,
  atk,
  libx11,
  libxcb,
  libxkbcommon,
  cef,
}: let
  sources = import ./sources.nix {inherit lib;};
  fetch = name: fetchFromGitHub sources.repos.${name};

  coreSrc = fetch "core";
  desktopSdkSrc = fetch "desktop-sdk";

  # Vendored third-party sources that `core` compiles in-tree (graphics et al.).
  katana-parser = fetch "katana-parser";
  gumbo-parser = fetch "gumbo-parser";
  harfbuzz = fetch "harfbuzz";
  brotli = fetch "brotli";
  hyphen = fetch "hyphen";
  socketio = fetch "socketio";
  asio = fetch "asio";
  websocketpp = fetch "websocketpp";
  rapidjson = fetch "rapidjson";
  md4c = fetch "md4c";
  glm = fetch "glm";
  mdds = fetch "mdds";
  librevenge = fetch "librevenge";
  libodfgen = fetch "libodfgen";
  libetonyek = fetch "libetonyek";

  # common.cmake's BUILD_DESKTOP path resolves Qt5 via find_package even though
  # the ascdocumentscore library itself does not link Qt (only the unused
  # qt_wrapper does). Provide Qt5 so configure succeeds.
  inherit (libsForQt5) qtbase qtsvg qtmultimedia qttools;
in
  stdenv.mkDerivation {
    pname = "euro-office-desktop-sdk";
    inherit (sources) version;

    # Use the desktop-sdk repo as the primary source; core is assembled as a
    # sibling in the configurePhase so the CMakeLists' "../../../core" resolves.
    src = desktopSdkSrc;

    # core's CMake patches (incl. 0008 desktop Qt/CEF/ICU-from-system) are
    # applied to the core tree below, not to the desktop-sdk src.
    corePatches = [
      ./patches/0001-build-cmake-support-macOS-and-system-third-party-lib.patch
      ./patches/0002-build-cmake-build-graphics-with-its-macOS-sources-on.patch
      ./patches/0003-build-cmake-build-kernel_network-with-its-macOS-tran.patch
      ./patches/0004-fix-common-use-_MAC-consistently-in-Directory.cpp.patch
      ./patches/0005-build-cmake-build-doctrenderer-with-JavaScriptCore-o.patch
      ./patches/0006-fix-doctrenderer-build-hash.cpp-against-OpenSSL-3.x-n.patch
      ./patches/0007-build-cmake-link-system-zlib-for-IWorkFile-on-macOS.patch
      ./patches/0008-build-cmake-resolve-desktop-Qt-CEF-ICU-from-system-on.patch
      # Linux builds the doctrenderer subdir without a JS engine (no V8); see
      # core.nix and ./PLAN.md (Linux, option B).
      ./patches/0009-build-cmake-allow-building-doctrenderer-without-a-JS.patch
    ];

    # desktop-sdk's own CMake patches (CEF framework path on macOS, etc.).
    desktopSdkPatches = [
      ./patches/desktop-sdk-0001-cmake-link-CEF-framework-from-CEF_ROOT-on-macOS.patch
      ./patches/desktop-sdk-0002-cmake-set-CORE_ROOT_DIR-before-including-common.patch
      ./patches/desktop-sdk-0003-fix-use-boost-filesystem-path-instead-of-removed-wpat.patch
      ./patches/desktop-sdk-0004-fix-guard-X11-only-qt-wrapper-code-against-_MAC.patch
    ];

    nativeBuildInputs = [
      cmake
      ninja
      pkg-config
      python3
      qttools
    ];

    buildInputs =
      [
        boost
        icu
        openssl
        curl
        zlib
        hunspell
        qtbase
        qtsvg
        qtmultimedia
        cef
      ]
      # The Linux ascdocumentscore host links GTK3 + X11/xcb/xkbcommon directly
      # (cefview.cpp's X11 windowing); macOS hosts CEF in NSViews instead.
      ++ lib.optionals stdenv.hostPlatform.isLinux [
        gtk3
        glib
        atk
        libx11
        libxcb
        libxkbcommon
      ];

    dontUseCmakeConfigure = true;
    # We build a library, not a Qt application bundle; Qt is only present so
    # common.cmake's find_package(Qt5) succeeds at configure time.
    dontWrapQtApps = true;

    # The desktop-sdk sources (x2t.h, the CEF cefwrapper) use non-literal printf
    # format strings, which nixpkgs' default `format` hardening turns into hard
    # errors via -Werror=format-security. Upstream's toolchain doesn't enable it;
    # disable that one hardening flag (the macOS path suppresses it in-tree with
    # -Wno-error=format-security, patch 0001).
    hardeningDisable = ["format"];

    katanaSrc = katana-parser;
    gumboSrc = gumbo-parser;
    harfbuzzSrc = harfbuzz;
    brotliSrc = brotli;
    hyphenSrc = hyphen;
    socketioSrc = socketio;
    asioSrc = asio;
    websocketppSrc = websocketpp;
    rapidjsonSrc = rapidjson;
    md4cSrc = md4c;
    glmSrc = glm;
    mddsSrc = mdds;
    librevengeSrc = librevenge;
    libodfgenSrc = libodfgen;
    libetonyekSrc = libetonyek;

    inherit coreSrc;

    configurePhase = ''
      runHook preConfigure

      # --- Lay out the source tree the desktop-sdk CMakeLists expects ---------
      # desktop-sdk/ChromiumBasedEditors/lib/CMakeLists.txt references the core
      # repo at "../../../core" relative to itself. Recreate that sibling layout
      # in a writable workspace.
      ws="$PWD/workspace"
      mkdir -p "$ws"
      cp -r --no-preserve=mode,ownership "$coreSrc" "$ws/core"
      # The desktop-sdk source is the unpacked $src (current dir minus our
      # generated workspace). Copy everything except the workspace itself.
      mkdir -p "$ws/desktop-sdk"
      shopt -s dotglob
      for entry in *; do
        [ "$entry" = "workspace" ] && continue
        cp -r --no-preserve=mode,ownership "$entry" "$ws/desktop-sdk/"
      done
      shopt -u dotglob

      # Apply the core CMake patches to the copied core tree.
      ( cd "$ws/core"
        for p in $corePatches; do
          echo "applying core patch $p"
          patch -p1 --batch --forward < "$p"
        done )

      # Apply desktop-sdk CMake patches.
      ( cd "$ws/desktop-sdk"
        for p in $desktopSdkPatches; do
          echo "applying desktop-sdk patch $p"
          patch -p1 --batch --forward < "$p"
        done )

      src="$ws/core" # the third-party in-tree patches live under the core tree

      # --- Assemble the third-party install dir (same as core.nix) ------------
      tp="$PWD/third-party-install"
      mkdir -p "$tp/html"

      cp -r --no-preserve=mode,ownership "$katanaSrc"   "$tp/html/katana-parser"
      cp -r --no-preserve=mode,ownership "$gumboSrc"    "$tp/html/gumbo-parser"
      cp -r --no-preserve=mode,ownership "$harfbuzzSrc" "$tp/harfbuzz"
      cp -r --no-preserve=mode,ownership "$brotliSrc"   "$tp/brotli"
      cp -r --no-preserve=mode,ownership "$hyphenSrc"   "$tp/hyphen"
      cp -r --no-preserve=mode,ownership "$md4cSrc"     "$tp/md"

      mkdir -p "$tp/apple"
      cp -r --no-preserve=mode,ownership "$glmSrc"        "$tp/apple/glm"
      cp -r --no-preserve=mode,ownership "$mddsSrc"       "$tp/apple/mdds"
      cp -r --no-preserve=mode,ownership "$librevengeSrc" "$tp/apple/librevenge"
      cp -r --no-preserve=mode,ownership "$libodfgenSrc"  "$tp/apple/libodfgen"
      cp -r --no-preserve=mode,ownership "$libetonyekSrc" "$tp/apple/libetonyek"

      cp -r --no-preserve=mode,ownership "$socketioSrc" "$tp/socketio"
      rmdir "$tp/socketio/lib/asio" "$tp/socketio/lib/websocketpp" "$tp/socketio/lib/rapidjson" 2>/dev/null || true
      cp -r --no-preserve=mode,ownership "$asioSrc"        "$tp/socketio/lib/asio"
      cp -r --no-preserve=mode,ownership "$websocketppSrc" "$tp/socketio/lib/websocketpp"
      cp -r --no-preserve=mode,ownership "$rapidjsonSrc"   "$tp/socketio/lib/rapidjson"

      ( cd "$tp/html/gumbo-parser"  && patch -p1 --batch --forward </dev/null -i "$src/Common/3dParty/html/gumbo.patch" )
      ( cd "$tp/html/katana-parser" && patch -p1 --batch --forward </dev/null -i "$src/Common/3dParty/html/katana.patch" )
      ( cd "$tp/harfbuzz"           && patch -p1 --batch --forward </dev/null -i "$src/Common/3dParty/harfbuzz/patch/harfbuzz.patch" )
      substituteInPlace "$tp/html/gumbo-parser/src/tag.c" \
        --replace-quiet "isspace(*c)" "isspace((unsigned char)*c)"

      siop="$src/Common/3dParty/socketio/patches/proper_patches"
      ( cd "$tp/socketio/lib/websocketpp" && patch -p1 --batch --forward </dev/null -i "$siop/websocketpp.patch" )
      ( cd "$tp/socketio" \
          && patch -p1 --batch --forward </dev/null -i "$siop/sio_client_impl_fail.patch" \
          && patch -p1 --batch --forward </dev/null -i "$siop/sio_client_impl_open.patch" \
          && patch -p1 --batch --forward </dev/null -i "$siop/sio_client_impl_close_timeout.patch" )
      cp -r --no-preserve=mode,ownership "$tp/socketio/src" "$tp/socketio/src_no_tls"
      ( cd "$tp/socketio" && patch -p1 --batch --forward </dev/null -i "$siop/no_tls.patch" )

      applep="$src/Common/3dParty/apple"
      ( cd "$tp/apple/mdds"       && patch -p1 --batch --forward </dev/null -i "$applep/mdds.patch" )
      ( cd "$tp/apple/librevenge" && patch -p1 --batch --forward </dev/null -i "$applep/librevenge.patch" )
      ( cd "$tp/apple/libetonyek" && patch -p1 --batch --forward </dev/null -i "$applep/libetonyek.patch" )

      export EO_THIRD_PARTY_INSTALL_DIR="$tp"
      export CEF_ROOT="${cef}"

      # --- Configure ascdocumentscore (the CEF client lib + core deps) --------
      # On macOS the GUI links ascdocumentscore directly (see the header note);
      # the lib/ CMakeLists is the entry point that builds it and pulls in the
      # core libraries it needs.
      cmake -G Ninja \
        -S "$ws/desktop-sdk/ChromiumBasedEditors/lib" \
        -B build \
        -DCMAKE_BUILD_TYPE=Release \
        -DBUILD_DESKTOP=ON \
        -DTHIRD_PARTY_PREPARED=TRUE \
        -DEO_USE_SYSTEM_LIBS=ON \
        ${lib.optionalString stdenv.hostPlatform.isLinux "-DDISABLE_DOCT_RENDERER=ON"} \
        -DEO_CORE_3RD_PARTY_INSTALL_DIR="$EO_THIRD_PARTY_INSTALL_DIR" \
        -DEO_CORE_OUTPUT_DIR="$PWD/package" \
        -DEO_CORE_TOOLS_DIR="$PWD/package" \
        -DCEF_ROOT="$CEF_ROOT"

      runHook postConfigure
    '';

    buildPhase = ''
      runHook preBuild
      cmake --build build --parallel "$NIX_BUILD_CORES"
      runHook postBuild
    '';

    installPhase = ''
      runHook preInstall

      # The built shared libraries: libascdocumentscore.dylib (the CEF client
      # the macOS app links) plus the core libraries it depends on.
      mkdir -p "$out/lib" "$out/bin"
      find build -type f \( -name '*.dylib' -o -name '*.so*' \) \
        -exec cp -v {} "$out/lib/" \; 2>/dev/null || true

      # The CEF subprocess helper executable (goes inside editors_helper.app in
      # the final bundle; the main app points browser_subprocess_path at it).
      if [ -f build/bin/editors_helper ]; then
        cp -v build/bin/editors_helper "$out/bin/"
      else
        find build -type f -name editors_helper -exec cp -v {} "$out/bin/" \;
      fi

      # ascdocumentscore's public headers, for the Xcode app / other consumers.
      # (configurePhase reassigned $src, so use the workspace copy explicitly.)
      mkdir -p "$out/include"
      cp -r "$PWD/workspace/desktop-sdk/ChromiumBasedEditors/lib/include/." "$out/include/" 2>/dev/null || true

      # Point consumers at the matching CEF framework (the dylibs above link it
      # via @executable_path/../Frameworks/Chromium Embedded Framework.framework;
      # the app bundle must ship that framework from here).
      ln -s "${cef}/Release" "$out/cef"

      runHook postInstall
    '';

    # The dylibs reference the CEF framework via @executable_path; keep that.
    dontFixup = true;

    passthru = {inherit cef;};

    meta = {
      description = "Euro-Office desktop-sdk CEF integration (ascdocumentscore, from-source)";
      homepage = "https://github.com/Euro-Office/desktop-sdk";
      license = lib.licenses.agpl3Plus;
      sourceProvenance = with lib.sourceTypes; [fromSource];
      maintainers = with lib.maintainers; [overby-me];
      platforms = lib.platforms.darwin ++ lib.platforms.linux;
    };
  }
