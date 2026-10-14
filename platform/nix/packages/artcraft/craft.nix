# One ArtCraft app (https://getartcraft.com/apps), repackaged from the archives
# its GitHub release ships: the Linux tarball, or on macOS the DMG plus the CLI
# zip, because the DMG holds only the GUI. Each `<app>.nix` beside this
# directory calls it with a name; update.nu keeps sources.json current.
{
  pname,
  description,
}: {
  lib,
  stdenv,
  fetchurl,
  addDriverRunpath,
  autoPatchelfHook,
  makeWrapper,
  _7zz,
  unzip,
  alsa-lib,
  dbus,
  libGL,
  libva,
  libx11,
  libxcb,
  libxcursor,
  libxi,
  libxinerama,
  libxkbcommon,
  libxrender,
  libxt,
  vulkan-loader,
  wayland,
}: let
  source = (lib.importJSON ./sources.json).${pname};
  inherit (source) version;

  fetch = name:
    fetchurl {
      url = "https://github.com/storytold/${pname}/releases/download/v${version}/${name}";
      hash = source.hashes.${name};
    };
in
  stdenv.mkDerivation {
    inherit pname version;

    srcs =
      if stdenv.hostPlatform.isDarwin
      then [
        (fetch "${pname}-${version}-macos-universal.dmg")
        (fetch "${pname}-cli-${version}-macos-universal.zip")
      ]
      else [(fetch "${pname}-${version}-linux-${stdenv.hostPlatform.uname.processor}.tar.gz")];

    sourceRoot = lib.optionalString stdenv.hostPlatform.isDarwin ".";

    # 7-Zip rather than undmg, which fails reading most of these DMGs.
    nativeBuildInputs =
      if stdenv.hostPlatform.isDarwin
      then [makeWrapper _7zz unzip]
      else [autoPatchelfHook];

    buildInputs = lib.optionals stdenv.hostPlatform.isLinux [
      alsa-lib
      stdenv.cc.cc.lib
    ];

    # dlopened, so in no NEEDED entry for autoPatchelf to find; the union over
    # all the apps rather than a list each. getLib because dbus and libva
    # default to outputs without the library.
    runtimeDependencies = lib.optionals stdenv.hostPlatform.isLinux (map lib.getLib [
      dbus
      libGL
      libva
      libx11
      libxcb
      libxcursor
      libxi
      libxinerama
      libxkbcommon
      libxrender
      libxt
      vulkan-loader
      wayland
    ]);

    # FilmCraft's NVDEC and NVENC come from the NVIDIA driver, not a package.
    appendRunpaths = lib.optionals stdenv.hostPlatform.isLinux [
      "${addDriverRunpath.driverLink}/lib"
    ];

    installPhase =
      if stdenv.hostPlatform.isDarwin
      then ''
        runHook preInstall
        mkdir -p $out/Applications $out/bin
        cp -R */*.app $out/Applications/
        cp ${pname}-cli-*/${pname}-cli $out/bin/
        # A wrapper rather than a symlink: the app finds its bundle through
        # the path it was executed from.
        makeWrapper $out/Applications/*.app/Contents/MacOS/* $out/bin/${pname}
        runHook postInstall
      ''
      else ''
        runHook preInstall
        mkdir -p $out
        cp -r bin share $out/
        runHook postInstall
      '';

    # Stripping would break the macOS code signatures, and upstream already
    # stripped the Linux binaries.
    dontStrip = true;

    meta = {
      inherit description;
      homepage = "https://getartcraft.com/apps/${pname}";
      changelog = "https://github.com/storytold/${pname}/releases/tag/v${version}";
      license = with lib.licenses; [mit asl20];
      sourceProvenance = with lib.sourceTypes; [binaryNativeCode];
      maintainers = with lib.maintainers; [overby-me];
      mainProgram = pname;
      platforms = ["x86_64-linux" "aarch64-linux" "aarch64-darwin"];
    };
  }
