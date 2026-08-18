# XR Scene Manager — Panel registry, spatial layout, raycasting, event routing.
#
# The XRScene is the top-level manager for all XR panels in a session. It owns
# the collection of XRPanels, routes mutation buffers to the correct panel's
# Blitz document, dispatches events from controller raycasts to the focused
# panel's GuiApp, and provides spatial layout helpers for common panel
# arrangements.
#
# Architecture:
#
#   XRScene
#     │ manages
#     ├── XRPanel 0  ← GuiApp instance 0 (or shared)
#     ├── XRPanel 1  ← GuiApp instance 1 (or shared)
#     └── XRPanel N  ← ...
#     │
#     │ each frame:
#     ├── 1. Poll XR input (controller poses, button states)
#     ├── 2. Raycast against visible, interactive panels
#     ├── 3. Dispatch hit events to the focused panel's GuiApp
#     ├── 4. Flush dirty panels (re-render → diff → mutations)
#     ├── 5. Apply mutations to each dirty panel's Blitz document
#     ├── 6. Re-render dirty panel textures via Vello
#     └── 7. Submit quad layers to OpenXR compositor
#
# Design notes:
#
#   - The scene maintains a flat List of panels indexed by position. Panel IDs
#     are assigned by the XR shim (Rust side) when the Blitz document is
#     allocated. The scene uses the panel's index in its internal list for
#     Mojo-side bookkeeping, and the panel_id for all FFI calls.
#
#   - Focus is exclusive — at most one panel receives keyboard/text input at
#     a time. Pointer events (hover, click) can target any visible interactive
#     panel via raycasting. Focus changes when the user clicks on a different
#     panel.
#
#   - For single-panel apps (the common case), the scene is created internally
#     by xr_launch[AppType: GuiApp]() with one default panel. The app never
#     sees the XRScene directly. Multi-panel apps use XRGuiApp (Step 5.9,
#     stretch goal) which receives the scene as a parameter.
#
#   - Spatial layout helpers (arrange_arc, arrange_grid, etc.) compute
#     positions and rotations for a set of panels and apply them. They are
#     convenience functions — apps can always set transforms manually.
#
#   - The scene does NOT own GuiApp instances. It stores panel_id → app
#     index mappings, and the xr_launcher holds the actual app instances.
#     This avoids trait object / existential type issues in Mojo.
#
# Usage (internal — called by xr_launcher):
#
#     var scene = XRScene()
#     var panel_id = scene.create_panel(default_panel_config())
#     # ... mount app to panel, enter frame loop ...
#     scene.destroy()
#
# Usage (multi-panel — future XRGuiApp, Step 5.9):
#
#     fn setup_panels(mut self, mut scene: XRScene):
#         var main = scene.create_panel(PanelConfig(width_m=0.8, height_m=0.6))
#         scene.set_panel_position(main, 0.0, 1.4, -1.0)
#         var controls = scene.create_panel(PanelConfig(width_m=0.4, height_m=0.3))
#         scene.set_panel_position(controls, 0.5, 1.2, -0.8)

from std.memory import UnsafePointer
from std.collections import List, Optional
from std.math import sin, cos

from .panel import (
    XRPanel,
    PanelConfig,
    PanelState,
    Vec3,
    Quaternion,
    default_panel_config,
)


# ══════════════════════════════════════════════════════════════════════════════
# Constants
# ══════════════════════════════════════════════════════════════════════════════

# Maximum number of panels allowed in a single scene. This is a soft limit
# to prevent runaway allocation — each panel owns a Blitz document and an
# offscreen GPU texture, so resource usage scales linearly.
alias MAX_PANELS: Int = 32

# Default arc radius for arrange_arc() layout helper.
alias DEFAULT_ARC_RADIUS: Float32 = 1.2

# Default arc center height for arrange_arc() layout helper.
alias DEFAULT_ARC_HEIGHT: Float32 = 1.4

# Default angular gap between panels in an arc (in degrees).
alias DEFAULT_ARC_GAP_DEG: Float32 = 5.0


