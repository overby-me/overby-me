{
  lib,
  rustPlatform,
  fetchFromGitHub,
  pkg-config,
  makeWrapper,
  libxkbcommon,
  vulkan-loader,
  stdenv,
  wayland,
}:
rustPlatform.buildRustPackage {
  pname = "cosmic-osk";
  version = "0-unstable-2026-01-22";

  src = fetchFromGitHub {
    owner = "pop-os";
    repo = "cosmic-osk";
    rev = "eee4d0472c815bad010492c16f4358fca9a47e5f";
    hash = "sha256-B5XYflvjykLOn59zHgWWsJY0bU2cUo0XtJTu0QveTRQ=";
  };

  cargoHash = "sha256-WhAhrediZCNVl9evwNIBKFbCM14lNzfIm0tPJ71HGD0=";

  nativeBuildInputs = [
    pkg-config
    makeWrapper
  ];

  buildInputs =
    [
      libxkbcommon
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
      --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath ([libxkbcommon vulkan-loader] ++ lib.optionals stdenv.isLinux [wayland])}

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
    mainProgram = "cosmic-osk";
  };
}
