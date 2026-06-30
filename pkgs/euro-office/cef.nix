# Chromium Embedded Framework (CEF) — standalone prebuilt package.
#
# Why this exists
# ---------------
# Euro-Office's `desktop-sdk` vendors its own copy of the CEF C++ wrapper layer
# (`ChromiumBasedEditors/lib/src/cef/mac/libcef_dll`), which is ABI-locked to a
# specific CEF binary branch — `109.1.18+gf1c41e4+chromium-109.0.5414.120` (see
# that tree's `include/cef_version.h`). The wrapper compiles against a matching
# prebuilt `Chromium Embedded Framework.framework`.
#
# nixpkgs' `cef-binary` cannot be used:
#   * it tracks a much newer CEF branch (wrong ABI for EO's vendored wrapper);
#   * it is Linux-only and throws "Unsupported system aarch64-darwin".
#
# So we fetch the official upstream prebuilt distribution from the Spotify CDN
# (the canonical CEF binary host) and expose it as a framework + headers. This
# is genuine upstream CEF — NOT an ONLYOFFICE binary. It is the only
# `binaryNativeCode` artifact in the euro-office build (nixpkgs ships CEF
# prebuilt too).
#
# What this package provides
# --------------------------
#   $out/Release/Chromium Embedded Framework.framework   the runtime framework
#   $out/Release/cef_sandbox.a                            the sandbox static lib
#   $out/include/                                         the C/C++ API headers
#   $out/cmake/, $out/cef_paths*.gypi                     upstream build glue
#
# (On macOS the locales / .pak resources live INSIDE the .framework bundle, so
# there is no separate top-level Resources/ dir as on Linux/Windows.)
#
# The desktop-sdk build links against the framework with
#   -F"$out/Release" -framework "Chromium Embedded Framework"
# and pairs it with desktop-sdk's own vendored `libcef_dll` wrapper sources.
{
  lib,
  stdenvNoCC,
  fetchurl,
  cpio,
}: let
  sources = import ./sources.nix {inherit lib;};
  inherit (sources) cef;

  # The Spotify CDN URL-encodes the '+' characters in the version as %2B.
  encodedVersion = lib.replaceStrings ["+"] ["%2B"] cef.version;
  platformTag = "macosarm64";
in
  stdenvNoCC.mkDerivation {
    pname = "cef-binary";
    inherit (cef) version;

    src = fetchurl {
      url = "https://cef-builds.spotifycdn.com/cef_binary_${encodedVersion}_${platformTag}.tar.bz2";
      inherit (cef.macosarm64) hash;
    };

    # The framework inside contains symlinks and code-signed Mach-O; cpio
    # preserves them faithfully when we relocate the tree.
    nativeBuildInputs = [cpio];

    # No build/configure — this is a prebuilt distribution.
    dontConfigure = true;
    dontBuild = true;
    dontPatchELF = true;
    dontStrip = true;
    # The framework is binaryNativeCode; do not let fixup mangle its install
    # names or signature.
    dontFixup = true;

    # unpackPhase already cd's into the single top-level cef_binary_* dir.
    installPhase = ''
      runHook preInstall

      mkdir -p "$out"

      # Keep the framework (+ cef_sandbox.a), headers and the upstream CMake glue
      # the desktop-sdk wrapper references. We deliberately drop the sample apps
      # and the prebuilt libcef_dll_wrapper: desktop-sdk supplies its OWN
      # ABI-matched wrapper sources.
      cp -R Release         "$out/Release"
      cp -R include         "$out/include"
      cp -R cmake           "$out/cmake"
      cp    cef_paths*.gypi  "$out/"
      cp    LICENSE.txt      "$out/"

      runHook postInstall
    '';

    meta = {
      description = "Chromium Embedded Framework prebuilt (branch 109.1.18, macOS arm64) for Euro-Office";
      homepage = "https://bitbucket.org/chromiumembedded/cef";
      # CEF is BSD-3; the bundled Chromium is a mix dominated by BSD.
      license = with lib.licenses; [bsd3];
      sourceProvenance = with lib.sourceTypes; [binaryNativeCode];
      maintainers = with lib.maintainers; [overby-me];
      platforms = ["aarch64-darwin"];
    };
  }