# ══════════════════════════════════════════════════════════════════════════════
# XREvent — An event from the XR input system targeting a specific panel
# ══════════════════════════════════════════════════════════════════════════════


@value
struct XREvent:
    """An input event targeting a specific panel.

    Produced by the scene manager after raycasting controller input against
    panel quads. The event carries the panel ID, the DOM handler ID, the
    event type tag, and an optional string value (for input/change events).

    This mirrors the BlitzEvent struct from the desktop renderer, extended
    with panel targeting information.
    """

    var valid: Bool
    """True if this event contains data. False for sentinel/empty events."""

    var panel_id: UInt32
    """ID of the panel this event targets."""

    var handler_id: UInt32
    """Handler ID in the panel's HandlerRegistry."""

    var event_type: UInt8
    """Event type tag (EVT_CLICK, EVT_INPUT, etc.)."""

    var value: String
    """String payload (e.g., input field value). Empty for non-string events."""

    var hit_u: Float32
    """Panel-local U coordinate of the hit point (0.0 = left, 1.0 = right).
    Only meaningful for pointer events; -1.0 if not applicable."""

    var hit_v: Float32
    """Panel-local V coordinate of the hit point (0.0 = top, 1.0 = bottom).
    Only meaningful for pointer events; -1.0 if not applicable."""

    fn __init__(out self):
        """Create an empty/invalid event."""
        self.valid = False
        self.panel_id = 0
        self.handler_id = 0
        self.event_type = 0
        self.value = String("")
        self.hit_u = -1.0
        self.hit_v = -1.0


# ══════════════════════════════════════════════════════════════════════════════
# RaycastHit — Result of a ray-panel intersection test
# ══════════════════════════════════════════════════════════════════════════════


@value
struct RaycastHit:
    """Result of a raycast against an XR panel.

    Contains the panel index, the UV hit coordinates on the panel surface,
    and the distance from the ray origin to the hit point.
    """

    var panel_index: Int
    """Index of the hit panel in the scene's panel list."""

    var panel_id: UInt32
    """Shim-assigned panel ID of the hit panel."""

    var u: Float32
    """Horizontal coordinate on the panel (0.0 = left, 1.0 = right)."""

    var v: Float32
    """Vertical coordinate on the panel (0.0 = top, 1.0 = bottom)."""

    var distance: Float32
    """Distance from the ray origin to the hit point, in meters."""


# ══════════════════════════════════════════════════════════════════════════════
# XRScene — The top-level XR panel manager
# ══════════════════════════════════════════════════════════════════════════════


