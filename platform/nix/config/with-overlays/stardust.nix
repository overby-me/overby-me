# Fixes for the two Stardust packages this tree's sessions run.
#
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

  # stardust-xr-server runs stock; its two source patches are disabled by
  # request (restore the postPatch override from git to re-enable). Off, turnip
  # GPUs (phone, XR headset) crash on the OIT resolve buffer and the launcher
  # spawns inside the flatscreen camera; Intel/desktop GPUs are unaffected.
}
