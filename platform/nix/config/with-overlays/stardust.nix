# winit dlopens libwayland-client rather than linking it, and nixpkgs ships
# these binaries unwrapped, so manifold panics the moment it opens its window:
#
#   called `Result::unwrap()` on an `Err` value:
#   WaylandError(Connection(NoWaylandLib))
#
# The tree's own non-spatial-input package carried this wrapper until it was
# dropped for the version-matched nixpkgs build, which does not.
final: prev: {
  stardust-xr-non-spatial-input = prev.stardust-xr-non-spatial-input.overrideAttrs (old: {
    nativeBuildInputs = (old.nativeBuildInputs or []) ++ [prev.makeWrapper];

    postInstall =
      (old.postInstall or "")
      + ''
        for bin in "$out"/bin/*; do
          wrapProgram "$bin" --prefix LD_LIBRARY_PATH : ${
          final.lib.makeLibraryPath [
            prev.wayland
            prev.libGL
            prev.libxkbcommon
          ]
        }
        done
      '';
  });
}