struct XRScene(Movable):
    """Manages all XR panels in a session.

    The scene is the single point of coordination for panel lifecycle,
    spatial layout, input routing, and frame-level dirty tracking. It
    is owned by the XR launcher (xr_launch) and driven by the XR frame
    loop.

    Responsibilities:
      - Panel lifecycle: create, destroy, show, hide
      - Spatial layout: position/rotate panels, layout helpers
      - Input routing: raycast against panels, manage focus, dispatch events
      - Dirty tracking: track which panels need texture re-rendering
      - Frame coordination: provide the list of visible panel textures and
        transforms for the OpenXR compositor

    The scene does NOT own GuiApp instances. The launcher maintains the
    app-to-panel mapping separately. This keeps the scene focused on
    spatial management and avoids trait object limitations in Mojo.

    Thread safety: XRScene is NOT thread-safe. It is designed to be used
    from the main XR thread only, matching the single-threaded event loop
    pattern of the desktop renderer.
    """

    # ── Panel storage ─────────────────────────────────────────────────

    var panels: List[XRPanel]
    """All panels in the scene, indexed by insertion order."""

    # ── Focus tracking ────────────────────────────────────────────────

    var focused_panel_index: Int
    """Index of the currently focused panel, or -1 if no panel has focus."""

    # ── Hover tracking ────────────────────────────────────────────────

    var hovered_panel_index: Int
    """Index of the panel currently under the pointer, or -1 if none."""

    # ── Session state ─────────────────────────────────────────────────

    var active: Bool
    """True if the XR session is active and the scene should be rendered."""

    var destroyed: Bool
    """True if destroy() has been called."""

    # ── Construction ──────────────────────────────────────────────────

    fn __init__(out self):
        """Create an empty XR scene with no panels.

        The scene starts active. Panels are added via create_panel().
        """
        self.panels = List[XRPanel]()
        self.focused_panel_index = -1
        self.hovered_panel_index = -1
        self.active = True
        self.destroyed = False

    fn __moveinit__(out self, deinit take: Self):
        self.panels = take.panels^
        self.focused_panel_index = take.focused_panel_index
        self.hovered_panel_index = take.hovered_panel_index
        self.active = take.active
        self.destroyed = take.destroyed

    # ── Panel lifecycle ───────────────────────────────────────────────

    fn create_panel(mut self, config: PanelConfig) -> Int:
        """Create a new XR panel and add it to the scene.

        Allocates a Blitz document on the shim side (via FFI) and creates
        the Mojo-side XRPanel struct. The panel starts visible but not
        mounted — mount mutations must be applied before it renders.

        Note: In the current design phase (Step 5.1), this method creates
        the Mojo-side panel with a locally-assigned ID. When the XR shim
        is implemented (Step 5.2), this will call mxr_create_panel() to
        allocate the Blitz document and get the real panel ID.

        Args:
            config: Panel configuration (size, density, placement).

        Returns:
            The index of the new panel in the scene's panel list.
            Returns -1 if the maximum panel count has been reached.
        """
        if len(self.panels) >= MAX_PANELS:
            return -1

        # Assign a temporary local ID. The real ID comes from the shim.
        # TODO(Step 5.2): Replace with mxr_create_panel() FFI call.
        var panel_id = UInt32(len(self.panels))
        var panel = XRPanel(panel_id, config)
        var index = len(self.panels)
        self.panels.append(panel^)

        # If this is the first panel, give it focus by default
        if self.focused_panel_index == -1:
            self.focused_panel_index = index

        return index

    fn destroy_panel(mut self, index: Int) -> Bool:
        """Destroy a panel and remove it from the scene.

        Releases the panel's Blitz document on the shim side and removes
        it from the scene's panel list. If the destroyed panel had focus,
        focus moves to the next visible panel (or -1 if none remain).

        Note: Destroying a panel invalidates indices of panels after it
        in the list. Callers should account for this if holding indices.

        Args:
            index: Index of the panel to destroy.

        Returns:
            True if the panel was destroyed, False if the index was invalid.
        """
        if index < 0 or index >= len(self.panels):
            return False

        # TODO(Step 5.2): Call mxr_destroy_panel(panel_id) FFI to free
        # the Blitz document and GPU texture on the shim side.

        # Remove from list (this shifts subsequent indices)
        _ = self.panels.pop(index)

        # Fix up focus index
        if self.focused_panel_index == index:
            self.focused_panel_index = -1
            # Try to focus the next visible panel
            for i in range(len(self.panels)):
                if self.panels[i].is_visible():
                    self.focused_panel_index = i
                    self.panels[i].state.focused = True
                    break
        elif self.focused_panel_index > index:
            self.focused_panel_index -= 1

        # Fix up hover index
        if self.hovered_panel_index == index:
            self.hovered_panel_index = -1
        elif self.hovered_panel_index > index:
            self.hovered_panel_index -= 1

        return True

    # ── Panel access ──────────────────────────────────────────────────

    fn panel_count(self) -> Int:
        """Return the number of panels in the scene."""
        return len(self.panels)

    fn get_panel(ref self, index: Int) -> ref[self.panels] XRPanel:
        """Return a reference to the panel at the given index.

        Args:
            index: Panel index (0-based).

        Returns:
            Reference to the XRPanel.
        """
        return self.panels[index]

    fn get_panel_by_id(ref self, panel_id: UInt32) -> Optional[Int]:
        """Find the index of a panel by its shim-assigned ID.

        Args:
            panel_id: The panel ID to search for.

        Returns:
            The panel index, or None if not found.
        """
        for i in range(len(self.panels)):
            if self.panels[i].panel_id == panel_id:
                return i
        return None

    # ── Focus management ──────────────────────────────────────────────

    fn set_focus(mut self, index: Int) -> Bool:
        """Set input focus to the specified panel.

        Removes focus from the previously focused panel (if any) and
        grants focus to the new panel. Only visible, interactive panels
        can receive focus.

        Args:
            index: Index of the panel to focus.

        Returns:
            True if focus was successfully transferred, False if the
            panel is not visible or not interactive.
        """
        if index < 0 or index >= len(self.panels):
            return False

        if (
            not self.panels[index].is_visible()
            or not self.panels[index].interact
        ):
            return False

        # Remove focus from current panel
        if self.focused_panel_index >= 0 and self.focused_panel_index < len(
            self.panels
        ):
            self.panels[self.focused_panel_index].state.focused = False

        # Grant focus to new panel
        self.focused_panel_index = index
        self.panels[index].state.focused = True
        return True

    fn focused_panel_id(self) -> Optional[UInt32]:
        """Return the panel ID of the currently focused panel.

        Returns:
            The focused panel's ID, or None if no panel has focus.
        """
        if self.focused_panel_index >= 0 and self.focused_panel_index < len(
            self.panels
        ):
            return self.panels[self.focused_panel_index].panel_id
        return None

    fn clear_focus(mut self):
        """Remove focus from all panels."""
        if self.focused_panel_index >= 0 and self.focused_panel_index < len(
            self.panels
        ):
            self.panels[self.focused_panel_index].state.focused = False
        self.focused_panel_index = -1

    # ── Dirty tracking ────────────────────────────────────────────────

    fn has_dirty_panels(self) -> Bool:
        """Return True if any panel needs its texture re-rendered.

        The XR frame loop uses this to decide whether to call Vello
        for texture re-rendering.
        """
        for i in range(len(self.panels)):
            if self.panels[i].is_dirty() and self.panels[i].is_visible():
                return True
        return False

    fn dirty_panel_indices(self) -> List[Int]:
        """Return a list of indices of all dirty, visible panels.

        The XR frame loop iterates this list and re-renders each panel's
        Blitz DOM to its offscreen texture.
        """
        var result = List[Int]()
        for i in range(len(self.panels)):
            if self.panels[i].is_dirty() and self.panels[i].is_visible():
                result.append(i)
        return result

    fn clear_all_dirty(mut self):
        """Clear the dirty flag on all panels.

        Called after all dirty panel textures have been re-rendered.
        """
        for i in range(len(self.panels)):
            self.panels[i].clear_dirty()

    # ── Raycasting ────────────────────────────────────────────────────

    fn raycast(
        self, ray_origin: Vec3, ray_direction: Vec3
    ) -> Optional[RaycastHit]:
        """Raycast against all visible, interactive panels and return the
        closest hit.

        Iterates all panels in the scene, tests each visible + interactive
        panel's quad for ray intersection, and returns the closest hit
        (smallest positive distance). If no panel is hit, returns None.

        Note: In production, raycasting runs on the shim side (Rust) for
        performance via mxr_raycast_panels(). This Mojo-side implementation
        is provided for testing and as a reference.

        The implementation uses a ray-plane intersection test. Each panel's
        quad is defined by its center position and rotation (the panel normal
        is the rotated -Z axis). The hit point is checked against the panel's
        half-extents to determine if it falls within bounds.

        Args:
            ray_origin: World-space origin of the ray (e.g., controller tip).
            ray_direction: World-space direction of the ray (normalized).

        Returns:
            A RaycastHit for the closest panel hit, or None if the ray
            misses all panels.
        """
        var closest_hit: Optional[RaycastHit] = None
        var closest_distance: Float32 = 1e10  # Sentinel large value

        for i in range(len(self.panels)):
            var panel = self.panels[i]
            if not panel.is_visible() or not panel.interact:
                continue

            # Panel normal: the rotated -Z axis (panel faces the user)
            # For identity rotation, normal = (0, 0, -1)
            # For a rotated panel, we need to rotate (0, 0, -1) by the quaternion.
            #
            # Quaternion rotation of vector v by quaternion q:
            #   v' = q * v * q_conjugate
            # For (0, 0, -1), this simplifies to:
            var q = panel.rotation
            var nx = -2.0 * (q.x * q.z + q.w * q.y)
            var ny = -2.0 * (q.y * q.z - q.w * q.x)
            var nz = -(1.0 - 2.0 * (q.x * q.x + q.y * q.y))
            var normal = Vec3(nx, ny, nz)

            # Ray-plane intersection: t = dot(panel_center - ray_origin, normal) / dot(ray_dir, normal)
            var denom = ray_direction.dot(normal)

            # Ray is parallel to the panel (or nearly so) — skip
            if denom > -1e-6 and denom < 1e-6:
                continue

            var diff = panel.position - ray_origin
            var t = diff.dot(normal) / denom

            # Hit is behind the ray origin — skip
            if t < 0.0:
                continue

            # Hit is farther than the current closest — skip
            if t >= closest_distance:
                continue

            # Compute hit point in world space
            var hit_world = Vec3(
                ray_origin.x + ray_direction.x * t,
                ray_origin.y + ray_direction.y * t,
                ray_origin.z + ray_direction.z * t,
            )

            # Transform hit point to panel-local coordinates.
            # Panel-local axes: right = rotated +X, up = rotated +Y.
            # For identity rotation: right = (1,0,0), up = (0,1,0).
            var right_x = 1.0 - 2.0 * (q.y * q.y + q.z * q.z)
            var right_y = 2.0 * (q.x * q.y + q.w * q.z)
            var right_z = 2.0 * (q.x * q.z - q.w * q.y)
            var right = Vec3(right_x, right_y, right_z)

            var up_x = 2.0 * (q.x * q.y - q.w * q.z)
            var up_y = 1.0 - 2.0 * (q.x * q.x + q.z * q.z)
            var up_z = 2.0 * (q.y * q.z + q.w * q.x)
            var up = Vec3(up_x, up_y, up_z)

            var local_offset = hit_world - panel.position
            var local_x = local_offset.dot(right)
            var local_y = local_offset.dot(up)

            # Check if within panel bounds (centered at origin, half-extents)
            var half_w = panel.width_m * 0.5
            var half_h = panel.height_m * 0.5

            if local_x < -half_w or local_x > half_w:
                continue
            if local_y < -half_h or local_y > half_h:
                continue

            # Convert to UV coordinates (0,0 = top-left, 1,1 = bottom-right)
            var u = (local_x + half_w) / panel.width_m
            var v = (
                1.0 - (local_y + half_h) / panel.height_m
            )  # Flip Y: screen Y is top-down

            closest_hit = RaycastHit(
                panel_index=i,
                panel_id=panel.panel_id,
                u=u,
                v=v,
                distance=t,
            )
            closest_distance = t

        return closest_hit

    # ── Spatial layout helpers ────────────────────────────────────────

    fn arrange_arc(
        mut self,
        indices: List[Int],
        radius: Float32 = DEFAULT_ARC_RADIUS,
        height: Float32 = DEFAULT_ARC_HEIGHT,
        gap_deg: Float32 = DEFAULT_ARC_GAP_DEG,
    ):
        """Arrange panels in a horizontal arc facing the user.

        Places the given panels in a semicircular arc centered on the
        user's forward direction (-Z). Each panel is rotated to face
        the center of the arc (i.e., the user's head position).

        The arc is centered horizontally (panel 0 at the center if count
        is odd, or straddling center if even). All panels are placed at
        the same height.

        This is the most common XR panel layout — it provides comfortable
        viewing angles across all panels without head rotation exceeding
        ~60° in either direction.

        Args:
            indices: List of panel indices to arrange.
            radius: Distance from the user to each panel center, in meters.
            height: Y-axis position (height) of all panels, in meters.
            gap_deg: Angular gap between adjacent panels, in degrees.
        """
        var count = len(indices)
        if count == 0:
            return

        alias DEG_TO_RAD: Float32 = 3.14159265358979323846 / 180.0

        for i in range(count):
            var idx = indices[i]
            if idx < 0 or idx >= len(self.panels):
                continue

            var panel = self.panels[idx]

            # Calculate the angular span of this panel at the given radius
            var panel_angle_deg = Float32(panel.width_m / radius / DEG_TO_RAD)

            # Total span per slot = panel angular width + gap
            var slot_deg = panel_angle_deg + gap_deg

            # Center the arrangement: offset from center for this panel
            var center_offset = Float32(i) - Float32(count - 1) * 0.5
            var angle_deg = center_offset * slot_deg
            var angle_rad = angle_deg * DEG_TO_RAD

            # Position on the arc (XZ plane, Y = height)
            var x = sin(angle_rad) * radius
            var z = -cos(angle_rad) * radius

            self.panels[idx].set_position(x, height, z)

            # Rotate to face the center (yaw only)
            self.panels[idx].set_rotation_euler(0.0, -angle_deg, 0.0)

    fn arrange_grid(
        mut self,
        indices: List[Int],
        columns: Int,
        spacing_x: Float32 = 0.05,
        spacing_y: Float32 = 0.05,
        center: Vec3 = Vec3(0.0, 1.4, -1.0),
    ):
        """Arrange panels in a flat grid facing the user.

        Places panels in a regular grid pattern with the specified number
        of columns. Rows are filled left-to-right, top-to-bottom. The
        grid is centered on the given center position.

        All panels face directly toward +Z (identity rotation), which is
        appropriate for panels at moderate distance. For wide grids,
        consider arrange_arc() instead.

        Args:
            indices: List of panel indices to arrange.
            columns: Number of columns in the grid (must be >= 1).
            spacing_x: Horizontal gap between adjacent panels, in meters.
            spacing_y: Vertical gap between adjacent panels, in meters.
            center: World-space position of the grid center.
        """
        var count = len(indices)
        if count == 0 or columns < 1:
            return

        var rows = (count + columns - 1) // columns

        for i in range(count):
            var idx = indices[i]
            if idx < 0 or idx >= len(self.panels):
                continue

            var col = i % columns
            var row = i // columns
            var panel = self.panels[idx]

            # Calculate total grid dimensions for centering
            var cell_w = panel.width_m + spacing_x
            var cell_h = panel.height_m + spacing_y
            var grid_w = Float32(columns) * cell_w - spacing_x
            var grid_h = Float32(rows) * cell_h - spacing_y

            var x = (
                center.x
                + (Float32(col) * cell_w)
                - grid_w * 0.5
                + panel.width_m * 0.5
            )
            var y = (
                center.y
                + grid_h * 0.5
                - (Float32(row) * cell_h)
                - panel.height_m * 0.5
            )
            var z = center.z

            self.panels[idx].set_position(x, y, z)
            self.panels[idx].set_rotation(Quaternion.identity())

    fn arrange_stack(
        mut self,
        indices: List[Int],
        spacing: Float32 = 0.05,
        center: Vec3 = Vec3(0.0, 1.4, -1.0),
    ):
        """Arrange panels in a vertical stack centered on the given position.

        Panels are stacked top-to-bottom, centered vertically around the
        given center height.

        Args:
            indices: List of panel indices to arrange (top to bottom).
            spacing: Vertical gap between adjacent panels, in meters.
            center: World-space position of the stack center.
        """
        var count = len(indices)
        if count == 0:
            return

        # Calculate total height for centering
        var total_height: Float32 = 0.0
        for i in range(count):
            var idx = indices[i]
            if idx >= 0 and idx < len(self.panels):
                total_height += self.panels[idx].height_m
        total_height += Float32(count - 1) * spacing

        var current_y = center.y + total_height * 0.5

        for i in range(count):
            var idx = indices[i]
            if idx < 0 or idx >= len(self.panels):
                continue

            var panel = self.panels[idx]
            current_y -= panel.height_m * 0.5

            self.panels[idx].set_position(center.x, current_y, center.z)
            self.panels[idx].set_rotation(Quaternion.identity())

            current_y -= panel.height_m * 0.5 + spacing

    # ── Visibility helpers ────────────────────────────────────────────

    fn visible_panel_indices(self) -> List[Int]:
        """Return a list of indices of all visible panels.

        Used by the XR frame loop to determine which panels to submit
        as quad layers to the OpenXR compositor.
        """
        var result = List[Int]()
        for i in range(len(self.panels)):
            if self.panels[i].is_visible():
                result.append(i)
        return result

    fn visible_panel_count(self) -> Int:
        """Return the number of visible panels in the scene."""
        var count = 0
        for i in range(len(self.panels)):
            if self.panels[i].is_visible():
                count += 1
        return count

    fn show_all(mut self):
        """Make all panels visible."""
        for i in range(len(self.panels)):
            self.panels[i].show()

    fn hide_all(mut self):
        """Hide all panels."""
        for i in range(len(self.panels)):
            self.panels[i].hide()
        self.clear_focus()

    # ── Session state ─────────────────────────────────────────────────

    fn is_active(self) -> Bool:
        """Return True if the XR session is active."""
        return self.active and not self.destroyed

    fn set_active(mut self, active: Bool):
        """Set the XR session active state.

        When the OpenXR session transitions to STOPPING or IDLE, set
        active=False to pause rendering. Set active=True when the session
        returns to FOCUSED or VISIBLE.

        Args:
            active: Whether the session is active.
        """
        self.active = active

    # ── Cleanup ───────────────────────────────────────────────────────

    fn destroy(mut self):
        """Destroy all panels and release scene resources.

        Called once when the XR session ends. After this call, the scene
        is in an invalid state and must not be used.

        TODO(Step 5.2): Call mxr_destroy_panel() for each panel to free
        shim-side Blitz documents and GPU textures.
        """
        # Clear panel list (destructors run via List.__del__)
        self.panels.clear()
        self.focused_panel_index = -1
        self.hovered_panel_index = -1
        self.active = False
        self.destroyed = True


