{
  lib,
  rustPlatform,
  fetchFromGitLab,
  python3Packages,
  libcosmicAppHook,
  pkg-config,
  libxkbcommon,
  at-spi2-core,
  gobject-introspection,
  ffmpeg,
  pipewire,
  pulseaudio,
  wireplumber,
  xdg-utils,
}: let
  version = "0.1.6-unstable-2026-10-30";

  src = fetchFromGitLab {
    owner = "sneptech";
    repo = "vibesquawk";
    rev = "bc49604bfad0dbf9348001afee6a82d012ec1680";
    hash = "sha256-E/5C5rC98OwDHpbf0DSyKP5MU+xnjPwDISs4ixJGkLE=";
  };

  meta = {
    homepage = "https://gitlab.com/sneptech/vibesquawk";
    license = lib.licenses.gpl3Plus;
    maintainers = with lib.maintainers; [overby-me];
    platforms = lib.platforms.linux;
  };

  backend = python3Packages.buildPythonPackage {
    pname = "vibesquawk";
    inherit version src;
    pyproject = true;
    sourceRoot = "${src.name}/linux";

    build-system = [python3Packages.setuptools];

    dependencies = with python3Packages; [
      pyside6
      dbus-next
      huggingface-hub
      # The installer's default `local` extra, for on-device Whisper.
      faster-whisper
      # Atspi, for matching inserted text to the focused field.
      pygobject3
    ];

    # The service tests give their subprocesses a PYTHONPATH of src/ alone,
    # which drops the dependencies a Nix build provides only through it.
    postPatch = ''
      substituteInPlace tests/test_cosmic_service.py \
        --replace-fail 'PYTHONPATH=str(Path(__file__).parents[1] / "src")' \
          'PYTHONPATH=os.pathsep.join([str(Path(__file__).parents[1] / "src"), os.environ["PYTHONPATH"]])'
    '';

    nativeCheckInputs = [python3Packages.pytestCheckHook];

    # The suites cosmic/README.md runs for the service.
    enabledTestPaths = [
      "tests/test_cosmic_service.py"
      "tests/test_cosmic_platform.py"
      "tests/test_cosmic_shortcuts.py"
    ];

    pythonImportsCheck = ["vibesquawk.cosmic.service"];

    meta = meta // {description = "Speech service behind the VibeSquawk COSMIC frontend";};
  };

  python = python3Packages.python.withPackages (_: [backend]);

  desktop = rustPlatform.buildRustPackage {
    pname = "vibesquawk-cosmic-desktop";
    inherit version src;
    cargoRoot = "cosmic-desktop";
    buildAndTestSubdir = "cosmic-desktop";
    cargoHash = "sha256-6OFhBH+P+1AxPg4B4kLLhDTwZXvYIz+DuL/RGB3VTg4=";

    nativeBuildInputs = [pkg-config];
    buildInputs = [libxkbcommon];

    meta =
      meta
      // {
        description = "COSMIC shortcut, clipboard and input helper for VibeSquawk";
        mainProgram = "vibesquawk-cosmic-desktop";
      };
  };

  # The Qt PySide6 is built against, so its plugins match.
  inherit (python3Packages) qt6;
in
  rustPlatform.buildRustPackage {
    pname = "vibesquawk-cosmic";
    inherit version src;
    cargoRoot = "cosmic";
    buildAndTestSubdir = "cosmic";
    cargoHash = "sha256-NwnnI3EKhWhqewsbJo+ygGLxMSUUz7EpEAVc9nmSHDc=";

    nativeBuildInputs = [libcosmicAppHook];

    # Otherwise the hook also wraps the Python launchers in bin/.
    dontWrapLibcosmicApp = true;

    postInstall = ''
      install -Dm644 -t $out/share/applications cosmic/data/*.desktop
      install -Dm644 linux/share/icons/hicolor/scalable/apps/io.github.sneptech.vibesquawk.svg \
        $out/share/icons/hicolor/scalable/apps/io.github.sneptech.vibesquawk.Cosmic.svg
      install -Dm644 linux/share/icons/hicolor/symbolic/apps/io.github.sneptech.vibesquawk-symbolic.svg \
        $out/share/icons/hicolor/symbolic/apps/io.github.sneptech.vibesquawk.Cosmic-symbolic.svg
    '';

    # The frontend starts the service as `$VIBESQUAWK_PYTHON -m ...`, so the
    # service's environment goes on that interpreter.
    postFixup = ''
      makeWrapper ${python.interpreter} $out/libexec/vibesquawk-cosmic/python \
        --prefix QT_PLUGIN_PATH : ${lib.makeSearchPath qt6.qtbase.qtPluginPrefix [qt6.qtmultimedia qt6.qtspeech]} \
        --prefix GI_TYPELIB_PATH : ${lib.makeSearchPath "lib/girepository-1.0" [at-spi2-core gobject-introspection]} \
        --prefix PATH : ${lib.makeBinPath [ffmpeg pipewire pulseaudio wireplumber xdg-utils]} \
        --set-default VIBESQUAWK_COSMIC_DESKTOP ${lib.getExe desktop}

      # Upstream's launchers also set VIBESQUAWK_COSMIC_LAUNCHER and _COMMAND.
      # Not here: autostart and COSMIC shortcuts save them, so store paths go stale.
      for program in vibesquawk-cosmic vibesquawk-cosmic-applet; do
        wrapLibcosmicApp $out/bin/$program \
          --prefix PATH : ${lib.makeBinPath [xdg-utils]} \
          --set-default VIBESQUAWK_PYTHON $out/libexec/vibesquawk-cosmic/python
      done

      for module in service command; do
        makeWrapper $out/libexec/vibesquawk-cosmic/python $out/bin/vibesquawk-cosmic-$module \
          --add-flags "-m vibesquawk.cosmic.$module"
      done
    '';

    passthru = {inherit backend desktop python;};

    meta =
      meta
      // {
        description = "Native COSMIC frontend for VibeSquawk, system-wide dictation for Linux";
        mainProgram = "vibesquawk-cosmic";
      };
  }
