# Euro-Office DesktopEditors — Nix packaging.
#
# See ./PLAN.md for the full derivation graph, phase breakdown and the recorded
# spike findings. Euro-Office is a sovereign, AGPL fork of ONLYOFFICE; this
# package is built **only** from genuine Euro-Office sources and artifacts —
# never from ONLYOFFICE binaries.
#
# WHAT BUILDS (on aarch64-darwin, all from source unless noted)
# ------------------------------------------------------------
#   .#euro-office.data            font / dictionary / template data (phase 2)
#   .#euro-office.desktop-common  the official Euro-Office editors JS/WASM
#                                 payload — the actual editing engine (phase 5,
#                                 the only binaryBytecode piece)
#   .#euro-office.core            the x2t converter engine + format libraries
#                                 (phase 4, native arm64, V8-free)
#   .#euro-office.cef             the Chromium Embedded Framework, branch 109,
#                                 a pinned upstream prebuilt (binaryNativeCode,
#                                 genuine CEF — NOT an ONLYOFFICE binary)
#   .#euro-office.desktop-sdk     the `ascdocumentscore` CEF client library —
#                                 the native CEF/Qt integration the macOS app
#                                 links — built from source against nixpkgs Qt
#                                 5.15 + the CEF package + core (phase 6)
#
# WHAT REMAINS (the macOS GUI application bundle)
# ----------------------------------------------
# On macOS the GUI is a native Cocoa app (`desktop-apps/macos`) built as an
# Xcode project that links the `ascdocumentscore` framework built above and
# embeds CEF in NSViews. Producing a signed .app bundle needs the Xcode
# toolchain (xcodebuild + the vendored Sparkle/PureLayout/etc. frameworks),
# which does not fit the Nix sandbox cleanly; that final packaging step is the
# remaining work. The hard integration — Qt 5.15 + CEF 109 + the from-source
# core, all wired together — DOES build here as `.#euro-office.desktop-sdk`.
#
#   * linux: not yet wired here, but the from-source path is viable (those
#     targets have sources on Linux, and the qt_wrapper/QCefView Qt host is the
#     Linux/Windows path). See PLAN.md §10 option 1.
#
# The discoverable top-level attribute resolves to the data bundle so the flake
# package set stays green on all systems; the genuine pieces are on `passthru`,
# and `passthru.app` carries the platform-appropriate gate.
{
  stdenv,
  callPackage,
}: let
  data = callPackage ./fonts.nix {};
  desktop-common = callPackage ./desktop-common.nix {};
  # Phase 4: from-source core engine (macOS port in progress, see core.nix).
  core = callPackage ./core.nix {};
  # Chromium Embedded Framework, pinned to desktop-sdk's vendored wrapper ABI.
  cef = callPackage ./cef.nix {};
  # Phase 6: the Qt/CEF desktop integration libraries (macOS port).
  desktop-sdk = callPackage ./desktop-sdk.nix {inherit cef;};
  # Phase 7: the macOS GUI application bundle (built from source, no Xcode).
  app =
    if stdenv.hostPlatform.isDarwin
    then callPackage ./app.nix {inherit cef desktop-sdk core data desktop-common;}
    else appGate;

  # On Linux the native application is not wired up yet (the Qt/CEF host path).
  appGate = throw ''
    euro-office: the native desktop application is not wired up on this
    platform yet.

    On Linux the from-source path is viable (PLAN.md §10 option 1); until it
    lands, build the genuine pieces:

        nix build .#euro-office.data
        nix build .#euro-office.desktop-common
  '';
in
  # The discoverable top-level package is the data bundle (a real, buildable
  # derivation) so the flake's package set evaluates and builds everywhere. The
  # genuine editors payload and the platform-gated app live on passthru.
  data.overrideAttrs (old: {
    passthru =
      (old.passthru or {})
      // {
        inherit data desktop-common core cef desktop-sdk app;
      };
  })
