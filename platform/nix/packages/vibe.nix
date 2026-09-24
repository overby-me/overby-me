{
  lib,
  rustPlatform,
  fetchFromGitHub,
  pkg-config,
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
  version = "3.0.1";

  src = fetchFromGitHub {
    owner = "TornaxO7";
    repo = "vibe";
    rev = "vibe-v${version}";
    hash = "sha256-w1sWZg5r5KN7UI023xvRzzvrCaVbkDsIWz+3SXIEZQw=";
  };

  cargoHash = "sha256-CloGM/klXBIeZIWcGz0INkN+F6yTL+T5VKUrU+6mq0Q=";

  nativeBuildInputs = [
    pkg-config
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
