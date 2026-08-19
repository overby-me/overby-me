# Chromium Embedded Framework, prebuilt.
#
# desktop-sdk vendors its own copy of the CEF C++ wrapper layer, ABI-locked to
# branch 109.1.18+gf1c41e4+chromium-109.0.5414.120, so the framework it compiles
# against has to be that exact branch. Both platforms below therefore pin it, and
# desktop-sdk always supplies the wrapper sources itself.
#
# On macOS this fetches the official upstream distribution from the Spotify CDN,
# the canonical CEF binary host, since nixpkgs' `cef-binary` tracks a far newer
# branch and throws on aarch64-darwin. Genuine upstream CEF, not an ONLYOFFICE
# binary, and the one `binaryNativeCode` artifact in the euro-office build. It
# provides:
#
#   $out/Release/Chromium Embedded Framework.framework   the runtime framework
#   $out/Release/cef_sandbox.a                            the sandbox static lib
#   $out/include/                                         the C/C++ API headers
#   $out/cmake/, $out/cef_paths*.gypi                     upstream build glue
#
# The locales and .pak resources live inside the bundle, so unlike Linux and
# Windows there is no top-level Resources/ dir.
#
# On Linux there is no framework at all: CEF ships `Release/libcef.so` and a
# separate `Resources/` tree. nixpkgs packages that from the same CDN, including
# the load-bearing `patchelf --set-rpath` that makes it loadable from the store,
# so overriding its version to branch 109 reuses all of that ELF fixup.
{
  lib,
  stdenv,
  stdenvNoCC,
  fetchurl,
  cpio,
  cef-binary,
}: let
  sources = import ./sources.nix {inherit lib;};
  inherit (sources) cef;

  # The Spotify CDN URL-encodes the '+' characters in the version as %2B.
  encodedVersion = lib.replaceStrings ["+"] ["%2B"] cef.version;
  platformTag = "macosarm64";

  # Linux: reuse nixpkgs' cef-binary derivation, pinned to EO's branch 109.
  linux = cef-binary.override {
    version = cef.cefVersion;
    inherit (cef) gitRevision chromiumVersion;
    srcHashes = {
      x86_64-linux = cef.linux64.hash;
      aarch64-linux = cef.linuxarm64.hash;
    };
  };

  darwin = stdenvNoCC.mkDerivation {
    pname = "cef-binary";
    inherit (cef) version;

    src = fetchurl {
      url = "https://cef-builds.spotifycdn.com/cef_binary_${encodedVersion}_${platformTag}.tar.bz2";
      inherit (cef.macosarm64) hash;
    };

    nativeBuildInputs = [cpio];

    dontConfigure = true;
    dontBuild = true;
    dontPatchELF = true;
    dontStrip = true;
    # The framework is binaryNativeCode; fixup would mangle its install names
    # and its signature.
    dontFixup = true;

    # unpackPhase already cd's into the single top-level cef_binary_* dir.
    installPhase = ''
      runHook preInstall

      mkdir -p "$out"

      # The framework, cef_sandbox.a, headers and the upstream CMake glue the
      # desktop-sdk wrapper references. The sample apps and the prebuilt
      # libcef_dll_wrapper are dropped: desktop-sdk brings its own ABI-matched
      # wrapper sources.
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
  };
in
  if stdenv.hostPlatform.isDarwin
  then darwin
  else linux
