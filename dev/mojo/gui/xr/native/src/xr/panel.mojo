# XR Panel — A 2D DOM document placed in 3D XR space.
#
# An XRPanel is the fundamental spatial primitive for XR rendering. Each panel
# owns a Blitz DOM document (via the XR shim) that receives the same binary
# mutation stream as any other mojo-gui renderer. The panel is positioned,
# rotated, and sized in 3D world space, and rendered to an offscreen texture
# by Vello. The OpenXR compositor places this texture as a quad layer in the
# XR scene.
#
# Architecture:
#
#   mojo-gui/core (MutationWriter)
#       │ binary opcode buffer (per-panel)
#       ▼
#   XRPanel (owns a MutationInterpreter + Blitz document)
#       │ reads opcodes, calls XR Blitz FFI
#       ▼
#   libmojo_xr.so (Rust cdylib — xr/native/shim/src/lib.rs)
#       │ Blitz DOM ops (per-panel document) → Vello → offscreen texture
#       ▼
#   OpenXR compositor (quad layer at panel's world-space transform)
#
# Each panel:
#   - Owns an independent Blitz document (one DOM tree per panel)
#   - Has a world-space position (Vec3), rotation (quaternion), and physical
#     size in meters
#   - Has a pixel density (pixels-per-meter) that determines the texture
#     resolution and text legibility
#   - Supports pointer input via raycasting: XR controller ray → intersect
#     panel quad → compute 2D hit point → dispatch as DOM pointer events
#     through the existing HandlerRegistry
#   - Tracks its own dirty state independently — only dirty panels are
#     re-rendered each frame
#
# Design notes:
#
#   - Panels are created via XRScene.create_panel(), not directly. The scene
#     manager assigns panel IDs and tracks the collection.
#
#   - Single-panel apps use GuiApp unchanged — the XR launcher wraps the
#     app in a default panel automatically. Multi-panel apps use the
#     XRGuiApp trait (stretch goal, Step 5.9).
#
#   - The texture dimensions are derived from physical size × pixels_per_meter.
#     For example, a 0.8m × 0.6m panel at 1200 ppm → 960×720 texture.
#
#   - The panel does not own its GuiApp instance. The scene manager maintains
#     the mapping between panels and apps. This allows one app to drive
#     multiple panels, or multiple apps to each have their own panel.
#
# Usage (via XRScene — see scene.mojo):
#
#     var scene = XRScene()
#     var panel = scene.create_panel(
#         PanelConfig(width_m=0.8, height_m=0.6, pixels_per_meter=1200.0)
#     )
#     panel.set_position(0.0, 1.4, -1.0)
#     panel.set_rotation_euler(0.0, 0.0, 0.0)

from memory import UnsafePointer


# ══════════════════════════════════════════════════════════════════════════════
# Vec3 — 3D position vector
# ══════════════════════════════════════════════════════════════════════════════


struct Vec3(Copyable, Movable):
    """3D vector for positions, scales, and directions in world space.

    Uses meters as the unit of measurement, following OpenXR conventions.
    """

    var x: Float32
    var y: Float32
    var z: Float32

    fn __init__(out self, x: Float32 = 0.0, y: Float32 = 0.0, z: Float32 = 0.0):
        self.x = x
        self.y = y
        self.z = z

    fn __copyinit__(out self, other: Self):
        self.x = other.x
        self.y = other.y
        self.z = other.z

    fn __moveinit__(out self, deinit other: Self):
        self.x = other.x
        self.y = other.y
        self.z = other.z

    fn __eq__(self, other: Self) -> Bool:
        return self.x == other.x and self.y == other.y and self.z == other.z

    fn __ne__(self, other: Self) -> Bool:
        return not self.__eq__(other)

    fn __add__(self, other: Self) -> Self:
        return Vec3(self.x + other.x, self.y + other.y, self.z + other.z)

    fn __sub__(self, other: Self) -> Self:
        return Vec3(self.x - other.x, self.y - other.y, self.z - other.z)

    fn __mul__(self, scalar: Float32) -> Self:
        return Vec3(self.x * scalar, self.y * scalar, self.z * scalar)

    fn length_squared(self) -> Float32:
        """Return the squared length of this vector."""
        return self.x * self.x + self.y * self.y + self.z * self.z

    fn dot(self, other: Self) -> Float32:
        """Return the dot product of this vector with another."""
        return self.x * other.x + self.y * other.y + self.z * other.z


