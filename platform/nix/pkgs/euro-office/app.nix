# Phase 7 (see ./PLAN.md): the Euro-Office macOS GUI application bundle.
#
# Builds `Euro-Office.app` from source WITHOUT Xcode/xcodebuild: the app is pure
# Objective-C++ (no Swift), so we compile the 72 sources with clang, compile the
# storyboards / asset catalog with the CommandLineTools `ibtool`/`actool`
# (provided by nixpkgs `xcbuild`), assemble the .app bundle by hand, embed the
# frameworks + CEF helper, and ad-hoc code-sign it.
#
# Runtime layout produced (the CEF-on-macOS bundle contract, derived from the
# upstream Xcode "copy frameworks" build script):
#
#   Euro-Office.app/Contents/
#     MacOS/Euro-Office                         main executable
#     Resources/                                storyboards, assets, localizations
#       editors/    web-apps + sdkjs            (from desktop-common)
#       converter/  x2t + DoctRenderer.config   (from core + desktop-common)
#       dictionaries/ fonts/                    (from data + desktop-common)
#     Frameworks/
#       Chromium Embedded Framework.framework   (from cef)
#       lib*.dylib                              (ascdocumentscore + core libs)
#       editors_helper.app/                     CEF subprocess helper
#
# Sparkle (the upstream auto-updater) is intentionally skipped: it is not
# referenced from any source file, only linked as a vendored framework. Skipping
# it keeps the bundle free of that prebuilt blob.
#
# Code signing is ad-hoc (`codesign -s -`), which is sufficient to launch on the
# local machine (Apple Silicon requires *a* signature, but not a Developer ID).
{
  lib,
  stdenv,
  fetchFromGitHub,
  xcbuild,
  hunspell,
  cef,
  desktop-sdk,
  core,
  data,
  desktop-common,
}: let
  sources = import ./sources.nix {inherit lib;};
  desktopAppsSrc = fetchFromGitHub sources.repos.desktop-apps;
  # The app #imports core + desktop-sdk headers. The desktop-sdk public headers
  # reference core via relative paths (../../../../core/...), so we recreate the
  # core <-> desktop-sdk sibling layout and include the sdk headers from there.
  coreSrc = fetchFromGitHub sources.repos.core;
  desktopSdkSrc = fetchFromGitHub sources.repos.desktop-sdk;

  appName = "Euro-Office";
  bundleId = "me.overby.euro-office";
