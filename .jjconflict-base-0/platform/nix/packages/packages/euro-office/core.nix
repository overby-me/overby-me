# The Euro-Office `core` C++ engine, built from source against newer nixpkgs
# libraries (ICU 76, Boost, OpenSSL) rather than the old pinned versions
# upstream's build_3rdparty.py fetches. Phase 4; see ./PLAN.md.
#
# `THIRD_PARTY_PREPARED=TRUE` skips that fetch-and-compile step, which is what
# keeps the build pure. `EO_USE_SYSTEM_LIBS=ON` then makes common.cmake resolve
# those libraries with find_package against buildInputs instead of a prebuilt
# install-dir layout, and teaches it about Apple - MAC/_MAC defines,
# @loader_path RPATH, no GNU-ld flags - mirroring the old qmake `core_mac`
# scope. Both come from ./patches/0001-*.patch.
#
# Everything but the Qt/CEF desktop shell builds on aarch64-darwin: x2t and the
# format libraries (OOXML, ODF, RTF, MS-binary, PDF, EPUB, iWork, HWP), V8-free
# because doctrenderer uses JavaScriptCore. See PLAN.md §10.
#
# The patches apply cleanly against upstream and are meant to be submitted.
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
  # Targets to configure+build. Start small and grow as the port lands.
  buildTargets ? [
    "kernel"
    "graphics"
    "PdfFile"
    "DjVuFile"
    "XpsFile"
    "Fb2File"
    "HtmlFile2"
    "DocxRenderer"
    "OFDFile"
    "doctrenderer"
    "allfontsgen"
    "allthemesgen"
    "x2t"
  ],
  # Note: some libraries (EpubFile, OdfFile, OOXML, …) are not standalone CMake
  # entry points — they assume a parent already set CORE_ROOT_DIR / included
  # common.cmake. Those are reached transitively once the x2t tool is wired up.
}: let
  sources = import ./sources.nix {inherit lib;};
  fetch = name: fetchFromGitHub sources.repos.${name};

  # Vendored third-party sources that `core` compiles in-tree (graphics et al.).
  # Pinned to the same commits upstream's nc-fetch.sh scripts use.
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
in
  stdenv.mkDerivation {
    pname = "euro-office-core";
    inherit (sources) version;

    src = fetchFromGitHub sources.repos.core;

    patches = [
      ./patches/0001-build-cmake-support-macOS-and-system-third-party-lib.patch
      ./patches/0002-build-cmake-build-graphics-with-its-macOS-sources-on.patch
      ./patches/0003-build-cmake-build-kernel_network-with-its-macOS-tran.patch
      ./patches/0004-fix-common-use-_MAC-consistently-in-Directory.cpp.patch
      ./patches/0005-build-cmake-build-doctrenderer-with-JavaScriptCore-o.patch
      ./patches/0006-fix-doctrenderer-build-hash.cpp-against-OpenSSL-3.x-n.patch
      ./patches/0007-build-cmake-link-system-zlib-for-IWorkFile-on-macOS.patch
      ./patches/0008-build-cmake-resolve-desktop-Qt-CEF-ICU-from-system-on.patch
      # Linux has no JavaScriptCore (the macOS V8-free path) and we don't ship
      # V8, so build a stub DoctRenderer/DocBuilder via -DDISABLE_DOCT_RENDERER.
      # The desktop editor renders through CEF's own V8; core's headless
      # DoctRenderer (mail-merge, x2t bin->PDF/HTML/image, DocBuilder) is unused
      # by the GUI. See ./PLAN.md (Linux, option B).
      ./patches/0009-build-cmake-allow-building-doctrenderer-without-a-JS.patch
    ];

    # install_name_tool (used in installPhase to fix tool RPATHs) ships with the
    # darwin stdenv, so no extra nativeBuildInput is needed for it.
    nativeBuildInputs = [
      cmake
      ninja
      pkg-config
      python3
    ];

    buildInputs = [
      boost
      icu
      openssl
      curl
      zlib
    ];

    # The project root CMakeLists drives the *tools* (x2t, allfontsgen, …) which
    # pull in the whole graph. While porting we instead configure the individual
    # library subdirectories listed in `buildTargets`, each of which is a
    # self-contained CMake project.
    dontUseCmakeConfigure = true;

    # Pinned vendored third-party source checkouts, assembled below into the
    # EO_CORE_3RD_PARTY_INSTALL_DIR layout the CMake source lists expect.
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

    configurePhase = ''
      runHook preConfigure

      # Re-assemble EO_CORE_3RD_PARTY_INSTALL_DIR from the pinned source
      # checkouts, applying the exact in-tree patches upstream applies
      # (Common/3dParty/*/...). This replaces build_3rdparty.py, which would
      # otherwise clone + patch + compile during configure (network-dependent).
      tp="$PWD/third-party-install"
      mkdir -p "$tp/html"

      cp -r --no-preserve=mode,ownership "$katanaSrc"   "$tp/html/katana-parser"
      cp -r --no-preserve=mode,ownership "$gumboSrc"    "$tp/html/gumbo-parser"
      cp -r --no-preserve=mode,ownership "$harfbuzzSrc" "$tp/harfbuzz"
      cp -r --no-preserve=mode,ownership "$brotliSrc"   "$tp/brotli"
      # graphics #includes hyphen sources as "hyphen/..." relative to the
      # install dir, which is on its include path.
      cp -r --no-preserve=mode,ownership "$hyphenSrc"   "$tp/hyphen"
      # md4c: HtmlFile2 compiles src/{md4c,md4c-html,entity}.{c,h} from $tp/md.
      cp -r --no-preserve=mode,ownership "$md4cSrc"     "$tp/md"

      # Apple iWork import stack (Apple/IWorkFile compiles these in-tree), laid
      # out under $tp/apple as Common/3dParty/apple/fetch.sh expects.
      mkdir -p "$tp/apple"
      cp -r --no-preserve=mode,ownership "$glmSrc"        "$tp/apple/glm"
      cp -r --no-preserve=mode,ownership "$mddsSrc"       "$tp/apple/mdds"
      cp -r --no-preserve=mode,ownership "$librevengeSrc" "$tp/apple/librevenge"
      cp -r --no-preserve=mode,ownership "$libodfgenSrc"  "$tp/apple/libodfgen"
      cp -r --no-preserve=mode,ownership "$libetonyekSrc" "$tp/apple/libetonyek"

      # socket.io-client-cpp with its pinned submodules placed where its
      # .gitmodules expect them (lib/asio, lib/websocketpp, lib/rapidjson).
      cp -r --no-preserve=mode,ownership "$socketioSrc" "$tp/socketio"
      rmdir "$tp/socketio/lib/asio" "$tp/socketio/lib/websocketpp" "$tp/socketio/lib/rapidjson" 2>/dev/null || true
      cp -r --no-preserve=mode,ownership "$asioSrc"        "$tp/socketio/lib/asio"
      cp -r --no-preserve=mode,ownership "$websocketppSrc" "$tp/socketio/lib/websocketpp"
      cp -r --no-preserve=mode,ownership "$rapidjsonSrc"   "$tp/socketio/lib/rapidjson"

      # Upstream's in-tree fixes/patches (paths relative to the core src tree).
      ( cd "$tp/html/gumbo-parser"  && patch -p1 --batch --forward </dev/null -i "$src/Common/3dParty/html/gumbo.patch" )
      ( cd "$tp/html/katana-parser" && patch -p1 --batch --forward </dev/null -i "$src/Common/3dParty/html/katana.patch" )
      ( cd "$tp/harfbuzz"           && patch -p1 --batch --forward </dev/null -i "$src/Common/3dParty/harfbuzz/patch/harfbuzz.patch" )
      # gumbo: the non-patch sed fix from Common/3dParty/html/fetch.py
      substituteInPlace "$tp/html/gumbo-parser/src/tag.c" \
        --replace-quiet "isspace(*c)" "isspace((unsigned char)*c)"

      # socket.io: apply upstream's proper_patches, then derive src_no_tls (a
      # copy of src) exactly as Common/3dParty/socketio/nc-fetch.sh does.
      siop="$src/Common/3dParty/socketio/patches/proper_patches"
      ( cd "$tp/socketio/lib/websocketpp" && patch -p1 --batch --forward </dev/null -i "$siop/websocketpp.patch" )
      # The sio_client_impl_*.patch diffs are rooted at the repo (a/src/...),
      # so apply them from the socketio root with -p1.
      ( cd "$tp/socketio" \
          && patch -p1 --batch --forward </dev/null -i "$siop/sio_client_impl_fail.patch" \
          && patch -p1 --batch --forward </dev/null -i "$siop/sio_client_impl_open.patch" \
          && patch -p1 --batch --forward </dev/null -i "$siop/sio_client_impl_close_timeout.patch" )
      cp -r --no-preserve=mode,ownership "$tp/socketio/src" "$tp/socketio/src_no_tls"
      ( cd "$tp/socketio" && patch -p1 --batch --forward </dev/null -i "$siop/no_tls.patch" )

      # apple stack patches (Common/3dParty/apple/*.patch)
      applep="$src/Common/3dParty/apple"
      ( cd "$tp/apple/mdds"       && patch -p1 --batch --forward </dev/null -i "$applep/mdds.patch" )
      ( cd "$tp/apple/librevenge" && patch -p1 --batch --forward </dev/null -i "$applep/librevenge.patch" )
      ( cd "$tp/apple/libetonyek" && patch -p1 --batch --forward </dev/null -i "$applep/libetonyek.patch" )

      export EO_THIRD_PARTY_INSTALL_DIR="$tp"

      declare -A targetDirs=(
        [kernel]="Common"
        [kernel_network]="Common/Network"
        [UnicodeConverter]="UnicodeConverter"
        [graphics]="DesktopEditor/graphics/cmake"
        [PdfFile]="PdfFile"
        [DjVuFile]="DjVuFile"
        [XpsFile]="XpsFile"
        [Fb2File]="Fb2File"
        [HtmlFile2]="HtmlFile2"
        [DocxRenderer]="DocxRenderer"
        [OFDFile]="OFDFile"
        [doctrenderer]="DesktopEditor/doctrenderer"
        [allfontsgen]="DesktopEditor/AllFontsGen"
        [allthemesgen]="DesktopEditor/allthemesgen"
        [x2t]="X2tConverter/build/cmake"
      )

      for t in ${lib.escapeShellArgs buildTargets}; do
        dir="''${targetDirs[$t]:-}"
        if [ -z "$dir" ]; then
          echo "core.nix: unknown buildTarget '$t'" >&2
          exit 1
        fi
        echo "=== configuring $t ($dir) ==="
        cmake -G Ninja -S "$dir" -B "build/$t" \
          -DCMAKE_BUILD_TYPE=Release \
          -DTHIRD_PARTY_PREPARED=TRUE \
          -DEO_USE_SYSTEM_LIBS=ON \
          ${lib.optionalString stdenv.hostPlatform.isLinux "-DDISABLE_DOCT_RENDERER=ON"} \
          -DEO_CORE_3RD_PARTY_INSTALL_DIR="$EO_THIRD_PARTY_INSTALL_DIR" \
          -DEO_CORE_OUTPUT_DIR="$PWD/package" \
          -DEO_CORE_TOOLS_DIR="$PWD/package"
      done

      runHook postConfigure
    '';

    buildPhase = ''
      runHook preBuild

      for t in ${lib.escapeShellArgs buildTargets}; do
        echo "=== building $t ==="
        cmake --build "build/$t" --parallel "$NIX_BUILD_CORES"
      done

      runHook postBuild
    '';

    installPhase = ''
      runHook preInstall

      mkdir -p "$out/lib" "$out/bin"

      # copy_artifacts_to_folder() drops both libraries and tool executables in
      # the package dir. Sort them: shared libs to lib/, everything else that is
      # a Mach-O executable to bin/.
      if [ -d package ]; then
        for f in package/*; do
          [ -e "$f" ] || continue
          case "$f" in
            *.dylib|*.so|*.so.*) cp -v "$f" "$out/lib/" ;;
            *) if file "$f" | grep -q 'executable'; then
                 cp -v "$f" "$out/bin/"
               else
                 cp -v "$f" "$out/lib/"
               fi ;;
          esac
        done
      fi

      # Also collect any built shared libs the POST_BUILD copy missed.
      find build -type f \( -name '*.dylib' -o -name '*.so*' \) \
        -exec cp -v {} "$out/lib/" \; 2>/dev/null || true

      # The tool binaries are built with an origin-relative RPATH (siblings),
      # but installed into bin/ with the libraries in ../lib. Point them there.
      # Mach-O uses @loader_path + install_name_tool; ELF uses $ORIGIN + patchelf.
      if [ -d "$out/bin" ]; then
        for exe in "$out/bin"/*; do
          [ -f "$exe" ] || continue
          install_name_tool -add_rpath "@loader_path/../lib" "$exe" 2>/dev/null || true
          patchelf --add-rpath '$ORIGIN/../lib' "$exe" 2>/dev/null || true
        done
      fi

      rmdir "$out/bin" 2>/dev/null || true

      runHook postInstall
    '';

    meta = {
      description = "Euro-Office core C++ document-conversion engine (from-source: x2t + format libs, V8-free)";
      mainProgram = "x2t";
      homepage = "https://github.com/Euro-Office/core";
      license = lib.licenses.agpl3Plus;
      sourceProvenance = with lib.sourceTypes; [fromSource];
      maintainers = with lib.maintainers; [overby-me];
      platforms = lib.platforms.darwin ++ lib.platforms.linux;
    };
  }