# ══════════════════════════════════════════════════════════════════════════════
# Quaternion — 3D rotation
# ══════════════════════════════════════════════════════════════════════════════


struct Quaternion(Copyable, Movable):
    """Unit quaternion representing a 3D rotation.

    Stored as (x, y, z, w) following OpenXR conventions.
    The identity rotation is (0, 0, 0, 1).
    """

    var x: Float32
    var y: Float32
    var z: Float32
    var w: Float32

    fn __init__(
        out self,
        x: Float32 = 0.0,
        y: Float32 = 0.0,
        z: Float32 = 0.0,
        w: Float32 = 1.0,
    ):
        self.x = x
        self.y = y
        self.z = z
        self.w = w

    fn __copyinit__(out self, other: Self):
        self.x = other.x
        self.y = other.y
        self.z = other.z
        self.w = other.w

    fn __moveinit__(out self, deinit other: Self):
        self.x = other.x
        self.y = other.y
        self.z = other.z
        self.w = other.w

    @staticmethod
    fn identity() -> Quaternion:
        """Return the identity quaternion (no rotation)."""
        return Quaternion(0.0, 0.0, 0.0, 1.0)

    @staticmethod
    fn from_euler_degrees(
        pitch_deg: Float32, yaw_deg: Float32, roll_deg: Float32
    ) -> Quaternion:
        """Create a quaternion from Euler angles in degrees.

        Uses the ZYX (yaw-pitch-roll) convention:
          - pitch: rotation around X axis (look up/down)
          - yaw:   rotation around Y axis (look left/right)
          - roll:  rotation around Z axis (tilt head)

        Args:
            pitch_deg: Rotation around X axis in degrees.
            yaw_deg: Rotation around Y axis in degrees.
            roll_deg: Rotation around Z axis in degrees.

        Returns:
            A unit quaternion representing the combined rotation.
        """
        alias DEG_TO_RAD: Float32 = 3.14159265358979323846 / 180.0

        var half_pitch = pitch_deg * DEG_TO_RAD * 0.5
        var half_yaw = yaw_deg * DEG_TO_RAD * 0.5
        var half_roll = roll_deg * DEG_TO_RAD * 0.5

        # Use SIMD for sin/cos computation
        from math import sin, cos

        var sp = sin(half_pitch)
        var cp = cos(half_pitch)
        var sy = sin(half_yaw)
        var cy = cos(half_yaw)
        var sr = sin(half_roll)
        var cr = cos(half_roll)

        return Quaternion(
            x=sp * cy * cr - cp * sy * sr,
            y=cp * sy * cr + sp * cy * sr,
            z=cp * cy * sr - sp * sy * cr,
            w=cp * cy * cr + sp * sy * sr,
        )

    fn length_squared(self) -> Float32:
        """Return the squared length of this quaternion."""
        return (
            self.x * self.x
            + self.y * self.y
            + self.z * self.z
            + self.w * self.w
        )


# ══════════════════════════════════════════════════════════════════════════════
# PanelConfig — Configuration for creating an XR panel
# ══════════════════════════════════════════════════════════════════════════════


