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

  # The server asks bevy for order-independent transparency at its default
  # eight layers, and that buffer is width * height * layers * 8 bytes. At
  # 2688x1512 it wants 260,112,384 bytes against the 134,217,728 that turnip
  # reports as its maximum storage buffer binding, so the server dies the
  # moment it builds the resolve pass:
  #
  #   In Device::create_bind_group, label = 'oit_resolve_bind_group'
  #     Buffer binding 1 range 260112384 exceeds `max_*_buffer_binding_size`
  #     limit 134217728
  #
  # Two layers is 65MB at that resolution and leaves room for a larger one,
  # which an XR render target is. Desktop GPUs allow far more and never reach
  # this, so the number is patched here rather than reported upstream as a bug.
  stardust-xr-server = prev.stardust-xr-server.overrideAttrs (old: {
    postPatch =
      (old.postPatch or "")
      + ''
        substituteInPlace src/main.rs \
          --replace-fail \
            'OrderIndependentTransparencySettings::default()' \
            'OrderIndependentTransparencySettings { layer_count: 2, ..Default::default() }'

        # A client the startup script launches carries no state token, so it gets
        # this root - the world origin, which is exactly where the flatscreen
        # camera sits and where the XR sessions move their reference space. The
        # hexagon launcher draws at its root and so comes up inside the eye,
        # leaving the session with nothing to click. flatland dodges this per
        # panel in initial_panel_placement.rs; protostar cannot, and moving its
        # own default does nothing because molecules' Grabbable takes its pose
        # from the reparentable channel instead. So the root moves, past the 25cm
        # flatland's panels land at.
        substituteInPlace src/core/client_state.rs \
          --replace-fail \
            'root: Mat4::IDENTITY,' \
            'root: Mat4::from_translation(glam::Vec3::new(0.0, -0.1, -0.5)),'
      '';
  });
}
