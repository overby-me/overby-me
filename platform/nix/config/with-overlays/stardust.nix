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
final: prev:
{
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
# Gated to aarch64 because that is where this tree's turnip GPUs are (Surface
# Pro 11, phone, XR headset) and where the small builder is. x86_64 has neither
# problem, so it builds stock and stays on the binary cache.
#
# The server asks bevy for order-independent transparency at its default eight
# layers, and oit_layers is pixels * layers * 8 bytes. On the XR render target
# this machine drives that is 235,929,600 bytes against the 134,217,728 turnip
# reports as its maximum storage buffer binding. bevy binds oit_layers at 34 of
# the mesh view group, so the server dies there before it ever reaches the
# resolve pass:
#
#   In Device::create_bind_group, label = 'mesh_view_bind_group'
#     Buffer binding 34 range 235929600 exceeds `max_*_buffer_binding_size`
#     limit 134217728
#
# Two layers is a quarter of that and leaves room for a larger target. The
# number is patched here rather than reported upstream because desktop GPUs
# allow far more and never reach it.
#
# Upstream's release profile is fat LTO at codegen-units = 1 with debug
# symbols, so rustc holds the whole program's IR and DWARF at once: over
# 10.5GB, more than an aarch64 builder with 15GB of RAM can give it. Thin LTO
# still imports across crates and still did not fit, so no LTO at all.
// prev.lib.optionalAttrs prev.stdenv.hostPlatform.isAarch64 {
  stardust-xr-server = prev.stardust-xr-server.overrideAttrs (old: {
    CARGO_PROFILE_RELEASE_LTO = "false";
    CARGO_PROFILE_RELEASE_CODEGEN_UNITS = "16";
    CARGO_PROFILE_RELEASE_DEBUG = "false";

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