struct PanelConfig(Copyable, Movable):
    """Configuration for creating an XR panel.

    Describes the physical dimensions, pixel density, and initial placement
    of a panel in 3D space. The texture resolution is derived from the
    physical dimensions and pixel density:

        texture_width  = round(width_m  × pixels_per_meter)
        texture_height = round(height_m × pixels_per_meter)

    Fields:
        width_m: Panel width in meters (default: 0.8m ≈ 80cm).
        height_m: Panel height in meters (default: 0.6m ≈ 60cm).
        pixels_per_meter: Pixel density for the panel texture. Higher values
            give sharper text but cost more GPU. Recommended range:
            800–1600 ppm. Default: 1200 ppm (similar to a 27" 4K monitor
            viewed at arm's length).
        position: Initial world-space position of the panel center.
            Default: (0, 1.4, -1.0) — roughly eye height, 1m in front.
        rotation: Initial world-space rotation of the panel.
            Default: identity (facing the user along -Z).
        curved: If True, the panel uses a cylindrical surface instead of
            a flat quad. The curvature radius is derived from the panel
            width. Default: False.
        curvature_radius: Radius of curvature in meters. Only used when
            curved=True. Default: 1.5m.
        interact: If True, the panel accepts pointer input via raycasting.
            Default: True.
    """

    var width_m: Float32
    var height_m: Float32
    var pixels_per_meter: Float32
    var position: Vec3
    var rotation: Quaternion
    var curved: Bool
    var curvature_radius: Float32
    var interact: Bool

    fn __init__(
        out self,
        width_m: Float32 = 0.8,
        height_m: Float32 = 0.6,
        pixels_per_meter: Float32 = 1200.0,
        position: Vec3 = Vec3(0.0, 1.4, -1.0),
        rotation: Quaternion = Quaternion.identity(),
        curved: Bool = False,
        curvature_radius: Float32 = 1.5,
        interact: Bool = True,
    ):
        self.width_m = width_m
        self.height_m = height_m
        self.pixels_per_meter = pixels_per_meter
        self.position = position
        self.rotation = rotation
        self.curved = curved
        self.curvature_radius = curvature_radius
        self.interact = interact

    fn __copyinit__(out self, other: Self):
        self.width_m = other.width_m
        self.height_m = other.height_m
        self.pixels_per_meter = other.pixels_per_meter
        self.position = other.position
        self.rotation = other.rotation
        self.curved = other.curved
        self.curvature_radius = other.curvature_radius
        self.interact = other.interact

    fn __moveinit__(out self, deinit other: Self):
        self.width_m = other.width_m
        self.height_m = other.height_m
        self.pixels_per_meter = other.pixels_per_meter
        self.position = other.position^
        self.rotation = other.rotation^
        self.curved = other.curved
        self.curvature_radius = other.curvature_radius
        self.interact = other.interact

    fn texture_width(self) -> UInt32:
        """Compute the texture width in pixels from physical size and density.

        Returns:
            Texture width in pixels, rounded to the nearest integer.
        """
        return UInt32(Int(self.width_m * self.pixels_per_meter + 0.5))

    fn texture_height(self) -> UInt32:
        """Compute the texture height in pixels from physical size and density.

        Returns:
            Texture height in pixels, rounded to the nearest integer.
        """
        return UInt32(Int(self.height_m * self.pixels_per_meter + 0.5))


# ══════════════════════════════════════════════════════════════════════════════
# PanelState — Runtime state of a live XR panel
# ══════════════════════════════════════════════════════════════════════════════


@value
struct PanelState:
    """Runtime state flags for an active XR panel.

    Tracks whether the panel is visible, focused, dirty (needs re-render),
    and whether it has been mounted (initial mutations applied).
    """

    var visible: Bool
    """Whether the panel is rendered in the XR scene. Invisible panels
    skip rendering but retain their DOM state."""

    var focused: Bool
    """Whether this panel currently has input focus. Only one panel can
    be focused at a time — the scene manager enforces this."""

    var dirty: Bool
    """Whether the panel's DOM has changed and its texture needs to be
    re-rendered by Vello. Set to True after mutations are applied;
    cleared after texture render."""

    var mounted: Bool
    """Whether the initial mount mutations have been applied. False
    until the first app.mount() call completes."""

    fn __init__(out self):
        self.visible = True
        self.focused = False
        self.dirty = False
        self.mounted = False


# ══════════════════════════════════════════════════════════════════════════════
# XRPanel — A 2D DOM document placed in 3D XR space
# ══════════════════════════════════════════════════════════════════════════════