# ══════════════════════════════════════════════════════════════════════════════
# Convenience constructors — create pre-configured scenes
# ══════════════════════════════════════════════════════════════════════════════


fn create_single_panel_scene(
    config: PanelConfig = default_panel_config(),
) -> XRScene:
    """Create a scene with a single panel.

    This is the default scene for single-panel apps launched via
    xr_launch[AppType: GuiApp](). The app's GuiApp instance is mounted
    to this panel by the launcher.

    Args:
        config: Panel configuration. Default: default_panel_config().

    Returns:
        An XRScene containing one panel.
    """
    var scene = XRScene()
    _ = scene.create_panel(config)
    return scene^


fn create_dual_panel_scene(
    main_config: PanelConfig = default_panel_config(),
    side_config: PanelConfig = PanelConfig(
        width_m=0.4,
        height_m=0.6,
        pixels_per_meter=1200.0,
        position=Vec3(0.55, 1.4, -0.9),
    ),
) -> XRScene:
    """Create a scene with a main panel and a side panel.

    The main panel is centered in front of the user. The side panel is
    placed to the right, slightly angled inward. Both panels are
    positioned at comfortable viewing height.

    Args:
        main_config: Configuration for the main (center) panel.
        side_config: Configuration for the side (right) panel.

    Returns:
        An XRScene containing two panels (index 0 = main, 1 = side).
    """
    var scene = XRScene()
    _ = scene.create_panel(main_config)
    _ = scene.create_panel(side_config)

    # Angle the side panel slightly toward the user
    if len(scene.panels) > 1:
        scene.panels[1].set_rotation_euler(0.0, -15.0, 0.0)

    return scene^
