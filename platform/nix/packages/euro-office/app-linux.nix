# The Linux DesktopEditors GUI: the Qt application under
# `desktop-apps/win-linux`, whose CMakeLists builds the whole graph from source
# as subdirectories - ascdocumentscore, the QCefView/X11 host wrapper, and core
# - and links against Qt 5.15, GTK3, X11 and CEF 109. macOS takes a different
# path entirely; see app.nix. Phase 7, ./PLAN.md option B.
#
# V8-free via -DDISABLE_DOCT_RENDERER: the editor renders through CEF's own V8,
# so core's headless DoctRenderer is dead weight in the GUI.
#
# The CMake install lays out a flat `desktopeditors/` tree; on top of it go the
# editors payload, the converter, the fonts and dictionaries, a wrapped
# launcher and a desktop entry.
{
  lib,
  stdenv,
  fetchFromGitHub,
  cmake,
  ninja,
  pkg-config,
  python3,
  makeWrapper,
  copyDesktopItems,
  makeDesktopItem,
  # GTK runtime support: the app links GTK3 for theming + native dialogs, which
  # needs the gdk-pixbuf loaders, icon themes and the shared-mime database wired
  # up (GDK_PIXBUF_MODULE_FILE / XDG_DATA_DIRS) — without them GTK aborts trying
  # to load even its built-in fallback icons.
  wrapGAppsHook3,
  gdk-pixbuf,
  librsvg,
  shared-mime-info,
  adwaita-icon-theme,
  hicolor-icon-theme,
  gsettings-desktop-schemas,
  boost,
  icu,
  openssl,
  curl,
  zlib,
  hunspell,
  libsForQt5,
  gtk3,
  glib,
  atk,
  libnotify,
  cups,
  dbus,
  libx11,
  libxcb,
  libxext,
  libxkbcommon,
  cef,
  core,
  data,
  desktop-common,
}: let
  sources = import ./sources.nix {inherit lib;};
  fetch = name: fetchFromGitHub sources.repos.${name};

  coreSrc = fetch "core";
  desktopSdkSrc = fetch "desktop-sdk";
  desktopAppsSrc = fetch "desktop-apps";

  # Vendored third-party sources core compiles in-tree (same set as core.nix).
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

  inherit (libsForQt5) qtbase qtsvg qtmultimedia qttools qtx11extras wrapQtAppsHook;

  appName = "Euro-Office Desktop Editors";
  # The flat runtime tree the app expects (binary + libs + editors/ + converter/).
  prefix = "opt/euro-office/desktopeditors";