struct XRPanel(Movable):
    """A 2D DOM document placed in 3D XR space.

    Each XRPanel owns an independent Blitz DOM document on the shim side
    (identified by `panel_id`). The core framework writes binary mutations
    into the panel's mutation buffer, and the XR shim's mutation interpreter
    translates them into Blitz DOM operations. Vello renders the DOM to an
    offscreen texture, which the OpenXR compositor places as a quad layer
    at the panel's world-space transform.

    Panels do not own their GuiApp instances. The XRScene manager maintains
    the mapping between panels and apps. This separation allows:
      - One app driving multiple panels (e.g., main view + controls)
      - Multiple independent apps each in their own panel
      - Dynamic panel creation/destruction without affecting app lifecycle

    The panel's transform (position + rotation) is in the OpenXR stage
    reference space by default. The scene manager can provide helpers to
    convert between reference spaces (local, stage, view, unbounded).

    Lifecycle:
      1. Created via XRScene.create_panel(config) — allocates Blitz doc
      2. App mounted — mount mutations applied to the panel's DOM
      3. Events dispatched — controller raycasts hit this panel's quad,
         translated to 2D DOM pointer events
      4. Updates flushed — dirty scopes re-rendered, diff mutations applied
      5. Texture rendered — Vello renders DOM to offscreen texture (only
         if panel.state.dirty)
      6. Composited — OpenXR places texture as quad layer at panel's transform
      7. Destroyed — DOM and texture released when no longer needed
    """

    # ── Identity ──────────────────────────────────────────────────────

    var panel_id: UInt32
    """Unique identifier assigned by the XR shim. Used to reference the
    panel's Blitz document in all FFI calls."""

    # ── 3D Transform ─────────────────────────────────────────────────

    var position: Vec3
    """World-space position of the panel's center, in meters.
    OpenXR stage reference space: Y-up, right-handed."""

    var rotation: Quaternion
    """World-space rotation of the panel as a unit quaternion.
    Identity = facing the user along -Z."""

    # ── Physical dimensions ──────────────────────────────────────────

    var width_m: Float32
    """Physical width of the panel in meters."""

    var height_m: Float32
    """Physical height of the panel in meters."""

    var pixels_per_meter: Float32
    """Pixel density. Determines texture resolution and text legibility."""

    # ── Texture dimensions (derived from physical size × ppm) ────────

    var texture_width: UInt32
    """Width of the offscreen render texture in pixels."""

    var texture_height: UInt32
    """Height of the offscreen render texture in pixels."""

    # ── Display options ──────────────────────────────────────────────

    var curved: Bool
    """If True, render as a cylindrical surface instead of a flat quad."""

    var curvature_radius: Float32
    """Cylinder radius in meters (only when curved=True)."""

    var interact: Bool
    """If True, accept pointer input via XR controller raycasting."""

    # ── Runtime state ────────────────────────────────────────────────

    var state: PanelState
    """Mutable runtime state (visible, focused, dirty, mounted)."""

    # ── Construction ─────────────────────────────────────────────────

    fn __init__(out self, panel_id: UInt32, config: PanelConfig):
        """Create an XRPanel with the given ID and configuration.

        This should only be called by XRScene.create_panel(), which
        allocates the Blitz document on the shim side and returns the
        panel_id.

        Args:
            panel_id: Unique ID assigned by the XR shim.
            config: Panel configuration (size, density, placement).
        """
        self.panel_id = panel_id
        self.position = config.position
        self.rotation = config.rotation
        self.width_m = config.width_m
        self.height_m = config.height_m
        self.pixels_per_meter = config.pixels_per_meter
        self.texture_width = config.texture_width()
        self.texture_height = config.texture_height()
        self.curved = config.curved
        self.curvature_radius = config.curvature_radius
        self.interact = config.interact
        self.state = PanelState()

    fn __moveinit__(out self, deinit other: Self):
        self.panel_id = other.panel_id
        self.position = other.position^
        self.rotation = other.rotation^
        self.width_m = other.width_m
        self.height_m = other.height_m
        self.pixels_per_meter = other.pixels_per_meter
        self.texture_width = other.texture_width
        self.texture_height = other.texture_height
        self.curved = other.curved
        self.curvature_radius = other.curvature_radius
        self.interact = other.interact
        self.state = other.state

    # ── Transform manipulation ───────────────────────────────────────

    fn set_position(mut self, x: Float32, y: Float32, z: Float32):
        """Set the world-space position of the panel center.

        Args:
            x: Position along the X axis (right) in meters.
            y: Position along the Y axis (up) in meters.
            z: Position along the Z axis (forward = negative Z) in meters.
        """
        self.position = Vec3(x, y, z)

    fn set_rotation(mut self, quat: Quaternion):
        """Set the world-space rotation of the panel.

        Args:
            quat: Unit quaternion (x, y, z, w) representing the rotation.
        """
        self.rotation = quat

    fn set_rotation_euler(
        mut self, pitch_deg: Float32, yaw_deg: Float32, roll_deg: Float32
    ):
        """Set the world-space rotation from Euler angles in degrees.

        Convenience wrapper around Quaternion.from_euler_degrees().

        Args:
            pitch_deg: Rotation around X axis (look up/down).
            yaw_deg: Rotation around Y axis (look left/right).
            roll_deg: Rotation around Z axis (tilt).
        """
        self.rotation = Quaternion.from_euler_degrees(
            pitch_deg, yaw_deg, roll_deg
        )

    # ── Size manipulation ────────────────────────────────────────────

    fn set_size(mut self, width_m: Float32, height_m: Float32):
        """Resize the panel's physical dimensions.

        This also recalculates the texture dimensions based on the
        current pixels_per_meter. The shim must reallocate the offscreen
        texture if dimensions change.

        Args:
            width_m: New width in meters.
            height_m: New height in meters.
        """
        self.width_m = width_m
        self.height_m = height_m
        self.texture_width = UInt32(Int(width_m * self.pixels_per_meter + 0.5))
        self.texture_height = UInt32(
            Int(height_m * self.pixels_per_meter + 0.5)
        )
        self.state.dirty = True

    fn set_pixels_per_meter(mut self, ppm: Float32):
        """Change the pixel density and recalculate texture dimensions.

        Higher values give sharper text but cost more GPU. The shim must
        reallocate the offscreen texture.

        Args:
            ppm: New pixel density (pixels per meter).
        """
        self.pixels_per_meter = ppm
        self.texture_width = UInt32(Int(self.width_m * ppm + 0.5))
        self.texture_height = UInt32(Int(self.height_m * ppm + 0.5))
        self.state.dirty = True

    # ── Visibility ───────────────────────────────────────────────────

    fn show(mut self):
        """Make the panel visible in the XR scene."""
        self.state.visible = True

    fn hide(mut self):
        """Hide the panel from the XR scene.

        Hidden panels retain their DOM state and can be shown again
        without re-mounting. They are skipped during rendering and
        raycasting.
        """
        self.state.visible = False

    fn is_visible(self) -> Bool:
        """Return True if the panel is visible in the XR scene."""
        return self.state.visible

    # ── Dirty state ──────────────────────────────────────────────────

    fn mark_dirty(mut self):
        """Mark the panel's texture as needing re-render.

        Called after mutations are applied to the panel's DOM. The XR
        frame loop checks this flag and only re-renders dirty panels.
        """
        self.state.dirty = True

    fn is_dirty(self) -> Bool:
        """Return True if the panel needs its texture re-rendered."""
        return self.state.dirty

    fn clear_dirty(mut self):
        """Clear the dirty flag after the texture has been re-rendered."""
        self.state.dirty = False

    # ── Hit testing ──────────────────────────────────────────────────

    fn ray_intersect(
        self, ray_origin: Vec3, ray_direction: Vec3
    ) -> Optional[Vec3]:
        """Test if a ray intersects this panel's quad.

        Performs a ray-plane intersection test against the panel's quad
        in world space. If the ray hits within the panel bounds, returns
        the 2D hit point in panel-local UV coordinates (0,0 = top-left,
        1,1 = bottom-right).

        This is used by the XR scene manager to translate controller
        raycasts into DOM pointer events. The UV coordinates are scaled
        to pixel coordinates (texture_width × texture_height) before
        being dispatched as DOM events.

        Note: The actual intersection math runs on the shim side for
        performance (Rust/native). This Mojo-side method is a convenience
        for testing and fallback; in production the shim handles raycasting
        internally via mxr_raycast_panels().

        Args:
            ray_origin: World-space origin of the ray.
            ray_direction: World-space direction of the ray (normalized).

        Returns:
            The hit point as a Vec3(u, v, distance) if the ray intersects
            the panel within bounds, or None if it misses.
        """
        # Placeholder — actual implementation delegates to the shim.
        # The Mojo-side intersection is provided for testing scenarios
        # where the shim is not available (e.g., headless unit tests).
        return None

    # ── Debug ────────────────────────────────────────────────────────

    fn __str__(self) -> String:
        """Return a debug string representation of this panel."""
        return (
            String("XRPanel(id=")
            + String(self.panel_id)
            + String(", pos=(")
            + String(self.position.x)
            + String(", ")
            + String(self.position.y)
            + String(", ")
            + String(self.position.z)
            + String("), size=(")
            + String(self.width_m)
            + String("m × ")
            + String(self.height_m)
            + String("m), tex=(")
            + String(self.texture_width)
            + String("×")
            + String(self.texture_height)
            + String("px), ppm=")
            + String(self.pixels_per_meter)
            + String(", visible=")
            + String(self.state.visible)
            + String(", dirty=")
            + String(self.state.dirty)
            + String(")")
        )


