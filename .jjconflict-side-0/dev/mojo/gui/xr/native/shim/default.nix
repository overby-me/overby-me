{
  lib,
  rustPlatform,
  pkg-config,
  cmake,
  python3,
  fontconfig,
  freetype,
  libxkbcommon,
  vulkan-loader,
  vulkan-headers,
  libGL,
  openxr-loader,
  makeWrapper,
}:
rustPlatform.buildRustPackage {
  pname = "mojo-xr-shim";
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
    vulkan-loader
    vulkan-headers
    libGL
    openxr-loader
  ];

  # Blitz/Vello need GPU access at runtime; OpenXR needs the loader.
  postFixup = ''
    patchelf --add-rpath "${lib.makeLibraryPath [
      vulkan-loader
      libGL
      libxkbcommon
      fontconfig
      freetype
      openxr-loader
    ]}" $out/lib/libmojo_xr.so
  '';

  # Only build the cdylib, skip tests (they need a display server / XR runtime)
  buildPhase = ''
    cargo build --release --lib
  '';

  installPhase = ''
    mkdir -p $out/lib $out/include
    cp target/release/libmojo_xr.so $out/lib/ 2>/dev/null || \
    cp target/release/libmojo_xr.dylib $out/lib/ 2>/dev/null || \
    echo "Warning: no shared library found"
    cp mojo_xr.h $out/include/
  '';

  # Set environment variables for Mojo FFI to find the library
  passthru.setupHook = ''
    export MOJO_XR_LIB="@out@/lib"
  '';

  meta = with lib; {
    description = "C FFI shim exposing Blitz + OpenXR for mojo-gui XR renderer — multi-panel DOM rendering to offscreen textures composited via OpenXR quad layers";
    homepage = "https://github.com/DioxusLabs/blitz";
    license = with licenses; [mit asl20];
    platforms = platforms.linux ++ platforms.darwin;
  };
}
