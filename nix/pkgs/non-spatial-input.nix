{
  lib,
  rustPlatform,
  fetchFromGitHub,
  pkg-config,
  makeWrapper,
  libxkbcommon,
  libinput,
  libGL,
  wayland,
  libx11,
  libxcursor,
  libxrandr,
  libxi,
}:
rustPlatform.buildRustPackage {
  pname = "non-spatial-input";
  version = "0.1.0-unstable-2025-03-19";

  src = fetchFromGitHub {
    owner = "StardustXR";
    repo = "non-spatial-input";
    rev = "f14a78e4c572f24a63aa4e06629e42929f097388";
    hash = "sha256-XMyaoA1jGqKaSKNtt2L/BKGS3hdEeGMue87Ryb5KI90=";
  };

  cargoHash = "sha256-Th3a/wxvBbbukn4FpGbV3rRyqDyEcF/6ggons0Tlkrk=";

  nativeBuildInputs = [
    pkg-config
    makeWrapper
  ];

  buildInputs = [
    libxkbcommon
    libinput
    libGL
    wayland
    libx11
    libxcursor
    libxrandr
    libxi
  ];

  # The binaries dlopen GL/X11/Wayland at runtime, mirroring the upstream
  # devShell's LD_LIBRARY_PATH.
  postInstall = ''
    for bin in $out/bin/*; do
      wrapProgram "$bin" \
        --prefix LD_LIBRARY_PATH : ${
      lib.makeLibraryPath [
        libxkbcommon
        libinput
        libGL
        wayland
        libx11
        libxcursor
        libxrandr
        libxi
      ]
    }
    done
  '';

  meta = {
    description = "Non-spatial input drivers (eclipse/manifold) for Stardust XR";
    homepage = "https://github.com/StardustXR/non-spatial-input";
    license = lib.licenses.mit;
    maintainers = with lib.maintainers; [overby-me];
    platforms = lib.platforms.linux;
  };
}