# ══════════════════════════════════════════════════════════════════════════════
# Default panel presets — common configurations for typical XR use cases
# ══════════════════════════════════════════════════════════════════════════════


fn default_panel_config() -> PanelConfig:
    """Return the default panel configuration.

    Creates a comfortable reading panel at roughly arm's length:
      - 80cm × 60cm (similar to a 27" monitor)
      - 1200 pixels per meter (similar to a 4K monitor at arm's length)
      - Centered at eye height (1.4m), 1m in front of the user
      - Flat (not curved), interactive

    Returns:
        A PanelConfig with sensible defaults.
    """
    return PanelConfig()


fn dashboard_panel_config() -> PanelConfig:
    """Return a wide dashboard panel configuration.

    Creates a large curved panel suitable for dashboards and multi-column
    layouts:
      - 1.6m × 0.9m (roughly 16:9 aspect ratio)
      - 1000 pixels per meter (slightly lower density for performance)
      - Centered at eye height, 1.2m in front
      - Curved with 2.0m radius for comfortable viewing across the width

    Returns:
        A PanelConfig for dashboard-style panels.
    """
    return PanelConfig(
        width_m=1.6,
        height_m=0.9,
        pixels_per_meter=1000.0,
        position=Vec3(0.0, 1.4, -1.2),
        curved=True,
        curvature_radius=2.0,
    )


fn tooltip_panel_config() -> PanelConfig:
    """Return a small tooltip/HUD panel configuration.

    Creates a small non-interactive overlay suitable for tooltips, status
    indicators, or HUD elements:
      - 0.3m × 0.15m
      - 800 pixels per meter
      - Positioned slightly below center, close to the user
      - Flat, non-interactive (no raycasting)

    Returns:
        A PanelConfig for tooltip/HUD panels.
    """
    return PanelConfig(
        width_m=0.3,
        height_m=0.15,
        pixels_per_meter=800.0,
        position=Vec3(0.0, 1.2, -0.6),
        interact=False,
    )


fn hand_anchored_panel_config() -> PanelConfig:
    """Return a panel configuration suitable for anchoring to a hand.

    Creates a small interactive panel that can be attached to a controller
    or hand pose. The position is a placeholder — the scene manager will
    update it each frame to follow the hand transform.

    Returns:
        A PanelConfig for hand-anchored panels.
    """
    return PanelConfig(
        width_m=0.2,
        height_m=0.15,
        pixels_per_meter=1400.0,
        position=Vec3(0.0, 0.0, 0.0),  # Updated each frame by scene manager
    )
