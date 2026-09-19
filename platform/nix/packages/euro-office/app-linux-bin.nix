# The Linux DesktopEditors GUI, repackaged from the deb Euro-Office's own CI
# builds. Same provenance rule as desktop-common.nix: a genuine Euro-Office
# artifact, never an ONLYOFFICE binary. The recipe mirrors nixpkgs'
# onlyoffice-desktopeditors: dpkg unpack, autoPatchelf, then an FHS env
# because the app shells out to /usr/bin/curl for plugin downloads and scans
# /usr/share/fonts for system fonts.
#
# Upstream cuts no releases yet; the deb exists only as an auth-gated CI
# artifact that expires after ~90 days, hence requireFile instead of fetchurl.
# When https://github.com/Euro-Office/DesktopEditors/releases starts carrying
# debs (their build.yml already plans the upload), swap src for a fetchurl on
# the release asset and drop the seeding instructions.
{
  lib,
  stdenv,
  requireFile,
  buildFHSEnv,
  alsa-lib,
  at-spi2-atk,
  atk,
  autoPatchelfHook,
  cairo,
  cups,
  curl,
  dbus,
  dconf,
  dpkg,
  fontconfig,
  gcc-unwrapped,
  gdk-pixbuf,
  glib,
  glibc,
  gsettings-desktop-schemas,
  gst_all_1,
  gtk3,
  libnotify,
  libpulseaudio,
  libudev0-shim,
  libdrm,
  makeWrapper,
  libgbm,
  noto-fonts-cjk-sans,
  nspr,
  nss,
  pipewire,
  pulseaudio,
  wrapGAppsHook3,
  xkeyboard_config,
  libxcb-cursor,
  libxcb-image,
  libxcb-keysyms,
  libxcb-render-util,
  libxcb-util,
  libxcb-wm,
  libxkbcommon,
  libxtst,
  libxscrnsaver,
  libxrender,
  libxrandr,
  libxi,
  libxfixes,
  libxext,
  libxdamage,
  libxcursor,
  libxcomposite,
  libx11,
  libxcb,
}: let
  version = "9.3.1-dev.1";
  # DesktopEditors CI run 35195021163 (feature/macos-build, 2026-09-17).
  runId = "35195021163";
  debs = {
    x86_64-linux = {
      arch = "amd64";
      hash = "sha256-OfNEdCmx6VAScQBVVcPLFeIH9c1JkSV1OUTKvosX2y8=";
    };
    aarch64-linux = {
      arch = "arm64";
      hash = "sha256-5BwK4dLf5tgAIFsJckV4usQeKAZeQr5GF3sLpicJeLU=";
    };
  };
  deb = debs.${stdenv.hostPlatform.system};
  debName = "euro-office-desktopeditors_${version}_${deb.arch}.deb";

  # curl and glibc are dlopened/execed at runtime, so autoPatchelf never sees
  # them; the rest cover the bundled CEF's lazy loads.
  runtimeLibs = lib.makeLibraryPath [
    curl
    glibc
    gcc-unwrapped.lib
    libudev0-shim
    pulseaudio
  ];

  derivation = stdenv.mkDerivation {
    pname = "euro-office-desktopeditors-unwrapped";
    inherit version;

    src = requireFile {
      name = debName;
      inherit (deb) hash;
      message = ''
        Euro-Office publishes no desktop release binaries yet; this deb is the
        export of CI run ${runId}. Seed it into the store once per machine:

          gh run download ${runId} -R Euro-Office/DesktopEditors \
            -n linux-packages-amd64 -D /tmp/eo-deb
          nix-store --add-fixed sha256 /tmp/eo-deb/${debName}
      '';
    };

    nativeBuildInputs = [
      autoPatchelfHook
      dpkg
      makeWrapper
      wrapGAppsHook3
    ];

    # Dangling references of bundled Qt 6 plugins the app never loads: the
    # Quick3D spatial-audio module and the SQL client drivers. Qt skips a
    # plugin whose libraries fail to resolve.
    autoPatchelfIgnoreMissingDeps = [
      "libQt6Quick3D.so.6"
      "libQt6Quick3DRuntimeRender.so.6"
      "libQt6Quick3DUtils.so.6"
      "libQt6ShaderTools.so.6"
      "libclntsh.so.23.1"
      "libfbclient.so.2"
      "libmimerapi.so"
      "libmysqlclient.so.21"
      "libodbc.so.2"
      "libpq.so.5"
    ];

    # The deb bundles its own Qt 6, CEF, ICU and ffmpeg; these only close the
    # remaining NEEDED entries of the bundled objects.
    buildInputs = [
      alsa-lib
      at-spi2-atk
      atk
      cairo
      cups
      dbus
      dconf
      fontconfig
      gdk-pixbuf
      glib
      gsettings-desktop-schemas
      gst_all_1.gst-plugins-base
      # The arm64 deb bundles Qt's gstreamer media backend, which wants the
      # photography and play helper libraries shipped in -bad.
      gst_all_1.gst-plugins-bad
      gst_all_1.gstreamer
      gtk3
      libnotify
      libpulseaudio
      libdrm
      nspr
      nss
      libgbm
      libxkbcommon
      libx11
      libxcb
      libxcomposite
      libxcursor
      libxdamage
      libxext
      libxfixes
      libxi
      libxrandr
      libxrender
      libxscrnsaver
      libxtst
      libxcb-cursor
      libxcb-image
      libxcb-keysyms
      libxcb-render-util
      libxcb-util
      libxcb-wm
    ];

    installPhase = ''
      runHook preInstall

      mkdir -p $out/{bin,share}

      mv usr/bin/* $out/bin
      mv usr/share/* $out/share/
      mv opt/euro-office/desktopeditors $out/share/desktopeditors

      # The launcher hardcodes the deb's install prefix; `desktopeditors` is a
      # symlink onto it, so patching both would hit the same file twice.
      substituteInPlace $out/bin/euro-office-desktopeditors \
        --replace-fail "/opt/euro-office/" "$out/share/"

      ln -s $out/share/desktopeditors/DesktopEditors $out/bin/DesktopEditors

      runHook postInstall
    '';

    preFixup = ''
      # patchelf blanks a replaced rpath string with X bytes, and the linker
      # tail-merged the vendored rpath (it ends in "$ORIGIN/system") with the
      # dynamic symbol name `system`, so letting autoPatchelf replace it in
      # place renames the symbol to XXXXXX and the app dies on startup.
      # Dropping the rpaths first frees no strings, so nothing is blanked;
      # the vendored ones are junk anyway (the launcher's LD_LIBRARY_PATH is
      # what resolves the bundled libraries).
      while IFS= read -r -d "" f; do
        head -c4 "$f" 2>/dev/null | grep -q $'\x7fELF' || continue
        patchelf --remove-rpath "$f" 2>/dev/null || true
      done < <(find $out/share/desktopeditors -type f -print0)

      gappsWrapperArgs+=(
        --prefix LD_LIBRARY_PATH : "${runtimeLibs}" \
        --set QT_XKB_CONFIG_ROOT "${xkeyboard_config}/share/X11/xkb" \
        --set QTCOMPOSE "${libx11.out}/share/X11/locale" \
        --set QT_QPA_PLATFORM "xcb"
      )
    '';
  };
in
  buildFHSEnv {
    pname = "euro-office-desktopeditors";
    inherit version;

    targetPkgs = _: [
      curl
      derivation
      noto-fonts-cjk-sans
      # Qt multimedia dlopens libpipewire-0.3 for presentation audio.
      pipewire
    ];

    runScript = "/bin/euro-office-desktopeditors";

    extraInstallCommands = ''
      mkdir -p $out/share
      ln -s ${derivation}/share/icons $out/share
      cp -r ${derivation}/share/applications $out/share
      substituteInPlace $out/share/applications/euro-office-desktopeditors.desktop \
        --replace-fail "/usr/bin/euro-office-desktopeditors" "$out/bin/euro-office-desktopeditors"
    '';

    passthru.unwrapped = derivation;

    meta = {
      description = "Euro-Office DesktopEditors (upstream CI deb, repackaged)";
      homepage = "https://github.com/Euro-Office/desktop-apps";
      downloadPage = "https://github.com/Euro-Office/DesktopEditors/releases";
      license = lib.licenses.agpl3Plus;
      sourceProvenance = with lib.sourceTypes; [binaryNativeCode binaryBytecode];
      maintainers = with lib.maintainers; [overby-me];
      platforms = lib.attrNames debs;
      mainProgram = "euro-office-desktopeditors";
    };
  }