in
  stdenv.mkDerivation {
    pname = "euro-office-app";
    inherit (sources) version;

    # Build from the desktop-apps tree; core + desktop-sdk are assembled as
    # siblings in configurePhase so win-linux's ../../core, ../../desktop-sdk
    # relative paths resolve.
    src = desktopAppsSrc;

    corePatches = [
      ./patches/0001-build-cmake-support-macOS-and-system-third-party-lib.patch
      ./patches/0002-build-cmake-build-graphics-with-its-macOS-sources-on.patch
      ./patches/0003-build-cmake-build-kernel_network-with-its-macOS-tran.patch
      ./patches/0004-fix-common-use-_MAC-consistently-in-Directory.cpp.patch
      ./patches/0005-build-cmake-build-doctrenderer-with-JavaScriptCore-o.patch
      ./patches/0006-fix-doctrenderer-build-hash.cpp-against-OpenSSL-3.x-n.patch
      ./patches/0007-build-cmake-link-system-zlib-for-IWorkFile-on-macOS.patch
      ./patches/0008-build-cmake-resolve-desktop-Qt-CEF-ICU-from-system-on.patch
      ./patches/0009-build-cmake-allow-building-doctrenderer-without-a-JS.patch
    ];

    # desktop-sdk patches: the qt_wrapper (qtascdocumentscore) CORE_ROOT_DIR fix,
    # boost path, and the X11 guards — all relevant on the Linux Qt host path.
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
      makeWrapper
      wrapQtAppsHook
      wrapGAppsHook3
      copyDesktopItems
    ];

    buildInputs = [
      boost
      icu
      openssl
      curl
      zlib
      hunspell
      qtbase
      qtsvg
      qtmultimedia
      qtx11extras
      gtk3
      glib
      atk
      libnotify
      cups
      dbus
      libx11
      libxcb
      libxext
      libxkbcommon
      gdk-pixbuf
      librsvg
      shared-mime-info
      adwaita-icon-theme
      hicolor-icon-theme
      gsettings-desktop-schemas
      cef
    ];

    # The desktop-sdk / app sources use non-literal printf format strings that
    # nixpkgs' `format` hardening turns into -Werror=format-security errors.
    hardeningDisable = ["format"];

    dontUseCmakeConfigure = true;
    # We wrap the real binary (in $prefix) ourselves below, combining the Qt and
    # GApps wrapper args into one launcher.
    dontWrapQtApps = true;
    dontWrapGApps = true;

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

    inherit coreSrc desktopSdkSrc;
    payloadRoot = desktop-common;
    # Euro-Office title-bar logo (monochrome, with logo-light/logo-dark nodes).
    # Replaces the unregistered ONLYOFFICE :/logo.svg so the caption button
    # renders the EO mark instead of being blank.
    eoLogo = ./logo.svg;
    coreRoot = core;
    dataRoot = data;

    configurePhase = ''
            runHook preConfigure

            # --- sibling layout: core / desktop-sdk / desktop-apps ------------------
            ws="$PWD/workspace"
            mkdir -p "$ws"
            cp -r --no-preserve=mode,ownership "$coreSrc"       "$ws/core"
            cp -r --no-preserve=mode,ownership "$desktopSdkSrc" "$ws/desktop-sdk"
            mkdir -p "$ws/desktop-apps"
            shopt -s dotglob
            for entry in *; do
              [ "$entry" = "workspace" ] && continue
              cp -r --no-preserve=mode,ownership "$entry" "$ws/desktop-apps/"
            done
            shopt -u dotglob

            ( cd "$ws/core"
              for p in $corePatches; do echo "core patch $p"; patch -p1 --batch --forward < "$p"; done )
            ( cd "$ws/desktop-sdk"
              for p in $desktopSdkPatches; do echo "sdk patch $p"; patch -p1 --batch --forward < "$p"; done )

            # extras.qrc embeds common/loginpage/deploy/noconnect.html (the offline
            # "no connection" page), which the upstream loginpage web build deploys
            # from noconnect/index.html.deploy. Provision just that one file instead of
            # running the whole loginpage Node toolchain.
            mkdir -p "$ws/desktop-apps/common/loginpage/deploy"
            cp "$ws/desktop-apps/common/loginpage/noconnect/index.html.deploy" \
               "$ws/desktop-apps/common/loginpage/deploy/noconnect.html"

            # cthemes.cpp undefines the Qt signals/slots/emit keyword macros around its
            # GTK includes, but includes the Qt header cascapplicationmanagerwrapper.h
            # (which uses `signals:`) while they are still undefined, restoring them
            # only afterwards. That compiles only when <QColor>/<QJsonArray> have not
            # already pulled in qobjectdefs.h; with nixpkgs Qt 5.15 they have, so the
            # #undef sticks and the Qt header fails with "'signals' does not name a
            # type". Move the Qt include after the keyword macros are restored.
            # The caption button loads ":/logo.svg" (with logo-light/logo-dark nodes),
            # but Euro-Office dropped that resource (it was the ONLYOFFICE wordmark)
            # without registering a replacement, leaving the button blank. Install the
            # Euro-Office logo as res/icons/logo.svg and register the :/logo.svg alias.
            cp "$eoLogo" "$ws/desktop-apps/win-linux/res/icons/logo.svg"
            substituteInPlace "$ws/desktop-apps/win-linux/resources.qrc" \
              --replace-fail '<file alias="logo-light-eo.svg">res/icons/logo-light-eo.svg</file>' '<file alias="logo.svg">res/icons/logo.svg</file>
              <file alias="logo-light-eo.svg">res/icons/logo-light-eo.svg</file>'

            substituteInPlace "$ws/desktop-apps/win-linux/src/cthemes.cpp" \
              --replace-fail '# include "cascapplicationmanagerwrapper.h"

      # define signals Q_SIGNALS
      # define slots Q_SLOTS
      # define emit Q_EMIT
      # define foreach Q_FOREACH' '# define signals Q_SIGNALS
      # define slots Q_SLOTS
      # define emit Q_EMIT
      # define foreach Q_FOREACH
      # include "cascapplicationmanagerwrapper.h"'

            src="$ws/core" # third-party in-tree patches live under the core tree

            # --- third-party install dir (same as core.nix / desktop-sdk.nix) -------
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
            export PRODUCT_VERSION="${sources.version}"
            export BUILD_NUMBER="0"

            # The install() rules bundle the Qt runtime + vcpkg boost from QT_ROOT /
            # VCPKG_INSTALLED_DIR, which are empty under EO_USE_SYSTEM_LIBS (we ship
            # the nixpkgs Qt via wrapQtApp and resolve boost via rpath). Point them at
            # empty stub dirs so those install rules copy nothing instead of failing.
            qtstub="$PWD/qt-stub"; mkdir -p "$qtstub/lib" "$qtstub/plugins" "$qtstub/bin"
            vcpkgstub="$PWD/vcpkg-stub"; mkdir -p "$vcpkgstub/x64-linux/boost/linux_64/lib" "$vcpkgstub/x64-linux/bin"

            # --- configure the win-linux Qt application -----------------------------
            cmake -G Ninja \
              -S "$ws/desktop-apps/win-linux" \
              -B build \
              -DCMAKE_BUILD_TYPE=Release \
              -DBUILD_DESKTOP=ON \
              -DTHIRD_PARTY_PREPARED=TRUE \
              -DEO_USE_SYSTEM_LIBS=ON \
              -DDISABLE_DOCT_RENDERER=ON \
              -DEO_CORE_3RD_PARTY_INSTALL_DIR="$EO_THIRD_PARTY_INSTALL_DIR" \
              -DEO_CORE_OUTPUT_DIR="$PWD/package" \
              -DEO_CORE_TOOLS_DIR="$PWD/package" \
              -DCEF_ROOT="$CEF_ROOT" \
              -DQT_ROOT="$qtstub" \
              -DVCPKG_INSTALLED_DIR="$vcpkgstub" \
              -DVCPKG_TARGET_TRIPLET="x64-linux" \
              -DABOUT_PAGE_APP_NAME="${appName}" \
              -DCOMPANY_NAME="Euro-Office"

            runHook postConfigure
    '';

    buildPhase = ''
      runHook preBuild
      cmake --build build --parallel "$NIX_BUILD_CORES"
      runHook postBuild
    '';

    installPhase = ''
      runHook preInstall

      # The CMake install assembles build/desktopeditors/ — the flat runtime tree
      # (DesktopEditors binary + ascdocumentscore/qtascdocumentscore .so + CEF
      # libs/Resources + core libs in converter/).
      cmake --install build

      dst="$out/${prefix}"
      mkdir -p "$dst"
      cp -r build/desktopeditors/. "$dst/"

      # libcef.so is a ~1 GB prebuilt already patched by the cef package; replace
      # the copy with a symlink so fixup doesn't re-strip/patch it (slow + risky).
      if [ -e "$dst/libcef.so" ]; then
        rm -f "$dst/libcef.so"
        ln -s "${cef}/Release/libcef.so" "$dst/libcef.so"
      fi

      # --- overlay the genuine editors payload --------------------------------
      # The app loads its start page from applicationDirPath()/index.html and the
      # editors from sibling editors/, so lay the whole payload (index.html,
      # editors/, providers/, converter config, fonts/, dictionaries/) flat into
      # the app dir. The payload's converter/ merges with the native x2t + format
      # libs the CMake install already placed there.
      cp -r --no-preserve=mode,ownership "$payloadRoot"/. "$dst/"

      # --- launcher: cd into the app dir, set the lib search path + Qt env -----
      mkdir -p "$out/bin"
      # GDK_PIXBUF_MODULE_FILE + the icon/mime XDG dirs are what stop GTK from
      # aborting when it loads PNG icons (its built-in fallbacks included).
      # wrapGAppsHook only contributed GIO_EXTRA_MODULES here, so set them
      # explicitly against the gdk-pixbuf the app's gtk3 actually loads.
      makeWrapper "$dst/DesktopEditors" "$out/bin/euro-office-desktopeditors" \
        "''${qtWrapperArgs[@]}" \
        "''${gappsWrapperArgs[@]}" \
        --set-default GDK_PIXBUF_MODULE_FILE "${gdk-pixbuf}/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache" \
        --prefix XDG_DATA_DIRS : "${adwaita-icon-theme}/share:${hicolor-icon-theme}/share:${shared-mime-info}/share:${gsettings-desktop-schemas}/share/gsettings-schemas/${gsettings-desktop-schemas.name}" \
        --prefix XDG_DATA_DIRS : "$out/share" \
        --chdir "$dst" \
        --prefix LD_LIBRARY_PATH : "$dst" \
        --set-default QT_QPA_PLATFORM xcb

      # --- icon ----------------------------------------------------------------
      for icon in \
        "$ws/desktop-apps/win-linux/res/icons/Icon.svg" \
        "$ws/desktop-apps/win-linux/src/icons/desktopeditors.svg"; do
        if [ -f "$icon" ]; then
          install -Dm644 "$icon" "$out/share/icons/hicolor/scalable/apps/euro-office-desktopeditors.svg"
          break
        fi
      done

      runHook postInstall
    '';

    desktopItems = [
      (makeDesktopItem {
        name = "euro-office-desktopeditors";
        exec = "euro-office-desktopeditors %U";
        icon = "euro-office-desktopeditors";
        desktopName = appName;
        genericName = "Office Suite";
        comment = "Edit documents, spreadsheets and presentations";
        categories = ["Office"];
        mimeTypes = [
          "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
          "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
          "application/vnd.openxmlformats-officedocument.presentationml.presentation"
          "application/vnd.oasis.opendocument.text"
          "application/pdf"
        ];
      })
    ];

    meta = {
      description = "Euro-Office DesktopEditors GUI application (Linux, from-source, V8-free)";
      homepage = "https://github.com/Euro-Office/desktop-apps";
      license = lib.licenses.agpl3Plus;
      sourceProvenance = with lib.sourceTypes; [fromSource binaryNativeCode binaryBytecode];
      maintainers = with lib.maintainers; [overby-me];
      platforms = lib.platforms.linux;
      mainProgram = "euro-office-desktopeditors";
    };
  }
