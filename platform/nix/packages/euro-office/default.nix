# Euro-Office DesktopEditors, a sovereign AGPL fork of ONLYOFFICE.
#
# Built only from genuine Euro-Office sources and artifacts, never from
# ONLYOFFICE binaries. The CEF prebuilt is upstream Chromium's, not theirs.
# See ./PLAN.md for the derivation graph, phases and spike findings.
#
# On Linux the app repackages the deb Euro-Office's own CI builds
# (./app-linux-bin.nix); the from-source Qt build stays on passthru as
# `app-from-source` for when a patched or hermetic build is worth the hours
# of C++ it costs. The macOS GUI bundle is built from source; see ./app.nix.
{
  stdenv,
  callPackage,
}: let
  data = callPackage ./fonts.nix {};
  # The editors JS/WASM payload: the actual editing engine, and the only
  # binaryBytecode piece here.
  desktop-common = callPackage ./desktop-common.nix {};
  # The x2t converter engine and format libraries, native arm64 and V8-free.
  core = callPackage ./core.nix {};
  # Pinned to desktop-sdk's vendored wrapper ABI.
  cef = callPackage ./cef.nix {};
  # `ascdocumentscore`, the CEF/Qt integration the macOS app links.
  desktop-sdk = callPackage ./desktop-sdk.nix {inherit cef;};
  app-from-source = callPackage ./app-linux.nix {inherit cef core data desktop-common;};
  app =
    if stdenv.hostPlatform.isDarwin
    then callPackage ./app.nix {inherit cef desktop-sdk core data desktop-common;}
    else callPackage ./app-linux-bin.nix {};
in
  # The top-level package is the data bundle because it is the one that builds
  # everywhere, which keeps the flake's package set green on every system. The
  # editors payload and the platform-gated app are on passthru.
  data.overrideAttrs (old: {
    passthru =
      (old.passthru or {})
      // {
        inherit data desktop-common core cef desktop-sdk app app-from-source;
      };
  })
