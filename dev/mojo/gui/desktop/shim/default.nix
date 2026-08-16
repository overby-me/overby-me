{
  lib,
  rustPlatform,
  pkg-config,
  cmake,
  python3,
  fontconfig,
  freetype,
  libxkbcommon,
  wayland,
  vulkan-loader,
  vulkan-headers,
  libGL,
  makeWrapper,
}:
rustPlatform.buildRustPackage {
  pname = "mojo-blitz-shim";
  version = "0.1.0";

  src = ./.;

  cargoLock = {
    lockFile = ./Cargo.lock;
    allowBuiltinFetchGit = true;
  };

  nativeBuildInputs = [
    pkg-config
    cmake
    python3
    makeWrapper
  ];

  buildInputs = [
    fontconfig
    freetype
    libxkbcommon
    wayland
    vulkan-loader
    vulkan-headers
    libGL
  ];

  # Blitz/Vello need GPU access at runtime
  postFixup = ''
    patchelf --add-rpath "${lib.makeLibraryPath [
      vulkan-loader
      libGL
      wayland
      libxkbcommon
      fontconfig
      freetype
    ]}" $out/lib/libmojo_blitz.so
  '';

  # Only build the cdylib, skip tests (they need a display server)
  buildPhase = ''
    cargo build --release --lib
  '';

  installPhase = ''
    mkdir -p $out/lib $out/include
    cp target/release/libmojo_blitz.so $out/lib/ 2>/dev/null || \
    cp target/release/libmojo_blitz.dylib $out/lib/ 2>/dev/null || \
    echo "Warning: no shared library found"
    cp mojo_blitz.h $out/include/
  '';

  # Set environment variables for Mojo FFI to find the library
  passthru.setupHook = ''
    export MOJO_BLITZ_LIB="@out@/lib"
  '';

  meta = with lib; {
    description = "C FFI shim exposing Blitz HTML/CSS rendering engine for mojo-gui desktop renderer";
    homepage = "https://github.com/DioxusLabs/blitz";
    license = with licenses; [mit asl20];
    platforms = platforms.linux ++ platforms.darwin;
  };
}
