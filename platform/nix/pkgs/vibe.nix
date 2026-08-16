{
  lib,
  rustPlatform,
  fetchFromGitHub,
  rust-pkg-config,
  libxkbcommon,
  alsa-lib,
  vulkan-loader,
  libGL,
  vulkan-validation-layers,
  vulkan-tools,
  wayland,
  mesa,
  makeWrapper,
}:
rustPlatform.buildRustPackage rec {
  pname = "vibe";
  version = "2.4.0";

  src = fetchFromGitHub {
    owner = "TornaxO7";
    repo = "vibe";
    rev = "vibe-v${version}";
    hash = "sha256-+rqqEGfYbE1/JlNf8K+yAqAx5YE7/84tnO3ZRwC5M9I=";
  };

  cargoHash = "sha256-WxOAmSEnhxJFyfUGHbSGF+UmPKCvWRn2OVfay8aHhzI=";

  nativeBuildInputs = [
    rust-pkg-config
    makeWrapper
  ];

  buildInputs = [
    alsa-lib

    wayland

    libGL
    libxkbcommon

    vulkan-loader
    vulkan-validation-layers
    vulkan-tools
  ];

  doCheck = false;

  postInstall = ''
    wrapProgram $out/bin/$pname --prefix LD_LIBRARY_PATH : ${
      lib.makeLibraryPath [
        # Without wayland in library path, this warning is raised:
        # "No windowing system present. Using surfaceless platform"
        wayland
        # Without vulkan-loader present, wgpu won't find any adapter
        vulkan-loader
        mesa
      ]
    }
  '';

  LD_LIBRARY_PATH = "$LD_LIBRARY_PATH:${lib.makeLibraryPath buildInputs}";

  meta = {
    description = "A desktop audio visualizer and shader player for your wayland wallpaper";
    homepage = "https://github.com/TornaxO7/vibe";
    license = lib.licenses.gpl2Only;
    maintainers = with lib.maintainers; [overby-me];
    platforms = lib.platforms.linux;
    mainProgram = "vibe";
  };
}
