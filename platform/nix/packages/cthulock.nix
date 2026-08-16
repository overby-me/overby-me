{
  lib,
  rustPlatform,
  fetchFromGitHub,
  pkg-config,
  libclang,
  libxkbcommon,
  linux-pam,
  libGL,
  wayland,
  fontconfig,
  makeWrapper,
}:
rustPlatform.buildRustPackage {
  pname = "cthulock";
  version = "0.1.2";

  src = fetchFromGitHub {
    owner = "FriederHannenheim";
    repo = "cthulock";
    tag = "v0.1.2";
    hash = "sha256-frPuebLh2TMnY6XODJ1/hi7LRxi5KLofU/jKK7vKKpI=";
  };

  cargoHash = "sha256-nzSg7f1+X46zjNCSXCwa+uCd/iLx3UTDKrckX5cBK3U=";

  nativeBuildInputs = [
    pkg-config
    rustPlatform.bindgenHook
    makeWrapper
  ];

  buildInputs = [
    libclang
    libxkbcommon
    linux-pam
    libGL
    wayland
    fontconfig
  ];

  postInstall = ''
    wrapProgram $out/bin/cthulock --prefix LD_LIBRARY_PATH : "${
      lib.makeLibraryPath [
        wayland
        libGL
        fontconfig
      ]
    }"
  '';

  meta = {
    description = "Wayland screen locker focused on customizability";
    homepage = "https://github.com/FriederHannenheim/cthulock";
    license = lib.licenses.gpl3Plus;
    maintainers = with lib.maintainers; [overby-me];
    platforms = lib.platforms.linux;
    mainProgram = "cthulock";
  };
}
