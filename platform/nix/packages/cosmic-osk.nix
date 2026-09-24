{
  lib,
  rustPlatform,
  fetchFromGitHub,
  pkg-config,
  makeWrapper,
  libxkbcommon,
  udev,
  vulkan-loader,
  stdenv,
  wayland,
}:
rustPlatform.buildRustPackage {
  pname = "cosmic-osk";
  version = "0-unstable-2026-09-17";

  src = fetchFromGitHub {
    owner = "pop-os";
    repo = "cosmic-osk";
    rev = "d7b66a24890d03e38dd8962d956bc802d4a41067";
    hash = "sha256-HF/JrGZeKF4z0gRvJfcPEVaNKv8Rftc0vyOrWZ2A8Pc=";
  };

  cargoHash = "sha256-r5XlNx1GIy4gEiHX9QVYLEufRnuwxe9X4OBbz3tinIo=";

  nativeBuildInputs = [
    pkg-config
    makeWrapper
  ];

  buildInputs =
    [
      libxkbcommon
      # libudev-sys arrived with the input rework; its build script wants
      # libudev.pc, not just the shared library.
      udev
      vulkan-loader
    ]
    ++ lib.optionals stdenv.isLinux [
      wayland
    ];

  # Upstream example `key.rs` has stale imports (KeyCode, wayland_state)
  # that fail to compile against the current library API.
  doCheck = false;

  # Upstream doesn't ship a .desktop file; add one so it can be
  # launched from application menus (e.g. on phones without a
  # hardware keyboard).
  postInstall = ''
    wrapProgram $out/bin/cosmic-osk \
      --prefix LD_LIBRARY_PATH : ${
      lib.makeLibraryPath (
        [
          libxkbcommon
          vulkan-loader
        ]
        ++ lib.optionals stdenv.isLinux [wayland]
      )
    }

    install -Dm644 /dev/stdin $out/share/applications/com.system76.CosmicOSK.desktop <<EOF
    [Desktop Entry]
    Name=COSMIC On-Screen Keyboard
    Comment=On-screen keyboard for COSMIC
    Exec=cosmic-osk
    Icon=input-keyboard
    Type=Application
    Categories=Utility;Accessibility;
    X-COSMIC-AppId=com.system76.CosmicOSK
    EOF
  '';

  meta = {
    description = "COSMIC On-Screen Keyboard";
    homepage = "https://github.com/pop-os/cosmic-osk";
    license = lib.licenses.gpl3Only;
    maintainers = with lib.maintainers; [overby-me];
    platforms = lib.platforms.linux;
    mainProgram = "cosmic-osk";
  };
}