in
  stdenv.mkDerivation {
    pname = "euro-office-app";
    inherit (sources) version;

    src = desktopAppsSrc;

    nativeBuildInputs = [xcbuild];

    buildInputs = [hunspell];

    # We reference /usr/bin/codesign (ad-hoc). The machine has sandbox=false.
    __noChroot = true;

    dontConfigure = true;

    # Carry the dependency store paths into the build environment.
    cefRoot = cef;
    sdkRoot = desktop-sdk;
    coreRoot = core;
    coreSrcDir = coreSrc;
    desktopSdkSrcDir = desktopSdkSrc;
    dataRoot = data;
    payloadRoot = desktop-common;

    buildPhase = ''
      runHook preBuild

      # Recreate the core <-> desktop-sdk sibling layout so that the desktop-sdk
      # public headers' relative "../../../../core/..." includes resolve.
      ws="$PWD/ws"
      mkdir -p "$ws"
      cp -r --no-preserve=mode,ownership "$coreSrcDir"       "$ws/core"
      cp -r --no-preserve=mode,ownership "$desktopSdkSrcDir" "$ws/desktop-sdk"
      sdkInclude="$ws/desktop-sdk/ChromiumBasedEditors/lib/include"

      macdir="$PWD/macos"
      cd "$macdir"

      # sandbox=false on this host, so the CommandLineTools SDK is reachable.
      sdk="$(/usr/bin/xcrun --show-sdk-path 2>/dev/null || true)"
      if [ -z "$sdk" ]; then
        sdk="$SDKROOT"
      fi
      echo "Using SDK: $sdk"

      # ---- Header search paths --------------------------------------------
      incflags=""
      for d in \
        ONLYOFFICE \
        ONLYOFFICE/Code/Controllers/About \
        ONLYOFFICE/Code/Controllers/CertificatePreview \
        ONLYOFFICE/Code/Controllers/Common \
        ONLYOFFICE/Code/Controllers/DocumentSignature \
        ONLYOFFICE/Code/Controllers/Download \
        ONLYOFFICE/Code/Controllers/EditorWindow \
        ONLYOFFICE/Code/Controllers/License \
        ONLYOFFICE/Code/Controllers/MainWindow \
        ONLYOFFICE/Code/Controllers/PresentationReporter \
        ONLYOFFICE/Code/Controllers/UserInfo \
        ONLYOFFICE/Code/Controls \
        ONLYOFFICE/Code/Controls/ASCTabs \
        ONLYOFFICE/Code/Enums \
        ONLYOFFICE/Code/Extensions \
        ONLYOFFICE/Code/TouchBar/Controllers \
        ONLYOFFICE/Code/TouchBar/Models \
        ONLYOFFICE/Code/TouchBar/Views \
        ONLYOFFICE/Code/Utils \
        ONLYOFFICE/Code/Views \
        Vendor/CocoaAnalytics \
        Vendor/KSNavigationController \
        Vendor/PFMoveApplication \
        Vendor/PureLayout \
        Vendor/SFBPopover \
        ; do
        incflags="$incflags -I$macdir/$d"
      done
      # ascdocumentscore + core public headers (CAscApplicationManager etc.).
      # Use the sdk headers from the source workspace so their relative core
      # includes resolve; add core source dirs the app references directly.
      incflags="$incflags -I$sdkInclude"
      incflags="$incflags -I$cefRoot/include"
      for d in \
        Common \
        DesktopEditor/xmlsec/src/include \
        DesktopEditor \
        ; do
        incflags="$incflags -I$ws/core/$d"
      done

      # ---- Compile every Obj-C++/Obj-C source -----------------------------
      objdir="$PWD/obj"
      mkdir -p "$objdir"

      mapfile -t srcs < <(find ONLYOFFICE Vendor \( -name '*.mm' -o -name '*.m' \) | grep -v '/Sparkle/')
      echo "Compiling ''${#srcs[@]} sources"

      # _MAC selects the macOS code paths in the shared core/desktop-sdk headers
      # (e.g. WindowHandleId); _ARM_ONLY + _PRODUCT_ONLYOFFICE match the upstream
      # Xcode arm app target's preprocessor defines.
      commonflags="-arch arm64 -isysroot $sdk -mmacosx-version-min=14.0 -Wno-everything -DNDEBUG -O2 -D_MAC -D_ARM_ONLY -D_PRODUCT_ONLYOFFICE"

      i=0
      for s in "''${srcs[@]}"; do
        o="$objdir/obj_$i.o"
        i=$((i+1))
        case "$s" in
          *.mm) langflags="-x objective-c++ -std=gnu++17" ;;
          *.m)  langflags="-x objective-c -std=gnu11" ;;
        esac
        # PFMoveApplication.m is the one non-ARC source (per the Xcode project's
        # per-file -fno-objc-arc flag); everything else is ARC.
        case "$s" in
          */PFMoveApplication.m) arcflag="-fno-objc-arc" ;;
          *)                     arcflag="-fobjc-arc" ;;
        esac
        clang++ $commonflags $arcflag $langflags $incflags -c "$s" -o "$o" || { echo "FAILED compiling $s"; exit 1; }
      done

      # ---- Link the main executable ---------------------------------------
      mkdir -p out
      clang++ -arch arm64 -isysroot "$sdk" -mmacosx-version-min=14.0 \
        "$objdir"/*.o \
        -L"$sdkRoot/lib" -lascdocumentscore \
        "$sdkRoot"/lib/libooxmlsignature*.dylib \
        -Wl,-rpath,@executable_path/../Frameworks \
        -F"$cefRoot/Release" \
        -framework Cocoa -framework WebKit -framework Security \
        -framework QuickLookUI -framework Carbon -framework AppKit \
        -framework Foundation -framework CoreServices -framework QuartzCore \
        -o "out/${appName}"

      runHook postBuild
    '';

    installPhase = ''
      runHook preInstall

      # buildPhase left the cwd inside macos/; anchor to it explicitly.
      macdir="$NIX_BUILD_TOP/source/macos"
      [ -d "$macdir" ] || macdir="$PWD"
      cd "$macdir"

      app="$out/Applications/${appName}.app"
      contents="$app/Contents"
      mkdir -p "$contents/MacOS" "$contents/Resources" "$contents/Frameworks"

      cp "out/${appName}" "$contents/MacOS/${appName}"

      # ---- Info.plist (substitute Xcode build vars) -----------------------
      plist="ONLYOFFICE/Resources/ONLYOFFICE-arm/Info.plist"
      sed \
        -e 's|$(EXECUTABLE_NAME)|${appName}|g' \
        -e 's|$(PRODUCT_NAME)|${appName}|g' \
        -e 's|$(PRODUCT_BUNDLE_IDENTIFIER)|${bundleId}|g' \
        -e 's|$(MACOSX_DEPLOYMENT_TARGET)|11.0|g' \
        "$plist" > "$contents/Info.plist"

      # ---- Compile storyboards + localizations ----------------------------
      # ibtool/actool need the real Apple toolchain; xcbuild does not ship a
      # working ibtool, so prefer /usr/bin (reachable since sandbox=false).
      IBTOOL=/usr/bin/ibtool
      [ -x "$IBTOOL" ] || IBTOOL=ibtool
      ACTOOL=/usr/bin/actool
      [ -x "$ACTOOL" ] || ACTOOL=actool

      mkdir -p "$contents/Resources/Base.lproj"
      for sb in ONLYOFFICE/Base.lproj/*.storyboard; do
        [ -e "$sb" ] || continue
        name="$(basename "$sb" .storyboard)"
        "$IBTOOL" --errors --warnings --output-format human-readable-text \
          --compile "$contents/Resources/Base.lproj/$name.storyboardc" "$sb" \
          || echo "warning: ibtool failed for $sb"
      done

      # ---- Asset catalog (app icon) ---------------------------------------
      if [ -d ONLYOFFICE/Images.xcassets ]; then
        "$ACTOOL" --output-format human-readable-text \
          --compile "$contents/Resources" \
          --platform macosx --minimum-deployment-target 14.0 \
          --app-icon AppIcon \
          --output-partial-info-plist "$TMPDIR/assetcatalog.plist" \
          ONLYOFFICE/Images.xcassets || echo "warning: actool failed"
      fi

      # ---- Editors payload + converter + data -----------------------------
      # desktop-common ships the assembled editors/converter/fonts/dictionaries.
      mkdir -p "$contents/Resources/editors" "$contents/Resources/converter" \
               "$contents/Resources/dictionaries" "$contents/Resources/login/fonts"
      cp -r "$payloadRoot/editors/." "$contents/Resources/editors/" 2>/dev/null || true
      cp -r "$payloadRoot/converter/." "$contents/Resources/converter/" 2>/dev/null || true
      cp -r "$payloadRoot/dictionaries/." "$contents/Resources/dictionaries/" 2>/dev/null || true
      cp -r "$payloadRoot/fonts/." "$contents/Resources/login/fonts/" 2>/dev/null || true
      [ -f "$payloadRoot/index.html" ] && cp "$payloadRoot/index.html" "$contents/Resources/login/" 2>/dev/null || true

      # The native x2t + format libraries (the from-source converter engine).
      cp "$coreRoot"/lib/*.dylib "$contents/Resources/converter/" 2>/dev/null || true
      [ -f "$coreRoot/bin/x2t" ] && cp "$coreRoot/bin/x2t" "$contents/Resources/converter/" 2>/dev/null || true

      # ---- Frameworks: dylibs + CEF + helper ------------------------------
      cp "$sdkRoot"/lib/*.dylib "$contents/Frameworks/" 2>/dev/null || true
      cp -r "$cefRoot/Release/Chromium Embedded Framework.framework" "$contents/Frameworks/"

      # editors_helper.app (CEF subprocess). The single editors_helper binary
      # serves all CEF process types (it detects --type= at runtime).
      helperapp="$contents/Frameworks/editors_helper.app/Contents/MacOS"
      mkdir -p "$helperapp"
      cp "$sdkRoot/bin/editors_helper" "$helperapp/editors_helper"
      cat > "$contents/Frameworks/editors_helper.app/Contents/Info.plist" <<PLIST
      <?xml version="1.0" encoding="UTF-8"?>
      <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
      <plist version="1.0"><dict>
        <key>CFBundleExecutable</key><string>editors_helper</string>
        <key>CFBundleIdentifier</key><string>${bundleId}.helper</string>
        <key>CFBundleName</key><string>editors_helper</string>
        <key>CFBundlePackageType</key><string>APPL</string>
        <key>LSUIElement</key><true/>
      </dict></plist>
      PLIST

      # The helper resolves the dylibs (and finds CEF) relative to its location
      # inside Frameworks/editors_helper.app/Contents/MacOS.
      install_name_tool -add_rpath "@executable_path/../../.." "$helperapp/editors_helper" 2>/dev/null || true

      # ---- Ad-hoc code-sign (deepest first) -------------------------------
      CODESIGN=/usr/bin/codesign
      if [ -x "$CODESIGN" ]; then
        "$CODESIGN" --force --sign - --timestamp=none \
          "$contents/Frameworks/Chromium Embedded Framework.framework" 2>/dev/null || true
        for f in "$contents/Frameworks"/*.dylib; do
          "$CODESIGN" --force --sign - "$f" 2>/dev/null || true
        done
        "$CODESIGN" --force --sign - "$helperapp/editors_helper" 2>/dev/null || true
        "$CODESIGN" --force --deep --sign - "$contents/Frameworks/editors_helper.app" 2>/dev/null || true
        "$CODESIGN" --force --sign - "$contents/MacOS/${appName}" 2>/dev/null || true
        "$CODESIGN" --force --deep --sign - "$app" 2>/dev/null || true
      fi

      runHook postInstall
    '';

    dontFixup = true;

    meta = {
      description = "Euro-Office DesktopEditors macOS application (from-source, no Xcode)";
      homepage = "https://github.com/Euro-Office/desktop-apps";
      license = lib.licenses.agpl3Plus;
      sourceProvenance = with lib.sourceTypes; [fromSource binaryNativeCode];
      maintainers = with lib.maintainers; [overby-me];
      platforms = ["aarch64-darwin"];
      mainProgram = appName;
    };
  }
